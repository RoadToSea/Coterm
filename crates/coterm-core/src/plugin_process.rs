//! v1.0 JSON-RPC-over-stdio adapter for plugin control requests.
//! Binary streams remain a separately negotiated future extension.

use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::plugin_runtime::PluginConnection;
use coterm_proto::runtime::{
    manifest::PluginManifest,
    message::{HostCommand, HostMessage, PluginEvent, PluginNotification},
    wire::{decode_response, encode_request, MAX_CONTROL_BYTES},
    ErrorCategory, RuntimeError, RuntimeFuture,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{broadcast, mpsc, Mutex},
    time::timeout,
};

const EVENT_QUEUE_CAPACITY: usize = 64;
const MAX_CONTROL_LINE_BYTES: usize = MAX_CONTROL_BYTES;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_FRONTEND_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct PluginPackage {
    pub root: PathBuf,
    pub manifest: PluginManifest,
    pub backend_entry: PathBuf,
    pub frontend_entry: Option<PathBuf>,
}

/// Reads and validates the package manifest and all declared local entry paths.
pub async fn load_plugin_package(
    package_root: &Path,
    platform: &str,
) -> Result<PluginPackage, RuntimeError> {
    let root = package_root
        .canonicalize()
        .map_err(|error| transport_error("plugin_package_unavailable", error))?;
    let manifest_path = root.join("manifest.json");
    let metadata = tokio::fs::metadata(&manifest_path)
        .await
        .map_err(|error| transport_error("plugin_manifest_unavailable", error))?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(protocol_error("plugin_manifest_too_large"));
    }
    let bytes = tokio::fs::read(&manifest_path)
        .await
        .map_err(|error| transport_error("plugin_manifest_read_failed", error))?;
    let manifest: PluginManifest = serde_json::from_slice(&bytes)
        .map_err(|error| protocol_error_with("plugin_manifest_invalid", error))?;
    validate_manifest(&manifest)?;

    let backend_entry = resolve_backend_entry(&root, &manifest, platform)?;
    let frontend_entry = manifest
        .frontend
        .as_ref()
        .map(|frontend| {
            resolve_package_file(&root, &frontend.entry, "plugin_frontend_entry_invalid")
        })
        .transpose()?;

    Ok(PluginPackage {
        root,
        manifest,
        backend_entry,
        frontend_entry,
    })
}

pub(crate) fn validate_manifest(manifest: &PluginManifest) -> Result<(), RuntimeError> {
    if manifest.manifest_version != 1 {
        return Err(protocol_error("plugin_manifest_version_unsupported"));
    }
    if !is_valid_plugin_id(&manifest.id)
        || manifest.name.trim().is_empty()
        || manifest.version.trim().is_empty()
    {
        return Err(protocol_error("plugin_manifest_identity_invalid"));
    }
    if manifest.runtime.interface_major != coterm_proto::runtime::lifecycle::INTERFACE_MAJOR {
        return Err(protocol_error("plugin_runtime_version_unsupported"));
    }
    if manifest.runtime.requires.iter().any(|feature| {
        !coterm_proto::runtime::lifecycle::REQUIRED_FEATURES.contains(&feature.as_str())
            && !coterm_proto::runtime::lifecycle::OPTIONAL_FEATURES.contains(&feature.as_str())
    }) {
        return Err(protocol_error("plugin_required_feature_unsupported"));
    }
    if manifest.instances.instance_type.trim().is_empty() || !manifest.instances.multiple {
        return Err(protocol_error("plugin_instance_declaration_invalid"));
    }
    Ok(())
}

fn is_valid_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(crate) fn resolve_package_file(
    root: &Path,
    relative: &str,
    invalid_code: &str,
) -> Result<PathBuf, RuntimeError> {
    let relative_path = Path::new(relative);
    if relative.trim().is_empty()
        || relative_path.is_absolute()
        || relative_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(protocol_error(invalid_code));
    }
    let entry = root
        .join(relative_path)
        .canonicalize()
        .map_err(|error| transport_error("plugin_entry_unavailable", error))?;
    if !entry.starts_with(root) || !entry.is_file() {
        return Err(protocol_error(invalid_code));
    }
    Ok(entry)
}

