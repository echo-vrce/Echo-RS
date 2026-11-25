//! Client-facing services on one TCP port: the Echo VR client's login, config,
//! matching and transaction websockets (any path; messages are dispatched by
//! type, so one URL serves them all), the game-server fleet socket, and a small
//! HTTP API.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{any, get, post};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::lobby::Hub;
use crate::sns::{self, ClientMessage, XpId};

#[derive(Clone)]
pub struct App {
    pub hub: Arc<Hub>,
    /// login session id → (user, login payload)
    pub sessions: Arc<DashMap<Uuid, (XpId, Value)>>,
    /// Routes that bridge an embedded game server to an upstream backend.
    pub upstreams: Arc<crate::upstream::Upstreams>,
    /// Shared secret external game servers need for `/fleet`
    /// (`--fleet-token`). Without one, only this machine may register.
    pub fleet_token: Option<Arc<str>>,
    /// Open client service connections per remote address.
    pub conns: Arc<DashMap<IpAddr, usize>>,
    /// Accounts created lately per remote address: (count, window start).
    pub new_accounts: Arc<DashMap<IpAddr, (u32, Instant)>>,
}

impl App {
    pub fn new(hub: Arc<Hub>, upstreams: Arc<crate::upstream::Upstreams>, fleet_token: Option<String>) -> Self {
        Self {
            hub,
            sessions: Arc::new(DashMap::new()),
            upstreams,
            fleet_token: fleet_token.filter(|t| !t.is_empty()).map(Arc::from),
            conns: Arc::new(DashMap::new()),
            new_accounts: Arc::new(DashMap::new()),
        }
    }
}

/// Client service connections one remote address may hold at once.
const MAX_CONNS_PER_IP: usize = 32;
/// Replies queued for a client that is not reading them; past this the
/// connection is dropped.
const MAX_QUEUED_REPLY_BYTES: usize = 8 * 1024 * 1024;
/// New accounts one remote address may create per `NEW_ACCOUNT_WINDOW`.
const MAX_NEW_ACCOUNTS: u32 = 20;
const NEW_ACCOUNT_WINDOW: Duration = Duration::from_secs(60 * 60);
/// Largest fleet (game server) message.
const MAX_FLEET_MESSAGE: usize = 1024 * 1024;

pub fn router(app: App) -> Router {
    Router::new()
        .route("/fleet", get(fleet_ws))
        .route("/upstream/:idx", get(crate::upstream::upstream_ws))
        .route("/status", get(status))
        .route("/admin/kick", post(admin_kick))
        .route("/admin/ban", post(admin_ban))
        .fallback(any(client_or_api))
        .with_state(app)
}

fn is_loopback(addr: &SocketAddr) -> bool {
    addr.ip().to_canonical().is_loopback()
}

async fn status(State(app): State<App>, ConnectInfo(addr): ConnectInfo<SocketAddr>) -> Json<crate::lobby::Stats> {
    Json(app.hub.stats(is_loopback(&addr)))
}

