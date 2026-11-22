//! SNS ("Social Network Service") wire protocol used by the Echo VR client on
//! its login / config / matching / transaction websockets, and by EchoRelay's
//! ServerDB socket.
//!
//! A websocket binary frame carries one or more messages, each framed as:
//! `marker[8] | symbol u64 LE | length u64 LE | payload[length]`.
//!
//! Layouts follow nakama3 `server/evr/*.go` (final PCVR build 34.4.631547)
//! and are pinned by golden vectors in `tests/fixtures/golden_vectors.json`.

use std::io::{Read, Write};
use std::net::Ipv4Addr;

use anyhow::{Context, Result, bail, ensure};
use uuid::Uuid;

pub const MARKER: [u8; 8] = [246, 64, 187, 120, 162, 231, 140, 187];
pub const MAX_PACKET: usize = 256 * 1024;
pub const MAX_MESSAGE: usize = 0x20000;
const MAX_JSON: usize = 4 * 1024 * 1024;

/// Message type symbols.
pub mod sym {
    // Login service
    pub const LOGIN_REQUEST: u64 = 0xbdb41ea9e67b200a;
    pub const LOGIN_SUCCESS: u64 = 0xa5acc1a90d0cce47;
    pub const LOGIN_FAILURE: u64 = 0xa5b9d5a3021ccf51;
    pub const GAME_SETTINGS: u64 = 0xed5be2c3632155f1;
    pub const UNREQUIRE: u64 = 0x43e6963ac76beee4;
    pub const LOGGED_IN_PROFILE_REQUEST: u64 = 0xfb772a4221fc8d70;
    pub const LOGGED_IN_PROFILE_SUCCESS: u64 = 0xfb763a5037fc8d77;
    pub const LOGGED_IN_PROFILE_FAILURE: u64 = 0xfb632e5a38ec8c61;
    pub const DOCUMENT_REQUEST: u64 = 0xfcced6f169822bb8;
    pub const DOCUMENT_SUCCESS: u64 = 0xd07ffd782fb7b509;
    pub const CHANNEL_INFO_REQUEST: u64 = 0x90758e58515724e0;
    pub const CHANNEL_INFO_RESPONSE: u64 = 0x6c8f16cd9f8964c5;
    pub const UPDATE_PROFILE: u64 = 0x6d54a19a3ff24415;
    pub const UPDATE_PROFILE_SUCCESS: u64 = 0xf25491d001cef757;
    pub const OTHER_PROFILE_REQUEST: u64 = 0x1231172031050cb2;
    pub const OTHER_PROFILE_SUCCESS: u64 = 0x1230073227050cb5;
    pub const OTHER_PROFILE_FAILURE: u64 = 0x1225133828150da3;
    pub const REMOTE_LOG_SET: u64 = 0x244b47685187eae1;
    pub const RECONCILE_IAP: u64 = 0x1bd0fc454c85573c;
    pub const RECONCILE_IAP_RESULT: u64 = 0x0dabc24265508a82;
    pub const GENERIC_MESSAGE: u64 = 0x013e99cb47eb3669;
    pub const USER_SERVER_PROFILE_UPDATE_REQUEST: u64 = 0xd2986849b36b9c72;
    pub const USER_SERVER_PROFILE_UPDATE_SUCCESS: u64 = 0xd299785ba56b9c75;
    // Config service
    pub const CONFIG_REQUEST: u64 = 0x82869f0b37eb4378;
    pub const CONFIG_SUCCESS: u64 = 0xb9cdaf586f7bd012;
    pub const CONFIG_FAILURE: u64 = 0x9e687a63dddd3870;
    // Matching service
    pub const FIND_SESSION: u64 = 0x312c2a01819aa3f5;
    pub const CREATE_SESSION: u64 = 0x599a6b1bbda3cc13;
    pub const JOIN_SESSION: u64 = 0x2f03468f77ffb211;
    pub const SESSION_SUCCESS_V4: u64 = 0x6d4de3650ee3110e;
    pub const SESSION_SUCCESS_V5: u64 = 0x6d4de3650ee3110f;
    pub const SESSION_FAILURE_V4: u64 = 0x4ae8365ebc45f96c;
    pub const MATCHMAKER_STATUS_REQUEST: u64 = 0x128b777ae0ebb650;
    pub const MATCHMAKER_STATUS: u64 = 0x8f28cf33dabfbecb;
    pub const PING_REQUEST: u64 = 0xfabf5f8719bfebf3;
    pub const PING_RESPONSE: u64 = 0x6047d0043033ae4f;
    pub const PLAYER_SESSIONS_REQUEST: u64 = 0x9af2fab2a0c81a05;
    pub const ENTRANTS_V3: u64 = 0xa1b9cae1f8588969;
    pub const PENDING_SESSION_CANCEL: u64 = 0x8da9eb83ffee9fd6;
    // EchoRelay ServerDB (unofficial ERGameServer* + official registration replies)
    pub const ER_REGISTRATION_REQUEST: u64 = 0x7777777777777777;
    pub const ER_REGISTRATION_SUCCESS: u64 = 0xb57a31cdd0f6fedf;
    pub const ER_REGISTRATION_FAILURE: u64 = 0xb56f25c7dfe6ffc9;
    pub const ER_START_SESSION: u64 = 0x7777777777770000;
    pub const ER_SESSION_STARTED: u64 = 0x7777777777770100;
    pub const ER_END_SESSION: u64 = 0x7777777777770200;
    pub const ER_PLAYERS_LOCKED: u64 = 0x7777777777770300;
    pub const ER_PLAYERS_UNLOCKED: u64 = 0x7777777777770400;
    pub const ER_ACCEPT_PLAYERS: u64 = 0x7777777777770500;
    pub const ER_PLAYERS_ACCEPTED: u64 = 0x7777777777770600;
    pub const ER_PLAYERS_REJECTED: u64 = 0x7777777777770700;
    pub const ER_REMOVE_PLAYER: u64 = 0x7777777777770800;
}