pub struct JsonRpcPluginProcess {
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<mpsc::Receiver<Result<PluginEvent, RuntimeError>>>,
    child: Mutex<Child>,
    request_gate: Mutex<()>,
    completions: Mutex<Option<mpsc::Receiver<PluginNotification>>>,
    pub events: broadcast::Sender<PluginNotification>,
    control_limit: Arc<AtomicUsize>,
}

impl JsonRpcPluginProcess {
    /// Starts a previously validated executable. stdout is reserved for JSON-RPC;
    /// stderr is inherited for plugin diagnostics.
    pub async fn spawn(executable: &Path, args: &[String]) -> Result<Self, RuntimeError> {
        let mut command = Command::new(executable);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            // A GUI host must not open a console for its stdio-only plugin backend.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .spawn()
            .map_err(|error| transport_error("plugin_start_failed", error))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| protocol_error("plugin_stdin_unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| protocol_error("plugin_stdout_unavailable"))?;
        let (sender, receiver) = mpsc::channel(EVENT_QUEUE_CAPACITY);
        let (completed, completions) = mpsc::channel(EVENT_QUEUE_CAPACITY);
        let (events, _) = broadcast::channel(EVENT_QUEUE_CAPACITY);
        let control_limit = Arc::new(AtomicUsize::new(MAX_CONTROL_BYTES));
        tokio::spawn(read_responses(
            stdout,
            sender,
            completed,
            events.clone(),
            control_limit.clone(),
        ));

        Ok(Self {
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(receiver),
            child: Mutex::new(child),
            request_gate: Mutex::new(()),
            completions: Mutex::new(Some(completions)),
            events,
            control_limit,
        })
    }

    /// 只在 initialize 成功且结果已校验后调用。
    pub fn set_control_limit(&self, bytes: u32) {
        self.control_limit.store(bytes as usize, Ordering::Release);
    }

    pub async fn take_completions(&self) -> Option<mpsc::Receiver<PluginNotification>> {
        self.completions.lock().await.take()
    }

    /// Sends one control request and waits for its matching response. The MVP
    /// adapter permits one in-flight control request. Its reader routes instance
    /// events and deferred completions independently of control replies.
    pub async fn request(&self, command: HostCommand) -> Result<Value, RuntimeError> {
        let _request_guard = self.request_gate.lock().await;
        let request_id = uuid::Uuid::new_v4().to_string();
        self.send(HostMessage {
            request_id: request_id.clone(),
            command,
        })
        .await?;

        let response = match timeout(DEFAULT_REQUEST_TIMEOUT, self.receive()).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                let _ = self.close().await;
                return Err(error);
            }
            Err(_) => {
                let _ = self.close().await;
                return Err(protocol_error("plugin_request_timed_out"));
            }
        };
        match response {
            Some(PluginEvent::Reply {
                request_id: response_id,
                result,
            }) if response_id == request_id => result,
            other => {
                let _ = self.close().await;
                Err(protocol_error(match other {
                    Some(PluginEvent::Reply { .. }) => "plugin_response_id_mismatch",
                    Some(_) => "plugin_unsolicited_event_unsupported",
                    None => "plugin_exited_before_response",
                }))
            }
        }
    }

    pub async fn process_id(&self) -> Option<u32> {
        self.child.lock().await.id()
    }
}

