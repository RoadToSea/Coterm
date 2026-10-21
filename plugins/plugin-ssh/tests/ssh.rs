use std::{sync::Arc, time::Duration};

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::Utc;
use coterm_plugin_ssh::{
    backend::Backend,
    session::{self, Config, Connection, OutputBuffer},
};
use russh::{
    server::{self, Session},
    Channel, ChannelId, Pty,
};
use serde_json::{json, Value};
use tokio::{
    net::TcpListener,
    task::JoinHandle,
    time::{sleep, timeout},
};

#[tokio::test]
async fn process_host_routes_approval_snapshot_and_hides_terminal_input() -> Result<()> {
    use coterm_core::{
        plugin_manager::ProcessPluginRuntime,
        plugin_runtime::{CallerContext, Operations, PluginRuntime, SubmitOperation},
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
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_coterm-plugin-ssh"));
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
    let runtime = ProcessPluginRuntime::new(
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
    runtime.start("coterm-plugin-ssh").await?;
    let instance = runtime
        .create_instance("coterm-plugin-ssh", serde_json::to_value(&server.config)?)
        .await?;
    assert_eq!(
        instance.label,
        format!("tester@127.0.0.1:{}", server.config.port)
    );
    assert!(runtime
        .frontend_source("coterm-plugin-ssh")
        .await?
        .is_some());
    let caller = |source| CallerContext {
        source,
        session_id: instance.session_id.clone(),
    };
    let operation = |name: &str, args: Value| SubmitOperation {
        plugin_id: instance.plugin_id.clone(),
        plugin_instance_id: Some(instance.plugin_instance_id.clone()),
        operation: name.to_owned(),
        args,
        wait_timeout_ms: if name == "ssh.command" {
            Some(100)
        } else {
            None
        },
    };
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
            &instance.plugin_instance_id,
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
                plugin_instance_id: Some(instance.plugin_instance_id.clone()),
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
                plugin_instance_id: Some(instance.plugin_instance_id.clone()),
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
                return Ok::<(), anyhow::Error>(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    runtime.stop("coterm-plugin-ssh").await?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

struct Echo;

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
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let source = String::from_utf8_lossy(data);
        if let Some(rest) = source.strip_prefix("printf '\\033]777;coterm;") {
            let token = rest.split(';').next().ok_or(russh::Error::Disconnect)?;
            let text = source
                .split("; eval '")
                .nth(1)
                .and_then(|s| s.split("'; __coterm_rc=").next())
                .unwrap_or("")
                .replace("'\\''", "'");
            let begin = format!("\x1b]777;coterm;{token};begin\x07");
            session.data(channel, begin.into_bytes())?;
            session.data(channel, format!("{text}\r\n").into_bytes())?;
            let code = if text.contains("false") { 1 } else { 0 };
            let end = format!("\x1b]777;coterm;{token};end;{code}\x07");
            if !text.contains("never-completes") {
                session.data(channel, end.into_bytes())?;
            }
        } else {
            session.data(channel, data.to_vec())?;
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
                if let Ok(session) = server::run_stream(config, socket, Echo).await {
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
    json!({"instance":{"pluginId":"coterm-plugin-ssh","pluginInstanceId":id,"sessionId":format!("session-{id}"),"label":id,"status":"starting"},"config":config})
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
    let handle = json!({"id":"test-secret","pluginInstanceId":"one","purpose":"ssh.password","expiresAt":expires});
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
                  dispatch: Value| json!({"operationId":"op","pluginInstanceId":id,"source":source,"effect":effect,"operation":operation,"args":args,"dispatch":dispatch});
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
    assert!(backend.request("approve",json!({"proposalId":"p","operationId":"op","pluginInstanceId":"two","args":{"text":"modified"}})).await.is_err());
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
}

impl McpWire {
    async fn connect(info: coterm_mcp::server::McpConnectionInfo) -> Result<Self> {
        use coterm_mcp::transport::{read_frame, write_frame};
        let socket = tokio::net::TcpStream::connect(info.address).await?;
        let (reader, writer) = socket.into_split();
        let mut client = Self {
            reader: tokio::io::BufReader::new(reader),
            writer,
            next_id: 0,
        };
        write_frame(&mut client.writer, &json!({"token":info.token})).await?;
        let auth = read_frame(&mut client.reader)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Missing auth"))?;
        assert_eq!(
            serde_json::from_slice::<Value>(&auth)?["authenticated"],
            true
        );
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
        plugin_manager::ProcessPluginRuntime,
        plugin_runtime::{CallerContext, Operations, PluginRuntime, SubmitOperation},
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
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_coterm-plugin-ssh"));
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
    let runtime = Arc::new(ProcessPluginRuntime::new(
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
    runtime.start("coterm-plugin-ssh").await?;
    let instance = runtime
        .create_instance("coterm-plugin-ssh", serde_json::to_value(&ssh.config)?)
        .await?;
    let other = runtime
        .create_instance("coterm-plugin-ssh", serde_json::to_value(&ssh.config)?)
        .await?;
    let bridge = coterm_mcp::server::BridgeServer::bind(runtime.clone()).await?;
    let mut client = McpWire::connect(bridge.connection_info()).await?;
    let tools = client.rpc("tools/list", json!({})).await?;
    let tools = tools["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing tools"))?;
    assert!(tools
        .iter()
        .any(|t| t["name"] == "coterm-plugin-ssh__ssh.command"));
    assert!(!tools
        .iter()
        .any(|t| t["name"] == "coterm-plugin-ssh__ssh.input"));
    let connect_args =
        json!({"pluginInstanceId":instance.plugin_instance_id,"fingerprint":ssh.fingerprint});
    let proposed = client
        .tool("coterm-plugin-ssh__ssh.connect", connect_args.clone())
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
    };
    let approved = runtime
        .approve(
            human(),
            ApproveProposal {
                operation_id: connect_id.clone(),
                proposal_id,
                plugin_instance_id: Some(instance.plugin_instance_id.clone()),
                args: connect_args,
            },
        )
        .await?;
    assert_eq!(approved.status, OperationStatus::Running);
    let requests = runtime.pending_interactions().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].plugin_instance_id, instance.plugin_instance_id);
    let status = client
        .tool(
            "coterm-plugin-ssh__ssh.status",
            json!({"pluginInstanceId":instance.plugin_instance_id}),
        )
        .await?;
    let domain: Value = serde_json::from_str(
        status["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing status"))?,
    )?;
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
            json!({"pluginInstanceId":instance.plugin_instance_id,"operationId":connect_id}),
        )
        .await?;
    assert_eq!(completed["structuredContent"]["status"], "completed");
    assert!(!completed.to_string().contains("local-test-password"));
    let mismatched = client
        .tool(
            "coterm.get_operation",
            json!({"pluginInstanceId":other.plugin_instance_id,"operationId":connect_id}),
        )
        .await?;
    assert_eq!(mismatched["isError"], true);
    // The original MCP request waits across approval and returns the plugin result.
    // A second call on the SAME connection must still be served while it waits.
    let mut pushed = runtime
        .subscribe_events(&instance.plugin_instance_id)
        .await?;
    client.next_id += 1;
    let waiting_id = client.next_id;
    coterm_mcp::transport::write_frame(&mut client.writer, &json!({
        "jsonrpc":"2.0","id":waiting_id,"method":"tools/call","params":{
            "name":"coterm-plugin-ssh__ssh.command","arguments":{"pluginInstanceId":instance.plugin_instance_id,"text":"original-waiting-command"}
        }
    })).await?;
    assert!(timeout(
        Duration::from_millis(50),
        coterm_mcp::transport::read_frame(&mut client.reader)
    )
    .await
    .is_err());
    let status = client
        .tool(
            "coterm-plugin-ssh__ssh.status",
            json!({"pluginInstanceId":instance.plugin_instance_id}),
        )
        .await?;
    assert_eq!(status["isError"], false);
    let pending_record = runtime
        .recent(human(), &instance.plugin_instance_id, 100)
        .await?
        .into_iter()
        .find(|r| r.status == OperationStatus::AwaitingApproval)
        .ok_or_else(|| anyhow::anyhow!("No awaiting command"))?;
    let result_id = pending_record.operation_id.clone();
    runtime.approve(human(), ApproveProposal {
        operation_id:result_id.clone(), proposal_id:pending_record.proposal_id.ok_or_else(|| anyhow::anyhow!("No proposal"))?,
        plugin_instance_id:Some(instance.plugin_instance_id.clone()),
        args:json!({"pluginInstanceId":instance.plugin_instance_id,"text":"approved-default-wait false"}),
    }).await?;
    let frame = timeout(
        Duration::from_secs(3),
        coterm_mcp::transport::read_frame(&mut client.reader),
    )
    .await??
    .ok_or_else(|| anyhow::anyhow!("No waiting response"))?;
    let response: Value = serde_json::from_slice(&frame)?;
    assert_eq!(response["id"], waiting_id);
    assert_eq!(
        response["result"]["_meta"]["coterm/operation"]["status"],
        "completed"
    );
    let domain: Value = serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No command result"))?,
    )?;
    assert_eq!(domain["exitCode"], 1);
    assert_eq!(response["result"]["isError"], true);
    let captured = STANDARD.decode(domain["data"].as_str().unwrap_or_default())?;
    assert_eq!(captured, b"approved-default-wait false\r\n");
    // HTTP clients use only a URL, sharing this same authenticated SSH instance.
    let http = coterm_mcp::http::HttpBridgeServer::bind_at(runtime.clone(), "127.0.0.1:0".parse()?)
        .await?;
    let url = http.connection_info().url;
    let http_client = reqwest::Client::builder().no_proxy().build()?;
    let initialized = http_client.post(&url)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","clientInfo":{"name":"ssh-http-test","version":"1"},"capabilities":{}
        }})).send().await?;
    assert_eq!(initialized.status(), reqwest::StatusCode::OK);
    let http_session = initialized
        .headers()
        .get("Mcp-Session-Id")
        .ok_or_else(|| anyhow::anyhow!("No HTTP session"))?
        .to_str()?
        .to_owned();
    let http_request = |message: Value| {
        http_client
            .post(&url)
            .header("Accept", "application/json, text/event-stream")
            .header("Mcp-Session-Id", &http_session)
            .header("MCP-Protocol-Version", "2025-06-18")
            .json(&message)
    };
    assert_eq!(
        http_request(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .send()
            .await?
            .status(),
        reqwest::StatusCode::ACCEPTED
    );
    let waiting = http_request(json!({"jsonrpc":"2.0","id":"http-command","method":"tools/call","params":{
        "name":"coterm-plugin-ssh__ssh.command","arguments":{"pluginInstanceId":instance.plugin_instance_id,"text":"http-original-command"}
    }})).send();
    tokio::pin!(waiting);
    assert!(timeout(Duration::from_millis(50), &mut waiting)
        .await
        .is_err());
    let status: Value = http_request(json!({"jsonrpc":"2.0","id":"http-status","method":"tools/call","params":{
        "name":"coterm-plugin-ssh__ssh.status","arguments":{"pluginInstanceId":instance.plugin_instance_id}
    }})).send().await?.json().await?;
    assert_eq!(status["result"]["isError"], false);
    let proposal = runtime
        .recent(human(), &instance.plugin_instance_id, 100)
        .await?
        .into_iter()
        .find(|r| r.status == OperationStatus::AwaitingApproval)
        .ok_or_else(|| anyhow::anyhow!("No HTTP approval proposal"))?;
    runtime.approve(human(), ApproveProposal {
        operation_id: proposal.operation_id,
        proposal_id: proposal.proposal_id.ok_or_else(|| anyhow::anyhow!("No HTTP proposal ID"))?,
        plugin_instance_id: Some(instance.plugin_instance_id.clone()),
        args: json!({"pluginInstanceId":instance.plugin_instance_id,"text":"http-human-approved false"}),
    }).await?;
    let response: Value = timeout(Duration::from_secs(3), waiting)
        .await??
        .json()
        .await?;
    assert_eq!(response["id"], "http-command");
    assert_eq!(response["result"]["isError"], true);
    assert_eq!(
        response["result"]["_meta"]["coterm/operation"]["status"],
        "completed"
    );
    let domain: Value = serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No HTTP command output"))?,
    )?;
    assert_eq!(domain["exitCode"], 1);
    assert_eq!(
        STANDARD.decode(domain["data"].as_str().unwrap_or_default())?,
        b"http-human-approved false\r\n"
    );
    // Closing the HTTP session cannot approve, cancel, or replay a pending write.
    let pending: Value = http_request(json!({"jsonrpc":"2.0","id":"http-pending","method":"tools/call","params":{
        "name":"coterm-plugin-ssh__ssh.command","arguments":{"pluginInstanceId":instance.plugin_instance_id,"text":"http-must-not-replay"},
        "_meta":{"coterm/waitTimeoutMs":0}
    }})).send().await?.json().await?;
    let pending_id = pending["result"]["_meta"]["coterm/operation"]["operationId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No immediate HTTP receipt"))?;
    assert_eq!(
        pending["result"]["_meta"]["coterm/operation"]["status"],
        "accepted"
    );
    assert_eq!(
        http_client
            .delete(&url)
            .header("Mcp-Session-Id", &http_session)
            .header("MCP-Protocol-Version", "2025-06-18")
            .send()
            .await?
            .status(),
        reqwest::StatusCode::NO_CONTENT
    );
    let record = runtime.get(human(), pending_id).await?;
    assert!(matches!(
        record.status,
        OperationStatus::Accepted | OperationStatus::AwaitingApproval
    ));
    runtime.cancel(human(), pending_id).await?;
    http.shutdown().await;
    // Visible data is pushed without any ssh.read calls and contains no marker.
    let visible = timeout(Duration::from_secs(3), async {
        let mut visible = Vec::new();
        while let Ok(notification) = pushed.recv().await {
            if let coterm_proto::runtime::message::PluginNotification::Event {
                name, payload, ..
            } = notification
            {
                if name == "ssh.output" {
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
                    plugin_id: instance.plugin_id.clone(),
                    plugin_instance_id: Some(instance.plugin_instance_id.clone()),
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
        .recent(human(), &instance.plugin_instance_id, 100)
        .await?
        .iter()
        .all(|r| r.operation != "ssh.status"));
    let mut slow = runtime
        .subscribe_events(&instance.plugin_instance_id)
        .await?;
    for _ in 0..80 {
        runtime
            .submit(
                human(),
                SubmitOperation {
                    plugin_id: instance.plugin_id.clone(),
                    plugin_instance_id: Some(instance.plugin_instance_id.clone()),
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
    let immediate = client.rpc("tools/call", json!({"name":"coterm-plugin-ssh__ssh.command", "arguments":{
        "pluginInstanceId":instance.plugin_instance_id,"text":"must-not-run"},"_meta":{"coterm/waitTimeoutMs":0}})).await?;
    assert_eq!(immediate["_meta"]["coterm/operation"]["status"], "accepted");
    let immediate_id = immediate["_meta"]["coterm/operation"]["operationId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No accepted receipt"))?;
    let cancelled = client
        .tool(
            "coterm.cancel_operation",
            json!({"pluginInstanceId":instance.plugin_instance_id,"operationId":immediate_id}),
        )
        .await?;
    assert_eq!(cancelled["structuredContent"]["status"], "cancelled");
    let pending = client
        .tool(
            "coterm-plugin-ssh__ssh.command",
            json!({"pluginInstanceId":instance.plugin_instance_id,"text":"not-approved-original"}),
        )
        .await?;
    let receipt = &pending["_meta"]["coterm/operation"];
    runtime.approve(human(), ApproveProposal {
        operation_id:receipt["operationId"].as_str().ok_or_else(|| anyhow::anyhow!("Missing command"))?.to_owned(),
        proposal_id:receipt["proposalId"].as_str().ok_or_else(|| anyhow::anyhow!("Missing command proposal"))?.to_owned(),
        plugin_instance_id:Some(instance.plugin_instance_id.clone()),
        args:json!({"pluginInstanceId":instance.plugin_instance_id,"text":"human-approved-edit"}),
    }).await?;
    // Human input goes into exactly the same shell the external Agent uses.
    runtime
        .submit(
            human(),
            SubmitOperation {
                plugin_id: instance.plugin_id.clone(),
                plugin_instance_id: Some(instance.plugin_instance_id.clone()),
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
                    "coterm-plugin-ssh__ssh.read",
                    json!({"pluginInstanceId":instance.plugin_instance_id,"cursor":"0"}),
                )
                .await?;
            let domain: Value = serde_json::from_str(
                output["content"][0]["text"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing output"))?,
            )?;
            let bytes = STANDARD.decode(domain["data"].as_str().unwrap_or_default())?;
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
    runtime.set_bypass(true).await;
    let waiting = client
        .tool(
            "coterm-plugin-ssh__ssh.connect",
            json!({"pluginInstanceId":other.plugin_instance_id,"fingerprint":ssh.fingerprint}),
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
            json!({"pluginInstanceId":other.plugin_instance_id,"operationId":operation_id}),
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
            "coterm-plugin-ssh__ssh.connect",
            json!({"pluginInstanceId":other.plugin_instance_id,"fingerprint":ssh.fingerprint}),
        )
        .await?;
    let destroyed_request = runtime
        .pending_interactions()
        .await
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No destroy-test prompt"))?;
    runtime.destroy_instance(&other.plugin_instance_id).await?;
    assert!(runtime.pending_interactions().await.is_empty());
    assert!(runtime
        .respond_secret(
            &destroyed_request.interaction_id,
            Some("late-after-destroy".to_owned())
        )
        .await
        .is_err());
    let records = runtime
        .recent(human(), &instance.plugin_instance_id, 100)
        .await?;
    assert!(!serde_json::to_string(&records)?.contains("local-test-password"));
    // A disconnected command must release the original waiter with UNKNOWN, not success.
    client.next_id += 1;
    let interrupted_id = client.next_id;
    coterm_mcp::transport::write_frame(&mut client.writer, &json!({"jsonrpc":"2.0","id":interrupted_id,"method":"tools/call","params":{
        "name":"coterm-plugin-ssh__ssh.command","arguments":{"pluginInstanceId":instance.plugin_instance_id,"text":"never-completes"}
    }})).await?;
    timeout(Duration::from_secs(3), async {
        loop {
            if runtime
                .recent(human(), &instance.plugin_instance_id, 100)
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
    runtime
        .submit(
            human(),
            SubmitOperation {
                plugin_id: instance.plugin_id.clone(),
                plugin_instance_id: Some(instance.plugin_instance_id.clone()),
                operation: "ssh.disconnect".into(),
                args: json!({}),
                wait_timeout_ms: None,
            },
        )
        .await?;
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
    runtime.stop("coterm-plugin-ssh").await?;
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
