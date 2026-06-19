use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tauri::{async_runtime, State};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{oneshot, Mutex, RwLock, Semaphore},
    time::timeout,
};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{Request, Response},
        Message,
    },
};
use uuid::Uuid;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const SESSION_TTL: Duration = Duration::from_secs(45);
const IO_BUFFER_SIZE: usize = 64 * 1024;
const MAX_WS_FRAME_SIZE: usize = 8 * 1024 * 1024;
const MAX_PENDING_HANDSHAKES: usize = 64;
const MAX_ACTIVE_BRIDGES: usize = 16;
const LOCALHOST_V4: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const LOCALHOST_V6: SocketAddr = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0);

#[derive(Clone)]
pub struct ProxyState {
    inner: Arc<Inner>,
}

struct Inner {
    sessions: RwLock<HashMap<String, ProxySession>>,
    listener: Mutex<Option<ProxyListener>>,
    handshake_slots: Arc<Semaphore>,
    bridge_slots: Arc<Semaphore>,
}

struct ProxyListener {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

#[derive(Clone)]
struct ProxySession {
    host: String,
    port: u16,
    secret: String,
    created_at: Instant,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySessionResponse {
    pub(crate) session_id: String,
    pub(crate) ws_url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatus {
    listen_addr: Option<String>,
    active_sessions: usize,
}

impl ProxyState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                sessions: RwLock::new(HashMap::new()),
                listener: Mutex::new(None),
                handshake_slots: Arc::new(Semaphore::new(MAX_PENDING_HANDSHAKES)),
                bridge_slots: Arc::new(Semaphore::new(MAX_ACTIVE_BRIDGES)),
            }),
        }
    }

    async fn ensure_listener(&self) -> Result<SocketAddr, String> {
        let mut guard = self.inner.listener.lock().await;
        if let Some(listener) = guard.as_ref() {
            return Ok(listener.addr);
        }

        let tcp_listener = match TcpListener::bind(LOCALHOST_V4).await {
            Ok(listener) => listener,
            Err(_) => TcpListener::bind(LOCALHOST_V6)
                .await
                .map_err(|err| format!("failed to bind local proxy listener: {err}"))?,
        };
        let addr = tcp_listener
            .local_addr()
            .map_err(|err| format!("failed to read local proxy address: {err}"))?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let state = self.clone();

        async_runtime::spawn(async move {
            state.accept_loop(tcp_listener, shutdown_rx).await;
        });

        *guard = Some(ProxyListener {
            addr,
            shutdown: Some(shutdown_tx),
        });

        Ok(addr)
    }

    pub(crate) async fn create_session(
        &self,
        host: String,
        port: u16,
    ) -> Result<ProxySessionResponse, String> {
        validate_target(&host, port)?;
        let addr = self.ensure_listener().await?;
        self.remove_expired_sessions().await;

        let session_id = Uuid::new_v4().to_string();
        let secret = Uuid::new_v4().to_string();
        let session = ProxySession {
            host,
            port,
            secret: secret.clone(),
            created_at: Instant::now(),
        };
        self.inner
            .sessions
            .write()
            .await
            .insert(session_id.clone(), session);

        Ok(ProxySessionResponse {
            session_id: session_id.clone(),
            ws_url: format!("ws://{}/vnc/{}?secret={}", addr, session_id, secret),
        })
    }

    async fn status(&self) -> ProxyStatus {
        let listen_addr = self
            .inner
            .listener
            .lock()
            .await
            .as_ref()
            .map(|listener| listener.addr.to_string());
        let active_sessions = self.inner.sessions.read().await.len();

        ProxyStatus {
            listen_addr,
            active_sessions,
        }
    }

    async fn accept_loop(self, listener: TcpListener, mut shutdown: oneshot::Receiver<()>) {
        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => break,
                accepted = listener.accept() => {
                    let Ok((stream, peer_addr)) = accepted else {
                        continue;
                    };
                    if !peer_addr.ip().is_loopback() {
                        continue;
                    }

                    let state = self.clone();
                    async_runtime::spawn(async move {
                        let _ = state.handle_ws_client(stream).await;
                    });
                }
            }
        }
    }

    async fn handle_ws_client(&self, stream: TcpStream) -> Result<(), String> {
        let handshake_permit = self
            .inner
            .handshake_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "too many pending proxy handshakes".to_string())?;
        let peer = stream
            .peer_addr()
            .map_err(|err| format!("failed to read peer address: {err}"))?;
        if !peer.ip().is_loopback() {
            return Err("proxy accepts loopback clients only".to_string());
        }

        let request_uri = Arc::new(std::sync::Mutex::new(String::new()));
        let request_origin = Arc::new(std::sync::Mutex::new(None::<String>));
        let captured_uri = request_uri.clone();
        let captured_origin = request_origin.clone();
        let ws_stream = timeout(
            HANDSHAKE_TIMEOUT,
            accept_hdr_async(stream, move |request: &Request, response: Response| {
                if let Ok(mut uri) = captured_uri.lock() {
                    *uri = request.uri().to_string();
                }
                if let Ok(mut origin) = captured_origin.lock() {
                    *origin = request
                        .headers()
                        .get("origin")
                        .and_then(|value| value.to_str().ok())
                        .map(ToOwned::to_owned);
                }
                Ok(response)
            }),
        )
        .await
        .map_err(|_| "websocket handshake timed out".to_string())?
        .map_err(|err| format!("websocket handshake failed: {err}"))?;
        drop(handshake_permit);

        let origin = request_origin
            .lock()
            .map_err(|_| "failed to read websocket origin".to_string())?
            .clone();
        if !is_allowed_origin(origin.as_deref()) {
            return Err("websocket origin is not allowed".to_string());
        }

        let (session_id, secret) = request_uri
            .lock()
            .map_err(|_| "failed to read websocket request uri".to_string())
            .and_then(|uri| extract_session_request(&uri))?;

        let session = self
            .inner
            .sessions
            .read()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| "proxy session not found or already used".to_string())?;

        if session.secret != secret {
            return Err("proxy session secret is invalid".to_string());
        }

        if session.created_at.elapsed() > SESSION_TTL {
            self.inner.sessions.write().await.remove(&session_id);
            return Err("proxy session expired".to_string());
        }

        let bridge_permit = self
            .inner
            .bridge_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "too many active proxy connections".to_string())?;

        let Some(session) = self.inner.sessions.write().await.remove(&session_id) else {
            return Err("proxy session already used".to_string());
        };
        if session.secret != secret {
            return Err("proxy session secret is invalid".to_string());
        }
        if session.created_at.elapsed() > SESSION_TTL {
            return Err("proxy session expired".to_string());
        }

        let target = format!("{}:{}", session.host, session.port);
        let tcp_stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(&target))
            .await
            .map_err(|_| format!("timed out connecting to {target}"))?
            .map_err(|err| format!("failed to connect to {target}: {err}"))?;
        tcp_stream
            .set_nodelay(true)
            .map_err(|err| format!("failed to set TCP_NODELAY: {err}"))?;

        let result = bridge(ws_stream, tcp_stream).await;
        drop(bridge_permit);
        result
    }

    async fn remove_expired_sessions(&self) {
        let mut sessions = self.inner.sessions.write().await;
        sessions.retain(|_, session| session.created_at.elapsed() <= SESSION_TTL);
    }
}

