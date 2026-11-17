//! The game-server engine: an all-Rust Echo VR dedicated server.
//!
//! One [`Instance`] owns one UDP port and hosts one lobby or match at a time.
//! It registers with a backend over the fleet protocol (protobuf `Envelope`
//! over websocket: this process's hub, or the EchoRelay/nakama bridge), and
//! runs a single task. While idle the task only waits on its socket and the
//! backend connection, so an idle instance costs no CPU. While a session runs
//! it wakes at 100 Hz for retransmits and at 20 Hz for the server frame.

pub mod balance;
pub mod combat;
pub mod gear;
pub mod filter;
pub mod goal;
pub mod msg;
pub mod props;
pub mod rules;
pub mod session;
pub mod stats;
pub mod transport;
pub mod veilnet;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use nevr_proto::rtapi::v1 as pb;
use nevr_proto::rtapi::v1::envelope::Message as Em;
use prost::Message as _;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tracing::{debug, info, warn};
use uuid::Uuid;

use session::{Session, SessionInfo};

#[derive(Clone, Copy, Debug)]
pub struct EngineConfig {
    /// Arena: players needed on EACH team before the pre-match countdown runs.
    pub arena_min_per_team: usize,
    /// Answer EchoCombat's VeilNet packets.
    pub veilnet: bool,
    /// EchoCombat's MEDIGUN is allowed (only from `echocombat.ini`).
    pub medigun: bool,
}

pub struct Instance {
    pub port: u16,
    pub task: JoinHandle<()>,
    stop: watch::Sender<bool>,
}

impl Instance {
    pub async fn stop(self) {
        self.stop.send_replace(true);
        let mut task = self.task;
        if tokio::time::timeout(Duration::from_secs(5), &mut task).await.is_err() {
            task.abort();
        }
    }
}

pub fn start(idx: u32, socket: UdpSocket, fleet_url: String, cfg: EngineConfig) -> std::io::Result<Instance> {
    let port = socket.local_addr()?.port();
    let (stop, stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        run(idx, socket, fleet_url, stop_rx, cfg).await;
    });
    Ok(Instance { port, task, stop })
}

fn env(m: Em) -> pb::Envelope {
    pb::Envelope { cid: Uuid::new_v4().to_string(), message: Some(m) }
}

fn session_event(lobby: Uuid, code: pb::lobby_session_event_message::Code) -> pb::Envelope {
    env(Em::LobbySessionEvent(pb::LobbySessionEventMessage { lobby_session_id: lobby.to_string(), code: code as i32 }))
}

/// Mode and level from the backend's session settings.
fn session_info(c: &pb::LobbySessionCreateMessage) -> SessionInfo {
    let lobby = c.lobby_session_id.parse().unwrap_or_else(|_| Uuid::new_v4());
    let settings: serde_json::Value = serde_json::from_str(&c.settings_json).unwrap_or_default();
    let num = |k: &str| {
        settings.get(k).and_then(|v| v.as_i64().map(|n| n as u64).or_else(|| v.as_u64()).or_else(|| v.as_str().map(crate::sns::symbol)))
    };
    let mode = num("gametype").filter(|m| *m != 0).unwrap_or(msg::SOCIAL_MODES[0]);
    let arena_modes = ["echo_arena", "echo_arena_private", "echo_arena_tournament", "echo_arena_public_ai", "mpl_tutorial_arena"];
    let combat_modes = ["echo_combat", "echo_combat_private", "echo_combat_tournament"];
    let combat = combat_modes.iter().any(|m| crate::sns::symbol(m) == mode);
    let level = num("level").filter(|l| *l != 0 && *l != u64::MAX);
    let level = match level {
        // A combat session needs a combat map: EchoRelay's public combat queue
        // sends none, which used to land players in the social lobby as combat
        // players. Pick one of the maps this server hosts.
        _ if combat && !level.is_some_and(|l| combat::LEVELS.contains(&l)) => combat_map(mode),
        Some(l) => l,
        None if arena_modes.iter().any(|m| crate::sns::symbol(m) == mode) => msg::LEVEL_ARENA,
        None => msg::LEVEL_SOCIAL,
    };
    SessionInfo { lobby, mode, level }
}

