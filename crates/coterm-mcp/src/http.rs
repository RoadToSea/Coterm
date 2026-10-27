//! Local, token-free MCP Streamable HTTP. Domain calls use the existing Agent router.
use std::{
    collections::HashMap,
    convert::Infallible,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::post,
    Router,
};
use coterm_core::plugin_manager::ProcessPluginRuntime;
use futures::stream;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::{
    net::TcpListener,
    sync::{watch, Mutex, Semaphore},
    task::JoinHandle,
};

use crate::{
    server::{initialize_result, ready_request, rpc_error, tools_list},
    transport::MAX_FRAME_BYTES,
};

pub const DEFAULT_PORT: u16 = 23517;
const SESSION_HEADER: &str = "mcp-session-id";
const VERSION_HEADER: &str = "mcp-protocol-version";
const MAX_SESSIONS: usize = 8;
const SESSION_IDLE: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Serialize)]
pub struct HttpConnectionInfo {
    pub url: String,
}

pub struct HttpBridgeServer {
    info: HttpConnectionInfo,
    shutdown: watch::Sender<bool>,
    sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
    task: Option<JoinHandle<()>>,
}

/// One live MCP client session, safe to expose to the desktop UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    /// Short, non-secret prefix that lets a user match a client without publishing the full ID.
    pub id_prefix: String,
    pub client_name: Option<String>,
    pub client_version: Option<String>,
    pub protocol_version: String,
    pub ready: bool,
    pub idle_seconds: u64,
    pub created_seconds_ago: u64,
}

impl HttpBridgeServer {
    pub async fn bind(runtime: Arc<ProcessPluginRuntime>) -> std::io::Result<Self> {
        Self::bind_at(
            runtime,
            SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT)),
        )
        .await
    }

    /// Alternate loopback ports are useful for isolated transport tests. No random fallback.
    pub async fn bind_at(
        runtime: Arc<ProcessPluginRuntime>,
        address: SocketAddr,
    ) -> std::io::Result<Self> {
        if address.ip() != Ipv4Addr::LOCALHOST {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "MCP HTTP requires 127.0.0.1",
            ));
        }
        let listener = TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        let (shutdown, mut stopped) = watch::channel(false);
        let sessions = Arc::new(Mutex::new(HashMap::new()));
        let state = Arc::new(HttpState {
            runtime,
            authority: address.to_string(),
            sessions: sessions.clone(),
            requests: Arc::new(Semaphore::new(128)),
            shutdown: shutdown.subscribe(),
        });
        let app = Router::new()
            .route(
                "/mcp",
                post(post_message).get(get_events).delete(delete_session),
            )
            .layer(middleware::from_fn_with_state(state.clone(), guard))
            .with_state(state);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = stopped.changed().await;
                })
                .await;
        });
        Ok(Self {
            info: HttpConnectionInfo {
                url: format!("http://{address}/mcp"),
            },
            shutdown,
            sessions,
            task: Some(task),
        })
    }

    pub fn connection_info(&self) -> HttpConnectionInfo {
        self.info.clone()
    }

    /// Live sessions in idle order. Values contain no secrets: IDs are prefixes only.
    pub async fn sessions(&self) -> Vec<SessionInfo> {
        let sessions = self.sessions.lock().await;
        let now = Instant::now();
        let mut infos: Vec<_> = sessions
            .iter()
            .map(|(id, session)| {
                let mut info = session.info(id);
                // Keep the reported idle time consistent with a single clock read.
                info.idle_seconds = session
                    .touched
                    .lock()
                    .map_or(0, |touched| now.duration_since(*touched).as_secs());
                info
            })
            .collect();
        infos.sort_by_key(|info| info.idle_seconds);
        infos
    }

    /// Closes a session by its short prefix. Returns false when the prefix is unknown or
    /// ambiguous, so a UI cannot close the wrong client.
    pub async fn close_session(&self, id_prefix: &str) -> bool {
        let session = {
            let mut sessions = self.sessions.lock().await;
            let matches: Vec<_> = sessions
                .keys()
                .filter(|id| id.starts_with(id_prefix))
                .cloned()
                .collect();
            if matches.len() != 1 {
                return false;
            }
            sessions.remove(&matches[0])
        };
        if let Some(session) = session {
            let _ = session.closed.send(true);
        }
        true
    }

    /// Wait for listener and in-flight HTTP handlers to close, without cancelling host operations.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for HttpBridgeServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

