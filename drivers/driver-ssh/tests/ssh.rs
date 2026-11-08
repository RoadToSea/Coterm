use std::{sync::Arc, time::Duration};

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use coterm_driver_ssh::{
    backend::Backend,
    session::{self, Config, Connection, OutputBuffer},
};
use russh::{
    server::{self, Session},
    Channel, ChannelId, Pty,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
    time::{sleep, timeout},
};

#[tokio::test]
async fn process_host_routes_approval_snapshot_and_hides_terminal_input() -> Result<()> {
    use coterm_core::{
        driver_manager::ProcessDriverRuntime,
        driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation},
    };
    use coterm_proto::runtime::{
        lifecycle::RuntimeLimits,
        operation::{ApproveProposal, OperationSource, OperationStatus},
        stream::FlowWindow,
    };
    let server = server().await?;
    let directory = std::env::temp_dir().join(format!(
        "coterm-ssh-test-{}-{}",
        std::process::id(),
        server.config.port
    ));
    std::fs::create_dir_all(directory.join("backend"))?;
    std::fs::create_dir_all(directory.join("frontend"))?;
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_coterm-driver-ssh"));
    let destination = directory.join("backend").join(
        executable
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Missing executable name"))?,
    );
    std::fs::copy(executable, destination)?;
    std::fs::write(
        directory.join("manifest.json"),
        include_str!("../manifest.json"),
    )?;
    std::fs::write(
        directory.join("frontend/index.js"),
        "export function mount() {}",
    )?;
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64",
        ("windows", "aarch64") => "windows-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("macos", "aarch64") => "macos-arm64",
        _ => anyhow::bail!("Unsupported test platform"),
    };
    let runtime = ProcessDriverRuntime::new(
        platform,
        RuntimeLimits {
            max_control_bytes: 1024 * 1024,
            max_data_chunk_bytes: 64 * 1024,
            initial_window: FlowWindow {
                chunks: 16,
                bytes: 1024 * 1024,
            },
        },
    );
    runtime.load(&directory).await?;
    runtime.start("coterm-driver-ssh").await?;
    let instance = runtime
        .create_instance("coterm-driver-ssh", serde_json::to_value(&server.config)?)
        .await?;
    assert_eq!(
        instance.label,
        format!("tester@127.0.0.1:{}", server.config.port)
    );
    assert!(runtime
        .frontend_source("coterm-driver-ssh")
        .await?
        .is_some());
    let caller = |source| CallerContext {
        source,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let operation = |name: &str, args: Value| SubmitOperation {
        driver_id: instance.driver_id.clone(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: name.to_owned(),
        args,
        wait_timeout_ms: if name == "ssh.command" {
            Some(100)
        } else {
            None
        },
    };
    let driver_info = runtime
        .submit(
            CallerContext {
                source: OperationSource::Human,
                session_id: String::new(),
                driver_id: Some(instance.driver_id.clone()),
            },
            SubmitOperation {
                driver_id: instance.driver_id.clone(),
                driver_instance_id: None,
                operation: "ssh.driver_info".to_owned(),
                args: json!({}),
                wait_timeout_ms: None,
            },
        )
        .await?;
    assert_eq!(
        driver_info
            .output
            .as_ref()
            .map(|output| output["instanceCount"].as_u64()),
        Some(Some(1))
    );
    let denied = runtime
        .submit(
            CallerContext {
                source: OperationSource::Agent,
                session_id: String::new(),
                driver_id: Some("another.driver".to_owned()),
            },
            SubmitOperation {
                driver_id: instance.driver_id.clone(),
                driver_instance_id: None,
                operation: "ssh.driver_info".to_owned(),
                args: json!({}),
                wait_timeout_ms: None,
            },
        )
        .await
        .expect_err("driver scope must be bound to the trusted caller target");
    assert_eq!(denied.code, "driver_scope_access_denied");
    let invalid = runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.command", json!({"text": 7})),
        )
        .await
        .expect_err("invalid operation arguments must be rejected by the host");
    assert_eq!(invalid.code, "operation_args_invalid");
    let handle = runtime
        .provide_secret(
            &instance.driver_instance_id,
            "ssh.password".to_owned(),
            "local-test-password".to_owned(),
        )
        .await?;
    let receipt = runtime
        .submit(
            caller(OperationSource::Human),
            operation(
                "ssh.connect",
                json!({"fingerprint":server.fingerprint,"secret":handle}),
            ),
        )
        .await?;
    assert_eq!(receipt.status, OperationStatus::Completed);
    let proposal = runtime
        .submit(
            caller(OperationSource::Agent),
            operation("ssh.command", json!({"text":"original-command"})),
        )
        .await?;
    assert_eq!(proposal.status, OperationStatus::AwaitingApproval);
    let invalid_approval = runtime
        .approve(
            caller(OperationSource::Human),
            ApproveProposal {
                operation_id: proposal.operation_id.clone(),
                proposal_id: proposal
                    .proposal_id
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Missing proposal"))?,
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                args: json!({"text": false}),
            },
        )
        .await
        .expect_err("approval snapshot must be schema validated");
    assert_eq!(invalid_approval.code, "operation_args_invalid");
    let receipt = runtime
        .approve(
            caller(OperationSource::Human),
            ApproveProposal {
                operation_id: proposal.operation_id,
                proposal_id: proposal
                    .proposal_id
                    .ok_or_else(|| anyhow::anyhow!("Missing proposal"))?,
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                args: json!({"text":"approved-edit"}),
            },
        )
        .await?;
    assert_eq!(receipt.status, OperationStatus::Running);
    let typed = runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.input", json!({"data":STANDARD.encode(b"human-input")})),
        )
        .await?;
    assert_eq!(
        runtime
            .get(caller(OperationSource::Human), &typed.operation_id)
            .await?
            .input,
        json!({"redacted":true})
    );
    timeout(Duration::from_secs(3), async {
        loop {
            let read = runtime
                .submit(
                    caller(OperationSource::Human),
                    operation("ssh.read", json!({"cursor":"0"})),
                )
                .await?;
            let value = read
                .output
                .ok_or_else(|| anyhow::anyhow!("Missing read output"))?;
            let bytes = STANDARD.decode(value["data"].as_str().unwrap_or_default())?;
            if bytes.windows(11).any(|window| window == b"human-input") {
                assert!(bytes.windows(13).any(|window| window == b"approved-edit"));
                assert!(!bytes
                    .windows(16)
                    .any(|window| window == b"original-command"));
                assert!(!bytes.windows(9).any(|window| window == b"]777;co"));
                return Ok::<(), anyhow::Error>(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    let pending_command = runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.command", json!({"text":"never-completes"})),
        )
        .await?;
    assert_eq!(pending_command.status, OperationStatus::Running);
    let cancellation = runtime
        .cancel(
            caller(OperationSource::Human),
            &pending_command.operation_id,
        )
        .await?;
    assert_eq!(cancellation.status, OperationStatus::Unknown);
    assert_eq!(
        cancellation.error.as_ref().map(|error| error.code.as_str()),
        Some("operation_cancellation_unconfirmed")
    );
    runtime.stop("coterm-driver-ssh").await?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[derive(Default)]
struct Echo {
    scripts: std::collections::HashMap<ChannelId, Vec<u8>>,
    children: std::collections::HashMap<ChannelId, tokio::process::ChildStdin>,
    real_shell: bool,
}

fn start_test_child(
    mut child: tokio::process::Child,
    channel: ChannelId,
    session: &Session,
) -> Option<tokio::process::ChildStdin> {
    let input = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let handle = session.handle();
    tokio::spawn(async move {
        let out = async {
            if let Some(mut stdout) = stdout {
                let mut bytes = [0; 4096];
                while let Ok(count) = stdout.read(&mut bytes).await {
                    if count == 0 {
                        break;
                    }
                    if handle.data(channel, bytes[..count].to_vec()).await.is_err() {
                        break;
                    }
                }
            }
        };
        let err = async {
            if let Some(mut stderr) = stderr {
                let mut bytes = [0; 4096];
                while let Ok(count) = stderr.read(&mut bytes).await {
                    if count == 0 {
                        break;
                    }
                    if handle
                        .extended_data(channel, 1, bytes[..count].to_vec())
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        };
        let (status, _, _) = tokio::join!(child.wait(), out, err);
        if let Ok(status) = status {
            let _ = handle
                .exit_status_request(channel, status.code().unwrap_or(255) as u32)
                .await;
        }
        let _ = handle.close(channel).await;
    });
    input
}

impl server::Handler for Echo {
    type Error = russh::Error;
    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if user == "tester" && password == "local-test-password" {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }
    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        session.data(channel, b"test shell ready\r\n".to_vec())?;
        Ok(())
    }
    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let source = String::from_utf8_lossy(data);
        if self.real_shell {
            session.channel_success(channel)?;
            // Test-only sudo policy: consume one password line, refuse wrong input,
            // then run the actual POSIX script without requiring administrator rights.
            let sudo = if source.contains("NOPASSWD-fixture") {
                "sudo() { shift 5; \"$@\"; }; "
            } else {
                "sudo() { __prompt=$4; shift 5; printf '%s' \"$__prompt\" >&2; IFS= read -r __pw; [ \"$__pw\" = local-sudo-password ] || return 1; unset __pw; \"$@\"; }; "
            };
            let command = if source.starts_with("sudo ") {
                format!("{sudo}{source}")
            } else {
                source.to_string()
            };
            let child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| russh::Error::Disconnect)?;
            let input = start_test_child(child, channel, session);
            if source.contains("__coterm_script=$(cat)")
                || source.contains("cat >")
                || source.starts_with("sudo ")
            {
                if let Some(input) = input {
                    self.children.insert(channel, input);
                }
            }
            return Ok(());
        }
        if source.contains("__coterm_script=$(cat)") {
            self.scripts.insert(channel, Vec::new());
            session.channel_success(channel)?;
            return Ok(());
        }
        // Read-only queries and exec commands run outside the shared PTY.
        if source.starts_with("printf 'user=") {
            session.data(
                channel,
                b"user=tester
uid=1000
os=TestOS
"
                .to_vec(),
            )?;
        } else if let Some(rest) = source.strip_prefix("__p='") {
            let target = rest
                .trim_start_matches(char::from(39))
                .split(char::from(39))
                .next()
                .unwrap_or("");
            if target.ends_with("/exists") {
                session.data(
                    channel,
                    format!(
                        "exists=1
type=regular file
size=11
path={target}
"
                    )
                    .into_bytes(),
                )?;
            } else {
                session.data(
                    channel,
                    format!(
                        "exists=0
path={target}
"
                    )
                    .into_bytes(),
                )?;
            }
        } else if source.starts_with("cat < '") {
            session.data(channel, b"alpha=1\nbeta=two\n".to_vec())?;
            session.exit_status_request(channel, 0)?;
        } else {
            session.data(channel, data.to_vec())?;
            if source.contains("false") {
                session.exit_status_request(channel, 1)?;
            } else {
                session.exit_status_request(channel, 0)?;
            }
        }
        session.close(channel)?;
        Ok(())
    }
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(input) = self.children.get_mut(&channel) {
            input
                .write_all(data)
                .await
                .map_err(|_| russh::Error::Disconnect)?;
            return Ok(());
        }
        if let Some(script) = self.scripts.get_mut(&channel) {
            script.extend_from_slice(data);
            return Ok(());
        }
        let source = String::from_utf8_lossy(data);
        if let Some(rest) = source.strip_prefix("printf '\\033]777;coterm;") {
            let token = rest.split(';').next().ok_or(russh::Error::Disconnect)?;
            // Current wrapper: __coterm_src='...'; ... eval "$__coterm_src" ...
            let text = source
                .split("__coterm_src='")
                .nth(1)
                .and_then(|s| s.split("'; __coterm_flag=").next())
                .unwrap_or("")
                .replace("'\''", "'");
            let begin = format!("\x1b]777;coterm;{token};begin\x07");
            session.data(channel, begin.into_bytes())?;
            session.data(channel, format!("{text}\r\n").into_bytes())?;
            if text.contains("separated-stderr") {
                session.extended_data(channel, 1, b"separate diagnostic\n".to_vec())?;
            }
            let (code, flag) = if text.trim() == "if" {
                (2, "syntax")
            } else if text.contains("false") {
                (1, "")
            } else {
                (0, "")
            };
            if flag == "syntax" {
                session.data(
                    channel,
                    b"bash: syntax error: unexpected end of file\r\n".to_vec(),
                )?;
            }
            let end = format!("\x1b]777;coterm;{token};end;{code};{flag}\x07");
            if !text.contains("never-completes") {
                session.data(channel, end.into_bytes())?;
            }
        } else if data == b"\r" {
            // A human Enter on the fixture PTY produces an ordinary unmarked reply.
            session.data(channel, b"\r\nfile.txt\r\nshell$ ".to_vec())?;
        } else {
            session.data(channel, data.to_vec())?;
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(input) = self.children.remove(&channel) {
            drop(input);
        } else if let Some(script) = self.scripts.remove(&channel) {
            let code = u32::from(String::from_utf8_lossy(&script).contains("false"));
            session.data(channel, script)?;
            session.exit_status_request(channel, code)?;
            session.close(channel)?;
        }
        Ok(())
    }
}

struct TestServer {
    config: Config,
    fingerprint: String,
    task: JoinHandle<()>,
}
impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server() -> Result<TestServer> {
    server_mode(false).await
}

async fn server_mode(real_shell: bool) -> Result<TestServer> {
    let key = russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)?;
    let fingerprint = key
        .public_key()
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::from_millis(5),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let config = config.clone();
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(
                    config,
                    socket,
                    Echo {
                        real_shell,
                        ..Default::default()
                    },
                )
                .await
                {
                    let _ = session.await;
                }
            });
        }
    });
    Ok(TestServer {
        config: Config {
            host: "127.0.0.1".to_owned(),
            port,
            username: "tester".to_owned(),
        },
        fingerprint,
        task,
    })
}