/// A random combat map this server hosts for `mode` (any, if it hosts none).
fn combat_map(mode: u64) -> u64 {
    let ok: Vec<u64> = combat::LEVELS.iter().copied().filter(|&l| filter::allows(mode, l)).collect();
    let from = if ok.is_empty() { combat::LEVELS.to_vec() } else { ok };
    from[rand::random::<usize>() % from.len()]
}

async fn run(idx: u32, socket: UdpSocket, url: String, mut stop: watch::Receiver<bool>, cfg: EngineConfig) {
    let port = socket.local_addr().map(|a| a.port()).unwrap_or(0);
    let mut backoff = Duration::from_millis(200);
    loop {
        if *stop.borrow() {
            return;
        }
        let ws = match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((ws, _)) => ws,
            Err(e) => {
                debug!(instance = idx, error = %e, "backend not reachable; retrying");
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = stop.changed() => return,
                }
                backoff = (backoff * 2).min(Duration::from_secs(5));
                continue;
            }
        };
        backoff = Duration::from_millis(200);
        let ended = serve(idx, port, &socket, ws, &mut stop, cfg).await;
        if ended {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// One backend connection. Returns true when the instance should stop.
/// Server frames for every game server, from one thread that sleeps to each
/// frame's own time. A tokio interval lands on whole milliseconds, so 120 Hz
/// frames came 8, 8 and 9 ms apart, and each client saw everyone's states
/// arrive that unevenly. The thread's sleep is a high-resolution timer on
/// Windows and Linux alike; a late frame is skipped, not caught up.
fn frame_ticks() -> watch::Receiver<u64> {
    static TICKS: std::sync::OnceLock<watch::Sender<u64>> = std::sync::OnceLock::new();
    TICKS
        .get_or_init(|| {
            let (tx, _) = watch::channel(0u64);
            let ticker = tx.clone();
            let run = move || {
                let mut next = Instant::now();
                loop {
                    next += Duration::from_nanos(1_000_000_000 / session::frame_hz().max(1));
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else {
                        next = now;
                    }
                    ticker.send_modify(|n| *n = n.wrapping_add(1));
                }
            };
            if let Err(e) = std::thread::Builder::new().name("frame-clock".into()).spawn(run) {
                tracing::error!(error = %e, "frame clock thread did not start");
            }
            tx
        })
        .subscribe()
}

async fn serve(idx: u32, port: u16, socket: &UdpSocket, ws: Ws, stop: &mut watch::Receiver<bool>, cfg: EngineConfig) -> bool {
    let (mut tx, mut rx) = ws.split();
    let reg = env(Em::GameServerRegistration(pb::GameServerRegistrationMessage {
        server_id: u64::from(idx) << 16 | u64::from(port),
        internal_ip_address: "127.0.0.1".into(),
        port: u32::from(port),
        time_step_usecs: 1_000_000 / 120,
        version: env!("CARGO_PKG_VERSION").into(),
        ..Default::default()
    }));
    if tx.send(WsMsg::Binary(reg.encode_to_vec())).await.is_err() {
        return false;
    }
    let mut session: Option<Session> = None;
    // Players' keys that came just before their session (nakama sends them early).
    let mut early_keys: Vec<pb::SnsLobbySessionSuccessV5Message> = Vec::new();
    let mut veil = veilnet::VeilNet::new(cfg.veilnet, cfg.medigun);
    let mut buf = vec![0u8; 2048];
    // Service (flush) every 10 ms; game frames at `--frame-hz`.
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frames = frame_ticks();
    let mut outbox: Vec<pb::Envelope> = Vec::new();

    let result = loop {
        tokio::select! {
            _ = stop.changed() => {
                if let Some(mut s) = session.take() {
                    s.close();
                    send_udp(socket, &mut s).await;
                    outbox.push(session_event(s.info.lobby, pb::lobby_session_event_message::Code::Ended));
                }
                for e in outbox.drain(..) {
                    let _ = tx.send(WsMsg::Binary(e.encode_to_vec())).await;
                }
                let _ = tx.close().await;
                break true;
            }
            m = rx.next() => {
                let data = match m {
                    Some(Ok(WsMsg::Binary(d))) => d,
                    Some(Ok(WsMsg::Ping(p))) => { let _ = tx.send(WsMsg::Pong(p)).await; continue }
                    Some(Ok(WsMsg::Text(t))) => { on_mute(idx, &t, session.as_mut()); continue }
                    Some(Ok(WsMsg::Close(_))) | None | Some(Err(_)) => break false,
                    Some(Ok(_)) => continue,
                };
                let Ok(e) = pb::Envelope::decode(data.as_slice()) else { continue };
                on_fleet(idx, port, e, &mut session, &mut early_keys, &mut outbox, cfg);
            }
            r = socket.recv_from(&mut buf) => {
                let Ok((n, from)) = r else { continue };
                handle_datagram(socket, &mut session, &mut veil, from, &buf[..n]).await;
                // Drain whatever else is already queued, then answer in one go.
                for _ in 0..256 {
                    match socket.try_recv_from(&mut buf) {
                        Ok((n, from)) => handle_datagram(socket, &mut session, &mut veil, from, &buf[..n]).await,
                        Err(_) => break,
                    }
                }
                if let Some(s) = session.as_mut() {
                    s.flush_reliable();
                    send_udp(socket, s).await;
                    outbox.append(&mut s.fleet_out);
                }
            }
            _ = frames.changed(), if session.is_some() => {
                let s = session.as_mut().expect("guarded");
                s.server_frame();
                send_udp(socket, s).await;
                outbox.append(&mut s.fleet_out);
            }
            _ = tick.tick(), if session.is_some() => {
                let s = session.as_mut().expect("guarded");
                let mut veil_out = veilnet::Out::new();
                veil.tick(Instant::now(), &mut veil_out);
                send_veil(socket, veil_out).await;
                s.service();
                send_udp(socket, s).await;
                outbox.append(&mut s.fleet_out);
                // Release the server once nobody is left.
                let idle_for = s.last_activity.elapsed();
                let limit = if s.ever_joined { Duration::from_secs(3) } else { Duration::from_secs(90) };
                if s.peer_count() == 0 && idle_for > limit {
                    info!(instance = idx, lobby = %s.info.lobby, "session empty; ending");
                    outbox.push(session_event(s.info.lobby, pb::lobby_session_event_message::Code::Ended));
                    session = None;
                }
            }
        }
        for e in outbox.drain(..) {
            if tx.send(WsMsg::Binary(e.encode_to_vec())).await.is_err() {
                break;
            }
        }
    };
    if let Some(mut s) = session.take() {
        s.close();
        send_udp(socket, &mut s).await;
    }
    result
}

/// This host's public IPv4 (what players are given), for the Ipv6Bridge
/// plugin's question; 0 until known.
static BRIDGE_IPV4: AtomicU32 = AtomicU32::new(0);

pub fn set_bridge_ipv4(ip: Ipv4Addr) {
    BRIDGE_IPV4.store(u32::from(ip), Ordering::Relaxed);
}

/// A dual-stack socket reports IPv4 players as `::ffff:a.b.c.d`: the rest
/// of the server sees them as plain IPv4, as on an IPv4-only socket.
fn canonical(a: SocketAddr) -> SocketAddr {
    SocketAddr::new(a.ip().to_canonical(), a.port())
}

/// Sends to `to`, mapping an IPv4 player into IPv6 form on a dual-stack socket.
async fn send_to(socket: &UdpSocket, v6: bool, d: &[u8], to: SocketAddr) -> std::io::Result<usize> {
    let to = match to {
        SocketAddr::V4(v) if v6 => SocketAddr::new(v.ip().to_ipv6_mapped().into(), v.port()),
        _ => to,
    };
    socket.send_to(d, to).await
}

async fn handle_datagram(socket: &UdpSocket, session: &mut Option<Session>, veil: &mut veilnet::VeilNet, from: SocketAddr, d: &[u8]) {
    let from = canonical(from);
    // EchoCombat's own packets, on the game's socket (never game traffic).
    if let Some((kind, id)) = veilnet::parse(d) {
        veil.sync_session(session.as_ref().map(|s| (s.info.lobby, s.info.mode, s.info.level)));
        let mut out = veilnet::Out::new();
        veil.handle(from, kind, id, Instant::now(), &mut out);
        send_veil(socket, out).await;
        return;
    }
    if d.len() == 16 && u64::from_le_bytes(d[..8].try_into().expect("8")) == msg::RAW_PING_REQUEST {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&msg::RAW_PING_ACK.to_le_bytes());
        out[8..].copy_from_slice(&d[8..]);
        let _ = send_to(socket, is_v6(socket), &out, from).await;
        return;
    }
    if d.len() == 16 && u64::from_le_bytes(d[..8].try_into().expect("8")) == msg::BRIDGE_QUERY {
        let ip = BRIDGE_IPV4.load(Ordering::Relaxed);
        if ip != 0 {
            let mut out = [0u8; 20];
            out[..8].copy_from_slice(&msg::BRIDGE_ANSWER.to_le_bytes());
            out[8..16].copy_from_slice(&d[8..]);
            out[16..].copy_from_slice(&Ipv4Addr::from(ip).octets());
            let _ = send_to(socket, is_v6(socket), &out, from).await;
            debug!(%from, ipv4 = %Ipv4Addr::from(ip), "ipv6 bridge query answered");
        }
        return;
    }
    if let Some(s) = session.as_mut() {
        s.on_packet(from, d);
    }
}