/// Hash a name into a RAD symbol (same function the game uses).
pub fn symbol(name: &str) -> u64 {
    crate::wire::symbol::symbol(name)
}

// ─── Framing ────────────────────────────────────────────────────────────────

/// Split a websocket frame into `(symbol, payload)` messages.
pub fn split(data: &[u8]) -> Result<Vec<(u64, &[u8])>> {
    ensure!(data.len() <= MAX_PACKET, "packet too large ({} bytes)", data.len());
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        ensure!(data.len() - pos >= 24, "truncated message header");
        ensure!(data[pos..pos + 8] == MARKER, "bad message marker at {pos}");
        let symbol = u64::from_le_bytes(data[pos + 8..pos + 16].try_into()?);
        let len = u64::from_le_bytes(data[pos + 16..pos + 24].try_into()?) as usize;
        ensure!(len <= MAX_MESSAGE, "message too large ({len} bytes)");
        let start = pos + 24;
        ensure!(data.len() - start >= len, "truncated message payload");
        out.push((symbol, &data[start..start + len]));
        pos = start + len;
    }
    Ok(out)
}

/// Append one framed message to `out`.
pub fn frame(out: &mut Vec<u8>, symbol: u64, payload: &[u8]) {
    out.reserve(24 + payload.len());
    out.extend_from_slice(&MARKER);
    out.extend_from_slice(&symbol.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(payload);
}

// ─── Primitives ─────────────────────────────────────────────────────────────

/// Cross-platform user id: `PLATFORM-ACCOUNT` (e.g. `OVR_ORG-123`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct XpId {
    pub platform: u64,
    pub account: u64,
}

const PLATFORMS: [&str; 8] = ["UNK", "STM", "DSC", "XBX", "OVR_ORG", "OVR", "BOT", "DMO"];

impl XpId {
    pub fn is_nil(&self) -> bool {
        self.platform == 0 && self.account == 0
    }
    pub fn token(&self) -> String {
        let p = PLATFORMS.get(self.platform as usize).copied().unwrap_or("UNK");
        format!("{p}-{}", self.account)
    }
    pub fn parse(s: &str) -> Option<Self> {
        let (p, a) = s.rsplit_once('-')?;
        let platform = PLATFORMS.iter().position(|x| *x == p)? as u64;
        Some(Self { platform, account: a.parse().ok()? })
    }
}