impl Drop for ProxyListener {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

#[tauri::command]
pub async fn create_proxy_session(
    state: State<'_, ProxyState>,
    host: String,
    port: u16,
) -> Result<ProxySessionResponse, String> {
    state.create_session(host, port).await
}

#[tauri::command]
pub async fn proxy_status(state: State<'_, ProxyState>) -> Result<ProxyStatus, String> {
    Ok(state.status().await)
}

async fn bridge(
    ws_stream: tokio_tungstenite::WebSocketStream<TcpStream>,
    tcp_stream: TcpStream,
) -> Result<(), String> {
    let (mut ws_write, mut ws_read) = ws_stream.split();
    let (mut tcp_read, mut tcp_write) = tcp_stream.into_split();

    let mut ws_to_tcp = tokio::spawn(async move {
        while let Some(message) = timeout(IDLE_TIMEOUT, ws_read.next())
            .await
            .map_err(|_| "websocket-to-tcp pump timed out".to_string())?
        {
            match message.map_err(|err| format!("websocket read failed: {err}"))? {
                Message::Binary(bytes) => {
                    if bytes.len() > MAX_WS_FRAME_SIZE {
                        return Err("websocket frame exceeds proxy limit".to_string());
                    }
                    tcp_write
                        .write_all(&bytes)
                        .await
                        .map_err(|err| format!("tcp write failed: {err}"))?;
                }
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Text(_) | Message::Frame(_) => {
                    return Err("proxy accepts binary websocket frames only".to_string());
                }
            }
        }

        let _ = tcp_write.shutdown().await;
        Ok::<(), String>(())
    });

    let mut tcp_to_ws = tokio::spawn(async move {
        let mut tcp_buffer = vec![0_u8; IO_BUFFER_SIZE];
        loop {
            let read = timeout(IDLE_TIMEOUT, tcp_read.read(&mut tcp_buffer))
                .await
                .map_err(|_| "tcp-to-websocket pump timed out".to_string())?
                .map_err(|err| format!("tcp read failed: {err}"))?;
            if read == 0 {
                break;
            }
            ws_write
                .send(Message::Binary(tcp_buffer[..read].to_vec().into()))
                .await
                .map_err(|err| format!("websocket write failed: {err}"))?;
        }

        let _ = ws_write.close().await;
        Ok::<(), String>(())
    });

    tokio::select! {
        result = &mut ws_to_tcp => {
            tcp_to_ws.abort();
            result.map_err(|err| format!("websocket-to-tcp task failed: {err}"))?
        }
        result = &mut tcp_to_ws => {
            ws_to_tcp.abort();
            result.map_err(|err| format!("tcp-to-websocket task failed: {err}"))?
        }
    }
}

fn validate_target(host: &str, port: u16) -> Result<(), String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("host is required".to_string());
    }
    if host.len() > 255 {
        return Err("host is too long".to_string());
    }
    if port == 0 {
        return Err("port must be between 1 and 65535".to_string());
    }
    if host.contains('/') || host.contains('\\') || host.contains('@') || host.contains(':') {
        return Err("host must be a hostname or IP address without a scheme or port".to_string());
    }
    if !host
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-'))
    {
        return Err("host contains unsupported characters".to_string());
    }

    Ok(())
}