fn is_v6(socket: &UdpSocket) -> bool {
    socket.local_addr().is_ok_and(|l| l.is_ipv6())
}

async fn send_veil(socket: &UdpSocket, out: veilnet::Out) {
    let v6 = is_v6(socket);
    for (to, d) in out {
        if let Err(e) = send_to(socket, v6, &d, to).await {
            debug!(%to, error = %e, "udp send failed");
        }
    }
}

async fn send_udp(socket: &UdpSocket, s: &mut Session) {
    let v6 = is_v6(socket);
    for (to, d) in s.udp_out.drain(..) {
        if let Err(e) = send_to(socket, v6, &d, to).await {
            debug!(%to, error = %e, "udp send failed");
        }
    }
}

/// atlas's server mute (`ERGameServerMuteEntrants`), which the bridge passes
/// on as a text frame: the fleet protobuf has no message for it.
#[derive(serde::Deserialize)]
pub(crate) struct MuteEntrants {
    pub entrant_ids: Vec<uuid::Uuid>,
    pub muted: bool,
}

fn on_mute(idx: u32, text: &str, session: Option<&mut Session>) {
    let m: MuteEntrants = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            warn!(instance = idx, error = %e, "unreadable mute from the backend");
            return;
        }
    };
    let Some(s) = session else {
        debug!(instance = idx, "mute with no session running; dropped");
        return;
    };
    let changed = s.set_muted(&m.entrant_ids, m.muted);
    info!(instance = idx, lobby = %s.info.lobby, muted = m.muted, entrants = ?m.entrant_ids, changed, "server mute");
}