impl std::fmt::Display for XpId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.token())
    }
}

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(self.remaining() >= n, "unexpected end of message (need {n}, have {})", self.remaining());
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into()?))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into()?))
    }
    /// Microsoft-layout GUID (first three groups little-endian).
    pub fn guid(&mut self) -> Result<Uuid> {
        Ok(Uuid::from_bytes_le(self.bytes(16)?.try_into()?))
    }
    pub fn xpid(&mut self) -> Result<XpId> {
        Ok(XpId { platform: self.u64()?, account: self.u64()? })
    }
    pub fn ipv4(&mut self) -> Result<Ipv4Addr> {
        let b: [u8; 4] = self.bytes(4)?.try_into()?;
        Ok(Ipv4Addr::from(b))
    }
    pub fn cstr(&mut self) -> Result<String> {
        let rest = &self.buf[self.pos..];
        let end = rest.iter().position(|&b| b == 0).context("unterminated string")?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + 1;
        Ok(s)
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.buf[self.pos..];
        self.pos = self.buf.len();
        s
    }
    /// Uncompressed JSON, optionally null-terminated, running to the end of
    /// the message or to the terminator.
    pub fn json_nt(&mut self) -> Result<serde_json::Value> {
        let rest = &self.buf[self.pos..];
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        let v = if rest[..end].iter().all(u8::is_ascii_whitespace) {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&rest[..end]).context("invalid JSON")?
        };
        self.pos += (end + 1).min(rest.len());
        Ok(v)
    }
}

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { buf: Vec::with_capacity(128) }
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(v);
        self
    }
    pub fn zeros(&mut self, n: usize) -> &mut Self {
        self.buf.resize(self.buf.len() + n, 0);
        self
    }
    pub fn guid(&mut self, v: Uuid) -> &mut Self {
        self.buf.extend_from_slice(&v.to_bytes_le());
        self
    }
    pub fn xpid(&mut self, v: XpId) -> &mut Self {
        self.u64(v.platform).u64(v.account)
    }
    pub fn ipv4(&mut self, v: Ipv4Addr) -> &mut Self {
        self.bytes(&v.octets())
    }
    pub fn cstr(&mut self, s: &str) -> &mut Self {
        self.bytes(s.as_bytes()).u8(0)
    }
    pub fn fixed_str(&mut self, s: &str, len: usize) -> &mut Self {
        let b = s.as_bytes();
        let n = b.len().min(len);
        self.bytes(&b[..n]).zeros(len - n)
    }
    pub fn json_nt(&mut self, json: &[u8]) -> &mut Self {
        self.bytes(json).u8(0)
    }
    /// `u32 uncompressed_len | zstd(json + NUL)`.
    pub fn json_zstd_nt(&mut self, json: &[u8]) -> Result<&mut Self> {
        let mut raw = Vec::with_capacity(json.len() + 1);
        raw.extend_from_slice(json);
        raw.push(0);
        self.u32(raw.len() as u32);
        let c = zstd::bulk::compress(&raw, 1)?;
        Ok(self.bytes(&c))
    }
    /// `u64 uncompressed_len | zlib(json)`.
    pub fn json_zlib(&mut self, json: &[u8]) -> Result<&mut Self> {
        self.u64(json.len() as u64);
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(json)?;
        let c = e.finish()?;
        Ok(self.bytes(&c))
    }
    pub fn finish(self, symbol: u64, out: &mut Vec<u8>) {
        frame(out, symbol, &self.buf);
    }
}

pub fn unzstd_json(r: &mut Reader<'_>) -> Result<serde_json::Value> {
    let len = r.u32()? as usize;
    ensure!(len <= MAX_JSON, "decompressed JSON too large");
    let raw = zstd::bulk::decompress(r.rest(), len)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    Ok(serde_json::from_slice(&raw[..end])?)
}

pub fn unzlib_json(r: &mut Reader<'_>) -> Result<serde_json::Value> {
    let len = r.u64()? as usize;
    ensure!(len <= MAX_JSON, "decompressed JSON too large");
    let mut raw = Vec::with_capacity(len);
    flate2::read::ZlibDecoder::new(r.rest()).take(MAX_JSON as u64).read_to_end(&mut raw)?;
    Ok(serde_json::from_slice(&raw)?)
}