struct HttpState {
    runtime: Arc<ProcessPluginRuntime>,
    authority: String,
    sessions: Arc<Mutex<HashMap<String, Arc<Session>>>>,
    requests: Arc<Semaphore>,
    shutdown: watch::Receiver<bool>,
}

struct Session {
    version: String,
    ready: AtomicBool,
    touched: std::sync::Mutex<Instant>,
    created: Instant,
    client_name: Option<String>,
    client_version: Option<String>,
    calls: Arc<Semaphore>,
    stream: Arc<Semaphore>,
    closed: watch::Sender<bool>,
}

impl Session {
    fn expired(&self) -> bool {
        self.touched
            .lock()
            .map_or(true, |t| t.elapsed() > SESSION_IDLE)
    }
    fn touch(&self) {
        if let Ok(mut touched) = self.touched.lock() {
            *touched = Instant::now();
        }
    }
    fn info(&self, id: &str) -> SessionInfo {
        let idle = self
            .touched
            .lock()
            .map_or(0, |touched| touched.elapsed().as_secs());
        SessionInfo {
            id_prefix: id.chars().take(8).collect(),
            client_name: self.client_name.clone(),
            client_version: self.client_version.clone(),
            protocol_version: self.version.clone(),
            ready: self.ready.load(Ordering::Acquire),
            idle_seconds: idle,
            created_seconds_ago: self.created.elapsed().as_secs(),
        }
    }
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}

async fn guard(State(state): State<Arc<HttpState>>, request: Request, next: Next) -> Response {
    if single_header(request.headers(), "host") != Some(state.authority.as_str()) {
        return error(StatusCode::FORBIDDEN, "Invalid Host");
    }
    if request.headers().contains_key("origin")
        && single_header(request.headers(), "origin")
            != Some(format!("http://{}", state.authority).as_str())
    {
        return error(StatusCode::FORBIDDEN, "Invalid Origin");
    }
    let Ok(_permit) = state.requests.clone().try_acquire_owned() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "Too many HTTP requests");
    };
    let mut shutdown = state.shutdown.clone();
    if *shutdown.borrow() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Coterm is closing");
    }
    tokio::select! {
        response = next.run(request) => response,
        _ = shutdown.changed() => error(StatusCode::SERVICE_UNAVAILABLE, "Coterm is closing"),
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(rpc_error(Value::Null, -32000, message))).into_response()
}

fn json_response(status: StatusCode, value: Value) -> Response {
    match serde_json::to_vec(&value) {
        Ok(bytes) if bytes.len() <= MAX_FRAME_BYTES => (
            status,
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        _ => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MCP response exceeds limit",
        ),
    }
}

fn accepts(headers: &HeaderMap, media: &str) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|s| s.split(','))
        .any(|s| s.split(';').next().is_some_and(|s| s.trim() == media))
}

async fn session(state: &HttpState, headers: &HeaderMap) -> Result<Arc<Session>, Response> {
    let id = single_header(headers, SESSION_HEADER)
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "Mcp-Session-Id required"))?;
    let mut sessions = state.sessions.lock().await;
    let Some(session) = sessions.get(id).cloned() else {
        return Err(error(
            StatusCode::NOT_FOUND,
            "MCP session expired; initialize again",
        ));
    };
    if session.expired() {
        sessions.remove(id);
        let _ = session.closed.send(true);
        return Err(error(
            StatusCode::NOT_FOUND,
            "MCP session expired; initialize again",
        ));
    }
    // Older clients may omit this header (the MCP transport default is 2025-03-26).
    let version = if headers.contains_key(VERSION_HEADER) {
        single_header(headers, VERSION_HEADER).unwrap_or_default()
    } else {
        "2025-03-26"
    };
    if !["2025-03-26", "2025-06-18"].contains(&version) || version != session.version {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Unsupported or mismatched MCP-Protocol-Version",
        ));
    }
    session.touch();
    Ok(session)
}

