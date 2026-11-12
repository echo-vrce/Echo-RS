//! Client-facing services on one TCP port: the Echo VR client's login, config,
//! matching and transaction websockets (any path; messages are dispatched by
//! type, so one URL serves them all), the game-server fleet socket, and a small
//! HTTP API.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{any, get};
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
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/fleet", get(fleet_ws))
        .route("/upstream/:idx", get(crate::upstream::upstream_ws))
        .route("/status", get(status))
        .route("/admin/kick", get(admin_kick))
        .route("/admin/ban", get(admin_ban))
        .fallback(any(client_or_api))
        .with_state(app)
}

async fn status(State(app): State<App>) -> Json<crate::lobby::Stats> {
    Json(app.hub.stats())
}

fn loopback_only(addr: &SocketAddr) -> Result<(), Response> {
    if addr.ip().is_loopback() { Ok(()) } else { Err((StatusCode::FORBIDDEN, "admin endpoints are loopback-only").into_response()) }
}

async fn admin_kick(State(app): State<App>, ConnectInfo(addr): ConnectInfo<SocketAddr>, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = loopback_only(&addr) {
        return r;
    }
    let Some(x) = q.get("xpid").and_then(|s| XpId::parse(s)) else {
        return (StatusCode::BAD_REQUEST, "xpid=OVR_ORG-123 required").into_response();
    };
    Json(json!({"kicked": app.hub.kick(x)})).into_response()
}

async fn admin_ban(State(app): State<App>, ConnectInfo(addr): ConnectInfo<SocketAddr>, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(r) = loopback_only(&addr) {
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
        Some(ws) => ws
            .max_message_size(sns::MAX_PACKET)
            .on_upgrade(move |socket| client_socket(app, socket, addr, uri.path().to_owned(), q))
            .into_response(),
        None => api(uri.path()).into_response(),
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
async fn client_socket(app: App, socket: WebSocket, addr: SocketAddr, path: String, q: HashMap<String, String>) {
    debug!(%addr, %path, "client service connected");
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    // Single writer task: replies from the hub (possibly delayed) and from
    // this loop are serialized here.
    let writer = tokio::spawn(async move {
        while let Some(mut buf) = rx.recv().await {
            // Coalesce anything else already queued into one frame.
            while let Ok(more) = rx.try_recv() {
                buf.extend_from_slice(&more);
            }
            if sink.send(Message::Binary(buf)).await.is_err() {
                break;
            }
        }
    });

    let mut conn = Conn { user: None, login_session: None };
    while let Some(Ok(msg)) = stream.next().await {
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
            if let Err(e) = handle(&app, &mut conn, decoded, &q, &tx).await {
                warn!(%addr, error = %e, "request failed");
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

async fn handle(app: &App, conn: &mut Conn, msg: ClientMessage, q: &HashMap<String, String>, tx: &mpsc::UnboundedSender<Vec<u8>>) -> anyhow::Result<()> {
    let mut out = Vec::new();
    match msg {
        ClientMessage::LoginRequest { xpid, profile, .. } => {
            if xpid.is_nil() {
                sns::login_failure(&mut out, xpid, 400, "Invalid user id");
                sns::unrequire(&mut out);
            } else {
                let name = q
                    .get("displayname")
                    .cloned()
                    .or_else(|| profile.get("displayname").and_then(Value::as_str).map(str::to_owned))
                    .unwrap_or_default();
                match app.store().login(xpid, &name, q.get("auth").map(String::as_str)) {
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
                v["lang"] = json!(lang);
                sns::document_success(&mut out, &kind, &serde_json::to_vec(&v)?)?;
            }
        }
        ClientMessage::ChannelInfoRequest => {
            if let Some(j) = app.store().resource("channel_info") {
                sns::channel_info(&mut out, &j)?;
            }
        }
        ClientMessage::UpdateProfile { session, xpid, profile } => {
            if conn.user_for(app, session) == Some(xpid) {
                app.store().update_client_profile(xpid, profile);
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
        drop(tx.send(out));
    }
    Ok(())
}

// ─── Fleet socket (nevr game servers register here) ─────────────────────────

async fn fleet_ws(State(app): State<App>, ConnectInfo(addr): ConnectInfo<SocketAddr>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |s| fleet_socket(app, s, addr)).into_response()
}

async fn fleet_socket(app: App, socket: WebSocket, addr: SocketAddr) {
    let peer_ip = match addr.ip() {
        IpAddr::V4(v) => v,
        IpAddr::V6(v) => v.to_ipv4_mapped().unwrap_or(Ipv4Addr::LOCALHOST),
    };
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<nevr_proto::Envelope>();
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
        let env = match nevr_proto::Envelope::decode(data.as_slice()) {
            Ok(e) => e,
            Err(e) => {
                warn!(%addr, error = %e, "bad fleet envelope");
                continue;
            }
        };
        match &env.message {
            Some(nevr_proto::envelope::Message::GameServerRegistration(reg)) if server_id.is_none() => {
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