async fn read_until(connection: &Connection, needle: &[u8]) -> Result<Vec<u8>> {
    timeout(Duration::from_secs(3), async {
        loop {
            let output = connection.output.lock().await.read(0)?;
            let data = STANDARD.decode(output["data"].as_str().unwrap_or_default())?;
            if data.windows(needle.len()).any(|window| window == needle) {
                return Ok(data);
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

#[tokio::test]
async fn verifies_host_and_password_and_shares_interactive_bytes() -> Result<()> {
    let server = server().await?;
    assert_eq!(session::probe(&server.config).await?, server.fingerprint);
    assert!(Connection::connect(
        &server.config,
        "SHA256:wrong".to_owned(),
        "local-test-password"
    )
    .await
    .is_err());
    assert!(
        Connection::connect(&server.config, server.fingerprint.clone(), "wrong-password")
            .await
            .is_err()
    );
    let first = Connection::connect(
        &server.config,
        server.fingerprint.clone(),
        "local-test-password",
    )
    .await?;
    let second = Connection::connect(
        &server.config,
        server.fingerprint.clone(),
        "local-test-password",
    )
    .await?;
    first.input(b"echo first\r".to_vec()).await?;
    let output = read_until(&first, b"echo first").await?;
    assert!(output.starts_with(b"test shell ready"));
    let other = read_until(&second, b"test shell ready").await?;
    assert!(!other.windows(10).any(|window| window == b"echo first"));
    first.resize(120, 35).await?;
    Ok(())
}

#[test]
fn output_is_bounded_and_reports_replay_gaps() -> Result<()> {
    let mut output = OutputBuffer::default();
    output.push(&vec![0xff; 512 * 1024]);
    let first = output.read(0)?;
    assert_eq!(first["gap"], true);
    assert!(
        STANDARD
            .decode(first["data"].as_str().unwrap_or_default())?
            .len()
            <= 32 * 1024
    );
    let cursor: u64 = first["cursor"].as_str().unwrap_or_default().parse()?;
    assert_eq!(output.read(cursor)?["gap"], false);
    assert!(output.read(u64::MAX).is_err());
    Ok(())
}

fn create(id: &str, config: &Config) -> Value {
    json!({"instance":{"driverId":"coterm-driver-ssh","driverInstanceId":id,"sessionId":format!("session-{id}"),"label":id,"status":"starting"},"config":config})
}

#[tokio::test]
async fn message_blocks_follow_approved_commands_and_remain_instance_bound() -> Result<()> {
    use coterm_core::driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation};
    use coterm_proto::runtime::{
        message::DriverNotification,
        operation::{ApproveProposal, OperationSource, OperationStatus},
    };
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "blocks").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let caller = |source| CallerContext {
        source,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let operation = |name: &str, args: Value| SubmitOperation {
        driver_id: instance.driver_id.clone(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: name.to_owned(),
        args,
        wait_timeout_ms: Some(3_000),
    };
    let mut events = runtime
        .subscribe_events(&instance.driver_instance_id)
        .await?;
    let proposal = runtime
        .submit(
            caller(OperationSource::Agent),
            operation("ssh.command", json!({"text":"unapproved-original"})),
        )
        .await?;
    assert_eq!(proposal.status, OperationStatus::AwaitingApproval);
    let empty = runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.blocks", json!({})),
        )
        .await?;
    assert_eq!(
        empty
            .output
            .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?["blocks"],
        json!([])
    );
    let approved = runtime
        .approve(
            caller(OperationSource::Human),
            ApproveProposal {
                operation_id: proposal.operation_id.clone(),
                proposal_id: proposal
                    .proposal_id
                    .ok_or_else(|| anyhow::anyhow!("missing proposal"))?,
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                args: json!({"text":"approved-edited"}),
            },
        )
        .await?;
    timeout(Duration::from_secs(3), async {
        loop {
            let record = runtime
                .get(caller(OperationSource::Human), &approved.operation_id)
                .await?;
            if record.status == OperationStatus::Completed {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    let exec = runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.command", json!({"text":"echo isolated","mode":"exec"})),
        )
        .await?;
    assert_eq!(exec.status, OperationStatus::Completed);
    // Password-like raw keys are not interpreted as a command or recorded as a block.
    runtime
        .submit(
            caller(OperationSource::Human),
            operation(
                "ssh.input",
                json!({"data":STANDARD.encode(b"private-input")}),
            ),
        )
        .await?;
    let result = runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.blocks", json!({})),
        )
        .await?;
    assert_eq!(result.status, OperationStatus::Completed);
    let snapshot = result
        .output
        .ok_or_else(|| anyhow::anyhow!("missing snapshot"))?;
    let blocks = snapshot["blocks"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing blocks"))?;
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["id"], exec.operation_id);
    assert_eq!(blocks[0]["channel"], "exec");
    assert_eq!(blocks[0]["output"], "echo isolated");
    assert_eq!(blocks[1]["command"], "approved-edited");
    assert_eq!(blocks[1]["actor"], "robot");
    assert_eq!(blocks[1]["output"], "approved-edited\n");
    assert_eq!(blocks[1]["state"], "completed");
    assert_eq!(blocks[1]["exitCode"], 0);
    // Agent context is an explicit human selection, never the entire block history.
    let unselected = runtime
        .submit(
            caller(OperationSource::Agent),
            operation("ssh.selected_blocks", json!({})),
        )
        .await?;
    assert_eq!(
        unselected
            .output
            .as_ref()
            .and_then(|value| value.get("blocks")),
        Some(&json!([]))
    );
    let chosen = runtime
        .submit(
            caller(OperationSource::Human),
            operation(
                "ssh.select_blocks",
                json!({"epoch":snapshot["epoch"],"ids":[approved.operation_id]}),
            ),
        )
        .await?;
    assert_eq!(chosen.status, OperationStatus::Completed);
    let selected = runtime
        .submit(
            caller(OperationSource::Agent),
            operation("ssh.selected_blocks", json!({})),
        )
        .await?;
    let selected = selected
        .output
        .ok_or_else(|| anyhow::anyhow!("missing selected context"))?;
    assert_eq!(selected["blocks"].as_array().map(Vec::len), Some(1));
    assert_eq!(selected["blocks"][0]["id"], approved.operation_id);
    let denied_selection = runtime
        .submit(
            caller(OperationSource::Agent),
            operation(
                "ssh.select_blocks",
                json!({"epoch":snapshot["epoch"],"ids":[]}),
            ),
        )
        .await;
    assert!(
        denied_selection.is_err()
            || denied_selection
                .as_ref()
                .is_ok_and(|receipt| receipt.status == OperationStatus::Failed)
    );
    let stale = runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.select_blocks", json!({"epoch":"stale","ids":[]})),
        )
        .await?;
    assert_eq!(stale.status, OperationStatus::Failed);
    let mut terminal_cursor = "0".to_owned();
    let mut boundaries = Vec::new();
    for _ in 0..256 {
        let page = runtime
            .submit(
                caller(OperationSource::Human),
                operation(
                    "ssh.read",
                    json!({"cursor":terminal_cursor,"terminal":true}),
                ),
            )
            .await?
            .output
            .ok_or_else(|| anyhow::anyhow!("missing terminal frame"))?;
        terminal_cursor = page["cursor"].as_str().unwrap_or("0").to_owned();
        if !page["annotation"].is_null() {
            boundaries.push(page["annotation"].clone());
        }
        if page["more"] == false {
            break;
        }
    }
    assert_eq!(boundaries.len(), 2);
    assert_eq!(boundaries[0]["kind"], "begin");
    assert_eq!(boundaries[0]["id"], approved.operation_id);
    assert_eq!(boundaries[0]["actor"], "robot");
    assert_eq!(boundaries[1]["kind"], "end");
    assert_eq!(
        runtime
            .submit(
                caller(OperationSource::Human),
                operation(
                    "ssh.select_blocks",
                    json!({"epoch":snapshot["epoch"],"ids":[]})
                )
            )
            .await?
            .status,
        OperationStatus::Completed
    );
    let encoded = serde_json::to_string(&snapshot)?;
    for hidden in [
        "unapproved-original",
        "private-input",
        "local-test-password",
        "__coterm_src",
        "777;coterm",
    ] {
        assert!(!encoded.contains(hidden), "unexpected block data: {hidden}");
    }
    let denied = runtime
        .submit(
            caller(OperationSource::Agent),
            operation("ssh.blocks", json!({})),
        )
        .await;
    assert!(
        denied.is_err()
            || denied
                .as_ref()
                .is_ok_and(|receipt| receipt.status == OperationStatus::Failed)
    );
    let other = runtime
        .create_instance("coterm-driver-ssh", serde_json::to_value(&ssh.config)?)
        .await?;
    let other_snapshot = runtime
        .submit(
            CallerContext {
                source: OperationSource::Human,
                session_id: other.session_id.clone(),
                driver_id: Some(other.driver_id.clone()),
            },
            SubmitOperation {
                driver_id: other.driver_id.clone(),
                driver_instance_id: Some(other.driver_instance_id.clone()),
                operation: "ssh.blocks".to_owned(),
                args: json!({}),
                wait_timeout_ms: None,
            },
        )
        .await?
        .output
        .ok_or_else(|| anyhow::anyhow!("missing other snapshot"))?;
    assert_eq!(other_snapshot["blocks"], json!([]));
    assert_ne!(snapshot["epoch"], other_snapshot["epoch"]);
    timeout(Duration::from_secs(3), async {
        loop {
            if let DriverNotification::Event { name, payload, .. } = events.recv().await? {
                if name == "ssh.blocks.changed" {
                    assert!(payload.get("command").is_none());
                    return Ok::<_, anyhow::Error>(());
                }
            }
        }
    })
    .await??;
    runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.disconnect", json!({})),
        )
        .await?;
    let retained = runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.blocks", json!({})),
        )
        .await?
        .output
        .ok_or_else(|| anyhow::anyhow!("missing retained snapshot"))?;
    assert_eq!(retained["blocks"], snapshot["blocks"]);
    connect_instance(&runtime, &instance, &ssh).await?;
    let reconnected = runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.blocks", json!({})),
        )
        .await?
        .output
        .ok_or_else(|| anyhow::anyhow!("missing new snapshot"))?;
    assert_ne!(reconnected["epoch"], snapshot["epoch"]);
    assert_eq!(reconnected["blocks"], json!([]));
    runtime.stop("coterm-driver-ssh").await?;
    Ok(())
}

#[tokio::test]
async fn human_enter_observations_reach_selected_agent_context_without_recording_raw_keys(
) -> Result<()> {
    use coterm_core::driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation};
    use coterm_proto::runtime::operation::{OperationSource, OperationStatus};
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "human-blocks").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let caller = |source| CallerContext {
        source,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let operation = |name: &str, args: Value| SubmitOperation {
        driver_id: instance.driver_id.clone(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: name.to_owned(),
        args,
        wait_timeout_ms: Some(3000),
    };
    let status = runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.status", json!({})),
        )
        .await?
        .output
        .ok_or_else(|| anyhow::anyhow!("missing status"))?;
    let connection_id = &status["connectionId"];
    // A stale frontend or malformed observation cannot submit to the current shell.
    for args in [
        json!({"data":STANDARD.encode(b"\r"),"connectionId":"stale","observation":{"text":"ls","lineOffset":0}}),
        json!({"data":STANDARD.encode(b"x"),"connectionId":connection_id,"observation":{"text":"ls","lineOffset":0}}),
        json!({"data":STANDARD.encode(b"\r"),"connectionId":connection_id,"observation":{"text":"ls\nsecret","lineOffset":0}}),
        json!({"data":STANDARD.encode(b"\r"),"observation":{"text":"ls","lineOffset":0}}),
    ] {
        let receipt = runtime
            .submit(caller(OperationSource::Human), operation("ssh.input", args))
            .await?;
        assert_eq!(receipt.status, OperationStatus::Failed);
    }
    let input = json!({"data":STANDARD.encode(b"\r"),"connectionId":connection_id,"observation":{"text":"ls","lineOffset":-1}});
    // The raw-input gate applies even when Agent bypass is on.
    runtime
        .set_bypass(&instance.driver_instance_id, true)
        .await?;
    let denied = runtime
        .submit(
            caller(OperationSource::Agent),
            operation("ssh.input", input.clone()),
        )
        .await;
    assert!(denied.is_err() || denied.is_ok_and(|r| r.status == OperationStatus::Failed));
    let receipt = runtime
        .submit(
            caller(OperationSource::Human),
            operation("ssh.input", input),
        )
        .await?;
    assert_eq!(receipt.status, OperationStatus::Completed);
    let snapshot = timeout(Duration::from_secs(3), async {
        loop {
            let snapshot = runtime
                .submit(
                    caller(OperationSource::Human),
                    operation("ssh.blocks", json!({})),
                )
                .await?
                .output
                .ok_or_else(|| anyhow::anyhow!("missing block snapshot"))?;
            if snapshot["blocks"][0]["output"]
                .as_str()
                .is_some_and(|s| s.contains("file.txt"))
            {
                return Ok::<_, anyhow::Error>(snapshot);
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    assert_eq!(snapshot["blocks"].as_array().map(Vec::len), Some(1));
    let block = &snapshot["blocks"][0];
    assert_eq!(block["command"], "ls");
    assert_eq!(block["actor"], "human");
    assert_eq!(block["boundary"], "observed");
    assert_eq!(block["exitCode"], Value::Null);
    let unselected = runtime
        .submit(
            caller(OperationSource::Agent),
            operation("ssh.selected_blocks", json!({})),
        )
        .await?;
    assert_eq!(
        unselected.output.as_ref().map(|v| &v["blocks"]),
        Some(&json!([]))
    );
    runtime
        .submit(
            caller(OperationSource::Human),
            operation(
                "ssh.select_blocks",
                json!({"epoch":snapshot["epoch"],"ids":[block["id"]]}),
            ),
        )
        .await?;
    // The next raw input closes the observation before its echo can enter context.
    runtime
        .submit(
            caller(OperationSource::Human),
            operation(
                "ssh.input",
                json!({"data":STANDARD.encode(b"private-not-context")}),
            ),
        )
        .await?;
    let selected = runtime
        .submit(
            caller(OperationSource::Agent),
            operation("ssh.selected_blocks", json!({})),
        )
        .await?
        .output
        .ok_or_else(|| anyhow::anyhow!("missing selected context"))?;
    let chosen = &selected["blocks"][0];
    assert_eq!(chosen["command"], "ls");
    assert_eq!(chosen["state"], "unknown");
    assert!(chosen["output"]
        .as_str()
        .is_some_and(|s| s.contains("file.txt") && !s.contains("private-not-context")));
    // Boundaries are retained for replay, without putting command text in events.
    let mut cursor = "0".to_owned();
    let mut boundaries = Vec::new();
    for _ in 0..64 {
        let frame = runtime
            .submit(
                caller(OperationSource::Human),
                operation("ssh.read", json!({"cursor":cursor,"terminal":true})),
            )
            .await?
            .output
            .ok_or_else(|| anyhow::anyhow!("missing terminal frame"))?;
        if frame["annotation"].is_object() {
            boundaries.push(frame["annotation"].clone());
        }
        cursor = frame["cursor"].as_str().unwrap_or("0").to_owned();
        if frame["more"] == false {
            break;
        }
    }
    assert_eq!(boundaries.len(), 2);
    assert_eq!(boundaries[0]["boundary"], "observed");
    assert_eq!(boundaries[0]["lineOffset"], -1);
    assert_eq!(boundaries[1]["kind"], "end");
    assert!(boundaries
        .iter()
        .all(|b| b.get("text").is_none() && b.get("command").is_none()));
    let recent = runtime
        .recent(
            caller(OperationSource::Human),
            &instance.driver_instance_id,
            100,
        )
        .await?;
    assert!(recent.iter().all(|r| r.operation != "ssh.input"));
    runtime.stop("coterm-driver-ssh").await?;
    Ok(())
}

#[tokio::test]
async fn proposals_never_send_until_approved_and_secrets_are_instance_bound() -> Result<()> {
    let server = server().await?;
    let mut backend = Backend::default();
    backend
        .request("create_instance", create("one", &server.config))
        .await?;
    backend
        .request("create_instance", create("two", &server.config))
        .await?;
    let expires = Utc::now() + chrono::Duration::seconds(30);
    let handle = json!({"id":"test-secret","driverInstanceId":"one","purpose":"ssh.password","expiresAt":expires});
    backend
        .request(
            "provide_secret",
            json!({"handle":handle,"value":"local-test-password"}),
        )
        .await?;
    let invoke = |id: &str,
                  operation: &str,
                  effect: &str,
                  args: Value,
                  source: &str,
                  dispatch: Value| json!({"operationId":"op","driverInstanceId":id,"source":source,"effect":effect,"operation":operation,"args":args,"dispatch":dispatch});
    let connect_args = json!({"fingerprint":server.fingerprint,"secret":handle});
    assert!(backend
        .request(
            "invoke",
            invoke(
                "two",
                "ssh.connect",
                "commit",
                connect_args.clone(),
                "human",
                json!({"mode":"execute"})
            )
        )
        .await
        .is_err());
    backend
        .request(
            "invoke",
            invoke(
                "one",
                "ssh.connect",
                "commit",
                connect_args.clone(),
                "human",
                json!({"mode":"execute"}),
            ),
        )
        .await?;
    // A short-lived password reference can never be replayed after consumption.
    backend
        .request(
            "invoke",
            invoke(
                "one",
                "ssh.disconnect",
                "commit",
                json!({}),
                "human",
                json!({"mode":"execute"}),
            ),
        )
        .await?;
    assert!(backend
        .request(
            "invoke",
            invoke(
                "one",
                "ssh.connect",
                "commit",
                connect_args,
                "human",
                json!({"mode":"execute"})
            )
        )
        .await
        .is_err());
    let prepared = backend
        .request(
            "invoke",
            invoke(
                "two",
                "ssh.command",
                "commit",
                json!({"text":"dangerous"}),
                "agent",
                json!({"mode":"prepare_proposal","proposalId":"p"}),
            ),
        )
        .await?;
    // Preparing works while disconnected: no shell write can have occurred.
    assert_eq!(prepared["args"]["text"], "dangerous");
    backend
        .request("reject", json!({"proposalId":"p","operationId":"op"}))
        .await?;
    assert!(backend.request("approve",json!({"proposalId":"p","operationId":"op","driverInstanceId":"two","args":{"text":"modified"}})).await.is_err());
    assert!(backend
        .request(
            "invoke",
            invoke(
                "one",
                "ssh.input",
                "commit",
                json!({"data":"c2VjcmV0"}),
                "agent",
                json!({"mode":"execute"})
            )
        )
        .await
        .is_err());
    Ok(())
}

struct McpWire {
    reader: tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
    next_id: u64,
    instance_names: std::collections::HashMap<String, String>,
}

impl McpWire {
    async fn connect(
        info: coterm_mcp::server::McpConnectionInfo,
        instances: &[coterm_proto::runtime::lifecycle::InstanceInfo],
    ) -> Result<Self> {
        use coterm_mcp::transport::write_frame;
        let socket = tokio::net::TcpStream::connect(info.address).await?;
        let (reader, writer) = socket.into_split();
        let mut client = Self {
            reader: tokio::io::BufReader::new(reader),
            writer,
            next_id: 0,
            instance_names: instances
                .iter()
                .map(|instance| (instance.driver_instance_id.clone(), instance.label.clone()))
                .collect(),
        };
        client.rpc("initialize", json!({"protocolVersion":"2025-06-18","clientInfo":{"name":"ssh-test","version":"1"},"capabilities":{}})).await?;
        write_frame(
            &mut client.writer,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await?;
        Ok(client)
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        use coterm_mcp::transport::{read_frame, write_frame};
        self.next_id += 1;
        write_frame(
            &mut self.writer,
            &json!({"jsonrpc":"2.0","id":self.next_id,"method":method,"params":params}),
        )
        .await?;
        let frame = timeout(Duration::from_secs(10), read_frame(&mut self.reader))
            .await??
            .ok_or_else(|| anyhow::anyhow!("MCP disconnected"))?;
        let response: Value = serde_json::from_slice(&frame)?;
        assert_eq!(response["id"], self.next_id);
        if response.get("error").is_some() {
            anyhow::bail!("MCP error: {}", response["error"]);
        }
        Ok(response["result"].clone())
    }

    async fn tool(&mut self, name: &str, args: Value) -> Result<Value> {
        let mut args = args;
        if let Some(object) = args.as_object_mut() {
            if let Some(instance_id) = object
                .remove("driverInstanceId")
                .and_then(|id| id.as_str().map(str::to_owned))
            {
                let instance_name = self
                    .instance_names
                    .get(&instance_id)
                    .ok_or_else(|| anyhow::anyhow!("Unknown test instance"))?;
                object.insert("name".to_owned(), json!(instance_name));
            }
        }
        self.rpc(
            "tools/call",
            json!({"name":name,"arguments":args,"_meta":{"coterm/waitTimeoutMs":100}}),
        )
        .await
    }
}

#[tokio::test]
async fn external_mcp_uses_shared_ssh_and_host_secret_and_approval_paths() -> Result<()> {
    use coterm_core::{
        driver_manager::ProcessDriverRuntime,
        driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation},
    };
    use coterm_proto::runtime::{
        lifecycle::RuntimeLimits,
        operation::{ApproveProposal, OperationSource, OperationStatus},
        stream::FlowWindow,
    };
    let ssh = server().await?;
    let directory = std::env::temp_dir().join(format!(
        "coterm-mcp-ssh-{}-{}",
        std::process::id(),
        ssh.config.port
    ));
    std::fs::create_dir_all(directory.join("backend"))?;
    std::fs::create_dir_all(directory.join("frontend"))?;
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_coterm-driver-ssh"));
    std::fs::copy(
        executable,
        directory.join("backend").join(
            executable
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Missing binary name"))?,
        ),
    )?;
    std::fs::write(
        directory.join("manifest.json"),
        include_str!("../manifest.json"),
    )?;
    std::fs::write(
        directory.join("frontend/index.js"),
        "export function mount() {}",
    )?;
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else {
        std::env::consts::OS
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let runtime = Arc::new(ProcessDriverRuntime::new(
        format!("{os}-{arch}"),
        RuntimeLimits {
            max_control_bytes: 1024 * 1024,
            max_data_chunk_bytes: 8192,
            initial_window: FlowWindow {
                chunks: 16,
                bytes: 1024 * 1024,
            },
        },
    ));
    runtime.load(&directory).await?;
    runtime.start("coterm-driver-ssh").await?;
    let instance = runtime
        .create_instance("coterm-driver-ssh", serde_json::to_value(&ssh.config)?)
        .await?;
    let other = runtime
        .create_instance("coterm-driver-ssh", serde_json::to_value(&ssh.config)?)
        .await?;
    let bridge =
        coterm_mcp::server::BridgeServer::bind_at(runtime.clone(), "127.0.0.1:0".parse()?).await?;
    let mut client =
        McpWire::connect(bridge.connection_info(), &[instance.clone(), other.clone()]).await?;
    let tools = client.rpc("tools/list", json!({})).await?;
    let tools = tools["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing tools"))?;
    assert!(tools.iter().any(|t| t["name"] == "coterm_ssh_command"));
    assert!(tools.iter().any(|t| t["name"] == "coterm_ssh_driver_info"));
    for name in ["coterm_ssh_read_file", "coterm_ssh_write_file"] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .ok_or_else(|| anyhow::anyhow!("Missing published file tool: {name}"))?;
        assert_eq!(tool["inputSchema"]["properties"]["sudo"]["type"], "boolean");
        assert_eq!(tool["inputSchema"]["properties"]["name"]["type"], "string");
        assert!(tool["inputSchema"]["properties"]
            .get("driverInstanceId")
            .is_none());
    }
    assert!(!tools.iter().any(|t| t["name"] == "coterm_ssh_input"));
    let driver_info = client.tool("coterm_ssh_driver_info", json!({})).await?;
    let driver_info = &driver_info["structuredContent"];
    assert_eq!(driver_info["instanceCount"], 2);
    let connect_args =
        json!({"driverInstanceId":instance.driver_instance_id,"fingerprint":ssh.fingerprint});
    let proposed = client
        .tool("coterm_ssh_connect", connect_args.clone())
        .await?;
    let receipt = &proposed["_meta"]["coterm/operation"];
    assert_eq!(receipt["status"], "awaiting_approval");
    assert!(runtime.pending_interactions().await.is_empty());
    let connect_id = receipt["operationId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing operation"))?
        .to_owned();
    let proposal_id = receipt["proposalId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing proposal"))?
        .to_owned();
    let human = || CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let approved = runtime
        .approve(
            human(),
            ApproveProposal {
                operation_id: connect_id.clone(),
                proposal_id,
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                args: connect_args,
            },
        )
        .await?;
    assert_eq!(approved.status, OperationStatus::Running);
    let requests = runtime.pending_interactions().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].driver_instance_id, instance.driver_instance_id);
    let status = client
        .tool(
            "coterm_ssh_status",
            json!({"driverInstanceId":instance.driver_instance_id}),
        )
        .await?;
    let domain = &status["structuredContent"];
    assert_eq!(domain["connected"], false);
    runtime
        .respond_secret(
            &requests[0].interaction_id,
            Some("local-test-password".to_owned()),
        )
        .await?;
    assert!(runtime
        .respond_secret(
            &requests[0].interaction_id,
            Some("late-password".to_owned())
        )
        .await
        .is_err());
    let completed = client
        .tool(
            "coterm.get_operation",
            json!({"driverInstanceId":instance.driver_instance_id,"operationId":connect_id}),
        )
        .await?;
    assert_eq!(completed["structuredContent"]["status"], "completed");
    assert!(!completed.to_string().contains("local-test-password"));
    let mismatched = client
        .tool(
            "coterm.get_operation",
            json!({"driverInstanceId":other.driver_instance_id,"operationId":connect_id}),
        )
        .await?;
    assert_eq!(mismatched["isError"], true);
    // Approval is immediately visible to the Agent. A second call on the SAME
    // connection remains usable, and the original operation is queried after approval.
    let mut pushed = runtime
        .subscribe_events(&instance.driver_instance_id)
        .await?;
    client.next_id += 1;
    let waiting_id = client.next_id;
    coterm_mcp::transport::write_frame(&mut client.writer, &json!({
        "jsonrpc":"2.0","id":waiting_id,"method":"tools/call","params":{
            "name":"coterm_ssh_command","arguments":{"name":instance.label,"commands":["original-waiting-command"]}
        }
    })).await?;
    let pending = timeout(
        Duration::from_millis(500),
        coterm_mcp::transport::read_frame(&mut client.reader),
    )
    .await??
    .ok_or_else(|| anyhow::anyhow!("No approval receipt"))?;
    let pending: Value = serde_json::from_slice(&pending)?;
    assert_eq!(pending["id"], waiting_id);
    assert_eq!(
        pending["result"]["structuredContent"]["status"],
        "pending_approval"
    );
    let status = client
        .tool(
            "coterm_ssh_status",
            json!({"driverInstanceId":instance.driver_instance_id}),
        )
        .await?;
    assert_eq!(status["isError"], false);
    let pending_record = runtime
        .recent(human(), &instance.driver_instance_id, 100)
        .await?
        .into_iter()
        .find(|r| r.status == OperationStatus::AwaitingApproval)
        .ok_or_else(|| anyhow::anyhow!("No awaiting command"))?;
    let result_id = pending_record.operation_id.clone();
    runtime.approve(human(), ApproveProposal {
        operation_id:result_id.clone(), proposal_id:pending_record.proposal_id.ok_or_else(|| anyhow::anyhow!("No proposal"))?,
        driver_instance_id:Some(instance.driver_instance_id.clone()),
        args:json!({"driverInstanceId":instance.driver_instance_id,"commands":["approved-default-wait false"]}),
    }).await?;
    wait_for_test_operation(&runtime, human(), &result_id).await?;
    let response = client
        .tool(
            "coterm.get_operation",
            json!({"name":instance.label,"operationId":result_id}),
        )
        .await?;
    assert_eq!(response["structuredContent"]["status"], "completed");
    let output = &response["structuredContent"]["output"];
    let domain = &output["structuredContent"];
    assert_eq!(domain["exitCode"], 1);
    assert_eq!(output["isError"], true);
    assert!(domain["text"]
        .as_str()
        .is_some_and(|text| text.contains("approved-default-wait false")));
    // A second MCP client gets its own approval receipt for the same SSH instance.
    // Closing that client must never approve, cancel, or replay a write.
    let second_bridge =
        coterm_mcp::server::BridgeServer::bind_at(runtime.clone(), "127.0.0.1:0".parse()?).await?;
    let mut second = McpWire::connect(
        second_bridge.connection_info(),
        std::slice::from_ref(&instance),
    )
    .await?;
    second.next_id += 1;
    let second_waiting_id = second.next_id;
    coterm_mcp::transport::write_frame(
        &mut second.writer,
        &json!({"jsonrpc":"2.0","id":second_waiting_id,"method":"tools/call","params":{
            "name":"coterm_ssh_command",
            "arguments":{"name":instance.label,"commands":["second-client-command"]}
        }}),
    )
    .await?;
    {
        let waiting = coterm_mcp::transport::read_frame(&mut second.reader);
        tokio::pin!(waiting);
        let frame = timeout(Duration::from_millis(500), &mut waiting)
            .await??
            .ok_or_else(|| anyhow::anyhow!("Missing second-client approval receipt"))?;
        let response: Value = serde_json::from_slice(&frame)?;
        assert_eq!(response["id"], second_waiting_id);
        assert_eq!(
            response["result"]["structuredContent"]["status"],
            "pending_approval"
        );
    }
    let status = second
        .tool(
            "coterm_ssh_status",
            json!({"driverInstanceId":instance.driver_instance_id}),
        )
        .await?;
    assert_eq!(status["isError"], false);
    let proposal = runtime
        .recent(human(), &instance.driver_instance_id, 100)
        .await?
        .into_iter()
        .find(|r| r.status == OperationStatus::AwaitingApproval)
        .ok_or_else(|| anyhow::anyhow!("No second-client approval proposal"))?;
    let second_operation = proposal.operation_id.clone();
    runtime
        .approve(
            human(),
            ApproveProposal {
                operation_id: proposal.operation_id,
                proposal_id: proposal
                    .proposal_id
                    .ok_or_else(|| anyhow::anyhow!("No proposal ID"))?,
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                args: json!({"driverInstanceId":instance.driver_instance_id,"commands":["second-human-approved false"]}),
            },
        )
        .await?;
    wait_for_test_operation(&runtime, human(), &second_operation).await?;
    let response = second
        .tool(
            "coterm.get_operation",
            json!({"name":instance.label,"operationId":second_operation}),
        )
        .await?;
    assert_eq!(response["structuredContent"]["status"], "completed");
    let output = &response["structuredContent"]["output"];
    assert_eq!(output["isError"], true);
    let domain = &output["structuredContent"];
    assert_eq!(domain["exitCode"], 1);
    assert!(domain["text"]
        .as_str()
        .is_some_and(|text| text.contains("second-human-approved false")));
    // An immediate receipt keeps the write pending; closing the client must not replay it.
    let pending = second
        .rpc(
            "tools/call",
            json!({
                "name":"coterm_ssh_command",
                "arguments":{"name":instance.label,"commands":["second-must-not-replay"]},
                "_meta":{"coterm/waitTimeoutMs":0}
            }),
        )
        .await?;
    let pending_id = pending["_meta"]["coterm/operation"]["operationId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No immediate receipt"))?
        .to_owned();
    assert_eq!(pending["_meta"]["coterm/operation"]["status"], "accepted");
    // Dropping a client closes only its own bridge connection.
    drop(second);
    let record = runtime.get(human(), &pending_id).await?;
    assert!(matches!(
        record.status,
        OperationStatus::Accepted | OperationStatus::AwaitingApproval
    ));
    runtime.cancel(human(), &pending_id).await?;
    drop(second_bridge);
    // Visible data is pushed without any ssh.read calls and contains no marker.
    let visible = timeout(Duration::from_secs(3), async {
        let mut visible = Vec::new();
        while let Ok(notification) = pushed.recv().await {
            if let coterm_proto::runtime::message::DriverNotification::Event {
                name, payload, ..
            } = notification
            {
                if name == "ssh.terminal.output" {
                    visible.extend(STANDARD.decode(payload["data"].as_str().unwrap_or_default())?);
                }
                if visible
                    .windows(b"approved-default-wait".len())
                    .any(|w| w == b"approved-default-wait")
                {
                    return Ok::<_, anyhow::Error>(visible);
                }
            }
        }
        anyhow::bail!("No pushed output")
    })
    .await??;
    assert!(!visible.contains(&27));
    // Repeated read-only requests are isolated from the recent user-action ledger.
    for _ in 0..125 {
        runtime
            .submit(
                human(),
                SubmitOperation {
                    driver_id: instance.driver_id.clone(),
                    driver_instance_id: Some(instance.driver_instance_id.clone()),
                    operation: "ssh.status".into(),
                    args: json!({}),
                    wait_timeout_ms: None,
                },
            )
            .await?;
    }
    assert_eq!(
        runtime.get(human(), &result_id).await?.status,
        OperationStatus::Completed
    );
    assert!(runtime
        .recent(human(), &instance.driver_instance_id, 100)
        .await?
        .iter()
        .all(|r| r.operation != "ssh.status"));
    let mut slow = runtime
        .subscribe_events(&instance.driver_instance_id)
        .await?;
    for _ in 0..80 {
        runtime
            .submit(
                human(),
                SubmitOperation {
                    driver_id: instance.driver_id.clone(),
                    driver_instance_id: Some(instance.driver_instance_id.clone()),
                    operation: "ssh.input".into(),
                    args: json!({"data":STANDARD.encode(b"z")}),
                    wait_timeout_ms: None,
                },
            )
            .await?;
    }
    timeout(Duration::from_secs(3), async {
        while slow.len() <= 64 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert!(matches!(
        slow.recv().await,
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
    ));
    assert_eq!(
        runtime.get(human(), &result_id).await?.status,
        OperationStatus::Completed
    );
    // Explicit zero waiting returns an accepted receipt; cancellation never sends it.
    let immediate = client
        .rpc(
            "tools/call",
            json!({"name":"coterm_ssh_command", "arguments":{
        "name":instance.label,"commands":["must-not-run"]},"_meta":{"coterm/waitTimeoutMs":0}}),
        )
        .await?;
    assert_eq!(immediate["_meta"]["coterm/operation"]["status"], "accepted");
    let immediate_id = immediate["_meta"]["coterm/operation"]["operationId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No accepted receipt"))?;
    let cancelled = client
        .tool(
            "coterm.cancel_operation",
            json!({"driverInstanceId":instance.driver_instance_id,"operationId":immediate_id}),
        )
        .await?;
    assert_eq!(cancelled["structuredContent"]["status"], "cancelled");
    let pending = client
        .tool(
            "coterm_ssh_command",
            json!({"driverInstanceId":instance.driver_instance_id,"commands":["not-approved-original"]}),
        )
        .await?;
    let receipt = &pending["_meta"]["coterm/operation"];
    runtime.approve(human(), ApproveProposal {
        operation_id:receipt["operationId"].as_str().ok_or_else(|| anyhow::anyhow!("Missing command"))?.to_owned(),
        proposal_id:receipt["proposalId"].as_str().ok_or_else(|| anyhow::anyhow!("Missing command proposal"))?.to_owned(),
        driver_instance_id:Some(instance.driver_instance_id.clone()),
        args:json!({"driverInstanceId":instance.driver_instance_id,"commands":["human-approved-edit"]}),
    }).await?;
    // Human input goes into exactly the same shell the external Agent uses.
    runtime
        .submit(
            human(),
            SubmitOperation {
                driver_id: instance.driver_id.clone(),
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                operation: "ssh.input".to_owned(),
                args: json!({"data":STANDARD.encode(b"human-terminal-input")}),
                wait_timeout_ms: None,
            },
        )
        .await?;
    timeout(Duration::from_secs(5), async {
        loop {
            let output = client
                .tool(
                    "coterm_ssh_read_lines",
                    json!({"driverInstanceId":instance.driver_instance_id,"lines":100}),
                )
                .await?;
            let domain = &output["structuredContent"];
            let bytes = domain["lines"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
                .into_bytes();
            if bytes.windows(20).any(|w| w == b"human-terminal-input") {
                assert!(bytes.windows(19).any(|w| w == b"human-approved-edit"));
                assert!(!bytes.windows(21).any(|w| w == b"not-approved-original"));
                break Ok::<(), anyhow::Error>(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    // Bypass skips approval but still requires the human authentication prompt.
    runtime.set_bypass(&other.driver_instance_id, true).await?;
    assert!(!runtime.bypass_enabled(&instance.driver_instance_id).await?);
    let waiting = client
        .tool(
            "coterm_ssh_connect",
            json!({"driverInstanceId":other.driver_instance_id,"fingerprint":ssh.fingerprint}),
        )
        .await?;
    assert_eq!(waiting["_meta"]["coterm/operation"]["status"], "running");
    let request = runtime
        .pending_interactions()
        .await
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No password prompt"))?;
    let operation_id = waiting["_meta"]["coterm/operation"]["operationId"].clone();
    let cancelled = client
        .tool(
            "coterm.cancel_operation",
            json!({"driverInstanceId":other.driver_instance_id,"operationId":operation_id}),
        )
        .await?;
    assert_eq!(cancelled["structuredContent"]["status"], "cancelled");
    assert!(runtime
        .respond_secret(&request.interaction_id, Some("late".to_owned()))
        .await
        .is_err());
    // Destroying an instance withdraws its outstanding interaction and rejects late input.
    client
        .tool(
            "coterm_ssh_connect",
            json!({"driverInstanceId":other.driver_instance_id,"fingerprint":ssh.fingerprint}),
        )
        .await?;
    let destroyed_request = runtime
        .pending_interactions()
        .await
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No destroy-test prompt"))?;
    runtime.destroy_instance(&other.driver_instance_id).await?;
    assert!(runtime.pending_interactions().await.is_empty());
    assert!(runtime
        .respond_secret(
            &destroyed_request.interaction_id,
            Some("late-after-destroy".to_owned())
        )
        .await
        .is_err());
    let records = runtime
        .recent(human(), &instance.driver_instance_id, 100)
        .await?;
    assert!(!serde_json::to_string(&records)?.contains("local-test-password"));
    // A running command accepts a cancellation request but remains UNKNOWN until the
    // remote shell confirms its completion boundary.
    runtime
        .set_bypass(&instance.driver_instance_id, true)
        .await?;
    client.next_id += 1;
    let interrupted_id = client.next_id;
    coterm_mcp::transport::write_frame(&mut client.writer, &json!({"jsonrpc":"2.0","id":interrupted_id,"method":"tools/call","params":{
        "name":"coterm_ssh_command","arguments":{"name":instance.label,"commands":["never-completes"]}
    }})).await?;
    timeout(Duration::from_secs(3), async {
        loop {
            if runtime
                .recent(human(), &instance.driver_instance_id, 100)
                .await?
                .iter()
                .any(|r| r.operation == "mcp.ssh.command" && r.status == OperationStatus::Running)
            {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await??;
    let active = runtime
        .recent(human(), &instance.driver_instance_id, 100)
        .await?
        .into_iter()
        .find(|record| {
            record.operation == "mcp.ssh.command" && record.status == OperationStatus::Running
        })
        .ok_or_else(|| anyhow::anyhow!("Running MCP operation not found"))?;
    let cancel = client.tool("coterm.cancel_operation", json!({"driverInstanceId":instance.driver_instance_id,"operationId":active.operation_id})).await?;
    assert_eq!(cancel["structuredContent"]["status"], "unknown");
    let interrupted = timeout(
        Duration::from_secs(3),
        coterm_mcp::transport::read_frame(&mut client.reader),
    )
    .await??
    .ok_or_else(|| anyhow::anyhow!("No disconnect response"))?;
    let interrupted: Value = serde_json::from_slice(&interrupted)?;
    assert_eq!(interrupted["id"], interrupted_id);
    assert_eq!(
        interrupted["result"]["_meta"]["coterm/operation"]["status"],
        "unknown"
    );
    assert_eq!(interrupted["result"]["isError"], true);
    runtime.stop("coterm-driver-ssh").await?;
    let changed = timeout(
        Duration::from_secs(3),
        coterm_mcp::transport::read_frame(&mut client.reader),
    )
    .await??
    .ok_or_else(|| anyhow::anyhow!("Missing tool directory notification"))?;
    assert_eq!(
        serde_json::from_slice::<Value>(&changed)?["method"],
        "notifications/tools/list_changed"
    );
    drop(bridge);
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

/// Builds one driver package directory for a test and returns its path.
async fn load_runtime(
    server: &TestServer,
    tag: &str,
) -> Result<(
    std::sync::Arc<coterm_core::driver_manager::ProcessDriverRuntime>,
    coterm_proto::runtime::lifecycle::InstanceInfo,
)> {
    use coterm_core::{driver_manager::ProcessDriverRuntime, driver_runtime::DriverRuntime};
    use coterm_proto::runtime::{lifecycle::RuntimeLimits, stream::FlowWindow};
    let directory = std::env::temp_dir().join(format!(
        "coterm-ssh-{tag}-{}-{}",
        std::process::id(),
        server.config.port
    ));
    std::fs::create_dir_all(directory.join("backend"))?;
    std::fs::create_dir_all(directory.join("frontend"))?;
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_coterm-driver-ssh"));
    std::fs::copy(
        executable,
        directory.join("backend").join(
            executable
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Missing binary name"))?,
        ),
    )?;
    std::fs::write(
        directory.join("manifest.json"),
        include_str!("../manifest.json"),
    )?;
    std::fs::write(
        directory.join("frontend/index.js"),
        "export function mount() {}",
    )?;
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else {
        std::env::consts::OS
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let runtime = std::sync::Arc::new(ProcessDriverRuntime::new(
        format!("{os}-{arch}"),
        RuntimeLimits {
            max_control_bytes: 1024 * 1024,
            max_data_chunk_bytes: 8192,
            initial_window: FlowWindow {
                chunks: 16,
                bytes: 1024 * 1024,
            },
        },
    ));
    runtime.load(&directory).await?;
    runtime.start("coterm-driver-ssh").await?;
    let instance = runtime
        .create_instance("coterm-driver-ssh", serde_json::to_value(&server.config)?)
        .await?;
    Ok((runtime, instance))
}

async fn connect_instance(
    runtime: &std::sync::Arc<coterm_core::driver_manager::ProcessDriverRuntime>,
    instance: &coterm_proto::runtime::lifecycle::InstanceInfo,
    server: &TestServer,
) -> Result<()> {
    use coterm_core::driver_runtime::{CallerContext, Operations, SubmitOperation};
    use coterm_proto::runtime::operation::{OperationSource, OperationStatus};
    let human = || CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let handle = runtime
        .provide_secret(
            &instance.driver_instance_id,
            "ssh.password".to_owned(),
            "local-test-password".to_owned(),
        )
        .await?;
    let receipt = runtime
        .submit(
            human(),
            SubmitOperation {
                driver_id: instance.driver_id.clone(),
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                operation: "ssh.connect".to_owned(),
                args: json!({"fingerprint":server.fingerprint,"secret":handle}),
                wait_timeout_ms: None,
            },
        )
        .await?;
    assert_eq!(receipt.status, OperationStatus::Completed);
    Ok(())
}

#[tokio::test]
async fn unified_command_snapshots_are_instance_bound_and_survive_disconnect() -> Result<()> {
    use coterm_core::driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation};
    use coterm_proto::runtime::{lifecycle::InstanceInfo, operation::OperationSource};
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "command-snapshots").await?;
    let other = runtime
        .create_instance("coterm-driver-ssh", serde_json::to_value(&ssh.config)?)
        .await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let caller = |instance: &InstanceInfo| CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let request = |instance: &InstanceInfo, name: &str, args: Value| SubmitOperation {
        driver_id: instance.driver_id.clone(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: name.to_owned(),
        args,
        wait_timeout_ms: Some(3000),
    };
    for (mode, text) in [
        ("terminal", "echo separated-stderr"),
        ("exec", "echo background"),
    ] {
        let args = if mode == "terminal" {
            json!({"text":text})
        } else {
            json!({"text":text,"mode":mode})
        };
        let result = runtime
            .submit(caller(&instance), request(&instance, "ssh.command", args))
            .await?;
        let output = result
            .output
            .ok_or_else(|| anyhow::anyhow!("missing command result"))?;
        assert_eq!(output["mode"], mode);
        assert_eq!(output["stdin"], text);
        assert_eq!(output["instanceId"], instance.driver_instance_id);
        assert_eq!(output["status"], "completed");
        assert!(output["stdout"]
            .as_str()
            .is_some_and(|value| value.contains(text)));
        assert!(output.get("stderr").is_some_and(Value::is_string));
        if mode == "terminal" {
            assert_eq!(output["stderr"], "separate diagnostic\n");
            assert_eq!(output["streamMode"], "separated");
        }
    }
    let history = runtime
        .submit(
            caller(&instance),
            request(&instance, "ssh.read_dialogue_pairs", json!({"count":2})),
        )
        .await?;
    let history = history
        .output
        .ok_or_else(|| anyhow::anyhow!("missing history"))?;
    assert_eq!(history["pairs"].as_array().map(Vec::len), Some(2));
    assert_eq!(history["pairs"][0]["mode"], "exec");
    let empty = runtime
        .submit(
            caller(&other),
            request(&other, "ssh.read_dialogue_pairs", json!({"count":2})),
        )
        .await?;
    assert_eq!(
        empty.output.as_ref().map(|value| &value["pairs"]),
        Some(&json!([]))
    );
    runtime
        .submit(
            caller(&instance),
            request(&instance, "ssh.disconnect", json!({})),
        )
        .await?;
    let lines = runtime
        .submit(
            caller(&instance),
            request(&instance, "ssh.read_lines", json!({"lines":100})),
        )
        .await?;
    let lines = lines
        .output
        .ok_or_else(|| anyhow::anyhow!("missing lines"))?;
    assert_eq!(lines["closed"], true);
    let text = lines["lines"].to_string();
    assert!(text.contains("separated-stderr"));
    assert!(!text.contains("separate diagnostic"));
    let after = runtime
        .submit(
            caller(&instance),
            request(&instance, "ssh.read_dialogue_pairs", json!({"count":2})),
        )
        .await?;
    assert_eq!(after.output.as_ref(), Some(&history));
    connect_instance(&runtime, &instance, &ssh).await?;
    let cleared = runtime
        .submit(
            caller(&instance),
            request(&instance, "ssh.read_dialogue_pairs", json!({"count":2})),
        )
        .await?;
    assert_eq!(
        cleared.output.as_ref().map(|value| &value["pairs"]),
        Some(&json!([]))
    );
    runtime.stop("coterm-driver-ssh").await?;
    Ok(())
}

#[tokio::test]
async fn selected_lines_are_exact_human_owned_and_connection_bound() -> Result<()> {
    use coterm_core::driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation};
    use coterm_proto::runtime::{
        lifecycle::InstanceInfo,
        operation::{OperationSource, OperationStatus},
    };
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "selected-lines").await?;
    let other = runtime
        .create_instance("coterm-driver-ssh", serde_json::to_value(&ssh.config)?)
        .await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let caller = |instance: &InstanceInfo, source| CallerContext {
        source,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let request = |instance: &InstanceInfo, name: &str, args: Value| SubmitOperation {
        driver_id: instance.driver_id.clone(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: name.to_owned(),
        args,
        wait_timeout_ms: Some(3000),
    };
    let status = runtime
        .submit(
            caller(&instance, OperationSource::Human),
            request(&instance, "ssh.status", json!({})),
        )
        .await?;
    let connection_id = status
        .output
        .ok_or_else(|| anyhow::anyhow!("missing status"))?["connectionId"]
        .clone();
    let selected = "selected partial line\n第二行";
    runtime
        .submit(
            caller(&instance, OperationSource::Human),
            request(
                &instance,
                "ssh.select_lines",
                json!({"connectionId":connection_id,"text":selected}),
            ),
        )
        .await?;
    let read = runtime
        .submit(
            caller(&instance, OperationSource::Agent),
            request(
                &instance,
                "mcp.ssh.read_selected_lines",
                json!({"driverInstanceId":instance.driver_instance_id}),
            ),
        )
        .await?;
    assert_eq!(
        read.output
            .as_ref()
            .map(|value| &value["structuredContent"]["text"]),
        Some(&json!(selected))
    );
    for (source, connection) in [
        (OperationSource::Agent, connection_id.clone()),
        (OperationSource::Human, json!("stale")),
    ] {
        let result = runtime
            .submit(
                caller(&instance, source),
                request(
                    &instance,
                    "ssh.select_lines",
                    json!({"connectionId":connection,"text":"fabricated"}),
                ),
            )
            .await;
        assert!(
            result.is_err()
                || result
                    .as_ref()
                    .is_ok_and(|receipt| receipt.status != OperationStatus::Completed)
        );
    }
    let empty = runtime
        .submit(
            caller(&other, OperationSource::Agent),
            request(
                &other,
                "mcp.ssh.read_selected_lines",
                json!({"driverInstanceId":other.driver_instance_id}),
            ),
        )
        .await?;
    assert_eq!(
        empty
            .output
            .as_ref()
            .map(|value| &value["structuredContent"]["text"]),
        Some(&json!(""))
    );
    runtime
        .submit(
            caller(&instance, OperationSource::Human),
            request(&instance, "ssh.disconnect", json!({})),
        )
        .await?;
    let cleared = runtime
        .submit(
            caller(&instance, OperationSource::Agent),
            request(
                &instance,
                "mcp.ssh.read_selected_lines",
                json!({"driverInstanceId":instance.driver_instance_id}),
            ),
        )
        .await?;
    assert_eq!(
        cleared
            .output
            .as_ref()
            .map(|value| &value["structuredContent"]["text"]),
        Some(&json!(""))
    );
    runtime.stop("coterm-driver-ssh").await?;
    Ok(())
}

#[tokio::test]
async fn running_command_output_is_readable_without_cursors_or_resubmission() -> Result<()> {
    use coterm_core::driver_runtime::DriverRuntime;
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "running-snapshot").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    runtime
        .set_bypass(&instance.driver_instance_id, true)
        .await?;
    let submitted = coterm_mcp::server::call_tool(&runtime, &json!({
        "name":"coterm_ssh_command", "arguments":{"name":instance.label,"commands":["never-completes"]},
        "_meta":{"coterm/waitTimeoutMs":50}
    })).await?;
    let operation_id = submitted["_meta"]["coterm/operation"]["operationId"].clone();
    assert!(operation_id.is_string());
    timeout(Duration::from_secs(3), async {
        loop {
            let lines = coterm_mcp::server::call_tool(&runtime, &json!({
                "name":"coterm_ssh_read_lines", "arguments":{"name":instance.label,"operationId":operation_id,"lines":2}
            })).await?;
            let domain = &lines["structuredContent"];
            if domain["lines"].to_string().contains("never-completes") {
                assert_eq!(domain["status"], "running");
                assert!(domain["exitCode"].is_null());
                break Ok::<(), anyhow::Error>(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    }).await??;
    let pairs = coterm_mcp::server::call_tool(
        &runtime,
        &json!({
            "name":"coterm_ssh_read_dialogue_pairs", "arguments":{"name":instance.label,"count":2}
        }),
    )
    .await?;
    let pairs = &pairs["structuredContent"]["pairs"];
    assert_eq!(pairs.as_array().map(Vec::len), Some(1));
    assert_eq!(pairs[0]["operationId"], operation_id);
    assert_eq!(pairs[0]["mode"], "terminal");
    assert_eq!(pairs[0]["status"], "running");
    assert_eq!(pairs[0]["stdin"], "never-completes");
    runtime.stop("coterm-driver-ssh").await?;
    Ok(())
}

#[tokio::test]
async fn syntax_errors_are_rejected_without_killing_the_shared_shell() -> Result<()> {
    use coterm_core::driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation};
    use coterm_proto::runtime::operation::{OperationSource, OperationStatus};
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "syntax").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let human = || CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let command = |text: &str| SubmitOperation {
        driver_id: instance.driver_id.clone(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: "ssh.command".to_owned(),
        args: json!({"text": text}),
        wait_timeout_ms: Some(3_000),
    };
    let receipt = runtime.submit(human(), command("if")).await?;
    assert_eq!(receipt.status, OperationStatus::Completed, "{receipt:?}");
    let output = receipt.output.ok_or_else(|| anyhow::anyhow!("no output"))?;
    assert_eq!(output["syntaxError"], true, "{output}");
    assert_eq!(output["exitCode"], 2, "{output}");
    // The same shell still accepts the next command.
    let receipt = runtime.submit(human(), command("echo still-alive")).await?;
    assert_eq!(receipt.status, OperationStatus::Completed, "{receipt:?}");
    let output = receipt.output.ok_or_else(|| anyhow::anyhow!("no output"))?;
    assert_eq!(output["exitCode"], 0);
    assert!(output["text"]
        .as_str()
        .is_some_and(|text| text.contains("still-alive")));
    let status = runtime
        .submit(
            human(),
            SubmitOperation {
                driver_id: instance.driver_id.clone(),
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                operation: "ssh.status".to_owned(),
                args: json!({}),
                wait_timeout_ms: None,
            },
        )
        .await?;
    assert_eq!(
        status.output.as_ref().map(|value| &value["connected"]),
        Some(&json!(true))
    );
    runtime.stop("coterm-driver-ssh").await?;
    Ok(())
}

#[tokio::test]
async fn agent_feedback_approval_returns_without_waiting_for_the_human() -> Result<()> {
    use coterm_core::driver_runtime::DriverRuntime;
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "agent-approval").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let first = timeout(
        Duration::from_millis(500),
        coterm_mcp::server::call_tool(
            &runtime,
            &json!({
                "name":"coterm_ssh_command",
                "arguments":{"name":instance.label,"mode":"exec","commands":["approval-feedback"]}
            }),
        ),
    )
    .await;
    runtime.stop("coterm-driver-ssh").await?;
    let receipt = first
        .map_err(|_| anyhow::anyhow!("approval must return immediately, not wait for a human"))??;
    assert_eq!(receipt["structuredContent"]["status"], "pending_approval");
    assert!(receipt["structuredContent"]["operationId"].is_string());
    Ok(())
}

#[tokio::test]
async fn agent_feedback_exec_keeps_32k_in_one_result() -> Result<()> {
    use coterm_core::driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation};
    use coterm_proto::runtime::operation::OperationSource;
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "agent-output").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let source = "x".repeat(32 * 1024);
    let receipt = runtime
        .submit(
            CallerContext {
                source: OperationSource::Human,
                session_id: instance.session_id.clone(),
                driver_id: Some(instance.driver_id.clone()),
            },
            SubmitOperation {
                driver_id: instance.driver_id.clone(),
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                operation: "ssh.command".into(),
                args: json!({"text":source,"mode":"exec"}),
                wait_timeout_ms: Some(3000),
            },
        )
        .await?;
    runtime.stop("coterm-driver-ssh").await?;
    let output = receipt
        .output
        .ok_or_else(|| anyhow::anyhow!("missing output"))?;
    assert_eq!(output["stdout"].as_str().map(str::len), Some(32 * 1024));
    assert_eq!(output["stdoutTruncated"], false);
    assert_eq!(output["stdout"].as_str(), Some(source.as_str()));
    // Verify the complete Agent path too: native MCP result, readable text and
    // retained operation record must all survive the host's result budget.
    let ssh = server_mode(true).await?;
    let (runtime, instance) = load_runtime(&ssh, "agent-mcp-output").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    runtime
        .set_bypass(&instance.driver_instance_id, true)
        .await?;
    let bridge =
        coterm_mcp::server::BridgeServer::bind_at(runtime.clone(), "127.0.0.1:0".parse()?).await?;
    let mut client =
        McpWire::connect(bridge.connection_info(), std::slice::from_ref(&instance)).await?;
    let source = "0".repeat(32 * 1024);
    let result = client
        .rpc(
            "tools/call",
            json!({"name":"coterm_ssh_command","arguments":{"mode":"exec","commands":["printf '%032768d' 0"]}}),
        )
        .await?;
    assert_eq!(
        result["structuredContent"]["stdout"].as_str(),
        Some(source.as_str())
    );
    assert_eq!(result["structuredContent"]["stdoutTruncated"], false);
    assert!(result["content"][0]["text"]
        .as_str()
        .is_some_and(|text| text.contains(&source)));
    assert!(serde_json::to_vec(&result)?.len() < coterm_mcp::transport::MAX_FRAME_BYTES);
    let queried = client
        .tool(
            "coterm.get_operation",
            json!({"operationId":result["_meta"]["coterm/operation"]["operationId"]}),
        )
        .await?;
    assert_eq!(
        queried["structuredContent"]["output"]["structuredContent"]["stdout"].as_str(),
        Some(source.as_str())
    );
    runtime.stop("coterm-driver-ssh").await?;
    Ok(())
}

#[tokio::test]
async fn agent_feedback_health_default_routing_and_legacy_aliases() -> Result<()> {
    use coterm_core::driver_runtime::DriverRuntime;
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "agent-routing").await?;
    let call = |name: &str, arguments: Value| json!({"name":name,"arguments":arguments});
    let no_connection =
        coterm_mcp::server::call_tool(&runtime, &call("coterm_ssh_status", json!({}))).await;
    assert!(no_connection.is_err(), "ready does not mean connected");
    let explicit = coterm_mcp::server::call_tool(
        &runtime,
        &call("coterm_ssh_status", json!({"name":instance.label})),
    )
    .await?;
    assert_eq!(explicit["structuredContent"]["connected"], false);
    connect_instance(&runtime, &instance, &ssh).await?;
    let connected =
        coterm_mcp::server::call_tool(&runtime, &call("coterm_ssh_status", json!({}))).await?;
    assert_eq!(connected["structuredContent"]["connected"], true);
    let other = runtime
        .create_instance("coterm-driver-ssh", serde_json::to_value(&ssh.config)?)
        .await?;
    let one =
        coterm_mcp::server::call_tool(&runtime, &call("coterm_ssh_status", json!({}))).await?;
    assert_eq!(
        one["structuredContent"]["connected"], true,
        "a disconnected second instance is not a candidate"
    );
    let health = coterm_mcp::server::call_tool(&runtime, &call("coterm.status", json!({}))).await?;
    assert_eq!(health["structuredContent"]["desktopAlive"], true);
    assert_eq!(health["structuredContent"]["connectedInstances"], 1);
    assert_eq!(health["structuredContent"]["activeInstances"], 2);
    assert!(!health.to_string().contains(&instance.driver_instance_id));
    for tool in ["coterm.get_operation", "coterm.cancel_operation"] {
        let private_target = coterm_mcp::server::call_tool(
            &runtime,
            &call(
                tool,
                json!({"driverInstanceId":instance.driver_instance_id,"operationId":"unused"}),
            ),
        )
        .await;
        assert!(private_target.is_err_and(|error| error.to_string().contains("Unknown argument")));
    }
    let tools = coterm_mcp::server::tools_list(&runtime).await?;
    let tools = tools["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing tools"))?;
    let exec = tools
        .iter()
        .find(|tool| tool["name"] == "coterm_ssh_command")
        .ok_or_else(|| anyhow::anyhow!("missing exec"))?;
    assert_eq!(exec["_meta"]["coterm/returnOnApproval"], true);
    assert!(!exec["inputSchema"]["required"]
        .as_array()
        .is_some_and(|required| required.contains(&json!("name"))));
    assert!(tools.iter().all(|tool| !tool["name"]
        .as_str()
        .is_some_and(|name| name.contains("__"))));
    let legacy = coterm_mcp::server::call_tool(
        &runtime,
        &call(
            "coterm-driver-ssh__ssh.status",
            json!({"name":instance.label}),
        ),
    )
    .await?;
    assert_eq!(legacy["structuredContent"]["connected"], true);
    for args in [
        json!({"name":null}),
        json!({"name":instance.label,"instanceName":instance.label}),
        json!({"driverInstanceId":instance.driver_instance_id}),
    ] {
        assert!(
            coterm_mcp::server::call_tool(&runtime, &call("coterm_ssh_status", args))
                .await
                .is_err()
        );
    }
    connect_instance(&runtime, &other, &ssh).await?;
    let ambiguous = coterm_mcp::server::call_tool(
        &runtime,
        &call("coterm_ssh_command", json!({"commands":["must-not-run"]})),
    )
    .await;
    assert!(ambiguous.is_err_and(|error| error.to_string().contains("Multiple connected")));
    assert!(runtime.pending_interactions().await.is_empty());
    runtime.stop("coterm-driver-ssh").await?;
    Ok(())
}

#[tokio::test]
async fn exec_is_isolated_from_the_shared_pty_and_streams_output() -> Result<()> {
    use coterm_core::driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation};
    use coterm_proto::runtime::operation::{OperationSource, OperationStatus};
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "exec").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let human = || CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let exec = |text: &str| SubmitOperation {
        driver_id: instance.driver_id.clone(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: "ssh.command".to_owned(),
        args: json!({"text": text,"mode":"exec"}),
        wait_timeout_ms: Some(3_000),
    };
    // The jobs row is driven by an event emitted before the command finishes.
    let mut events = runtime
        .subscribe_events(&instance.driver_instance_id)
        .await?;
    let receipt = runtime.submit(human(), exec("echo isolated")).await?;
    assert_eq!(receipt.status, OperationStatus::Completed, "{receipt:?}");
    let mut started = false;
    let mut finished = false;
    timeout(Duration::from_secs(3), async {
        while let Ok(notification) = events.recv().await {
            if let coterm_proto::runtime::message::DriverNotification::Event {
                name, payload, ..
            } = notification
            {
                if name != "ssh.job" || payload["operationId"] != receipt.operation_id {
                    continue;
                }
                assert_eq!(payload["text"], "echo isolated");
                if payload["done"] == false {
                    started = true;
                } else {
                    finished = true;
                    return;
                }
            }
        }
    })
    .await?;
    assert!(started && finished, "job lifecycle events missing");
    let output = receipt.output.ok_or_else(|| anyhow::anyhow!("no output"))?;
    assert_eq!(output["exitCode"], 0);
    assert_eq!(output["stdout"], "echo isolated");
    // Exec output is not written into the interactive terminal replay.
    let read = runtime
        .submit(
            human(),
            SubmitOperation {
                driver_id: instance.driver_id.clone(),
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                operation: "ssh.read".to_owned(),
                args: json!({"cursor":"0"}),
                wait_timeout_ms: None,
            },
        )
        .await?;
    let replay = read.output.ok_or_else(|| anyhow::anyhow!("no replay"))?;
    let bytes = STANDARD.decode(replay["data"].as_str().unwrap_or_default())?;
    assert!(!bytes.windows(8).any(|window| window == b"isolated"));
    // Progress polling uses the same bounded job buffer.
    let operation_id = receipt.operation_id.clone();
    let output_view = runtime
        .submit(
            human(),
            SubmitOperation {
                driver_id: instance.driver_id.clone(),
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                operation: "ssh.output".to_owned(),
                args: json!({"operationId": operation_id}),
                wait_timeout_ms: None,
            },
        )
        .await?;
    let view = output_view
        .output
        .ok_or_else(|| anyhow::anyhow!("no view"))?;
    assert_eq!(view["status"], "completed");
    assert_eq!(view["stdout"], "echo isolated");
    assert_eq!(view["exitCode"], 0);
    runtime.stop("coterm-driver-ssh").await?;
    Ok(())
}

#[tokio::test]
async fn read_only_queries_use_fixed_scripts_outside_the_pty() -> Result<()> {
    use coterm_core::driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation};
    use coterm_proto::runtime::operation::{OperationSource, OperationStatus};
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "readonly").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let human = || CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let call = |operation: &str, args: Value| SubmitOperation {
        driver_id: instance.driver_id.clone(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: operation.to_owned(),
        args,
        wait_timeout_ms: Some(3_000),
    };
    let info = runtime
        .submit(human(), call("ssh.system_info", json!({})))
        .await?;
    assert_eq!(info.status, OperationStatus::Completed, "{info:?}");
    let info = info.output.ok_or_else(|| anyhow::anyhow!("no info"))?;
    assert_eq!(info["uid"], "1000");
    assert_eq!(info["os"], "TestOS");
    let file = runtime
        .submit(
            human(),
            call("ssh.file_info", json!({"path":"/tmp/exists"})),
        )
        .await?;
    let file = file.output.ok_or_else(|| anyhow::anyhow!("no file info"))?;
    assert_eq!(file["exists"], "1");
    assert_eq!(file["size"], "11");
    // Read-only tools never require approval, even for an Agent source.
    let agent = CallerContext {
        source: OperationSource::Agent,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let agent_info = runtime
        .submit(agent, call("ssh.system_info", json!({})))
        .await?;
    assert_eq!(agent_info.status, OperationStatus::Completed);
    runtime.stop("coterm-driver-ssh").await?;
    Ok(())
}

#[test]
fn file_tools_are_explicit_bounded_and_write_is_a_sensitive_commit() {
    use coterm_proto::runtime::operation::OperationEffect;

    let operations = coterm_driver_ssh::backend::catalog();
    let read = operations
        .iter()
        .find(|operation| operation.name == "ssh.read_file")
        .expect("ssh.read_file descriptor");
    assert_eq!(read.effect, OperationEffect::Read);
    let write = operations
        .iter()
        .find(|operation| operation.name == "ssh.write_file")
        .expect("ssh.write_file descriptor");
    assert_eq!(write.effect, OperationEffect::Commit);
    assert!(write.sensitive_input);
    assert_eq!(
        write.input_schema["properties"]["content"]["maxLength"],
        65536
    );

    let tools = coterm_driver_ssh::backend::mcp_tools();
    assert!(tools
        .iter()
        .any(|tool| tool.operation == "mcp.ssh.read_file"));
    assert!(tools
        .iter()
        .any(|tool| tool.operation == "mcp.ssh.write_file"));
}

#[tokio::test]
async fn read_file_returns_utf8_text_on_an_isolated_channel() -> Result<()> {
    let ssh = server().await?;
    let connection =
        Connection::connect(&ssh.config, ssh.fingerprint.clone(), "local-test-password").await?;
    let result = connection.read_file("/tmp/config file").await?;
    assert_eq!(result["text"], "alpha=1\nbeta=two\n");
    assert_eq!(result["bytes"], 17);
    connection.disconnect().await?;
    Ok(())
}

/// A shell that accepts commands but never emits a completion marker for them.
struct Stuck;

impl server::Handler for Stuck {
    type Error = russh::Error;
    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if user == "tester" && password == "local-test-password" {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }
    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }
    async fn data(
        &mut self,
        channel: ChannelId,
        _: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        // Never answer, so cancellation cannot confirm completion.
        let _ = (channel, session);
        Ok(())
    }
}

async fn stuck_server() -> Result<TestServer> {
    let key = russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)?;
    let fingerprint = key
        .public_key()
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::from_millis(5),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let config = config.clone();
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, socket, Stuck).await {
                    let _ = session.await;
                }
            });
        }
    });
    Ok(TestServer {
        config: Config {
            host: "127.0.0.1".to_owned(),
            port,
            username: "tester".to_owned(),
        },
        fingerprint,
        task,
    })
}