fn extract_session_request(uri: &str) -> Result<(String, String), String> {
    let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
    let Some(session_id) = path.strip_prefix("/vnc/") else {
        return Err("websocket path must be /vnc/<session_id>".to_string());
    };
    if session_id.is_empty()
        || session_id.len() > 64
        || !session_id
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() || ch == '-')
    {
        return Err("invalid proxy session id".to_string());
    }

    let secret = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(key, value)| (key == "secret").then_some(value))
        .ok_or_else(|| "proxy session secret is required".to_string())?;
    if secret.len() > 64 || !secret.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-') {
        return Err("invalid proxy session secret".to_string());
    }

    Ok((session_id.to_string(), secret.to_string()))
}

fn is_allowed_origin(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };

    origin == "tauri://localhost"
        || origin == "http://tauri.localhost"
        || origin.starts_with("http://127.0.0.1:")
        || origin.starts_with("http://localhost:")
}

#[cfg(test)]
mod tests {
    use super::{extract_session_request, is_allowed_origin, validate_target};

    #[test]
    fn parses_session_request_with_secret() {
        let (session_id, secret) = extract_session_request(
            "/vnc/5f7d7d47-1ca8-4da3-9177-33065173336b?secret=57a07d14-5db0-44c6-97cc-6f8e7ed9c8fb",
        )
        .expect("session request should parse");

        assert_eq!(session_id, "5f7d7d47-1ca8-4da3-9177-33065173336b");
        assert_eq!(secret, "57a07d14-5db0-44c6-97cc-6f8e7ed9c8fb");
    }

    #[test]
    fn rejects_missing_or_malformed_session_secret() {
        assert!(extract_session_request("/vnc/5f7d7d47-1ca8-4da3-9177-33065173336b").is_err());
        assert!(extract_session_request(
            "/vnc/5f7d7d47-1ca8-4da3-9177-33065173336b?secret=not%20hex"
        )
        .is_err());
    }

    #[test]
    fn validates_hostname_targets() {
        assert!(validate_target("127.0.0.1", 5900).is_ok());
        assert!(validate_target("localhost", 5900).is_ok());
        assert!(validate_target("vnc-host.internal", 5900).is_ok());
        assert!(validate_target("http://127.0.0.1", 5900).is_err());
        assert!(validate_target("127.0.0.1:5900", 5900).is_err());
        assert!(validate_target("::1", 5900).is_err());
        assert!(validate_target("127.0.0.1", 0).is_err());
    }

    #[test]
    fn allows_expected_webview_origins() {
        assert!(is_allowed_origin(None));
        assert!(is_allowed_origin(Some("tauri://localhost")));
        assert!(is_allowed_origin(Some("http://127.0.0.1:1420")));
        assert!(is_allowed_origin(Some("http://localhost:1420")));
        assert!(!is_allowed_origin(Some("https://example.com")));
    }
}
