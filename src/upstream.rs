//! Bridges for embedded game servers that serve an external backend instead
//! of this process's own matchmaker.
//!
//! Each embedded nevr instance connects its fleet client to
//! `ws://127.0.0.1:<port>/upstream/<idx>`; this module opens the matching
//! upstream connection and relays:
//!
//! * **nakama**: nakama's fleet speaks the same protobuf `Envelope` protocol
//!   (bare, or wrapped in the EVR `NEVRProtobufMessageV1` frame in legacy
//!   mode). Frames are relayed as-is; only the registration's internal IP is
//!   rewritten from loopback to this host's LAN address.
//! * **EchoRelay**: ServerDB speaks the `ERGameServer*` SNS messages its
//!   `pnsradgameserver.dll` uses. Every message is translated both ways.
//!
//! Both bridges note when an instance is busy, so the pool can scale.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use axum::extract::ws::{Message as AxMsg, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, State};
use axum::response::{IntoResponse, Response};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use nevr_proto::envelope::Message as Em;
use nevr_proto::rtapi::v1 as pb;
use prost::Message as _;
use tokio_tungstenite::tungstenite::Message as TMsg;
use tracing::{debug, error, info, warn};

use crate::lobby::{VERSION_LOCK, env_msg};
use crate::services::App;
use crate::sns::{self, Reader, Writer, sym};

const NEVR_PROTOBUF_V1: u64 = 0x9ee5_107d_9e29_fd63;

#[derive(Clone, Debug)]
pub enum Backend {
    /// This process's own services (instances register on `/fleet`).
    Builtin,
    Nakama { url: String },
    EchoRelay { url: String },
}

#[derive(Default)]
pub struct InstanceState {
    pub busy: AtomicBool,
    pub idle_since_ms: AtomicU64,
}

pub struct Upstreams {
    pub backend: Backend,
    pub lan_ip: Ipv4Addr,
    pub region: u64,
    /// Instance index → occupancy.
    pub instances: DashMap<u32, Arc<InstanceState>>,
    start: Instant,
}