#[tokio::test]
async fn unconfirmed_cancel_resets_the_terminal_instead_of_blocking_it() -> Result<()> {
    let ssh = stuck_server().await?;
    let connection =
        Connection::connect(&ssh.config, ssh.fingerprint.clone(), "local-test-password").await?;
    connection
        .command("op-stuck".into(), false, "sleep 20")
        .await?;
    // Give the input a moment to reach the remote shell before cancelling.
    sleep(Duration::from_millis(50)).await;
    let cancel = connection.cancel("op-stuck".to_owned()).await?;
    assert_eq!(cancel["outcome"], "unknown");
    assert_eq!(cancel["terminalReset"], true);
    // The connection remains usable: a fresh command is accepted on the new shell.
    connection.command("op-next".into(), false, "true").await?;
    assert!(!connection.is_closed());
    connection.disconnect().await?;
    Ok(())
}

#[tokio::test]
async fn explicit_reset_abandons_an_unknown_command_and_keeps_exec_available() -> Result<()> {
    let ssh = server().await?;
    let connection =
        Connection::connect(&ssh.config, ssh.fingerprint.clone(), "local-test-password").await?;
    connection
        .command("op-never".into(), false, "never-completes")
        .await?;
    let reset = connection.reset().await?;
    assert_eq!(reset["reset"], true);
    // A subsequent command runs normally after the reset.
    connection
        .command("op-after".into(), false, "echo after-reset")
        .await?;
    let output = read_until(&connection, b"after-reset").await?;
    assert!(output.windows(11).any(|window| window == b"after-reset"));
    connection.reset().await?;
    assert!(!connection.is_closed());
    connection.disconnect().await?;
    Ok(())
}