async fn post_message(State(state): State<Arc<HttpState>>, request: Request) -> Response {
    if !accepts(request.headers(), "application/json")
        || !accepts(request.headers(), "text/event-stream")
    {
        return error(
            StatusCode::NOT_ACCEPTABLE,
            "Accept must include application/json and text/event-stream",
        );
    }
    if single_header(request.headers(), "content-type")
        .and_then(|s| s.split(';').next())
        .map(str::trim)
        != Some("application/json")
    {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json",
        );
    }
    let (parts, body) = request.into_parts();
    let bytes = match tokio::time::timeout(Duration::from_secs(10), to_bytes(body, MAX_FRAME_BYTES))
        .await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "MCP request body exceeds limit",
            )
        }
        Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "MCP request body timed out"),
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
    let id = value.get("id").cloned();
    let is_response = id.is_some()
        && (value.get("result").is_some() || value.get("error").is_some())
        && value.get("method").is_none();
    if !value.is_object()
        || value["jsonrpc"] != "2.0"
        || (!value["method"].is_string() && !is_response)
        || id
            .as_ref()
            .is_some_and(|id| !id.is_string() && !id.is_i64() && !id.is_u64())
        || value.get("params").is_some_and(|p| !p.is_object())
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            rpc_error(Value::Null, -32600, "Invalid Request"),
        );
    }
    let method = value["method"].as_str().unwrap_or_default();
    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
    if method == "initialize" {
        let Some(id) = id else {
            return error(StatusCode::BAD_REQUEST, "initialize requires an id");
        };
        if parts.headers.contains_key(SESSION_HEADER) {
            return json_response(
                StatusCode::BAD_REQUEST,
                rpc_error(id, -32600, "Initialize without an existing session"),
            );
        }
        let mut result = match initialize_result(&params) {
            Ok(result) => result,
            Err((code, message)) => {
                return json_response(StatusCode::BAD_REQUEST, rpc_error(id, code, &message))
            }
        };
        if result["protocolVersion"] == "2024-11-05" {
            result["protocolVersion"] = json!("2025-06-18");
        }
        let mut sessions = state.sessions.lock().await;
        sessions.retain(|_, session| {
            if session.expired() {
                let _ = session.closed.send(true);
                false
            } else {
                true
            }
        });
        if sessions.len() >= MAX_SESSIONS {
            // A client that has no session ID yet cannot act on "close an existing session".
            // Report the live sessions, a retry hint and a server-side recovery route.
            let mut active: Vec<_> = sessions
                .iter()
                .map(|(id, session)| session.info(id))
                .collect();
            active.sort_by_key(|session| session.idle_seconds);
            let body = json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32000,
                    "message": "Too many MCP sessions; close a session from Coterm's MCP panel or retry after an idle session expires",
                    "data": {
                        "code": "mcp_sessions_full",
                        "limit": MAX_SESSIONS,
                        "active": active.len(),
                        "idleTimeoutSeconds": SESSION_IDLE.as_secs(),
                        "recovery": {
                            "desktop": "Open the MCP panel in Coterm and close an unused session.",
                            "client": "Stop the client that owns the session so it sends DELETE /mcp.",
                            "retryAfterSeconds": 30
                        },
                        "sessions": active
                    }
                }
            });
            let mut response = json_response(StatusCode::TOO_MANY_REQUESTS, body);
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("30"));
            return response;
        }
        let session_id = uuid::Uuid::new_v4().to_string();
        let (closed, _) = watch::channel(false);
        let client_info = &params["clientInfo"];
        sessions.insert(
            session_id.clone(),
            Arc::new(Session {
                version: result["protocolVersion"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                ready: AtomicBool::new(false),
                touched: std::sync::Mutex::new(Instant::now()),
                created: Instant::now(),
                client_name: client_info["name"].as_str().map(str::to_owned),
                client_version: client_info["version"].as_str().map(str::to_owned),
                calls: Arc::new(Semaphore::new(16)),
                stream: Arc::new(Semaphore::new(1)),
                closed,
            }),
        );
        let mut response = json_response(
            StatusCode::OK,
            json!({"jsonrpc":"2.0","id":id,"result":result}),
        );
        if let Ok(header) = HeaderValue::from_str(&session_id) {
            response.headers_mut().insert(SESSION_HEADER, header);
        }
        return response;
    }
    let session = match session(&state, &parts.headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    if id.is_none() || is_response {
        if method == "notifications/initialized" {
            session.ready.store(true, Ordering::Release);
        }
        // No server-to-client requests are issued by this Tools-only bridge.
        return StatusCode::ACCEPTED.into_response();
    }
    let id = id.unwrap_or(Value::Null);
    if !session.ready.load(Ordering::Acquire) && method != "ping" {
        return json_response(
            StatusCode::OK,
            rpc_error(id, -32000, "MCP initialization required"),
        );
    }
    let Ok(_permit) = session.calls.clone().try_acquire_owned() else {
        return json_response(
            StatusCode::TOO_MANY_REQUESTS,
            rpc_error(id, -32000, "Too many in-flight calls"),
        );
    };
    let mut closed = session.closed.subscribe();
    if *closed.borrow() {
        return error(
            StatusCode::NOT_FOUND,
            "MCP session closed; initialize again",
        );
    }
    let result = tokio::select! {
        result = ready_request(&state.runtime, method, &params) => result,
        _ = closed.changed() => return error(StatusCode::NOT_FOUND, "MCP session closed; initialize again"),
    };
    let value = match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err((code, message)) => rpc_error(id, code, &message),
    };
    json_response(StatusCode::OK, value)
}