/// Admin requests: from this machine, addressed to a loopback host name (a
/// DNS-rebinding page reaches loopback under its own name), and carrying
/// `X-ERS-Admin: 1`, which a web page cannot add to a cross-site request
/// without a CORS preflight this server never answers.
fn admin_only(addr: &SocketAddr, headers: &HeaderMap) -> Result<(), Response> {
    let deny = |why: &'static str| Err((StatusCode::FORBIDDEN, why).into_response());
    if !is_loopback(addr) {
        return deny("admin endpoints are loopback-only");
    }
    let host = headers.get(axum::http::header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let name = if let Some(rest) = host.strip_prefix('[') { rest.split(']').next().unwrap_or("") } else { host.rsplit_once(':').map_or(host, |(h, _)| h) };
    if !matches!(name.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "::1") {
        return deny("admin endpoints answer only to localhost, 127.0.0.1 or [::1]");
    }
    if headers.get("x-ers-admin").and_then(|h| h.to_str().ok()) != Some("1") {
        return deny("admin requests need the header X-ERS-Admin: 1");
    }
    Ok(())
}

async fn admin_kick(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = admin_only(&addr, &headers) {
        return r;
    }
    let Some(x) = q.get("xpid").and_then(|s| XpId::parse(s)) else {
        return (StatusCode::BAD_REQUEST, "xpid=OVR_ORG-123 required").into_response();
    };
    Json(json!({"kicked": app.hub.kick(x)})).into_response()
}

async fn admin_ban(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = admin_only(&addr, &headers) {
        return r;
    }
    let Some(x) = q.get("xpid").and_then(|s| XpId::parse(s)) else {
        return (StatusCode::BAD_REQUEST, "xpid=OVR_ORG-123 required").into_response();
    };
    let minutes = q.get("minutes").and_then(|m| m.parse().ok()).unwrap_or(60 * 24 * 365);
    let ok = app.store().ban(x, minutes);
    if ok && minutes > 0 {
        app.hub.kick(x);
    }
    Json(json!({"banned": ok, "minutes": minutes})).into_response()
}

impl App {
    fn store(&self) -> &crate::store::Store {
        &self.hub.store
    }
}

/// Websocket upgrade on any other path is a client service connection; plain
/// HTTP is the `apiservice_host` API.
async fn client_or_api(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<HashMap<String, String>>,
    uri: Uri,
    ws: Option<WebSocketUpgrade>,
) -> Response {
    match ws {
        Some(ws) => {
            let Some(slot) = ConnSlot::take(&app.conns, addr.ip().to_canonical()) else {
                warn!(%addr, "too many connections from one address");
                return (StatusCode::TOO_MANY_REQUESTS, "too many connections").into_response();
            };
            ws.max_message_size(sns::MAX_PACKET)
                .on_upgrade(move |socket| client_socket(app, socket, addr, uri.path().to_owned(), q, slot))
                .into_response()
        }
        None => api(uri.path()).into_response(),
    }
}

/// One counted connection from an address; released when dropped.
struct ConnSlot {
    conns: Arc<DashMap<IpAddr, usize>>,
    ip: IpAddr,
}

impl ConnSlot {
    /// Loopback (the probe, tools on this machine) is not limited.
    fn take(conns: &Arc<DashMap<IpAddr, usize>>, ip: IpAddr) -> Option<Self> {
        let mut n = conns.entry(ip).or_insert(0);
        if *n >= MAX_CONNS_PER_IP && !ip.is_loopback() {
            return None;
        }
        *n += 1;
        Some(Self { conns: conns.clone(), ip })
    }
}

impl Drop for ConnSlot {
    fn drop(&mut self) {
        self.conns.remove_if_mut(&self.ip, |_, n| {
            *n = n.saturating_sub(1);
            *n == 0
        });
    }
}

fn api(path: &str) -> Json<Value> {
    if path.ends_with("/status/services") {
        Json(json!({"services": {"login": "up", "config": "up", "matching": "up", "transaction": "up"}}))
    } else if path.ends_with("/status/news") {
        Json(json!({"news": []}))
    } else {
        Json(json!({"ok": true, "server": "echo-rust-server"}))
    }
}

/// One client service websocket.
async fn client_socket(app: App, socket: WebSocket, addr: SocketAddr, path: String, q: HashMap<String, String>, _slot: ConnSlot) {
    debug!(%addr, %path, "client service connected");
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    // Bytes this loop queued that the writer has not sent yet: a client
    // that asks without reading would otherwise grow the queue forever.
    let queued = Arc::new(AtomicUsize::new(0));
    // Single writer task: replies from the hub (possibly delayed) and from
    // this loop are serialized here.
    let sent = queued.clone();
    let writer = tokio::spawn(async move {
        while let Some(mut buf) = rx.recv().await {
            // Coalesce anything else already queued into one frame.
            while let Ok(more) = rx.try_recv() {
                buf.extend_from_slice(&more);
            }
            let n = buf.len();
            if sink.send(Message::Binary(buf)).await.is_err() {
                break;
            }
            let _ = sent.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |q| Some(q.saturating_sub(n)));
        }
    });

    let mut conn = Conn { user: None, login_session: None };
    'conn: while let Some(Ok(msg)) = stream.next().await {
        let data = match msg {
            Message::Binary(b) => b,
            Message::Close(_) => break,
            _ => continue,
        };
        let messages = match sns::split(&data) {
            Ok(m) => m,
            Err(e) => {
                warn!(%addr, error = %e, "bad frame");
                break;
            }
        };
        for (symbol, payload) in messages {
            let decoded = match ClientMessage::decode(symbol, payload) {
                Ok(m) => m,
                Err(e) => {
                    warn!(%addr, symbol = format_args!("{symbol:#018x}"), error = %e, "undecodable message");
                    continue;
                }
            };
            if let Err(e) = handle(&app, &mut conn, decoded, &q, &tx, addr, &queued).await {
                warn!(%addr, error = %e, "request failed");
            }
            if queued.load(Ordering::Relaxed) > MAX_QUEUED_REPLY_BYTES {
                warn!(%addr, "client is not reading its replies; disconnecting");
                break 'conn;
            }
        }
    }
    // Only the login connection owns the login session.
    if let Some(sid) = conn.login_session {
        if let Some((_, (x, _))) = app.sessions.remove(&sid) {
            app.hub.client_gone(x);
            info!(player = %x, "logged out");
        }
    }
    drop(tx);
    drop(writer.await);
}