#[tokio::test]
async fn command_regex_rules_control_approval_without_changing_effect_or_checks() -> Result<()> {
    use coterm_core::driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation};
    use coterm_proto::runtime::operation::{OperationSource, OperationStatus};
    let ssh = server().await?;
    let (runtime, instance) = load_runtime(&ssh, "allow").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let agent = CallerContext {
        source: OperationSource::Agent,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let human = CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let exec = || SubmitOperation {
        driver_id: instance.driver_id.clone(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: "mcp.ssh.command".to_owned(),
        args: json!({"driverInstanceId":instance.driver_instance_id,"mode":"exec","commands":["echo allowed"]}),
        wait_timeout_ms: Some(3_000),
    };
    // Without command rules an Agent commit still becomes a proposal.
    let proposal = runtime.submit(agent.clone(), exec()).await?;
    assert_eq!(proposal.status, OperationStatus::AwaitingApproval);
    // Rejection is a human action; the Agent cannot clear its own proposal.
    assert!(runtime
        .reject(
            agent.clone(),
            &proposal.operation_id,
            proposal
                .proposal_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("no proposal"))?
        )
        .await
        .is_err());
    runtime
        .reject(
            human.clone(),
            &proposal.operation_id,
            proposal
                .proposal_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("no proposal"))?,
        )
        .await?;
    // Command text is matched independently of the operation name.
    runtime
        .set_approval_policy(
            &instance.driver_instance_id,
            coterm_core::approval_policy::ApprovalPolicy {
                mode: coterm_core::approval_policy::ApprovalMode::AllowMatches,
                patterns: vec![r"^echo allowed$".to_owned()],
            },
        )
        .await?;
    let receipt = runtime.submit(agent.clone(), exec()).await?;
    assert_eq!(receipt.status, OperationStatus::Completed, "{receipt:?}");
    // Effect is still commit in the ledger: command rules skipped approval only.
    let record = runtime.get(agent.clone(), &receipt.operation_id).await?;
    assert!(matches!(
        record.effect,
        coterm_proto::runtime::operation::OperationEffect::Commit
    ));
    // A command not matched by the new rule still needs approval.
    runtime
        .set_approval_policy(
            &instance.driver_instance_id,
            coterm_core::approval_policy::ApprovalPolicy {
                mode: coterm_core::approval_policy::ApprovalMode::AllowMatches,
                patterns: vec![r"^ls$".to_owned()],
            },
        )
        .await?;
    let proposal = runtime.submit(agent.clone(), exec()).await?;
    assert_eq!(proposal.status, OperationStatus::AwaitingApproval);
    // Reverse mode intercepts matching text and allows the other commands.
    runtime
        .set_approval_policy(
            &instance.driver_instance_id,
            coterm_core::approval_policy::ApprovalPolicy {
                mode: coterm_core::approval_policy::ApprovalMode::ApproveMatches,
                patterns: vec![r"^echo allowed$".to_owned()],
            },
        )
        .await?;
    assert_eq!(
        runtime.submit(agent.clone(), exec()).await?.status,
        OperationStatus::AwaitingApproval
    );
    runtime
        .set_approval_policy(
            &instance.driver_instance_id,
            coterm_core::approval_policy::ApprovalPolicy {
                mode: coterm_core::approval_policy::ApprovalMode::ApproveMatches,
                patterns: vec![r"^rm".to_owned()],
            },
        )
        .await?;
    assert_eq!(
        runtime.submit(agent.clone(), exec()).await?.status,
        OperationStatus::Completed
    );
    runtime.stop("coterm-driver-ssh").await?;
    Ok(())
}