fn on_fleet(
    idx: u32,
    port: u16,
    e: pb::Envelope,
    session: &mut Option<Session>,
    early_keys: &mut Vec<pb::SnsLobbySessionSuccessV5Message>,
    out: &mut Vec<pb::Envelope>,
    cfg: EngineConfig,
) {
    let Some(m) = e.message else { return };
    match m {
        Em::GameServerRegistrationSuccess(r) => {
            debug!(instance = idx, port, server_id = r.server_id, external_ip = %r.external_ip_address, "registered");
        }
        Em::LobbySessionCreate(c) => {
            if let Some(mut old) = session.take() {
                old.close();
            }
            let info = session_info(&c);
            if !filter::allows(info.mode, info.level) {
                warn!(
                    instance = idx, port, lobby = %info.lobby,
                    mode = %filter::mode_name(info.mode), level = %filter::level_name(info.level),
                    "session refused: mode or map not hosted here"
                );
                out.push(session_event(info.lobby, pb::lobby_session_event_message::Code::Ended));
                return;
            }
            info!(
                instance = idx, port, lobby = %info.lobby,
                mode = format_args!("{:#018x}", info.mode), level = format_args!("{:#018x}", info.level),
                "session started"
            );
            let lobby = info.lobby;
            let mut sess = Session::new(info, cfg.arena_min_per_team);
            for k in early_keys.drain(..) {
                if k.lobby_id.parse::<Uuid>().map_or(true, |l| l == lobby || l.is_nil()) {
                    sess.add_keys(&k);
                }
            }
            *session = Some(sess);
            out.push(session_event(lobby, pb::lobby_session_event_message::Code::Started));
        }
        Em::LobbySessionSuccessV5(s) => match session.as_mut() {
            Some(sess) => sess.add_keys(&s),
            None => {
                // Kept for the session that is about to start.
                if early_keys.len() < 64 {
                    early_keys.push(s);
                }
                debug!(instance = idx, "entrant keys arrived before the session; holding them");
            }
        },
        Em::LobbyEntrantsAccept(a) => {
            if let Some(s) = session.as_mut() {
                s.on_accept(&a.entrant_ids, true);
                out.append(&mut s.fleet_out);
            }
        }
        Em::LobbyEntrantReject(r) => {
            if let Some(s) = session.as_mut() {
                s.on_accept(&r.entrant_ids, false);
                out.append(&mut s.fleet_out);
            }
        }
        Em::LobbyEntrantRemoved(r) => {
            if let Some(s) = session.as_mut() {
                s.kick(&r.entrant_id);
                s.fleet_out.clear(); // the backend already knows
            }
        }
        Em::LobbySessionEvent(ev) if ev.code == pb::lobby_session_event_message::Code::Ended as i32 => {
            if let Some(mut s) = session.take() {
                info!(instance = idx, lobby = %s.info.lobby, "backend ended the session");
                s.close();
                // Datagrams left in s.udp_out are best effort; the peers time out anyway.
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(settings: &str) -> SessionInfo {
        session_info(&pb::LobbySessionCreateMessage { settings_json: settings.into(), ..Default::default() })
    }

    #[test]
    fn combat_sessions_get_a_combat_map() {
        let combat = crate::sns::symbol("echo_combat") as i64;
        // EchoRelay's public combat queue: no map, or the social lobby's.
        for s in [format!(r#"{{"gametype":{combat}}}"#), format!(r#"{{"gametype":{combat},"level":{}}}"#, msg::LEVEL_SOCIAL as i64)] {
            assert!(combat::LEVELS.contains(&info(&s).level), "{s}");
        }
        // A combat map asked for is kept.
        let s = format!(r#"{{"gametype":{combat},"level":{}}}"#, combat::LEVEL_FISSION as i64);
        assert_eq!(info(&s).level, combat::LEVEL_FISSION);
        // Social and arena unchanged.
        assert_eq!(info("{}").level, msg::LEVEL_SOCIAL);
        let arena = crate::sns::symbol("echo_arena") as i64;
        assert_eq!(info(&format!(r#"{{"gametype":{arena}}}"#)).level, msg::LEVEL_ARENA);
    }
}