// ─── Client → server messages ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Entrant {
    pub xpid: XpId,
    /// -1 any, 0 blue, 1 orange, 2 spectator, 3 social, 4 moderator.
    pub role: i8,
}

#[derive(Debug, Clone)]
pub struct SessionSettings {
    pub appid: String,
    pub mode: i64,
    pub level: i64,
    pub features: Vec<String>,
}

impl SessionSettings {
    fn from_json(v: &serde_json::Value) -> Self {
        Self {
            appid: v.get("appid").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
            mode: v.get("gametype").and_then(|x| x.as_i64()).unwrap_or(0),
            level: v.get("level").and_then(|x| x.as_i64()).unwrap_or(-1),
            features: v
                .get("features")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect())
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum LobbyRequestKind {
    Find { current_lobby: Uuid },
    Create { region: u64, lobby_type: u8 },
    Join { lobby_id: Uuid, moderate: Option<XpId> },
}

#[derive(Debug, Clone)]
pub struct LobbyRequest {
    pub kind: LobbyRequestKind,
    pub version_lock: u64,
    pub mode: u64,
    pub level: u64,
    pub platform: u64,
    pub login_session: Uuid,
    pub cross_play: bool,
    pub group_id: Uuid,
    pub settings: SessionSettings,
    pub entrants: Vec<Entrant>,
}

const FLAG_TEAM_INDEXES: u32 = 1;
const FLAG_CROSSPLAY: u32 = 2;
const MAX_ENTRANTS: usize = 16;

fn read_entrants(r: &mut Reader<'_>, n: usize, with_roles: bool) -> Result<Vec<Entrant>> {
    let mut e = (0..n)
        .map(|_| Ok(Entrant { xpid: r.xpid()?, role: -1 }))
        .collect::<Result<Vec<_>>>()?;
    if with_roles && r.remaining() >= n {
        for x in &mut e {
            x.role = r.i8()?;
        }
    }
    Ok(e)
}

#[derive(Debug, Clone)]
pub enum ClientMessage {
    LoginRequest { prev_session: Uuid, xpid: XpId, profile: serde_json::Value },
    LoggedInProfileRequest { session: Uuid, xpid: XpId },
    DocumentRequest { lang: String, kind: String },
    ChannelInfoRequest,
    UpdateProfile { session: Uuid, xpid: XpId, profile: serde_json::Value },
    OtherProfileRequest { xpid: XpId },
    ConfigRequest { kind: String, id: String },
    ReconcileIap { session: Uuid, xpid: XpId },
    Lobby(LobbyRequest),
    MatchmakerStatusRequest,
    PingResponse,
    PlayerSessionsRequest { session: Uuid, xpid: XpId, lobby_id: Uuid, players: Vec<XpId> },
    PendingSessionCancel { session: Uuid },
    /// Logs, telemetry, generic social messages: acknowledged and dropped.
    Ignored(u64),
    Unknown(u64),
}

impl ClientMessage {
    pub fn decode(symbol: u64, p: &[u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        Ok(match symbol {
            sym::LOGIN_REQUEST => Self::LoginRequest {
                prev_session: r.guid()?,
                xpid: r.xpid()?,
                profile: r.json_nt()?,
            },
            sym::LOGGED_IN_PROFILE_REQUEST => Self::LoggedInProfileRequest { session: r.guid()?, xpid: r.xpid()? },
            sym::DOCUMENT_REQUEST => Self::DocumentRequest { lang: r.cstr()?, kind: r.cstr()? },
            sym::CHANNEL_INFO_REQUEST => Self::ChannelInfoRequest,
            sym::UPDATE_PROFILE => Self::UpdateProfile { session: r.guid()?, xpid: r.xpid()?, profile: r.json_nt()? },
            sym::OTHER_PROFILE_REQUEST => Self::OtherProfileRequest { xpid: r.xpid()? },
            sym::CONFIG_REQUEST => {
                r.skip(1)?;
                let v = r.json_nt()?;
                let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_owned();
                Self::ConfigRequest { kind: s("type"), id: s("id") }
            }
            sym::RECONCILE_IAP => Self::ReconcileIap { session: r.guid()?, xpid: r.xpid()? },
            sym::FIND_SESSION => {
                let version_lock = r.u64()?;
                let mode = r.u64()?;
                let level = r.u64()?;
                let platform = r.u64()?;
                let login_session = r.guid()?;
                let count = r.u8()? as usize;
                ensure!(count <= MAX_ENTRANTS, "too many entrants");
                let flags = r.u32()?;
                r.skip(3)?;
                let current_lobby = r.guid()?;
                let group_id = r.guid()?;
                let settings = SessionSettings::from_json(&r.json_nt()?);
                let entrants = read_entrants(&mut r, count, flags & FLAG_TEAM_INDEXES != 0)?;
                Self::Lobby(LobbyRequest {
                    kind: LobbyRequestKind::Find { current_lobby },
                    version_lock,
                    mode,
                    level,
                    platform,
                    login_session,
                    cross_play: flags & FLAG_CROSSPLAY != 0,
                    group_id,
                    settings,
                    entrants,
                })
            }
            sym::CREATE_SESSION => {
                let region = r.u64()?;
                let version_lock = r.u64()?;
                let mode = r.u64()?;
                let level = r.u64()?;
                let platform = r.u64()?;
                let login_session = r.guid()?;
                let count = r.i8()?;
                ensure!((0..=MAX_ENTRANTS as i8).contains(&count), "bad entrant count");
                r.skip(7)?;
                let lobby_type = r.u8()?;
                ensure!(lobby_type <= 2, "bad lobby type {lobby_type}");
                r.skip(3)?;
                let flags = r.u32()?;
                let group_id = r.guid()?;
                let settings = SessionSettings::from_json(&r.json_nt()?);
                let entrants = read_entrants(&mut r, count as usize, flags & FLAG_TEAM_INDEXES != 0)?;
                Self::Lobby(LobbyRequest {
                    kind: LobbyRequestKind::Create { region, lobby_type },
                    version_lock,
                    mode,
                    level,
                    platform,
                    login_session,
                    cross_play: flags & FLAG_CROSSPLAY != 0,
                    group_id,
                    settings,
                    entrants,
                })
            }
            sym::JOIN_SESSION => {
                let mut lobby_id = r.guid()?;
                let version_lock = r.u64()?;
                let platform = r.u64()?;
                let login_session = r.guid()?;
                let flags = r.u32()?;
                r.skip(4)?;
                let count = (flags & 0xff) as usize;
                ensure!(count <= MAX_ENTRANTS, "too many entrants");
                let join_flags = r.u64()?;
                let mut moderate = None;
                if join_flags & (1 << 3) != 0 {
                    // -moderateuser: the "lobby id" carries the target user.
                    let b = lobby_id.to_bytes_le();
                    moderate = Some(XpId {
                        platform: u64::from(b[3]),
                        account: u64::from_le_bytes(b[8..16].try_into()?),
                    });
                    lobby_id = Uuid::nil();
                }
                let settings = SessionSettings::from_json(&r.json_nt()?);
                let entrants = read_entrants(&mut r, count, flags & FLAG_TEAM_INDEXES != 0)?;
                Self::Lobby(LobbyRequest {
                    kind: LobbyRequestKind::Join { lobby_id, moderate },
                    version_lock,
                    mode: settings.mode as u64,
                    level: settings.level as u64,
                    platform,
                    login_session,
                    cross_play: flags & FLAG_CROSSPLAY != 0,
                    group_id: Uuid::nil(),
                    settings,
                    entrants,
                })
            }
            sym::MATCHMAKER_STATUS_REQUEST => Self::MatchmakerStatusRequest,
            sym::PING_RESPONSE => Self::PingResponse,
            sym::PLAYER_SESSIONS_REQUEST => {
                let session = r.guid()?;
                let xpid = r.xpid()?;
                let lobby_id = r.guid()?;
                let _platform = r.u64()?;
                let n = r.u64()? as usize;
                ensure!(n <= MAX_ENTRANTS, "too many players");
                let players = (0..n).map(|_| r.xpid()).collect::<Result<_>>()?;
                Self::PlayerSessionsRequest { session, xpid, lobby_id, players }
            }
            sym::PENDING_SESSION_CANCEL => Self::PendingSessionCancel { session: r.guid()? },
            sym::REMOTE_LOG_SET
            | sym::GENERIC_MESSAGE
            | sym::USER_SERVER_PROFILE_UPDATE_REQUEST
            | sym::UNREQUIRE => Self::Ignored(symbol),
            other => Self::Unknown(other),
        })
    }
}

// ─── Server → client messages ───────────────────────────────────────────────

pub fn unrequire(out: &mut Vec<u8>) {
    let mut w = Writer::new();
    w.u8(0);
    w.finish(sym::UNREQUIRE, out);
}

pub fn login_success(out: &mut Vec<u8>, session: Uuid, xpid: XpId) {
    let mut w = Writer::new();
    w.guid(session).xpid(xpid);
    w.finish(sym::LOGIN_SUCCESS, out);
}

pub fn login_failure(out: &mut Vec<u8>, xpid: XpId, status: u64, msg: &str) {
    let mut w = Writer::new();
    w.xpid(xpid).u64(status).cstr(msg);
    w.finish(sym::LOGIN_FAILURE, out);
}

pub fn game_settings(out: &mut Vec<u8>, json: &[u8]) -> Result<()> {
    let mut w = Writer::new();
    w.json_zlib(json)?;
    w.finish(sym::GAME_SETTINGS, out);
    Ok(())
}

pub fn config_success(out: &mut Vec<u8>, kind: &str, id: &str, json: &[u8]) -> Result<()> {
    let mut w = Writer::new();
    w.u64(symbol(kind)).u64(symbol(id)).json_zstd_nt(json)?;
    w.finish(sym::CONFIG_SUCCESS, out);
    Ok(())
}

pub fn config_failure(out: &mut Vec<u8>, kind: &str, id: &str) {
    let info = serde_json::json!({"type": kind, "identifier": id, "errorCode": 404, "error": "resource not found"});
    let mut w = Writer::new();
    w.u64(symbol(kind)).u64(symbol(id)).json_nt(info.to_string().as_bytes());
    w.finish(sym::CONFIG_FAILURE, out);
}

pub fn document_success(out: &mut Vec<u8>, kind: &str, json: &[u8]) -> Result<()> {
    let mut w = Writer::new();
    w.u64(symbol(kind)).json_zstd_nt(json)?;
    w.finish(sym::DOCUMENT_SUCCESS, out);
    Ok(())
}

pub fn logged_in_profile_success(out: &mut Vec<u8>, xpid: XpId, json: &[u8]) -> Result<()> {
    let mut w = Writer::new();
    w.xpid(xpid).json_zstd_nt(json)?;
    w.finish(sym::LOGGED_IN_PROFILE_SUCCESS, out);
    Ok(())
}

pub fn other_profile_success(out: &mut Vec<u8>, xpid: XpId, json: &[u8]) -> Result<()> {
    let mut w = Writer::new();
    w.xpid(xpid).json_zstd_nt(json)?;
    w.finish(sym::OTHER_PROFILE_SUCCESS, out);
    Ok(())
}

pub fn other_profile_failure(out: &mut Vec<u8>, xpid: XpId, status: u64, msg: &str) {
    let mut w = Writer::new();
    w.xpid(xpid).u64(status).cstr(msg);
    w.finish(sym::OTHER_PROFILE_FAILURE, out);
}

pub fn channel_info(out: &mut Vec<u8>, json: &[u8]) -> Result<()> {
    let mut w = Writer::new();
    w.json_zlib(json)?;
    w.finish(sym::CHANNEL_INFO_RESPONSE, out);
    Ok(())
}

pub fn update_profile_success(out: &mut Vec<u8>, xpid: XpId) {
    let mut w = Writer::new();
    w.xpid(xpid);
    w.finish(sym::UPDATE_PROFILE_SUCCESS, out);
}

pub fn reconcile_iap_result(out: &mut Vec<u8>, xpid: XpId) {
    let j = br#"{"balance":{"currency":{"echopoints":{"val":0}}},"transactionid":1}"#;
    let mut w = Writer::new();
    w.xpid(xpid).json_nt(j);
    w.finish(sym::RECONCILE_IAP_RESULT, out);
}

pub fn matchmaker_status(out: &mut Vec<u8>) {
    let mut w = Writer::new();
    w.u32(0);
    w.finish(sym::MATCHMAKER_STATUS, out);
}

pub fn entrants_v3(out: &mut Vec<u8>, xpid: XpId, entrant_id: Uuid, team: i16) {
    let mut w = Writer::new();
    w.u8(0xff).xpid(xpid).guid(entrant_id).u16(team as u16).u16(0).u32(0);
    w.finish(sym::ENTRANTS_V3, out);
}

/// Lobby session failure codes (`LobbySessionFailureErrorCode`).
pub mod fail {
    pub const BAD_REQUEST: u32 = 2;
    pub const SERVER_DOES_NOT_EXIST: u32 = 4;
    pub const SERVER_INCOMPATIBLE: u32 = 5;
    pub const SERVER_FIND_FAILED: u32 = 6;
    pub const SERVER_IS_LOCKED: u32 = 7;
    pub const SERVER_IS_FULL: u32 = 8;
    pub const INTERNAL_ERROR: u32 = 9;
}

pub fn session_failure_v4(out: &mut Vec<u8>, mode: u64, group: Uuid, code: u32, msg: &str) {
    let mut w = Writer::new();
    w.u64(mode).guid(group).u32(code).u32(255).fixed_str(msg, 64).u64(0);
    w.finish(sym::SESSION_FAILURE_V4, out);
}

/// Per-direction packet encoder configuration bitfield.
#[derive(Clone, Copy, Debug)]
pub struct EncoderConfig {
    pub encryption: bool,
    pub mac: bool,
    pub mac_digest: u16,
    pub mac_key: u16,
    pub enc_key: u16,
    pub random_key: u16,
}

impl EncoderConfig {
    pub const SERVER: Self = Self { encryption: true, mac: true, mac_digest: 0x20, mac_key: 0x20, enc_key: 0x20, random_key: 0x20 };
    pub const CLIENT: Self = Self { encryption: true, mac: true, mac_digest: 0x40, mac_key: 0x20, enc_key: 0x20, random_key: 0x20 };
    pub fn flags(&self) -> u64 {
        u64::from(self.encryption)
            | u64::from(self.mac) << 1
            | u64::from(self.mac_digest & 0xfff) << 2
            | u64::from(self.mac_key & 0xfff) << 26
            | u64::from(self.enc_key & 0xfff) << 38
            | u64::from(self.random_key & 0xfff) << 50
    }
}

/// Everything a client and its game server need to talk: endpoint, team and
/// the per-entrant packet keys.
#[derive(Clone, Debug)]
pub struct SessionSuccess {
    pub mode: u64,
    pub lobby_id: Uuid,
    pub group_id: Uuid,
    pub internal_ip: Ipv4Addr,
    pub external_ip: Ipv4Addr,
    pub port: u16,
    pub team: i16,
    pub session_flags: u8,
    pub server_seq: u64,
    pub server_mac: [u8; 32],
    pub server_enc: [u8; 32],
    pub server_rnd: [u8; 32],
    pub client_seq: u64,
    pub client_mac: [u8; 32],
    pub client_enc: [u8; 32],
    pub client_rnd: [u8; 32],
}

/// Packet sequence ids are 63-bit on the wire (the client masks the top bit).
pub const SEQ_MASK: u64 = 0x7fff_ffff_ffff_ffff;

impl SessionSuccess {
    pub fn generate(mode: u64, lobby_id: Uuid, group_id: Uuid, internal_ip: Ipv4Addr, external_ip: Ipv4Addr, port: u16, team: i16) -> Self {
        use rand::RngCore;
        let mut rng = rand::rngs::OsRng;
        let mut k = || {
            let mut b = [0u8; 32];
            rng.fill_bytes(&mut b);
            b
        };
        let (server_mac, server_enc, server_rnd, client_mac, client_enc, client_rnd) = (k(), k(), k(), k(), k(), k());
        Self {
            mode,
            lobby_id,
            group_id,
            internal_ip,
            external_ip,
            port,
            team,
            session_flags: 0,
            server_seq: rand::random::<u64>() & SEQ_MASK,
            server_mac,
            server_enc,
            server_rnd,
            client_seq: rand::random::<u64>() & SEQ_MASK,
            client_mac,
            client_enc,
            client_rnd,
        }
    }

    pub fn endpoint_string(&self) -> String {
        format!("{}:{}:{}", self.internal_ip, self.external_ip, self.port)
    }

    pub fn encode_v5(&self, out: &mut Vec<u8>) {
        let mut w = Writer::new();
        w.u64(self.mode)
            .guid(self.lobby_id)
            .guid(self.group_id)
            .ipv4(self.internal_ip)
            .ipv4(self.external_ip)
            .bytes(&self.port.to_be_bytes())
            .u16(self.team as u16)
            .u8(self.session_flags)
            .zeros(3)
            .u64(EncoderConfig::SERVER.flags())
            .u64(EncoderConfig::CLIENT.flags())
            .u64(self.server_seq)
            .bytes(&self.server_mac)
            .bytes(&self.server_enc)
            .bytes(&self.server_rnd)
            .u64(self.client_seq)
            .bytes(&self.client_mac)
            .bytes(&self.client_enc)
            .bytes(&self.client_rnd);
        w.finish(sym::SESSION_SUCCESS_V5, out);
    }

    /// The fleet (protobuf) form handed to a game server.
    pub fn to_proto(&self) -> crate::wire::pb::SnsLobbySessionSuccessV5Message {
        crate::wire::pb::SnsLobbySessionSuccessV5Message {
            game_mode: self.mode,
            lobby_id: self.lobby_id.to_string(),
            group_id: self.group_id.to_string(),
            endpoint: self.endpoint_string(),
            team_index: i32::from(self.team),
            user_slot: 0,
            flags32: 0,
            session_flags: u32::from(self.session_flags),
            server_encoder_flags: EncoderConfig::SERVER.flags(),
            client_encoder_flags: EncoderConfig::CLIENT.flags(),
            server_sequence_id: self.server_seq & SEQ_MASK,
            server_mac_key: self.server_mac.to_vec(),
            server_enc_key: self.server_enc.to_vec(),
            server_random_key: self.server_rnd.to_vec(),
            client_sequence_id: self.client_seq & SEQ_MASK,
            client_mac_key: self.client_mac.to_vec(),
            client_enc_key: self.client_enc.to_vec(),
            client_random_key: self.client_rnd.to_vec(),
        }
    }
}

/// Decode the 8-byte-aligned SessionSuccess v5 payload (EchoRelay → server).
pub fn decode_session_success_v5(p: &[u8]) -> Result<SessionSuccess> {
    let mut r = Reader::new(p);
    let mode = r.u64()?;
    let lobby_id = r.guid()?;
    let group_id = r.guid()?;
    let internal_ip = r.ipv4()?;
    let external_ip = r.ipv4()?;
    let port = u16::from_be_bytes(r.bytes(2)?.try_into()?);
    let team = r.u16()? as i16;
    let session_flags = r.u8()?;
    r.skip(3)?;
    let sflags = r.u64()?;
    let cflags = r.u64()?;
    let key = |r: &mut Reader<'_>, flags: u64, shift: u32| -> Result<[u8; 32]> {
        let n = ((flags >> shift) & 0xfff) as usize;
        if n != 32 {
            bail!("unsupported key size {n}");
        }
        Ok(r.bytes(32)?.try_into()?)
    };
    let server_seq = r.u64()?;
    let server_mac = key(&mut r, sflags, 26)?;
    let server_enc = key(&mut r, sflags, 38)?;
    let server_rnd = key(&mut r, sflags, 50)?;
    let client_seq = r.u64()?;
    let client_mac = key(&mut r, cflags, 26)?;
    let client_enc = key(&mut r, cflags, 38)?;
    let client_rnd = key(&mut r, cflags, 50)?;
    Ok(SessionSuccess {
        mode,
        lobby_id,
        group_id,
        internal_ip,
        external_ip,
        port,
        team,
        session_flags,
        server_seq,
        server_mac,
        server_enc,
        server_rnd,
        client_seq,
        client_mac,
        client_enc,
        client_rnd,
    })
}

#[cfg(test)]
#[path = "sns_tests.rs"]
mod tests;