async fn wait_for_test_operation(
    runtime: &coterm_core::driver_manager::ProcessDriverRuntime,
    caller: coterm_core::driver_runtime::CallerContext,
    id: &str,
) -> Result<coterm_proto::runtime::operation::OperationRecord> {
    use coterm_core::driver_runtime::Operations;
    use coterm_proto::runtime::operation::OperationStatus;
    timeout(Duration::from_secs(5), async {
        loop {
            let record = runtime.get(caller.clone(), id).await?;
            if matches!(
                record.status,
                OperationStatus::Completed
                    | OperationStatus::Failed
                    | OperationStatus::Unknown
                    | OperationStatus::Cancelled
            ) {
                return Ok::<_, anyhow::Error>(record);
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

#[tokio::test]
async fn real_mcp_file_tools_and_multiline_exec_preserve_content_and_stop_on_failure() -> Result<()>
{
    use coterm_core::driver_runtime::{CallerContext, Operations, DriverRuntime};
    use coterm_proto::runtime::operation::{ApproveProposal, OperationSource, OperationStatus};
    let ssh = server_mode(true).await?;
    let (runtime, instance) = load_runtime(&ssh, "scripts").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let bridge =
        coterm_mcp::server::BridgeServer::bind_at(runtime.clone(), "127.0.0.1:0".parse()?).await?;
    let mut client =
        McpWire::connect(bridge.connection_info(), std::slice::from_ref(&instance)).await?;
    let human = CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let directory =
        std::env::temp_dir().join(format!("coterm-script-fixture-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory)?;
    let remote = directory.to_string_lossy().replace('\\', "/");
    let path = format!("{remote}/config.txt");
    let content = "setting='hello world'\n".repeat(260);
    let args =
        json!({"driverInstanceId":instance.driver_instance_id,"path":path,"content":content});
    let proposal = client.tool("coterm_ssh_write_file", args.clone()).await?;
    assert_eq!(
        proposal["_meta"]["coterm/operation"]["status"], "awaiting_approval",
        "{proposal}"
    );
    assert!(!directory.join("config.txt").exists());
    let operation = &proposal["_meta"]["coterm/operation"];
    let approved = runtime
        .approve(
            human.clone(),
            ApproveProposal {
                operation_id: operation["operationId"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("no operation"))?
                    .to_owned(),
                proposal_id: operation["proposalId"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("no proposal"))?
                    .to_owned(),
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                args,
            },
        )
        .await?;
    assert_eq!(approved.status, OperationStatus::Completed, "{approved:?}");
    assert_eq!(
        std::fs::read_to_string(directory.join("config.txt"))?,
        content
    );
    let read = client
        .rpc(
            "tools/call",
            json!({
                "name":"coterm_ssh_read_file", "arguments":{"name":instance.label,"path":path},
                "_meta":{"coterm/waitTimeoutMs":5000}
            }),
        )
        .await?;
    assert_eq!(read["structuredContent"]["text"], content, "{read}");
    runtime
        .set_approval_policy(
            &instance.driver_instance_id,
            coterm_core::approval_policy::ApprovalPolicy {
                mode: coterm_core::approval_policy::ApprovalMode::AllowMatches,
                patterns: vec![r"(?s).*".to_owned()],
            },
        )
        .await?;
    let result = client
        .rpc(
            "tools/call",
            json!({
                "name":"coterm_ssh_command",
                "arguments":{"name":instance.label,"mode":"exec","commands":[
                    format!("cd '{}'\nexport COTERM_VALUE='a b'", remote.replace('\'', "'\\''")),
                    format!("cat <<'EOF' > composed.conf\n{content}EOF"),
                    "printf '\\033[32m%s\\033[0m\\n' \"$COTERM_VALUE\"; printf 'stderr-ok' >&2",
                    "false", "printf never > should-not-exist"
                ]}, "_meta":{"coterm/waitTimeoutMs":5000}
            }),
        )
        .await?;
    assert_eq!(result["structuredContent"]["exitCode"], 1, "{result}");
    assert_eq!(result["isError"], true);
    assert_eq!(
        result["structuredContent"]["stdout"],
        "\x1b[32ma b\x1b[0m\n"
    );
    assert_eq!(result["structuredContent"]["stderr"], "stderr-ok");
    assert!(result["content"][0]["text"]
        .as_str()
        .is_some_and(|text| text.contains("a b") && text.contains("stderr-ok")));
    assert_eq!(
        std::fs::read_to_string(directory.join("composed.conf"))?,
        content
    );
    assert!(!directory.join("should-not-exist").exists());
    // Worst-case quote expansion must still travel over stdin, not an OS argv.
    let prefix = "printf boundary-64k\n#";
    let script = format!("{prefix}{}", "'".repeat(65536 - prefix.len()));
    let boundary = client
        .rpc(
            "tools/call",
            json!({
                "name":"coterm_ssh_command",
                "arguments":{"name":instance.label,"mode":"exec","commands":[script]},
                "_meta":{"coterm/waitTimeoutMs":5000}
            }),
        )
        .await?;
    assert_eq!(boundary["structuredContent"]["exitCode"], 0, "{boundary}");
    assert_eq!(boundary["structuredContent"]["stdout"], "boundary-64k");
    runtime.stop("coterm-driver-ssh").await?;
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}

#[tokio::test]
async fn sudo_uses_one_desktop_secret_after_approval_without_leaking_or_replaying_writes(
) -> Result<()> {
    use coterm_core::driver_runtime::{CallerContext, Operations, DriverRuntime, SubmitOperation};
    use coterm_proto::runtime::{
        interaction::InteractionKind,
        operation::{ApproveProposal, OperationSource, OperationStatus},
    };
    let ssh = server_mode(true).await?;
    let (runtime, instance) = load_runtime(&ssh, "sudo").await?;
    connect_instance(&runtime, &instance, &ssh).await?;
    let mut events = runtime
        .subscribe_events(&instance.driver_instance_id)
        .await?;
    let human = CallerContext {
        source: OperationSource::Human,
        session_id: instance.session_id.clone(),
        driver_id: Some(instance.driver_id.clone()),
    };
    let agent = CallerContext {
        source: OperationSource::Agent,
        ..human.clone()
    };
    let directory =
        std::env::temp_dir().join(format!("coterm-sudo-fixture-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory)?;
    let path = directory
        .join("protected.txt")
        .to_string_lossy()
        .replace('\\', "/");
    let call = |operation: &str, args: Value| SubmitOperation {
        driver_id: instance.driver_id.clone(),
        driver_instance_id: Some(instance.driver_instance_id.clone()),
        operation: operation.to_owned(),
        args,
        wait_timeout_ms: Some(0),
    };
    let args = json!({"driverInstanceId":instance.driver_instance_id,"path":path,"content":"first\n","sudo":true});
    let receipt = runtime
        .submit(agent.clone(), call("mcp.ssh.write_file", args.clone()))
        .await?;
    assert!(runtime.pending_interactions().await.is_empty());
    assert!(!directory.join("protected.txt").exists());
    let record = timeout(Duration::from_secs(3), async {
        loop {
            let record = runtime.get(human.clone(), &receipt.operation_id).await?;
            if record.status == OperationStatus::AwaitingApproval {
                return Ok::<_, anyhow::Error>(record);
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    runtime
        .approve(
            human.clone(),
            ApproveProposal {
                operation_id: receipt.operation_id.clone(),
                proposal_id: record
                    .proposal_id
                    .ok_or_else(|| anyhow::anyhow!("no proposal"))?,
                driver_instance_id: Some(instance.driver_instance_id.clone()),
                args,
            },
        )
        .await?;
    let requests = runtime.pending_interactions().await;
    assert_eq!(requests.len(), 1);
    assert!(
        matches!(&requests[0].kind, InteractionKind::Secret { purpose } if purpose == "ssh.sudo")
    );
    assert!(!directory.join("protected.txt").exists());
    runtime
        .respond_secret(
            &requests[0].interaction_id,
            Some("local-sudo-password".to_owned()),
        )
        .await?;
    let record = wait_for_test_operation(&runtime, human.clone(), &receipt.operation_id).await?;
    assert_eq!(record.status, OperationStatus::Completed, "{record:?}");
    assert!(!serde_json::to_string(&record)?.contains("local-sudo-password"));
    assert_eq!(
        std::fs::read_to_string(directory.join("protected.txt"))?,
        "first\n"
    );
    assert!(runtime
        .respond_secret(
            &requests[0].interaction_id,
            Some("local-sudo-password".to_owned())
        )
        .await
        .is_err());
    // Bypass skips the operation approval, never the separate sudo secret interaction.
    runtime
        .set_bypass(&instance.driver_instance_id, true)
        .await?;
    for (operation, args, secret, expected) in [
        (
            "mcp.ssh.read_file",
            json!({"driverInstanceId":instance.driver_instance_id,"path":path,"sudo":true}),
            Some("local-sudo-password"),
            OperationStatus::Completed,
        ),
        (
            "mcp.ssh.command",
            json!({"driverInstanceId":instance.driver_instance_id,"commands":["printf elevated"],"mode":"exec","sudo":true}),
            Some("local-sudo-password"),
            OperationStatus::Completed,
        ),
        (
            "mcp.ssh.command",
            json!({"driverInstanceId":instance.driver_instance_id,"commands":[format!("printf changed >> '{}'", path.replace('\'', "'\\''"))],"mode":"exec","sudo":true}),
            Some("wrong-sudo-password"),
            OperationStatus::Unknown,
        ),
        (
            "mcp.ssh.write_file",
            json!({"driverInstanceId":instance.driver_instance_id,"path":path,"content":"should-not-write","sudo":true}),
            None,
            OperationStatus::Cancelled,
        ),
    ] {
        let receipt = runtime.submit(agent.clone(), call(operation, args)).await?;
        let request = timeout(Duration::from_secs(3), async {
            loop {
                if let Some(request) = runtime
                    .pending_interactions()
                    .await
                    .into_iter()
                    .find(|r| r.operation_id.as_deref() == Some(&receipt.operation_id))
                {
                    return request;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        runtime
            .respond_secret(&request.interaction_id, secret.map(str::to_owned))
            .await?;
        let record =
            wait_for_test_operation(&runtime, human.clone(), &receipt.operation_id).await?;
        assert_eq!(record.status, expected, "{record:?}");
        let serialized = serde_json::to_string(&record)?;
        assert!(
            !serialized.contains("local-sudo-password")
                && !serialized.contains("wrong-sudo-password")
        );
        if operation == "mcp.ssh.command" && expected == OperationStatus::Completed {
            let output = record
                .output
                .ok_or_else(|| anyhow::anyhow!("no exec result"))?;
            assert_eq!(
                output["structuredContent"]["exitCode"],
                if secret == Some("wrong-sudo-password") {
                    1
                } else {
                    0
                },
                "{output}"
            );
        }
    }
    let nopasswd = directory
        .join("NOPASSWD-fixture")
        .to_string_lossy()
        .replace('\\', "/");
    let receipt = runtime.submit(agent.clone(), call("mcp.ssh.write_file", json!({"driverInstanceId":instance.driver_instance_id,"path":nopasswd,"content":"exact bytes\n","sudo":true}))).await?;
    let request = timeout(Duration::from_secs(3), async {
        loop {
            if let Some(request) = runtime
                .pending_interactions()
                .await
                .into_iter()
                .find(|r| r.operation_id.as_deref() == Some(&receipt.operation_id))
            {
                return request;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    runtime
        .respond_secret(
            &request.interaction_id,
            Some("must-not-be-in-file".to_owned()),
        )
        .await?;
    let record = wait_for_test_operation(&runtime, human.clone(), &receipt.operation_id).await?;
    assert_eq!(record.status, OperationStatus::Completed, "{record:?}");
    assert_eq!(
        std::fs::read_to_string(directory.join("NOPASSWD-fixture"))?,
        "exact bytes\n"
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("protected.txt"))?,
        "first\n"
    );
    let late = runtime.submit(agent.clone(), call("mcp.ssh.write_file", json!({"driverInstanceId":instance.driver_instance_id,"path":path,"content":"late-write","sudo":true}))).await?;
    let pending = timeout(Duration::from_secs(3), async {
        loop {
            if let Some(request) = runtime
                .pending_interactions()
                .await
                .into_iter()
                .find(|r| r.operation_id.as_deref() == Some(&late.operation_id))
            {
                return request;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    let disconnected = runtime
        .submit(human.clone(), call("ssh.disconnect", json!({})))
        .await?;
    let disconnected =
        wait_for_test_operation(&runtime, human.clone(), &disconnected.operation_id).await?;
    assert_eq!(disconnected.status, OperationStatus::Completed);
    connect_instance(&runtime, &instance, &ssh).await?;
    runtime
        .respond_secret(
            &pending.interaction_id,
            Some("local-sudo-password".to_owned()),
        )
        .await?;
    let record = wait_for_test_operation(&runtime, human.clone(), &late.operation_id).await?;
    assert_eq!(record.status, OperationStatus::Failed, "{record:?}");
    assert_eq!(
        std::fs::read_to_string(directory.join("protected.txt"))?,
        "first\n"
    );
    while let Ok(event) = events.try_recv() {
        let serialized = serde_json::to_string(&event)?;
        assert!(
            !serialized.contains("local-sudo-password")
                && !serialized.contains("wrong-sudo-password")
        );
    }
    runtime.stop("coterm-driver-ssh").await?;
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}