impl super::plugin_runtime::PluginConnection for JsonRpcPluginProcess {
    fn send<'a>(&'a self, message: HostMessage) -> RuntimeFuture<'a, ()> {
        Box::pin(async move {
            let encoded = encode_request(message, self.control_limit.load(Ordering::Acquire))?;
            let mut stdin = self.stdin.lock().await;
            stdin
                .write_all(&encoded)
                .await
                .map_err(|error| transport_error("plugin_stdin_write_failed", error))?;
            stdin
                .flush()
                .await
                .map_err(|error| transport_error("plugin_stdin_flush_failed", error))
        })
    }

    fn receive<'a>(&'a self) -> RuntimeFuture<'a, Option<PluginEvent>> {
        Box::pin(async move {
            match self.stdout.lock().await.recv().await {
                Some(event) => event.map(Some),
                None => Ok(None),
            }
        })
    }

    fn close<'a>(&'a self) -> RuntimeFuture<'a, ()> {
        Box::pin(async move {
            let mut child = self.child.lock().await;
            if child
                .try_wait()
                .map_err(|error| transport_error("plugin_wait_failed", error))?
                .is_none()
            {
                child
                    .start_kill()
                    .map_err(|error| transport_error("plugin_stop_failed", error))?;
                child
                    .wait()
                    .await
                    .map_err(|error| transport_error("plugin_wait_failed", error))?;
            }
            Ok(())
        })
    }
}

/// Resolves the platform-specific backend entry without allowing it to escape the package.
pub fn resolve_backend_entry(
    package_root: &Path,
    manifest: &coterm_proto::runtime::manifest::PluginManifest,
    platform: &str,
) -> Result<PathBuf, RuntimeError> {
    let relative = manifest
        .backend
        .get(platform)
        .ok_or_else(|| protocol_error("plugin_platform_unsupported"))?;
    let root = package_root
        .canonicalize()
        .map_err(|error| transport_error("plugin_package_unavailable", error))?;
    resolve_package_file(&root, relative, "plugin_entry_invalid")
}

async fn read_responses(
    stdout: ChildStdout,
    sender: mpsc::Sender<Result<PluginEvent, RuntimeError>>,
    completed: mpsc::Sender<PluginNotification>,
    events: broadcast::Sender<PluginNotification>,
    control_limit: Arc<AtomicUsize>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let line = match read_bounded_line(&mut reader, MAX_CONTROL_LINE_BYTES).await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                let _ = sender
                    .send(Err(transport_error("plugin_stdout_read_failed", error)))
                    .await;
                break;
            }
        };
        if line.len() > control_limit.load(Ordering::Acquire) {
            let _ = sender
                .send(Err(protocol_error("plugin_control_too_large")))
                .await;
            break;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            if value["jsonrpc"] == "2.0"
                && value.get("id").is_none()
                && value["method"] == "coterm.notification"
            {
                match serde_json::from_value::<PluginNotification>(value["params"].clone()) {
                    Ok(event @ PluginNotification::Event { .. }) => {
                        let _ = events.send(event);
                    }
                    Ok(event) => {
                        if completed.try_send(event).is_err() {
                            let _ = sender
                                .send(Err(protocol_error("plugin_completion_queue_overflow")))
                                .await;
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = sender
                            .send(Err(protocol_error("plugin_notification_invalid")))
                            .await;
                        break;
                    }
                }
                continue;
            }
        }
        let event = match decode_response(&line) {
            Ok(event) => event,
            Err(error) => {
                let _ = sender.send(Err(error)).await;
                break;
            }
        };
        if sender.send(Ok(event)).await.is_err() {
            break;
        }
    }
    // Every consumer receives an explicit close, even though the process owns a sender.
    let _ = events.send(PluginNotification::Event {
        plugin_instance_id: String::new(),
        name: "runtime.closed".to_owned(),
        payload: Value::Null,
    });
}

async fn read_bounded_line<R>(reader: &mut R, max_bytes: usize) -> io::Result<Option<String>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unterminated plugin control line",
            ));
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if line.len().saturating_add(consumed) > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "plugin control line exceeds size limit",
            ));
        }
        line.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }

    // Keep the terminator so the negotiated byte limit includes LF / CRLF.
    String::from_utf8(line)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn protocol_error(code: &str) -> RuntimeError {
    RuntimeError {
        category: ErrorCategory::Transport,
        code: code.to_owned(),
        message: code.replace('_', " "),
        details: None,
    }
}

fn protocol_error_with(code: &str, error: impl std::fmt::Display) -> RuntimeError {
    let mut result = protocol_error(code);
    result.details = Some(json!({ "cause": error.to_string() }));
    result
}