struct Conn {
    user: Option<XpId>,
    login_session: Option<Uuid>,
}

impl Conn {
    fn user_for(&mut self, app: &App, session: Uuid) -> Option<XpId> {
        if self.user.is_none() {
            self.user = app.sessions.get(&session).map(|e| e.0);
        }
        self.user
    }
}

/// Whether `ip` may create another account now (and counts it if so).
fn may_create_account(app: &App, ip: IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    let now = Instant::now();
    if app.new_accounts.len() > 10_000 {
        app.new_accounts.retain(|_, (_, start)| now.duration_since(*start) < NEW_ACCOUNT_WINDOW);
    }
    let mut e = app.new_accounts.entry(ip).or_insert((0, now));
    if now.duration_since(e.1) >= NEW_ACCOUNT_WINDOW {
        *e = (0, now);
    }
    if e.0 >= MAX_NEW_ACCOUNTS {
        return false;
    }
    e.0 += 1;
    true
}

async fn handle(
    app: &App,
    conn: &mut Conn,
    msg: ClientMessage,
    q: &HashMap<String, String>,
    tx: &mpsc::UnboundedSender<Vec<u8>>,
    addr: SocketAddr,
    queued: &AtomicUsize,
) -> anyhow::Result<()> {
    let mut out = Vec::new();
    match msg {
        ClientMessage::LoginRequest { xpid, profile, .. } => {
            if xpid.is_nil() {
                sns::login_failure(&mut out, xpid, 400, "Invalid user id");
                sns::unrequire(&mut out);
            } else if !app.store().exists(xpid) && !may_create_account(app, addr.ip().to_canonical()) {
                warn!(%addr, player = %xpid, "new account refused: too many from this address");
                sns::login_failure(&mut out, xpid, 429, "Too many new accounts from this address; try again later");
                sns::unrequire(&mut out);
            } else {
                let name = q
                    .get("displayname")
                    .cloned()
                    .or_else(|| profile.get("displayname").and_then(Value::as_str).map(str::to_owned))
                    .unwrap_or_default();
                // The password hash is deliberately slow: off the async workers.
                let (store, auth) = (app.hub.store.clone(), q.get("auth").cloned());
                let login = {
                    let name = name.clone();
                    tokio::task::spawn_blocking(move || store.login(xpid, &name, auth.as_deref())).await?
                };
                match login {
                    Ok(account) => {
                        let sid = Uuid::new_v4();
                        app.sessions.insert(sid, (xpid, profile));
                        conn.user = Some(xpid);
                        conn.login_session = Some(sid);
                        sns::login_success(&mut out, sid, xpid);
                        sns::unrequire(&mut out);
                        if let Some(settings) = app.store().resource("game_settings") {
                            sns::game_settings(&mut out, &settings)?;
                        }
                        info!(player = %xpid, name = %account.display_name, "logged in");
                    }
                    Err(reason) => {
                        warn!(player = %xpid, %reason, "login refused");
                        sns::login_failure(&mut out, xpid, 401, &reason);
                        sns::unrequire(&mut out);
                    }
                }
            }
        }
        ClientMessage::LoggedInProfileRequest { session, xpid } => {
            let login = app.sessions.get(&session).map(|e| e.1.clone()).unwrap_or(Value::Null);
            match app.store().profiles_json(xpid, &login) {
                Some(j) => sns::logged_in_profile_success(&mut out, xpid, &j)?,
                None => {
                    let mut w = sns::Writer::new();
                    w.xpid(xpid).u64(404).cstr("profile not found");
                    w.finish(sns::sym::LOGGED_IN_PROFILE_FAILURE, &mut out);
                }
            }
        }
        ClientMessage::DocumentRequest { lang, kind } => {
            if let Some(doc) = app.store().resource(&kind) {
                let mut v: Value = serde_json::from_slice(&doc)?;
                if let Some(o) = v.as_object_mut() {
                    o.insert("lang".into(), json!(lang));
                }
                sns::document_success(&mut out, &kind, &serde_json::to_vec(&v)?)?;
            }
        }
        ClientMessage::ChannelInfoRequest => {
            if let Some(j) = app.store().resource("channel_info") {
                sns::channel_info(&mut out, &j)?;
            }
        }
        ClientMessage::UpdateProfile { session, xpid, profile } => {
            if conn.user_for(app, session) == Some(xpid) && !app.store().update_client_profile(xpid, profile) {
                warn!(player = %xpid, "profile update that is not a JSON object: ignored");
            }
            sns::update_profile_success(&mut out, xpid);
        }
        ClientMessage::OtherProfileRequest { xpid } => match app.store().server_profile_json(xpid) {
            Some(j) => sns::other_profile_success(&mut out, xpid, &j)?,
            None => sns::other_profile_failure(&mut out, xpid, 404, "profile not found"),
        },
        ClientMessage::ConfigRequest { kind, id } => {
            match app.store().resource(&format!("config_{id}")) {
                Some(j) => sns::config_success(&mut out, &kind, &id, &j)?,
                None => sns::config_failure(&mut out, &kind, &id),
            }
            sns::unrequire(&mut out);
        }
        ClientMessage::ReconcileIap { xpid, .. } => sns::reconcile_iap_result(&mut out, xpid),
        ClientMessage::Lobby(req) => {
            let who = conn.user_for(app, req.login_session).or_else(|| req.entrants.first().map(|e| e.xpid));
            match who {
                Some(x) if app.sessions.contains_key(&req.login_session) => {
                    let hub = app.hub.clone();
                    let tx = tx.clone();
                    // Matching may wait for a server; don't block this socket.
                    tokio::spawn(async move { hub.lobby_request(req, x, tx).await });
                }
                _ => {
                    let mode = req.mode;
                    sns::session_failure_v4(&mut out, mode, req.group_id, sns::fail::BAD_REQUEST, "Not logged in");
                }
            }
        }
        ClientMessage::MatchmakerStatusRequest => sns::matchmaker_status(&mut out),
        ClientMessage::PlayerSessionsRequest { session, xpid, lobby_id, .. } => {
            if conn.user_for(app, session) == Some(xpid) {
                if let Some((entrant, team)) = app.hub.player_sessions(xpid, lobby_id) {
                    sns::entrants_v3(&mut out, xpid, entrant, team);
                }
            }
        }
        ClientMessage::PendingSessionCancel { .. } | ClientMessage::PingResponse | ClientMessage::Ignored(_) => {}
        ClientMessage::Unknown(s) => debug!(symbol = format_args!("{s:#018x}"), "unhandled message"),
    }
    if !out.is_empty() {
        queued.fetch_add(out.len(), Ordering::Relaxed);
        drop(tx.send(out));
    }
    Ok(())
}

