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
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
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
use uuid::Uuid;

use crate::lobby::{VERSION_LOCK, env_msg};
use crate::services::App;
use crate::sns::{self, Reader, Writer, sym};

const NEVR_PROTOBUF_V1: u64 = 0x9ee5_107d_9e29_fd63;
/// nakama's own ServerDB message for a loadout a player saved
/// (`ERGameServerSaveLoadoutRequest`). EchoRelay uses this symbol for
/// something else, so it only goes to nakama.
const NAKAMA_SAVE_LOADOUT: u64 = 0x7777_7777_7777_0B00;
/// `SNSUserServerProfileUpdateRequest`, sent to EchoRelay's login service.
const PROFILE_UPDATE_REQUEST: u64 = 0xd298_6849_b36b_9c72;

/// Players' platform ids by entrant session, noted by the game servers in
/// this process (the backend messages carry only the session).
static ENTRANTS: LazyLock<DashMap<Uuid, [u64; 2]>> = LazyLock::new(DashMap::new);

pub fn note_entrant(entrant: Uuid, xpid: [u64; 2]) {
    if !entrant.is_nil() {
        ENTRANTS.insert(entrant, xpid);
    }
}

/// Cosmetic and loadout slot names by symbol: EchoRelay stores loadouts by
/// name, while the game sends symbols.
static COSMETIC_NAMES: LazyLock<HashMap<u64, &'static str>> = LazyLock::new(|| {
    const SLOTS: &[&str] = &[
        "unified", "banner", "booster", "bracer", "chassis", "decal", "decal_body", "decalback", "decalborder", "emissive", "emote",
        "secondemote", "goal_fx", "medal", "pattern", "pattern_body", "pip", "tag", "tint", "tint_alignment_a", "tint_alignment_b",
        "tint_body", "title",
    ];
    include_str!("engine/data/cosmetic_names.txt")
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .chain(SLOTS.iter().copied())
        .map(|n| (sns::symbol(n), n))
        .collect()
});

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
    /// EchoRelay's login service, where loadout changes are saved.
    pub login_url: Option<String>,
    /// Instance index → occupancy.
    pub instances: DashMap<u32, Arc<InstanceState>>,
    /// The registration request's padding after the port: nakama reads 10
    /// bytes, EchoRelay 2. Nakama's layout goes first; a backend that ends the
    /// registration without ever sounding like nakama gets EchoRelay's next.
    wide_registration: AtomicBool,
    registration_confirmed: AtomicBool,
    start: Instant,
}