async fn delete_session(State(state): State<Arc<HttpState>>, headers: HeaderMap) -> Response {
    let session = match session(&state, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    if let Some(id) = single_header(&headers, SESSION_HEADER) {
        state.sessions.lock().await.remove(id);
    }
    let _ = session.closed.send(true);
    StatusCode::NO_CONTENT.into_response()
}

async fn get_events(State(state): State<Arc<HttpState>>, headers: HeaderMap) -> Response {
    if !accepts(&headers, "text/event-stream") {
        return error(
            StatusCode::NOT_ACCEPTABLE,
            "Accept must include text/event-stream",
        );
    }
    let session = match session(&state, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    if !session.ready.load(Ordering::Acquire) {
        return error(StatusCode::BAD_REQUEST, "MCP initialization required");
    }
    let Ok(permit) = session.stream.clone().try_acquire_owned() else {
        return error(StatusCode::CONFLICT, "Only one event stream per session");
    };
    let events = stream::unfold(
        (
            state.clone(),
            session.clone(),
            permit,
            state.shutdown.clone(),
            session.closed.subscribe(),
            Value::Null,
            tokio::time::interval(Duration::from_secs(1)),
        ),
        |(state, session, permit, mut shutdown, mut closed, mut last, mut interval)| async move {
            loop {
                if *shutdown.borrow() || *closed.borrow() {
                    return None;
                }
                tokio::select! {
                    _ = shutdown.changed() => return None,
                    _ = closed.changed() => return None,
                    _ = interval.tick() => {
                        session.touch();
                        let tools = match tools_list(&state.runtime).await { Ok(tools) => tools, Err(_) => return None };
                        if tools != last {
                            last = tools;
                            // Always announce on reconnect. No event IDs or replay buffer are used.
                            let event = Event::default().event("message").data(json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}).to_string());
                            return Some((Ok::<_, Infallible>(event), (state, session, permit, shutdown, closed, last, interval)));
                        }
                    }
                }
            }
        },
    );
    Sse::new(events)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}