fn transport_error(code: &str, error: io::Error) -> RuntimeError {
    protocol_error_with(code, error)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use coterm_proto::runtime::message::PluginEvent;
    use tokio::io::BufReader;

    use super::{decode_response, read_bounded_line, resolve_backend_entry, validate_manifest};

    #[tokio::test]
    async fn bounds_control_lines_and_rejects_truncated_eof() {
        let mut reader = BufReader::new(&b"12345\n"[..]);
        let error = read_bounded_line(&mut reader, 4).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        let mut reader = BufReader::new(&b"{}"[..]);
        assert_eq!(
            read_bounded_line(&mut reader, 2).await.unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        assert_eq!(read_bounded_line(&mut reader, 2).await.unwrap(), None);
        let mut reader = BufReader::new(&b"{}\r\n"[..]);
        assert_eq!(
            read_bounded_line(&mut reader, 4).await.unwrap(),
            Some("{}\r\n".to_owned())
        );
    }

    #[test]
    fn validates_manifest_identity_features_and_package_paths() {
        let manifest: coterm_proto::runtime::manifest::PluginManifest =
            serde_json::from_value(serde_json::json!({
                "manifestVersion": 1,
                "id": "com.example.serial",
                "name": "Serial",
                "version": "0.1.0",
                "runtime": { "interfaceMajor": 1, "requires": ["instances"] },
                "backend": { "test-platform": "backend/plugin" },
                "frontend": null,
                "mcp": { "provider": "backend" },
                "instances": { "type": "serial", "multiple": true },
                "capabilities": []
            }))
            .expect("valid manifest shape");
        validate_manifest(&manifest).expect("supported manifest");

        let mut unsupported = manifest.clone();
        unsupported
            .runtime
            .requires
            .push("binary-streams".to_owned());
        assert_eq!(
            validate_manifest(&unsupported).unwrap_err().code,
            "plugin_required_feature_unsupported"
        );

        let mut escaping = manifest;
        escaping
            .backend
            .insert("test-platform".to_owned(), "../outside".to_owned());
        assert_eq!(
            resolve_backend_entry(Path::new("."), &escaping, "test-platform")
                .unwrap_err()
                .code,
            "plugin_entry_invalid"
        );
    }

    #[test]
    fn decodes_json_rpc_success_and_error_responses() {
        let success = decode_response(r#"{"jsonrpc":"2.0","id":"req-1","result":{"ready":true}}"#)
            .expect("valid success response");
        assert!(matches!(
            success,
            PluginEvent::Reply { request_id, result: Ok(value) }
                if request_id == "req-1" && value["ready"] == true
        ));

        let error = decode_response(
            r#"{"jsonrpc":"2.0","id":"req-2","error":{"code":-32000,"message":"busy"}}"#,
        )
        .expect("valid error response");
        assert!(matches!(
            error,
            PluginEvent::Reply { request_id, result: Err(error) }
                if request_id == "req-2"
                    && matches!(error.category, coterm_proto::runtime::ErrorCategory::Domain)
                    && error.code == "plugin_rpc_-32000"
        ));
    }

    #[test]
    fn rejects_invalid_versions_and_ambiguous_response_shapes() {
        assert_eq!(
            decode_response(r#"{"jsonrpc":"1.0","id":"req","result":null}"#)
                .unwrap_err()
                .code,
            "plugin_jsonrpc_version_invalid"
        );
        assert_eq!(
            decode_response(
                r#"{"jsonrpc":"2.0","id":"req","result":true,"error":{"code":1,"message":"bad"}}"#
            )
            .unwrap_err()
            .code,
            "plugin_response_shape_invalid"
        );
    }

    #[test]
    fn accepts_json_null_as_a_present_success_result() {
        let response = decode_response(r#"{"jsonrpc":"2.0","id":"req-null","result":null}"#)
            .expect("null is a valid JSON-RPC result");
        assert!(matches!(
            response,
            PluginEvent::Reply { request_id, result: Ok(value) }
                if request_id == "req-null" && value.is_null()
        ));
    }
}
