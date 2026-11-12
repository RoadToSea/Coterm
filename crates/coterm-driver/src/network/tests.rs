use super::{catalog, config::Config, Backend};
use anyhow::{anyhow, Result};
use coterm_core::{
    driver::{CancelTarget, DriverBackend, DriverNotification, HostCommand},
    model::{
        lifecycle::{CreateInstance, InstanceInfo, InstanceStatus},
        operation::{ApproveProposal, DispatchMode, InvokeRequest, OperationSource},
    },
};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::{sleep, timeout},
};

struct Harness<const TELNET: bool> {
    backend: Backend<TELNET>,
    notifications: mpsc::Receiver<DriverNotification>,
    next: usize,
}
impl<const TELNET: bool> Harness<TELNET> {
    fn new() -> Self {
        let mut backend = Backend::default();
        let notifications = backend.take_notifications().unwrap();
        Self {
            backend,
            notifications,
            next: 0,
        }
    }
    async fn create(&mut self, id: &str, config: Value) -> Result<()> {
        self.backend
            .request(HostCommand::CreateInstance(CreateInstance {
                instance: InstanceInfo {
                    driver_id: catalog::driver_id(TELNET).into(),
                    driver_instance_id: id.into(),
                    session_id: format!("session-{id}"),
                    name: id.into(),
                    status: InstanceStatus::Ready,
                },
                config,
            }))
            .await
            .map_err(|e| anyhow!(e.message))?;
        Ok(())
    }
    fn request(&mut self, id: &str, name: &str, args: Value) -> InvokeRequest {
        self.next += 1;
        let operation = format!("{}.{name}", catalog::prefix(TELNET));
        let effect = catalog::catalog(TELNET)
            .into_iter()
            .find(|d| d.name == operation)
            .unwrap()
            .effect;
        InvokeRequest {
            operation_id: format!("operation-{}", self.next),
            driver_instance_id: Some(id.into()),
            operation,
            args,
            effect,
            source: OperationSource::Human,
            dispatch: DispatchMode::Execute,
        }
    }
    async fn call(&mut self, id: &str, name: &str, args: Value) -> Result<Value> {
        let request = self.request(id, name, args);
        self.backend
            .request(HostCommand::Invoke(request))
            .await
            .map_err(|e| anyhow!(e.message))
    }
    async fn completion(&mut self, id: &str) -> Result<Value> {
        timeout(Duration::from_secs(3), async {
            loop {
                match self.notifications.recv().await {
                    Some(DriverNotification::OperationCompleted {
                        operation_id,
                        result,
                    }) if operation_id == id => return Ok(result),
                    Some(DriverNotification::OperationFailed {
                        operation_id,
                        error,
                    }) if operation_id == id => return Err(anyhow!(error.message)),
                    None => return Err(anyhow!("notification stream closed")),
                    _ => {}
                }
            }
        })
        .await?
    }
    async fn connect(&mut self, id: &str, listener: &TcpListener) -> Result<TcpStream> {
        let deferred = self.call(id, "connect", json!({})).await?;
        let (socket, _) = timeout(Duration::from_secs(2), listener.accept()).await??;
        self.completion(deferred["_cotermDeferred"].as_str().unwrap())
            .await?;
        Ok(socket)
    }
    async fn observed(&mut self, id: &str, value: &str) -> Result<Value> {
        timeout(Duration::from_secs(2), async {
            loop {
                let result = self.call(id, "read_lines", json!({"lines":100})).await?;
                if result.to_string().contains(value) {
                    return Ok(result);
                }
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await?
    }
}
async fn listener() -> Result<TcpListener> {
    Ok(TcpListener::bind(("127.0.0.1", 0)).await?)
}
fn config(listener: &TcpListener) -> Result<Value> {
    Ok(json!({"host":"127.0.0.1","port":listener.local_addr()?.port()}))
}

#[test]
fn configuration_and_catalog_do_not_import_ssh_or_uart_semantics() -> Result<()> {
    let tcp = Config::parse(json!({"host":"board.local"}), false)?;
    assert_eq!(tcp.port, 8080);
    assert_eq!(tcp.receive_newlines, "lf");
    let telnet = Config::parse(json!({"host":"::1"}), true)?;
    assert_eq!(telnet.port, 23);
    assert!(!telnet.auto_reconnect);
    for patch in [
        json!({"host":"tcp://board"}),
        json!({"port":0}),
        json!({"password":"secret"}),
        json!({"connectTimeoutMs":99}),
        json!({"proxy":{"type":"http","host":"a\r\nb","port":80}}),
        json!({"username":"root"}),
        json!({"mode":"shell","prompt":""}),
    ] {
        assert!(tcp.patch(&patch, false).is_err(), "{patch}");
    }
    for telnet in [false, true] {
        let definition = catalog::catalog(telnet);
        assert!(definition.iter().all(|d| !d.name.contains("exec")
            && !d.name.contains("file")
            && !d.name.contains("signals")));
        assert!(catalog::mcp_tools(telnet)
            .iter()
            .all(|d| !d.descriptor["inputSchema"].to_string().contains("cursor")));
        assert!(definition
            .iter()
            .filter(|d| d.name.ends_with("command"))
            .all(|d| d.sensitive_input));
    }
    assert!(!catalog::catalog(true)
        .iter()
        .any(|d| d.name.ends_with("command_hex")));
    Ok(())
}

#[tokio::test]
async fn raw_tcp_preserves_bytes_separates_instances_and_keeps_disconnected_reads() -> Result<()> {
    let listener = listener().await?;
    let mut h = Harness::<false>::new();
    h.create("one", config(&listener)?).await?;
    h.create("two", config(&listener)?).await?;
    let mut one = h.connect("one", &listener).await?;
    let mut two = h.connect("two", &listener).await?;
    one.write_all(&[0, 0x80, 0xff, b'\n', 0xe4, 0xb8]).await?;
    one.write_all(&[0xad, b'\n']).await?;
    two.write_all(b"second-only\n").await?;
    h.observed("one", "中").await?;
    let second = h.observed("two", "second-only").await?;
    assert!(!second.to_string().contains("中"));
    let raw = h.call("one", "read_hex", json!({"cursor":0})).await?;
    assert_eq!(raw["hex"], "0080ff0ae4b8ad0a");
    for operation in ["read_selected_lines", "read_dialogue_pairs"] {
        assert!(h.call("one", operation, json!({})).await.is_err());
    }
    let status = h.call("one", "status", json!({})).await?;
    let epoch = status["connectionId"].clone();
    assert!(h
        .call("one", "write", json!({"connectionId":"stale","text":"bad"}))
        .await
        .is_err());
    let mut illegal = h.request(
        "one",
        "write",
        json!({"connectionId":epoch,"text":"unapproved"}),
    );
    illegal.source = OperationSource::Agent;
    assert!(h
        .backend
        .request(HostCommand::Invoke(illegal))
        .await
        .is_err());
    h.call(
        "one",
        "write_hex",
        json!({"connectionId":epoch,"hex":"00 80 ff"}),
    )
    .await?;
    let mut bytes = [0; 3];
    timeout(Duration::from_secs(1), one.read_exact(&mut bytes)).await??;
    assert_eq!(bytes, [0, 128, 255]);
    drop(one);
    timeout(Duration::from_secs(2), async {
        loop {
            if h.call("one", "status", json!({})).await?["connected"] == false {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await??;
    let retained = h.call("one", "read_lines", json!({"lines":5})).await?;
    assert_eq!(retained["closed"], true);
    assert!(retained.to_string().contains("中"));
    assert_eq!(h.call("two", "status", json!({})).await?["connected"], true);
    h.backend
        .request(HostCommand::Shutdown)
        .await
        .map_err(|e| anyhow!(e.message))?;
    Ok(())
}

#[tokio::test]
async fn shell_batch_only_sends_after_approval_and_returns_observed_pairs() -> Result<()> {
    let listener = listener().await?;
    let mut h = Harness::<false>::new();
    let mut cfg = config(&listener)?;
    cfg["mode"] = json!("shell");
    cfg["prompt"] = json!("root# ");
    h.create("board", cfg).await?;
    let mut socket = h.connect("board", &listener).await?;
    socket.write_all(b"root# ").await?;
    h.observed("board", "root#").await?;
    let args = json!({"commands":["first","second"],"intervalMs":0,"waitTimeoutMs":1000});
    let mut request = h.request("board", "command", args.clone());
    let operation = request.operation_id.clone();
    request.source = OperationSource::Agent;
    request.dispatch = DispatchMode::PrepareProposal {
        proposal_id: "proposal".into(),
    };
    let draft = h
        .backend
        .request(HostCommand::Invoke(request))
        .await
        .map_err(|e| anyhow!(e.message))?;
    assert_eq!(draft["args"], args);
    assert_eq!(draft["operation"], "tcp.command");
    let mut byte = [0; 1];
    assert!(timeout(Duration::from_millis(30), socket.read(&mut byte))
        .await
        .is_err());
    h.backend
        .request(HostCommand::Approve(ApproveProposal {
            operation_id: operation.clone(),
            proposal_id: "proposal".into(),
            driver_instance_id: Some("board".into()),
            args,
        }))
        .await
        .map_err(|e| anyhow!(e.message))?;
    let peer = tokio::spawn(async move {
        for command in ["first", "second"] {
            let mut input = vec![0; command.len() + 2];
            socket.read_exact(&mut input).await?;
            assert_eq!(input, format!("{command}\r\n").as_bytes());
            socket
                .write_all(format!("{command}\r\n{command}-result\r\nroot# ").as_bytes())
                .await?;
        }
        Ok::<_, anyhow::Error>(socket)
    });
    let result = h.completion(&operation).await?;
    let _socket = peer.await??;
    assert_eq!(result["stderr"], "");
    assert!(result["exitCode"].is_null());
    assert_eq!(result["streamMode"], "merged");
    assert_eq!(result["deviceCompletion"], "prompt_observed");
    assert!(result["stdout"].as_str().unwrap().contains("second-result"));
    let pairs = h
        .call("board", "read_dialogue_pairs", json!({"count":2}))
        .await?;
    assert_eq!(pairs["pairs"][0]["stdin"], "second");
    assert_eq!(pairs["pairs"][1]["stdin"], "first");
    h.call("board", "disconnect", json!({})).await?;
    Ok(())
}

#[tokio::test]
async fn shell_timeout_stops_remaining_commands_and_keeps_partial_result() -> Result<()> {
    let listener = listener().await?;
    let mut h = Harness::<false>::new();
    let mut cfg = config(&listener)?;
    cfg["mode"] = json!("shell");
    cfg["prompt"] = json!("# ");
    h.create("board", cfg).await?;
    let mut socket = h.connect("board", &listener).await?;
    socket.write_all(b"# ").await?;
    h.observed("board", "#").await?;
    let accepted = h
        .call(
            "board",
            "command",
            json!({"commands":["long","never"],"waitTimeoutMs":100}),
        )
        .await?;
    let mut first = [0; 6];
    socket.read_exact(&mut first).await?;
    assert_eq!(&first, b"long\r\n");
    socket.write_all(b"partial output\n").await?;
    let error = timeout(Duration::from_secs(2), async {
        loop {
            if let Some(DriverNotification::OperationFailed {
                operation_id,
                error,
            }) = h.notifications.recv().await
            {
                if operation_id == accepted["_cotermDeferred"].as_str().unwrap() {
                    return error;
                }
            }
        }
    })
    .await?;
    let details = error.details.unwrap();
    assert_eq!(details["sentCommands"], 1);
    assert_eq!(details["stderr"], "");
    assert!(details["stdout"]
        .as_str()
        .unwrap()
        .contains("partial output"));
    assert_eq!(details["deviceCompletion"], "unknown");
    let mut bytes = [0; 8];
    assert!(timeout(Duration::from_millis(40), socket.read(&mut bytes))
        .await
        .is_err());
    h.call("board", "disconnect", json!({})).await?;
    Ok(())
}

#[tokio::test]
async fn reconnect_has_new_epoch_no_input_replay_and_preserves_interpretation() -> Result<()> {
    let listener = listener().await?;
    let mut h = Harness::<false>::new();
    let mut cfg = config(&listener)?;
    cfg["autoReconnect"] = json!(true);
    cfg["reconnectIntervalMs"] = json!(250);
    h.create("board", cfg).await?;
    let mut first = h.connect("board", &listener).await?;
    let epoch = h.call("board", "status", json!({})).await?["connectionId"].clone();
    h.call(
        "board",
        "write",
        json!({"connectionId":epoch,"text":"once"}),
    )
    .await?;
    let mut bytes = [0; 4];
    first.read_exact(&mut bytes).await?;
    assert_eq!(&bytes, b"once");
    h.call(
        "board",
        "configure",
        json!({"mode":"shell","prompt":"root# "}),
    )
    .await?;
    let mut request = h.request("board", "command", json!({"commands":["stale"]}));
    let operation = request.operation_id.clone();
    request.dispatch = DispatchMode::PrepareProposal {
        proposal_id: "old".into(),
    };
    h.backend
        .request(HostCommand::Invoke(request))
        .await
        .map_err(|e| anyhow!(e.message))?;
    drop(first);
    let (mut next, _) = timeout(Duration::from_secs(2), listener.accept()).await??;
    timeout(Duration::from_secs(1), async {
        loop {
            let status = h.call("board", "status", json!({})).await?;
            if status["connected"] == true && status["connectionId"] != epoch {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await??;
    assert!(h
        .call(
            "board",
            "write",
            json!({"connectionId":epoch,"text":"stale"})
        )
        .await
        .is_err());
    assert!(h
        .backend
        .request(HostCommand::Approve(ApproveProposal {
            operation_id: operation,
            proposal_id: "old".into(),
            driver_instance_id: Some("board".into()),
            args: json!({"commands":["stale"]})
        }))
        .await
        .is_err());
    assert!(timeout(Duration::from_millis(40), next.read(&mut bytes))
        .await
        .is_err());
    next.write_all(b"root# ").await?;
    h.observed("board", "root#").await?;
    let command = h
        .call(
            "board",
            "command",
            json!({"commands":["okay"],"waitTimeoutMs":1000}),
        )
        .await?;
    let mut input = [0; 6];
    next.read_exact(&mut input).await?;
    assert_eq!(&input, b"okay\r\n");
    next.write_all(b"okay\r\nresult\r\nroot# ").await?;
    h.completion(command["_cotermDeferred"].as_str().unwrap())
        .await?;
    h.call("board", "disconnect", json!({})).await?;
    drop(next);
    assert!(timeout(Duration::from_millis(350), listener.accept())
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn telnet_negotiation_username_and_resize_never_enter_received_history() -> Result<()> {
    let listener = listener().await?;
    let mut h = Harness::<true>::new();
    let mut cfg = config(&listener)?;
    cfg["username"] = json!("root");
    cfg["terminalType"] = json!("vt100");
    h.create("board", cfg).await?;
    let mut socket = h.connect("board", &listener).await?;
    let wire = [
        255, 251, 1, 255, 253, 31, 255, 253, 24, 255, 250, 24, 1, 255, 240,
    ];
    for byte in wire {
        socket.write_all(&[byte]).await?;
    }
    socket.write_all(b"device login:").await?;
    let expected = [
        255, 253, 1, 255, 251, 31, 255, 250, 31, 0, 80, 0, 24, 255, 240, 255, 251, 24, 255, 250,
        24, 0, b'v', b't', b'1', b'0', b'0', 255, 240, b'r', b'o', b'o', b't', 13, 10,
    ];
    let mut reply = vec![0; expected.len()];
    timeout(Duration::from_secs(2), socket.read_exact(&mut reply)).await??;
    assert_eq!(reply, expected);
    h.observed("board", "device login:").await?;
    assert_eq!(
        h.call("board", "read_hex", json!({})).await?["hex"],
        "646576696365206c6f67696e3a"
    );
    let status = h.call("board", "status", json!({})).await?;
    assert_eq!(status["remoteEcho"], true);
    h.call(
        "board",
        "resize",
        json!({"connectionId":status["connectionId"],"cols":255,"rows":40}),
    )
    .await?;
    let mut window = [0; 10];
    socket.read_exact(&mut window).await?;
    assert_eq!(window, [255, 250, 31, 0, 255, 255, 0, 40, 255, 240]);
    h.call("board", "disconnect", json!({})).await?;
    Ok(())
}

#[tokio::test]
async fn pending_proxy_connect_is_deferred_cancellable_and_does_not_block_other_instances(
) -> Result<()> {
    let listener = listener().await?;
    let mut h = Harness::<false>::new();
    let cfg = json!({"host":"board.local","proxy":{"type":"socks5","host":"127.0.0.1","port":listener.local_addr()?.port()},
        "autoReconnect":true,"connectTimeoutMs":1000});
    h.create("one", cfg).await?;
    let accepted = h.call("one", "connect", json!({})).await?;
    let (_socket, _) = listener.accept().await?;
    h.create("two", json!({"host":"localhost"})).await?;
    assert_eq!(
        h.call("two", "status", json!({})).await?["connected"],
        false
    );
    let result = h
        .backend
        .request(HostCommand::Cancel(CancelTarget::Operation(
            accepted["_cotermDeferred"].as_str().unwrap().into(),
        )))
        .await
        .map_err(|e| anyhow!(e.message))?;
    assert_eq!(result["outcome"], "stopped");
    assert_eq!(
        h.call("one", "status", json!({})).await?["reconnecting"],
        false
    );
    assert!(timeout(Duration::from_millis(300), listener.accept())
        .await
        .is_err());
    assert!(h
        .completion(accepted["_cotermDeferred"].as_str().unwrap())
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn proxy_deadline_covers_negotiation_and_initial_failure_is_not_retried() -> Result<()> {
    let listener = listener().await?;
    let mut h = Harness::<false>::new();
    h.create(
        "board",
        json!({"host":"board.local","connectTimeoutMs":100,"autoReconnect":true,
        "proxy":{"type":"http","host":"127.0.0.1","port":listener.local_addr()?.port()}}),
    )
    .await?;
    let accepted = h.call("board", "connect", json!({})).await?;
    let (_socket, _) = listener.accept().await?;
    assert!(h
        .completion(accepted["_cotermDeferred"].as_str().unwrap())
        .await
        .unwrap_err()
        .to_string()
        .contains("timed out"));
    assert!(timeout(Duration::from_millis(300), listener.accept())
        .await
        .is_err());
    Ok(())
}

#[tokio::test]
async fn cancel_stops_a_stream_batch_without_replaying_or_claiming_device_completion() -> Result<()>
{
    let listener = listener().await?;
    let mut h = Harness::<false>::new();
    h.create("board", config(&listener)?).await?;
    let mut socket = h.connect("board", &listener).await?;
    let accepted = h
        .call(
            "board",
            "command_hex",
            json!({"commands":["0080ff","deadbeef"],"intervalMs":60000}),
        )
        .await?;
    let mut first = [0; 3];
    timeout(Duration::from_secs(1), socket.read_exact(&mut first)).await??;
    assert_eq!(first, [0, 128, 255]);
    assert!(h
        .call("board", "command", json!({"commands":["busy"]}))
        .await
        .is_err());
    let id = accepted["_cotermDeferred"].as_str().unwrap();
    let outcome = h
        .backend
        .request(HostCommand::Cancel(CancelTarget::Operation(id.into())))
        .await
        .map_err(|e| anyhow!(e.message))?;
    assert_eq!(outcome["outcome"], "stopped");
    assert!(h.completion(id).await.is_err());
    assert!(timeout(Duration::from_millis(40), socket.read(&mut first))
        .await
        .is_err());
    h.call("board", "disconnect", json!({})).await?;
    Ok(())
}

#[tokio::test]
async fn slow_consumers_have_bounded_history_and_explicit_gaps() -> Result<()> {
    let listener = listener().await?;
    let mut h = Harness::<false>::new();
    h.create("board", config(&listener)?).await?;
    let mut socket = h.connect("board", &listener).await?;
    let source = "data\n".repeat(100_000);
    socket.write_all(source.as_bytes()).await?;
    drop(socket);
    timeout(Duration::from_secs(3), async {
        loop {
            if h.call("board", "status", json!({})).await?["connected"] == false {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await??;
    let replay = h.call("board", "read", json!({"cursor":0})).await?;
    assert_eq!(replay["gap"], true);
    assert_eq!(replay["closed"], true);
    assert!(replay["bytes"].as_u64().unwrap() <= 1024);
    let lines = h.call("board", "read_lines", json!({"lines":1000})).await?;
    assert_eq!(lines["dropped"], true);
    assert!(lines.to_string().len() < 12 * 1024);
    h.call("board", "disconnect", json!({})).await?;
    Ok(())
}