impl Upstreams {
    pub fn new(backend: Backend, lan_ip: Ipv4Addr, region: &str) -> Arc<Self> {
        Arc::new(Self { backend, lan_ip, region: sns::symbol(region), instances: DashMap::new(), start: Instant::now() })
    }
    fn state(&self, idx: u32) -> Arc<InstanceState> {
        self.instances.entry(idx).or_default().clone()
    }
    fn set_busy(&self, idx: u32, busy: bool) {
        let st = self.state(idx);
        st.busy.store(busy, Ordering::Relaxed);
        if !busy {
            st.idle_since_ms.store(self.start.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
    }
    /// Instances idle for at least `min_idle`.
    pub fn idle_instances(&self, min_idle: Duration) -> Vec<u32> {
        let now = self.start.elapsed().as_millis() as u64;
        self.instances
            .iter()
            .filter(|e| !e.busy.load(Ordering::Relaxed) && now.saturating_sub(e.idle_since_ms.load(Ordering::Relaxed)) >= min_idle.as_millis() as u64)
            .map(|e| *e.key())
            .collect()
    }
    pub fn busy_count(&self) -> usize {
        self.instances.iter().filter(|e| e.busy.load(Ordering::Relaxed)).count()
    }
}

pub async fn upstream_ws(State(app): State<App>, ConnectInfo(addr): ConnectInfo<SocketAddr>, Path(idx): Path<u32>, ws: WebSocketUpgrade) -> Response {
    if !addr.ip().is_loopback() {
        return (axum::http::StatusCode::FORBIDDEN, "local game servers only").into_response();
    }
    let up = app.upstreams.clone();
    ws.on_upgrade(move |s| async move {
        up.set_busy(idx, false);
        let r = match up.backend.clone() {
            Backend::Nakama { url } => bridge_nakama(&up, idx, s, &url).await,
            Backend::EchoRelay { url } => bridge_echorelay(&up, idx, s, &url).await,
            Backend::Builtin => Ok(()),
        };
        up.set_busy(idx, false);
        if let Err(e) = r {
            warn!(instance = idx, error = %e, "upstream bridge closed");
        }
    })
    .into_response()
}

// ─── nakama ─────────────────────────────────────────────────────────────────

/// Decode an envelope from either framing.
fn decode_any(data: &[u8]) -> Option<(pb::Envelope, bool)> {
    if data.starts_with(&sns::MARKER) {
        let msgs = sns::split(data).ok()?;
        msgs.into_iter().find(|(s, _)| *s == NEVR_PROTOBUF_V1).and_then(|(_, p)| pb::Envelope::decode(p).ok()).map(|e| (e, true))
    } else {
        pb::Envelope::decode(data).ok().map(|e| (e, false))
    }
}

fn encode_as(env: &pb::Envelope, legacy: bool) -> Vec<u8> {
    let b = env.encode_to_vec();
    if legacy {
        let mut out = Vec::with_capacity(b.len() + 24);
        sns::frame(&mut out, NEVR_PROTOBUF_V1, &b);
        out
    } else {
        b
    }
}

fn observe(up: &Upstreams, idx: u32, env: &pb::Envelope) {
    match &env.message {
        Some(Em::LobbySessionCreate(_)) => up.set_busy(idx, true),
        Some(Em::LobbySessionEvent(e)) if e.code == pb::lobby_session_event_message::Code::Ended as i32 => up.set_busy(idx, false),
        _ => {}
    }
}

async fn bridge_nakama(up: &Upstreams, idx: u32, local: WebSocket, url: &str) -> Result<()> {
    let (remote, _) = tokio_tungstenite::connect_async(url).await.with_context(|| format!("connecting to nakama at {url}"))?;
    info!(instance = idx, "bridged to nakama");
    let (mut rtx, mut rrx) = remote.split();
    let (mut ltx, mut lrx) = local.split();
    loop {
        tokio::select! {
            m = lrx.next() => {
                let Some(Ok(m)) = m else { break };
                let AxMsg::Binary(mut data) = m else { if matches!(m, AxMsg::Close(_)) { break } continue };
                if let Some((mut env, legacy)) = decode_any(&data) {
                    if let Some(Em::GameServerRegistration(reg)) = &mut env.message {
                        if reg.internal_ip_address.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback()) {
                            reg.internal_ip_address = up.lan_ip.to_string();
                            data = encode_as(&env, legacy);
                        }
                    }
                    observe(up, idx, &env);
                }
                rtx.send(TMsg::Binary(data)).await?;
            }
            m = rrx.next() => {
                let Some(Ok(m)) = m else { break };
                match m {
                    TMsg::Binary(data) => {
                        if let Some((env, _)) = decode_any(&data) { observe(up, idx, &env); }
                        ltx.send(AxMsg::Binary(data)).await?;
                    }
                    TMsg::Ping(p) => rtx.send(TMsg::Pong(p)).await?,
                    TMsg::Close(_) => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

// ─── EchoRelay ServerDB ─────────────────────────────────────────────────────

fn er_message(symbol: u64, w: Writer) -> Vec<u8> {
    let mut out = Vec::new();
    w.finish(symbol, &mut out);
    out
}

fn one_byte(symbol: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(0);
    er_message(symbol, w)
}

/// nevr → EchoRelay. `None` means the envelope has no ServerDB equivalent.
fn to_echorelay(up: &Upstreams, idx: u32, env: &pb::Envelope) -> Option<Vec<u8>> {
    Some(match env.message.as_ref()? {
        Em::GameServerRegistration(reg) => {
            let internal = reg.internal_ip_address.parse::<Ipv4Addr>().ok().filter(|ip| !ip.is_loopback()).unwrap_or(up.lan_ip);
            let mut w = Writer::new();
            w.u64(reg.server_id).ipv4(internal).u16(reg.port as u16).zeros(2).u64(up.region).u64(VERSION_LOCK);
            er_message(sym::ER_REGISTRATION_REQUEST, w)
        }
        Em::LobbySessionEvent(e) => match pb::lobby_session_event_message::Code::try_from(e.code) {
            Ok(pb::lobby_session_event_message::Code::Started) => one_byte(sym::ER_SESSION_STARTED),
            Ok(pb::lobby_session_event_message::Code::Ended) => {
                up.set_busy(idx, false);
                one_byte(sym::ER_END_SESSION)
            }
            Ok(pb::lobby_session_event_message::Code::Locked) => one_byte(sym::ER_PLAYERS_LOCKED),
            Ok(pb::lobby_session_event_message::Code::Unlocked) => one_byte(sym::ER_PLAYERS_UNLOCKED),
            _ => return None,
        },
        Em::LobbyEntrantConnected(m) => {
            let mut w = Writer::new();
            for id in &m.entrant_ids {
                w.guid(id.parse().ok()?);
            }
            er_message(sym::ER_ACCEPT_PLAYERS, w)
        }
        Em::LobbyEntrantRemoved(m) => {
            let mut w = Writer::new();
            w.guid(m.entrant_id.parse().ok()?);
            er_message(sym::ER_REMOVE_PLAYER, w)
        }
        _ => return None,
    })
}

/// EchoRelay → nevr.
fn from_echorelay(up: &Upstreams, idx: u32, symbol: u64, p: &[u8]) -> Result<Option<pb::Envelope>> {
    let mut r = Reader::new(p);
    Ok(Some(match symbol {
        sym::ER_REGISTRATION_SUCCESS => {
            let server_id = r.u64()?;
            let ip = r.ipv4()?;
            info!(instance = idx, server_id, external_ip = %ip, "registered with EchoRelay ServerDB");
            env_msg(Em::GameServerRegistrationSuccess(pb::GameServerRegistrationSuccessMessage { server_id, external_ip_address: ip.to_string() }))
        }
        sym::ER_REGISTRATION_FAILURE => {
            let code = r.u8().unwrap_or(0);
            bail!("EchoRelay refused registration (code {code}: 0 bad request, 1 kicked/banned, 2 connection test failed, 3 database error, 4 api key/restricted)");
        }
        sym::ER_START_SESSION => {
            let session = r.guid()?;
            let channel = r.guid()?;
            let limit = r.u8()?;
            let _entrant_descriptors = r.u8()?;
            let lobby_type = r.u8()?;
            r.skip(1)?;
            let settings = r.json_nt()?;
            up.set_busy(idx, true);
            info!(instance = idx, lobby = %session, "EchoRelay started a session");
            env_msg(Em::LobbySessionCreate(pb::LobbySessionCreateMessage {
                lobby_session_id: session.to_string(),
                lobby_type: i32::from(lobby_type),
                group_id: channel.to_string(),
                max_entrants: i32::from(limit),
                settings_json: settings.to_string(),
                features: Vec::new(),
                time_step_usecs: 0,
            }))
        }
        sym::SESSION_SUCCESS_V5 => {
            let s = sns::decode_session_success_v5(p)?;
            env_msg(Em::LobbySessionSuccessV5(s.to_proto()))
        }
        sym::ER_PLAYERS_ACCEPTED => {
            r.skip(1)?;
            let mut ids = Vec::new();
            while r.remaining() >= 16 {
                ids.push(r.guid()?.to_string());
            }
            info!(instance = idx, players = ids.len(), "EchoRelay accepted players");
            env_msg(Em::LobbyEntrantsAccept(pb::LobbyEntrantsAcceptMessage { entrant_ids: ids }))
        }
        sym::ER_PLAYERS_REJECTED => {
            let code = r.u8()?;
            let mut ids = Vec::new();
            while r.remaining() >= 16 {
                ids.push(r.guid()?.to_string());
            }
            warn!(instance = idx, players = ids.len(), code, "EchoRelay rejected players");
            env_msg(Em::LobbyEntrantReject(pb::LobbyEntrantsRejectMessage { entrant_ids: ids, code: i32::from(code) }))
        }
        sym::SESSION_SUCCESS_V4 | sym::UNREQUIRE => return Ok(None),
        other => {
            debug!(instance = idx, symbol = format_args!("{other:#018x}"), "unhandled ServerDB message");
            return Ok(None);
        }
    }))
}

async fn bridge_echorelay(up: &Upstreams, idx: u32, local: WebSocket, url: &str) -> Result<()> {
    let (remote, _) = tokio_tungstenite::connect_async(url).await.with_context(|| format!("connecting to EchoRelay ServerDB at {url}"))?;
    debug!(instance = idx, "connected to EchoRelay ServerDB");
    let (mut rtx, mut rrx) = remote.split();
    let (mut ltx, mut lrx) = local.split();
    loop {
        tokio::select! {
            m = lrx.next() => {
                let Some(Ok(m)) = m else { break };
                let AxMsg::Binary(data) = m else { if matches!(m, AxMsg::Close(_)) { break } continue };
                let Ok(env) = pb::Envelope::decode(data.as_slice()) else { continue };
                if let Some(out) = to_echorelay(up, idx, &env) {
                    rtx.send(TMsg::Binary(out)).await?;
                }
            }
            m = rrx.next() => {
                let Some(Ok(m)) = m else { break };
                match m {
                    TMsg::Binary(data) => {
                        for (symbol, payload) in sns::split(&data)? {
                            match from_echorelay(up, idx, symbol, payload) {
                                Ok(Some(env)) => ltx.send(AxMsg::Binary(env.encode_to_vec())).await?,
                                Ok(None) => {}
                                Err(e) => { error!(instance = idx, error = %e, "ServerDB"); return Err(e) }
                            }
                        }
                    }
                    TMsg::Ping(p) => rtx.send(TMsg::Pong(p)).await?,
                    TMsg::Close(_) => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