impl Upstreams {
    pub fn new(backend: Backend, lan_ip: Ipv4Addr, region: &str, login_url: Option<String>) -> Arc<Self> {
        let login_url = login_url.filter(|u| !u.trim().is_empty()).or_else(|| match &backend {
            Backend::EchoRelay { url } | Backend::Nakama { url } if is_serverdb(url) => Some(login_from_serverdb(url)),
            _ => None,
        });
        Arc::new(Self { backend, lan_ip, region: sns::symbol(region), login_url, instances: DashMap::new(), wide_registration: AtomicBool::new(true), registration_confirmed: AtomicBool::new(false), start: Instant::now() })
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
    if !addr.ip().to_canonical().is_loopback() {
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

/// A nakama `serverdb_host` (`…/serverdb?…`) speaks EchoRelay's ServerDB
/// protocol, the one the game's own server DLL uses, not nevr's protobuf fleet.
fn is_serverdb(url: &str) -> bool {
    url.split('?').next().is_some_and(|p| p.trim_end_matches('/').ends_with("/serverdb"))
}

/// Why a remote socket ended, for the log.
fn ended(m: Option<Result<TMsg, tokio_tungstenite::tungstenite::Error>>) -> String {
    match m {
        None => "connection closed".into(),
        Some(Err(e)) => e.to_string(),
        Some(Ok(TMsg::Close(Some(f)))) => format!("closed by server: {} {}", u16::from(f.code), f.reason),
        Some(Ok(TMsg::Close(None))) => "closed by server".into(),
        Some(Ok(_)) => "ended".into(),
    }
}

async fn bridge_nakama(up: &Upstreams, idx: u32, local: WebSocket, url: &str) -> Result<()> {
    if is_serverdb(url) {
        return bridge_echorelay(up, idx, local, url).await;
    }
    let (remote, _) = tokio_tungstenite::connect_async(url).await.context("connecting to nakama")?;
    debug!(instance = idx, "bridged to nakama");
    let (mut rtx, mut rrx) = remote.split();
    let (mut ltx, mut lrx) = local.split();
    loop {
        tokio::select! {
            m = lrx.next() => {
                let Some(Ok(m)) = m else { break };
                let AxMsg::Binary(mut data) = m else { if matches!(m, AxMsg::Close(_)) { break } continue };
                if let Some((mut env, legacy)) = decode_any(&data) {
                    // nakama ignores this one as protobuf: it reads native's
                    // SNSUserServerProfileUpdateRequest into its statistics.
                    if let Some(Em::UserServerProfileUpdateRequest(m)) = &env.message {
                        if let Some(frame) = profile_update_frame(m) {
                            info!(instance = idx, player = ?m.user_id.as_ref().map(|u| &u.id), "sending match stats to nakama");
                            rtx.send(TMsg::Binary(frame.into())).await?;
                        }
                        continue;
                    }
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
                let m = match m { Some(Ok(m)) if !matches!(m, TMsg::Close(_)) => m, other => bail!("{}", ended(other)) };
                match m {
                    TMsg::Binary(data) => {
                        if let Some((env, _)) = decode_any(&data) { observe(up, idx, &env); }
                        ltx.send(AxMsg::Binary(data)).await?;
                    }
                    TMsg::Ping(p) => rtx.send(TMsg::Pong(p)).await?,
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
            let pad = if up.wide_registration.load(Ordering::Relaxed) { 10 } else { 2 };
            w.u64(reg.server_id).ipv4(internal).u16(reg.port as u16).zeros(pad).u64(up.region).u64(VERSION_LOCK);
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
            debug!(instance = idx, server_id, external_ip = %ip, "registered with ServerDB");
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
    let (remote, _) = tokio_tungstenite::connect_async(url).await.context("connecting to ServerDB")?;
    debug!(instance = idx, "connected to EchoRelay ServerDB");
    let mut nakama = false;
    let mut registering = false;
    let (mut rtx, mut rrx) = remote.split();
    let (mut ltx, mut lrx) = local.split();
    loop {
        tokio::select! {
            m = lrx.next() => {
                let Some(Ok(m)) = m else { break };
                let AxMsg::Binary(data) = m else { if matches!(m, AxMsg::Close(_)) { break } continue };
                let Ok(env) = pb::Envelope::decode(data.as_slice()) else { continue };
                if let Some(Em::UserServerProfileUpdateRequest(m)) = &env.message {
                    // A combat match's stats and XP for a player: native's
                    // SNSUserServerProfileUpdateRequest. nakama takes it on this
                    // (authenticated) socket and adds it to its statistics;
                    // EchoRelay on its login service.
                    if let Some(frame) = profile_update_frame(m) {
                        if nakama {
                            info!(instance = idx, player = ?m.user_id.as_ref().map(|u| &u.id), "sending match stats to nakama");
                            rtx.send(TMsg::Binary(frame.into())).await?;
                        } else if let Some(login) = up.login_url.clone() {
                            tokio::spawn(async move { echorelay_profile_update(&login, frame).await });
                        } else {
                            warn!(instance = idx, "match stats but no login service to keep them");
                        }
                    }
                    continue;
                }
                if let Some(Em::GameServerSaveLoadout(save)) = &env.message {
                    if nakama {
                        if let Some(out) = nakama_save_loadout(save) {
                            rtx.send(TMsg::Binary(out.into())).await?;
                        }
                    } else if let Some(login) = up.login_url.clone() {
                        let save = save.clone();
                        tokio::spawn(async move { echorelay_save_loadout(&login, &save).await });
                    } else {
                        warn!(instance = idx, "loadout changed but there is no login service to save it to");
                    }
                    continue;
                }
                registering |= matches!(env.message, Some(Em::GameServerRegistration(_)));
                if let Some(out) = to_echorelay(up, idx, &env) {
                    rtx.send(TMsg::Binary(out)).await?;
                }
            }
            m = rrx.next() => {
                let m = match m {
                    Some(Ok(m)) if !matches!(m, TMsg::Close(_)) => m,
                    other => {
                        retry_layout(up, idx, registering, nakama);
                        bail!("{}", ended(other))
                    }
                };
                match m {
                    TMsg::Binary(data) => {
                        for (symbol, payload) in sns::split(&data)? {
                            // Only nakama sends protobuf frames on this socket
                            // (with its registration reply).
                            if symbol == NEVR_PROTOBUF_V1 {
                                if !nakama {
                                    info!(instance = idx, "backend is nakama");
                                }
                                nakama = true;
                                up.wide_registration.store(true, Ordering::Relaxed);
                                up.registration_confirmed.store(true, Ordering::Relaxed);
                                // nakama explains a refusal here, not in the legacy failure message.
                                if let Ok(env) = pb::Envelope::decode(payload) {
                                    match env.message {
                                        Some(Em::Error(e)) => warn!(instance = idx, code = e.code, reason = %e.message, "nakama reported an error"),
                                        Some(Em::GameServerRegistrationSuccess(ok)) => info!(instance = idx, server_id = ok.server_id, external_ip = %ok.external_ip_address, "registered with nakama"),
                                        _ => {}
                                    }
                                }
                                continue;
                            }
                            if symbol == sym::ER_REGISTRATION_SUCCESS {
                                up.registration_confirmed.store(true, Ordering::Relaxed);
                                // nakama's own reply (protobuf) came first and was logged.
                                if !nakama {
                                    info!(instance = idx, "registered with EchoRelay");
                                }
                            }
                            match from_echorelay(up, idx, symbol, payload) {
                                Ok(Some(env)) => ltx.send(AxMsg::Binary(env.encode_to_vec())).await?,
                                Ok(None) => {}
                                Err(e) => { retry_layout(up, idx, registering, nakama); error!(instance = idx, error = %e, "ServerDB"); return Err(e) }
                            }
                        }
                    }
                    TMsg::Ping(p) => rtx.send(TMsg::Pong(p)).await?,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// A registration ended without success. Unless the backend has shown it is
/// nakama, or some registration already worked, try the other layout next.
fn retry_layout(up: &Upstreams, idx: u32, registering: bool, nakama: bool) {
    if !registering || nakama || up.registration_confirmed.load(Ordering::Relaxed) {
        return;
    }
    let wide = !up.wide_registration.load(Ordering::Relaxed);
    up.wide_registration.store(wide, Ordering::Relaxed);
    info!(instance = idx, layout = if wide { "nakama" } else { "EchoRelay" }, "registration was not accepted; trying the other registration layout");
}

// ─── loadout saves ──────────────────────────────────────────────────────────

/// EchoRelay's login service next to its ServerDB: same host, `/login`.
fn login_from_serverdb(url: &str) -> String {
    let base = url.split('?').next().unwrap_or(url).trim_end_matches('/');
    format!("{}/login", base.strip_suffix("/serverdb").unwrap_or(base))
}

fn sym_hex(v: u64) -> String {
    format!("0x{v:016X}")
}

/// nakama: its `ERGameServerSaveLoadoutRequest` on the ServerDB socket.
fn nakama_save_loadout(m: &pb::GameServerSaveLoadoutMessage) -> Option<Vec<u8>> {
    let entrant: Uuid = m.entrant_id.parse().ok()?;
    let Some(xpid) = ENTRANTS.get(&entrant).map(|x| *x) else {
        warn!(%entrant, "loadout changed for a player with no platform id yet; not saved");
        return None;
    };
    let instances: Vec<serde_json::Value> = m
        .loadout_instances
        .iter()
        .map(|i| {
            let items: serde_json::Map<String, serde_json::Value> =
                i.items.iter().map(|it| (sym_hex(it.slot_type), serde_json::Value::String(sym_hex(it.equipped_item)))).collect();
            serde_json::json!({ "instance_name": sym_hex(i.instance_name), "items": items })
        })
        .collect();
    let json = serde_json::json!({ "slot": m.loadout_slot, "number": m.jersey_number, "loadout_instances": instances });
    let mut w = Writer::new();
    w.guid(entrant).u64(xpid[0]).u64(xpid[1]).u32(m.loadout_slot as u32).json_nt(json.to_string().as_bytes());
    info!(%entrant, items = m.loadout_instances.iter().map(|i| i.items.len()).sum::<usize>(), "saving loadout to nakama");
    Some(er_message(NAKAMA_SAVE_LOADOUT, w))
}

/// `SNSUserServerProfileUpdateRequest`: `[platform u64][account u64][payload json]`.
fn profile_update_frame(m: &pb::SnsUserServerProfileUpdateRequestMessage) -> Option<Vec<u8>> {
    let x = sns::XpId::parse(&m.user_id.as_ref()?.id)?;
    let mut w = Writer::new();
    w.u64(x.platform).u64(x.account).json_nt(m.payload.as_bytes());
    Some(er_message(PROFILE_UPDATE_REQUEST, w))
}

async fn echorelay_profile_update(login: &str, frame: Vec<u8>) {
    let r = async {
        let (mut ws, _) = tokio_tungstenite::connect_async(login).await.context("connecting to the login service")?;
        ws.send(TMsg::Binary(frame.into())).await?;
        let _ = tokio::time::timeout(Duration::from_secs(5), ws.next()).await;
        let _ = ws.close(None).await;
        anyhow::Ok(())
    };
    match r.await {
        Ok(()) => info!("sent match stats to EchoRelay"),
        Err(e) => warn!(error = %e, "could not send match stats to EchoRelay"),
    }
}

/// EchoRelay: a server profile update on its login service, as the native
/// game server sends, with the loadout by name.
async fn echorelay_save_loadout(login: &str, m: &pb::GameServerSaveLoadoutMessage) {
    let Ok(entrant) = m.entrant_id.parse::<Uuid>() else { return };
    let Some(xpid) = ENTRANTS.get(&entrant).map(|x| *x) else {
        warn!(%entrant, "loadout changed for a player with no platform id yet; not saved");
        return;
    };
    let mut instances = serde_json::Map::new();
    let mut unknown = 0;
    for i in &m.loadout_instances {
        let mut slots = serde_json::Map::new();
        for it in &i.items {
            match (COSMETIC_NAMES.get(&it.slot_type), COSMETIC_NAMES.get(&it.equipped_item)) {
                (Some(s), Some(v)) => {
                    slots.insert((*s).to_string(), serde_json::Value::String((*v).to_string()));
                }
                _ => unknown += 1,
            }
        }
        let name = COSMETIC_NAMES.get(&i.instance_name).copied().unwrap_or("unified");
        instances.insert(name.to_string(), serde_json::json!({ "slots": slots }));
    }
    if unknown > 0 {
        warn!(%entrant, unknown, "some loadout items have no known name; they keep their saved value");
    }
    let mut loadout = serde_json::json!({ "instances": instances });
    if m.jersey_number > 0 {
        loadout["number"] = serde_json::json!(m.jersey_number);
    }
    let body = serde_json::json!({ "sessionid": m.lobby_session_id, "update": { "loadout": loadout } });
    let mut w = Writer::new();
    w.u64(xpid[0]).u64(xpid[1]).json_nt(body.to_string().as_bytes());
    let frame = er_message(PROFILE_UPDATE_REQUEST, w);

    let r = async {
        let (mut ws, _) = tokio_tungstenite::connect_async(login).await.context("connecting to the login service")?;
        ws.send(TMsg::Binary(frame.into())).await?;
        // EchoRelay answers with UserServerProfileUpdateSuccess; wait briefly for it.
        let _ = tokio::time::timeout(Duration::from_secs(5), ws.next()).await;
        let _ = ws.close(None).await;
        anyhow::Ok(())
    };
    match r.await {
        Ok(()) => info!(%entrant, "saved loadout to EchoRelay"),
        Err(e) => warn!(%entrant, error = %e, "could not save loadout to EchoRelay"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosmetic_names_hash_like_the_game() {
        // Slot and item symbols from a captured SR15NetCurrentLoadoutResponse.
        assert_eq!(COSMETIC_NAMES.get(&0xc8c3_3e48_2e7a_10a4), Some(&"tint"));
        assert_eq!(COSMETIC_NAMES.get(&0x2fc2_8196_587d_528e), Some(&"unified"));
        assert!(COSMETIC_NAMES.len() > 900);
    }

    #[test]
    fn login_url_sits_beside_serverdb() {
        assert_eq!(login_from_serverdb("ws://127.0.0.1:777/serverdb"), "ws://127.0.0.1:777/login");
        assert_eq!(login_from_serverdb("ws://h:80/serverdb?api_key=x&regions=a"), "ws://h:80/login");
        assert!(is_serverdb("ws://h:80/serverdb?x=1") && !is_serverdb("ws://h:7350/ws/nevr?token=t"));
    }
}
