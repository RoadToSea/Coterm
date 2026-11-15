//! In-process Streamable HTTP transport. Tool execution stays in the shared Runtime.

use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use coterm_core::runtime::Runtime;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    body::{Bytes, Incoming},
    header,
    server::conn::http1,
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{json, Value};
use tokio::{
    net::TcpListener,
    sync::{oneshot, Semaphore},
    task::{JoinHandle, JoinSet},
    time::timeout,
};

use crate::server::{initialize_result, ready_request, rpc_error};

/// Device-local default; desktop settings may select any available nonzero port.
pub const DEFAULT_PORT: u16 = 51234;
/// Exact HTTP path; other paths or query strings return 404.
pub const ENDPOINT_PATH: &str = "/mcp";
/// UTF-8 JSON body budget per request or response, excluding HTTP headers.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
/// Requests beyond this in-flight limit receive HTTP 503 without tool dispatch.
pub const MAX_CONCURRENT_REQUESTS: usize = 128;
/// Accepted connection task budget; excess sockets are immediately closed.
pub const MAX_CONNECTIONS: usize = MAX_CONCURRENT_REQUESTS * 2;
/// Supported revisions in preference order for initialize negotiation.
pub const PROTOCOL_VERSIONS: [&str; 2] = ["2025-06-18", "2025-03-26"];
const INPUT_TIMEOUT: Duration = Duration::from_secs(10);

type HttpResponse = Response<Full<Bytes>>;

/// Owns a loopback listener and its bounded connection tasks; drop aborts them.
pub struct HttpServer {
    address: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl HttpServer {
    /// Bind `runtime` at a loopback `address` (port 0 is allowed for fixtures).
    /// Returns IO errors for non-loopback addresses or unavailable ports.
    pub async fn bind_at(runtime: Arc<Runtime>, address: SocketAddr) -> std::io::Result<Self> {
        if !address.ip().is_loopback() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Coterm MCP requires a loopback address",
            ));
        }
        let listener = TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(accept_connections(listener, runtime, address, stopped));
        Ok(Self {
            address,
            stop: Some(stop),
            task,
        })
    }

    /// Return the actual bound socket address, including an OS-assigned fixture port.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Return the client-facing Streamable HTTP URL; it contains no credentials.
    pub fn url(&self) -> String {
        format!("http://{}{ENDPOINT_PATH}", self.address)
    }

    /// Stop accepting and close connection tasks before returning; no operation is replayed.
    pub async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = (&mut self.task).await;
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn accept_connections(
    listener: TcpListener,
    runtime: Arc<Runtime>,
    address: SocketAddr,
    mut stopped: oneshot::Receiver<()>,
) {
    let mut connections = JoinSet::new();
    let requests = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
    loop {
        tokio::select! {
            _ = &mut stopped => { connections.shutdown().await; break; }
            accepted = listener.accept() => {
                let Ok((socket, peer)) = accepted else { break };
                if !peer.ip().is_loopback() || connections.len() >= MAX_CONNECTIONS { continue; }
                let runtime = runtime.clone();
                let requests = requests.clone();
                connections.spawn(async move {
                    let service = service_fn(move |request| {
                        let runtime = runtime.clone();
                        let requests = requests.clone();
                        async move { Ok::<_, Infallible>(handle(request, runtime, requests, address).await) }
                    });
                    let _ = http1::Builder::new()
                        .timer(TokioTimer::new())
                        .header_read_timeout(INPUT_TIMEOUT)
                        .max_headers(64)
                        .serve_connection(TokioIo::new(socket), service)
                        .await;
                });
            }
            _ = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

async fn handle(
    request: Request<Incoming>,
    runtime: Arc<Runtime>,
    requests: Arc<Semaphore>,
    address: SocketAddr,
) -> HttpResponse {
    if let Some(response) = reject_request(&request, address) {
        return response;
    }
    let Ok(_permit) = requests.try_acquire_owned() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many in-flight MCP requests",
        );
    };
    let version = request.headers().get("mcp-protocol-version").cloned();
    let collected = timeout(
        INPUT_TIMEOUT,
        Limited::new(request.into_body(), MAX_MESSAGE_BYTES).collect(),
    )
    .await;
    let bytes = match collected {
        Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "MCP request body timed out"),
        Ok(Err(problem)) if problem.is::<http_body_util::LengthLimitError>() => {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "MCP request body exceeds the limit",
            )
        }
        Ok(Err(_)) => return error(StatusCode::BAD_REQUEST, "Incomplete MCP request body"),
        Ok(Ok(body)) => body.to_bytes(),
    };
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                rpc_error(Value::Null, -32700, "Parse error"),
            )
        }
    };
    dispatch(runtime, value, version).await
}