// ─── Fleet socket (game servers register here) ──────────────────────────────

/// Game servers on this machine may always register. Others need the fleet
/// token (`?token=` or `Authorization: Bearer`), and none get in when no
/// token is set: a registered server is handed players and their keys.
async fn fleet_ws(
    State(app): State<App>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    if !is_loopback(&addr) {
        let given = q.get("token").map(String::as_str).or_else(|| {
            headers.get(axum::http::header::AUTHORIZATION).and_then(|h| h.to_str().ok()).and_then(|h| h.strip_prefix("Bearer "))
        });
        let ok = match (&app.fleet_token, given) {
            (Some(want), Some(got)) => crate::wire::crypto::ct_eq(want.as_bytes(), got.as_bytes()),
            _ => false,
        };
        if !ok {
            warn!(%addr, "game server registration refused: no or wrong fleet token");
            return (StatusCode::FORBIDDEN, "fleet token required").into_response();
        }
    }
    ws.max_message_size(MAX_FLEET_MESSAGE).on_upgrade(move |s| fleet_socket(app, s, addr)).into_response()
}

async fn fleet_socket(app: App, socket: WebSocket, addr: SocketAddr) {
    let peer_ip = match addr.ip().to_canonical() {
        IpAddr::V4(v) => v,
        IpAddr::V6(v) => v.to_ipv4_mapped().unwrap_or(Ipv4Addr::LOCALHOST),
    };
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<crate::wire::pb::Envelope>();
    let writer = tokio::spawn(async move {
        while let Some(env) = rx.recv().await {
            if sink.send(Message::Binary(env.encode_to_vec())).await.is_err() {
                break;
            }
        }
    });
    let mut server_id = None;
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Binary(data) = msg else {
            if matches!(msg, Message::Close(_)) {
                break;
            }
            continue;
        };
        let env = match crate::wire::pb::Envelope::decode(data.as_slice()) {
            Ok(e) => e,
            Err(e) => {
                warn!(%addr, error = %e, "bad fleet envelope");
                continue;
            }
        };
        match &env.message {
            Some(crate::wire::pb::envelope::Message::GameServerRegistration(reg)) if server_id.is_none() => {
                server_id = Some(app.hub.register_server(tx.clone(), reg, peer_ip));
            }
            _ => {
                if let Some(id) = server_id {
                    app.hub.on_server_message(id, env);
                }
            }
        }
    }
    if let Some(id) = server_id {
        app.hub.unregister_server(id);
    }
    drop(tx);
    writer.abort();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(host: &str, admin: bool) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(axum::http::header::HOST, host.parse().expect("host"));
        if admin {
            h.insert("x-ers-admin", "1".parse().expect("v"));
        }
        h
    }

    #[test]
    fn admin_needs_loopback_a_local_host_name_and_the_header() {
        let local: SocketAddr = "127.0.0.1:5000".parse().expect("addr");
        let remote: SocketAddr = "203.0.113.5:5000".parse().expect("addr");
        assert!(admin_only(&local, &headers("127.0.0.1:7777", true)).is_ok());
        assert!(admin_only(&local, &headers("localhost:7777", true)).is_ok());
        assert!(admin_only(&local, &headers("[::1]:7777", true)).is_ok());
        assert!(admin_only(&local, &headers("127.0.0.1:7777", false)).is_err(), "no header: a web page");
        assert!(admin_only(&local, &headers("evil.example:7777", true)).is_err(), "DNS rebinding");
        assert!(admin_only(&remote, &headers("127.0.0.1:7777", true)).is_err());
    }

    #[test]
    fn connection_slots_are_counted_and_released() {
        let conns = Arc::new(DashMap::new());
        let ip: IpAddr = "203.0.113.5".parse().expect("ip");
        let slots: Vec<_> = (0..MAX_CONNS_PER_IP).map(|_| ConnSlot::take(&conns, ip).expect("slot")).collect();
        assert!(ConnSlot::take(&conns, ip).is_none());
        drop(slots);
        assert!(conns.is_empty());
        assert!(ConnSlot::take(&conns, ip).is_some());
    }
}