fn reject_request(request: &Request<Incoming>, address: SocketAddr) -> Option<HttpResponse> {
    if !trusted_origin(request, address) {
        return Some(error(StatusCode::FORBIDDEN, "Untrusted Host or Origin"));
    }
    if request.uri().path() != ENDPOINT_PATH || request.uri().query().is_some() {
        return Some(error(StatusCode::NOT_FOUND, "Unknown endpoint"));
    }
    if request.method() != Method::POST {
        let mut response = empty(StatusCode::METHOD_NOT_ALLOWED);
        response
            .headers_mut()
            .insert(header::ALLOW, header::HeaderValue::from_static("POST"));
        return Some(response);
    }
    if !accepts(request, "application/json") || !accepts(request, "text/event-stream") {
        return Some(error(
            StatusCode::NOT_ACCEPTABLE,
            "Accept must include application/json and text/event-stream",
        ));
    }
    if !json_content(request) {
        return Some(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json without content encoding",
        ));
    }
    None
}

fn trusted_origin(request: &Request<Incoming>, address: SocketAddr) -> bool {
    if request.headers().get_all(header::HOST).iter().count() != 1
        || request.headers().get_all(header::ORIGIN).iter().count() > 1
    {
        return false;
    }
    let authorities = [address.to_string(), format!("localhost:{}", address.port())];
    let Some(host) = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    if !authorities
        .iter()
        .any(|authority| authority.eq_ignore_ascii_case(host))
    {
        return false;
    }
    if request
        .uri()
        .authority()
        .is_some_and(|authority| !authority.as_str().eq_ignore_ascii_case(host))
    {
        return false;
    }
    match request.headers().get(header::ORIGIN) {
        None => true,
        Some(origin) => origin.to_str().is_ok_and(|origin| {
            authorities
                .iter()
                .any(|authority| origin == format!("http://{authority}"))
        }),
    }
}

fn accepts(request: &Request<Incoming>, media_type: &str) -> bool {
    request
        .headers()
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| {
            value.split(',').any(|item| {
                let mut parts = item.split(';');
                let accepted = parts.next().unwrap_or_default().trim();
                (accepted.eq_ignore_ascii_case(media_type) || accepted == "*/*")
                    && !parts.any(|parameter| {
                        parameter
                            .trim()
                            .strip_prefix("q=")
                            .is_some_and(|q| q.parse::<f32>().map_or(true, |q| q <= 0.0))
                    })
            })
        })
}

fn json_content(request: &Request<Incoming>) -> bool {
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let encoding = request.headers().get(header::CONTENT_ENCODING);
    content_type.is_some_and(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("application/json")
    }) && encoding.is_none_or(|value| value == "identity")
}

async fn dispatch(
    runtime: Arc<Runtime>,
    value: Value,
    version: Option<header::HeaderValue>,
) -> HttpResponse {
    if let Some(response) = reject_message(&value, version.as_ref()) {
        return response;
    }
    let id = value.get("id").cloned();
    let Some(method) = value["method"].as_str() else {
        if value.get("method").is_none()
            && id.is_some()
            && (value.get("result").is_some() ^ value.get("error").is_some())
        {
            return empty(StatusCode::ACCEPTED);
        }
        return json_response(
            StatusCode::BAD_REQUEST,
            rpc_error(Value::Null, -32600, "Invalid Request"),
        );
    };
    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return json_response(
            StatusCode::BAD_REQUEST,
            rpc_error(id.unwrap_or(Value::Null), -32602, "Invalid parameters"),
        );
    }
    let Some(id) = id else {
        return empty(StatusCode::ACCEPTED);
    };
    let result = if method == "initialize" {
        initialize_result(&params)
    } else {
        ready_request(&runtime, method, &params).await
    };
    let response = match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err((code, message)) => rpc_error(id, code, &message),
    };
    json_response(StatusCode::OK, response)
}

fn reject_message(value: &Value, version: Option<&header::HeaderValue>) -> Option<HttpResponse> {
    let id = value.get("id");
    if !value.is_object()
        || value["jsonrpc"] != "2.0"
        || id
            .as_ref()
            .is_some_and(|id| !id.is_string() && !id.is_number())
    {
        return Some(json_response(
            StatusCode::BAD_REQUEST,
            rpc_error(Value::Null, -32600, "Invalid Request"),
        ));
    }
    let method = value["method"].as_str();
    if method != Some("initialize")
        && version.as_ref().is_some_and(|version| {
            version
                .to_str()
                .map_or(true, |version| !PROTOCOL_VERSIONS.contains(&version))
        })
    {
        return Some(error(
            StatusCode::BAD_REQUEST,
            "Unsupported MCP-Protocol-Version",
        ));
    }
    None
}

fn empty(status: StatusCode) -> HttpResponse {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

fn json_response(status: StatusCode, value: Value) -> HttpResponse {
    let mut body = value.to_string();
    if body.len() > MAX_MESSAGE_BYTES {
        body = rpc_error(
            value["id"].clone(),
            -32603,
            "MCP response exceeds limit; an operation may already have executed, do not resubmit",
        )
        .to_string();
    }
    let mut response = empty(status);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    *response.body_mut() = Full::new(Bytes::from(body));
    response
}

fn error(status: StatusCode, message: &str) -> HttpResponse {
    json_response(status, rpc_error(Value::Null, -32000, message))
}
