//! One lobby/match: entrants, the join sequence, message relay, replicated
//! game state, real per-peer ping, disc ownership and the arena rules.
//!
//! Everything here is driven by two calls: [`Session::on_packet`] for each
//! datagram and [`Session::server_frame`] at 20 Hz. Relayed player traffic is
//! forwarded as soon as it is decoded; nothing waits for the next frame.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use nevr_proto::rtapi::v1 as pb;
use nevr_proto::rtapi::v1::envelope::Message as Em;
use rad_protocol::codec::decoder::DecodedMessage;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::msg::{self, *};
use super::transport::{SessionCrypto, Transport, mac_matches, reliable_batch};

pub const MAX_SLOTS: usize = 16;
/// Server frames per second (frame message, replication, rules).
pub const FRAME_HZ: u64 = 20;
const PEER_TIMEOUT: Duration = Duration::from_secs(12);
const ENTRANT_HEADER: usize = 0x68;

const TEAM_BLUE: u8 = 0;
const TEAM_ORANGE: u8 = 1;
const TEAM_SPECTATOR: u8 = 2;
const TEAM_SOCIAL: u8 = 3;
const TEAM_MODERATOR: u8 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Handshake,
    AwaitingAccept,
    Joined,
}

struct Peer {
    t: Transport,
    stage: Stage,
    last_seen: Instant,
    slot: Option<usize>,
    genidx: u16,
    team: u8,
    team_slot: u16,
    entrant_id: Uuid,
    session_raw: [u8; 16],
    requested_team: Option<u8>,
    entrant_header: Option<Vec<u8>>,
    entrant_latest: Option<Vec<u8>>,
    entrant_states: u32,
    loadout: Option<Vec<u8>>,
    /// Per-slot generation the client stamps on its own transforms.
    generation: Option<u16>,
    deferred: Vec<(u32, u64, Vec<u8>)>,
    join_tail_in: Option<u32>,
    owner_resend_in: Option<u32>,
    join_complete: bool,
    rep_ready: bool,
    rep_sent: bool,
    rep_acked: bool,
    rep_seq: u64,
    ping_token: u64,
    ping_dirty: bool,
    echo_logged: Instant,
    disc_seen: u32,
    disc_max_depth: i32,
    /// (type, reliable, len, object id) -> count, logged and cleared every 2 s.
    seen_types: HashMap<(u64, bool, usize, u32), u32>,
    stats: Stats,
    /// Stats as of the last `SR15NetReliableStatUpdate` (it carries deltas).
    stats_sent: Stats,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Stats {
    points: u32,
    goals: u32,
    shots: u32,
    two_pt: u32,
    three_pt: u32,
    /// Frames (FRAME_HZ) holding the disc during play.
    possession: u32,
}

impl Peer {
    fn new(t: Transport) -> Self {
        Self {
            t,
            stage: Stage::Handshake,
            last_seen: Instant::now(),
            slot: None,
            genidx: 0,
            team: TEAM_BLUE,
            team_slot: 0,
            entrant_id: Uuid::nil(),
            session_raw: [0; 16],
            requested_team: None,
            entrant_header: None,
            entrant_latest: None,
            entrant_states: 0,
            loadout: None,
            generation: None,
            deferred: Vec::new(),
            join_tail_in: None,
            owner_resend_in: None,
            join_complete: false,
            rep_ready: false,
            rep_sent: false,
            rep_acked: false,
            rep_seq: 0,
            ping_token: 0,
            stats: Stats::default(),
            stats_sent: Stats::default(),
            ping_dirty: false,
            echo_logged: Instant::now(),
            disc_seen: 0,
            disc_max_depth: 0,
            seen_types: HashMap::new(),
        }
    }

    /// Network round trip in µs, measured by the transport from mailbox acks.
    fn rtt_us(&self) -> Option<u32> {
        self.t.rtt_us()
    }
}

struct PendingKeys {
    crypto: SessionCrypto,
    created: Instant,
}

#[derive(Clone, Copy, Default)]
struct TouchEntry {
    owner: u32,
    stamp: u64,
    generation: u8,
}

/// The server clock every session publishes, built like native's: the frame
/// word counts 120 Hz ticks and `server_us` is exactly frame × 8333 µs, both
/// measured from machine uptime. That keeps them small (native's are ~4e6
/// frames) and still never going backwards when this process restarts, which
/// matters because the client drops any frame not above the last one it saw
/// and keeps that memory from one server to the next. The client's own match
/// timers (the tube countdown, the doors) run on this clock.
pub const TICK_US: u64 = 8333;

fn clock_us() -> u64 {
    use std::sync::OnceLock;
    static BASE: OnceLock<(Instant, u64)> = OnceLock::new();
    let (at, base) = *BASE.get_or_init(|| (Instant::now(), system_uptime_us()));
    base + at.elapsed().as_micros() as u64
}

#[cfg(windows)]
fn system_uptime_us() -> u64 {
    // SAFETY: no arguments, no preconditions.
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() * 1000 }
}

#[cfg(not(windows))]
fn system_uptime_us() -> u64 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()))
        .map(|secs| (secs * 1e6) as u64)
        .unwrap_or(0)
}

/// Current wire frame word (120 Hz ticks) and its server time.
fn wire_clock() -> (u64, u64) {
    let frame = clock_us() / TICK_US;
    (frame, frame * TICK_US)
}

/// Goal-cycle graph states the fresh round start reuses (names from nevr's
/// `ARENA_GOAL_STATES` recovery).
const ST_PRE_LAUNCH_ENTER: u64 = ST_GOAL[8];

// ── Arena rules ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// pre_match, waiting for `min_per_team` on each team (0: the first join).
    Waiting,
    /// pre_match countdown running; launch follows.
    Countdown,
    /// round_start: the launch.
    Launch,
    Playing,
    Goal,
    Over,
}

#[derive(Clone, Copy, Debug)]
enum Act {
    State(u64),
    Event(u64, u64, u16),
    Countdown(f32),
    Phase(Phase),
    Park(Option<bool>),
    ArmGoals,
    /// Per-frame disc reset + light/FX during the launch count (None: off).
    LaunchFx(Option<(u64, u64)>),
    /// Set the low byte of a baseline flag var.
    Flag(u64, u8),
}

const PRE_MATCH_SECS: f32 = 45.0;
const LAUNCH_SECS: f32 = 15.0;
/// The count native runs between round_start and pre_launch.
const HOLD_SECS: f32 = 4.5;
const ROUND_SECS: f32 = 300.0;

struct Arena {
    phase: Phase,
    state: u64,
    blue: u32,
    red: u32,
    clock: f32,
    countdown: f32,
    countdown_running: bool,
    disc_owner: u32,
    parked_for: Option<bool>,
    /// Where the disc stopped in the goal; the server holds it there until
    /// the relaunch park.
    frozen: Option<(i32, i32, i32)>,
    /// Match over: send the top awards on the next tick.
    awards_due: bool,
    /// Low nibble of the disc state word: native adds 3 per goal, and the
    /// client snaps to the server's disc whenever it changes.
    disc_resets: u32,
    goals_armed: bool,
    schedule: Vec<(u64, Act)>,
    min_per_team: usize,
    disc_us: u64,
    launch_fx: Option<(u64, u64)>,
    /// Flag vars changed from the baseline: (hash, low byte).
    flags: Vec<(u64, u8)>,
}

impl Arena {
    fn new(min_per_team: usize) -> Self {
        Self {
            phase: Phase::Waiting,
            state: ST_PRE_MATCH,
            blue: 0,
            red: 0,
            clock: ROUND_SECS,
            countdown: PRE_MATCH_SECS,
            countdown_running: false,
            disc_owner: u32::MAX,
            parked_for: None,
            frozen: None,
            awards_due: false,
            disc_resets: 0,
            goals_armed: false,
            schedule: Vec::new(),
            min_per_team,
            disc_us: 0,
            launch_fx: None,
            flags: Vec::new(),
        }
    }

    fn at(&mut self, frame: u64, act: Act) {
        self.schedule.push((frame, act));
    }

    fn clock_runs(&self) -> bool {
        matches!(self.state, ST_PLAYING | ST_MAIN_LOOP_ENTER)
    }

    /// Both teams populated: run pre_match countdown → launch → play.
    fn start_round(&mut self, f: u64) {
        let ev = |s, m, fl| Act::Event(s, m, fl);
        self.phase = Phase::Countdown;
        self.at(f, ev(EV_ENABLE_SCORING, MOD_MATCH, FILTER_ALL));
        for s in [EV_RESTART_CELEBRATION, EV_RESTART_GOALS, EV_MATCH_NOT_READY, EV_RESTART_DISC] {
            self.at(f + 1, ev(s, MOD_MATCH, FILTER_ALL));
        }
        self.at(f + 4, ev(EV_LIGHT_WHITE, MOD_DISC_FX, FILTER_ALL));
        self.at(f + 4, ev(EV_FX_WHITE, MOD_DISC_FX, FILTER_ALL));
        self.at(f, Act::Countdown(PRE_MATCH_SECS));
        // When the pre-match count hits 0 (recorded native game): the
        // pre-match-done state with the round-start events, the tunnel doors
        // open, the graph walks round_start into a 4.5 s count, then
        // pre_launch runs the 15 s count in the tubes.
        let r = f + (PRE_MATCH_SECS as u64) * FRAME_HZ;
        self.at(r, Act::Phase(Phase::Launch));
        self.at(r, Act::State(ST_PRE_MATCH_DONE));
        self.at(r, ev(EV_DISABLE_LOCAL_DISC, MOD_MATCH, FILTER_ALL));
        self.at(r, ev(EV_ROUND_START, MOD_MATCH, FILTER_ALL));
        self.at(r + 1, Act::State(ST_GOAL[4]));
        self.at(r + 1, Act::Flag(VAR_TUNNEL_DOORS, 1));
        self.at(r + 2, Act::State(ST_GOAL[5]));
        self.at(r + 2, Act::Flag(VAR_ROUND_BEGUN, 1));
        self.at(r + 3, Act::State(ST_GOAL[6]));
        self.at(r + 4, Act::State(ST_GOAL[7]));
        self.at(r + 4, Act::Countdown(HOLD_SECS));
        let e = r + 4 + (HOLD_SECS * FRAME_HZ as f32) as u64;
        self.at(e, Act::State(ST_PRE_LAUNCH_ENTER));
        self.at(e, ev(EV_RESTART_DISC, MOD_MATCH, FILTER_ALL));
        self.schedule_launch(e + 1, (EV_LIGHT_WHITE, EV_FX_WHITE));
    }

    /// pre_launch: the 15 s count in the tubes (round_start events already
    /// sent by the caller), then play. Native restates the disc reset and its
    /// white FX every frame of this window, which holds the disc in place.
    fn schedule_launch(&mut self, l: u64, fx: (u64, u64)) {
        let ev = |s, m, fl| Act::Event(s, m, fl);
        self.at(l, Act::Phase(Phase::Launch));
        self.at(l, Act::State(ST_ROUND_START));
        self.at(l, Act::Countdown(LAUNCH_SECS));
        self.at(l, Act::LaunchFx(Some(fx)));
        // Count over: scoring on, launch gates open, main loop, play.
        let p = l + (LAUNCH_SECS as u64) * FRAME_HZ;
        self.at(p, Act::LaunchFx(None));
        self.at(p, Act::Park(None));
        self.at(p, ev(EV_ENABLE_SCORING, MOD_MATCH, FILTER_ALL));
        self.at(p + 1, Act::Flag(VAR_LAUNCH_GATES, 1));
        self.at(p + 1, ev(EV_NEW_ROUND_STARTED, MOD_MATCH, FILTER_ALL));
        self.at(p + 2, Act::State(ST_MAIN_LOOP_ENTER));
        self.at(p + 2, Act::Flag(VAR_NOT_PLAYING, 0));
        self.at(p + 2, Act::Phase(Phase::Playing));
        self.at(p + 3, Act::State(ST_PLAYING));
        self.at(p + 3, Act::ArmGoals);
        // Native: the announcer follows ~2.5 s into play, sent twice.
        self.at(p + 50, ev(EV_ANNOUNCER_ROUND_START, MOD_ANNOUNCER, FILTER_ALL_ALT));
        self.at(p + 50, ev(EV_ANNOUNCER_ROUND_START, MOD_ANNOUNCER, FILTER_ALL_ALT));
    }

    /// A goal, as the recorded native game sequenced it: score events and
    /// point_score with the gates shut, a 15 s celebration, then the same
    /// round_start walk and 4.5 s count as a fresh round, then relaunch.
    fn goal(&mut self, g: u64, blue_scored: bool, points: u32) {
        let ev = |s| Act::Event(s, MOD_MATCH, FILTER_ALL);
        let ev13 = |s, m| Act::Event(s, m, FILTER_ALL_ALT);
        self.goals_armed = false;
        self.phase = Phase::Goal;
        if blue_scored {
            self.blue += points;
        } else {
            self.red += points;
        }
        for _ in 0..2 {
            self.at(g, ev13(EV_GOAL_A, MOD_MATCH));
            self.at(g, ev13(EV_GOAL_B, MOD_MATCH));
        }
        self.at(g, ev(EV_GAME_SCORE));
        self.at(g, Act::State(ST_GOAL[0]));
        self.at(g, Act::Flag(VAR_LAUNCH_GATES, 0));
        self.at(g + 1, Act::State(ST_GOAL[1]));
        self.at(g + 1, Act::Flag(VAR_HOLD, 1));
        self.at(g + 1, Act::Countdown(LAUNCH_SECS));
        let call = if points >= 3 { EV_ANNOUNCER_GOAL_3 } else { EV_ANNOUNCER_GOAL_2 };
        self.at(g + 1, ev13(call, MOD_ANNOUNCER));
        let c = g + 1 + (LAUNCH_SECS as u64) * FRAME_HZ;
        self.at(c, Act::State(ST_GOAL[2]));
        self.at(c, ev(EV_ROUND_RESTART));
        self.at(c + 1, Act::State(ST_GOAL[3]));
        self.at(c + 1, ev(EV_DISABLE_LOCAL_DISC));
        self.at(c + 1, ev(EV_ROUND_START));
        self.at(c + 2, Act::State(ST_GOAL[4]));
        self.at(c + 2, Act::Event(EV_REALLY_RESTART, u64::MAX, FILTER_ALL));
        self.at(c + 3, Act::State(ST_GOAL[5]));
        self.at(c + 4, Act::State(ST_GOAL[6]));
        self.at(c + 4, Act::Flag(VAR_DISC_SPAWN, if blue_scored { 1 } else { 0 }));
        self.at(c + 5, Act::State(ST_GOAL[7]));
        self.at(c + 5, Act::Countdown(HOLD_SECS));
        self.at(c + 5, Act::Park(Some(blue_scored)));
        let e = c + 5 + (HOLD_SECS * FRAME_HZ as f32) as u64;
        self.at(e - 10, Act::Flag(VAR_HOLD, 0));
        self.at(e, Act::State(ST_PRE_LAUNCH_ENTER));
        self.at(e, ev(EV_RESTART_DISC));
        // The disc goes to the team that conceded; native lit it orange after
        // blue scored. The blue pair is unrecorded, so white stands in.
        let fx = if blue_scored { (EV_LIGHT_ORANGE, EV_FX_ORANGE) } else { (EV_LIGHT_WHITE, EV_FX_WHITE) };
        self.schedule_launch(e + 1, fx);
    }

    fn end_match(&mut self, f: u64) {
        self.schedule.clear();
        self.goals_armed = false;
        self.phase = Phase::Over;
        self.awards_due = true;
        let ev = |s| Act::Event(s, MOD_MATCH, FILTER_ALL);
        let hold = 15 * FRAME_HZ;
        let steps = [0, 1, hold, hold + 3 * FRAME_HZ, hold + 13 * FRAME_HZ, hold + 13 * FRAME_HZ + 1, hold + 28 * FRAME_HZ];
        let winner = if self.blue >= self.red { VAR_BLUE_ROUND_SCORE } else { VAR_RED_ROUND_SCORE };
        self.at(f, Act::Flag(VAR_END_DOORS, 1));
        self.at(f, Act::Flag(VAR_DISC_SPAWN, 2));
        self.at(f, Act::Flag(VAR_NOT_PLAYING, 1));
        self.at(f, Act::Flag(winner, 1));
        self.at(f, Act::Flag(VAR_LAUNCH_GATES, 0));
        self.at(f + 1, Act::Flag(VAR_HOLD, 1));
        self.at(f + 1, Act::Countdown(15.0));
        self.at(f + hold, Act::Flag(VAR_CELEBRATION, 1));
        self.at(f + hold + 3 * FRAME_HZ, Act::Countdown(10.0));
        self.at(f + hold + 13 * FRAME_HZ + 1, Act::Countdown(15.0));
        for (i, (at, st)) in steps.iter().zip(ST_MATCH_END).enumerate() {
            self.at(f + at, Act::State(st));
            let e = match i {
                0 => Some(EV_MATCH_OVER),
                2 => Some(EV_TELEPORT_CELEBRATION),
                4 => Some(EV_PERSONAL_STATS),
                6 => Some(EV_RETURN_TO_LOBBY),
                _ => None,
            };
            if let Some(e) = e {
                self.at(f + at, ev(e));
            }
        }
    }

    fn set_flag(&mut self, h: u64, v: u8) {
        match self.flags.iter_mut().find(|(k, _)| *k == h) {
            Some(e) => e.1 = v,
            None => self.flags.push((h, v)),
        }
    }
}

// ── Session ────────────────────────────────────────────────────────────────

pub struct SessionInfo {
    pub lobby: Uuid,
    pub mode: u64,
    pub level: u64,
}

pub struct Session {
    pub info: SessionInfo,
    social: bool,
    arena_level: bool,
    peers: HashMap<SocketAddr, Peer>,
    pending: Vec<PendingKeys>,
    slot_gen: [u16; MAX_SLOTS],
    slot_addr: [Option<SocketAddr>; MAX_SLOTS],
    frame: u64,
    /// Last wire frame word published (120 Hz ticks of the server clock).
    wire_frame: u64,
    started: Instant,
    arena: Arena,
    touch: HashMap<u32, TouchEntry>,
    touch_seq: [u16; MAX_SLOTS],
    parked_loadouts: Vec<(SocketAddr, usize)>,
    roster_dirty: bool,
    pub fleet_out: Vec<pb::Envelope>,
    pub udp_out: Vec<(SocketAddr, Vec<u8>)>,
    pub last_activity: Instant,
    pub ever_joined: bool,
}

fn env(m: Em) -> pb::Envelope {
    pb::Envelope { cid: Uuid::new_v4().to_string(), message: Some(m) }
}

fn key32(v: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 32];
    let n = v.len().min(32);
    k[..n].copy_from_slice(&v[..n]);
    k
}

fn swizzle(raw: [u8; 16]) -> Uuid {
    let mut b = raw;
    b[0..4].reverse();
    b[4..6].reverse();
    b[6..8].reverse();
    Uuid::from_bytes(b)
}

fn u16_at(p: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([p[o], p[o + 1]])
}

impl Session {
    pub fn new(info: SessionInfo, min_per_team: usize) -> Self {
        let social = SOCIAL_MODES.contains(&info.mode) || info.level == LEVEL_SOCIAL;
        let arena_level = info.level == LEVEL_ARENA;
        Self {
            info,
            social,
            arena_level,
            peers: HashMap::new(),
            pending: Vec::new(),
            slot_gen: [0; MAX_SLOTS],
            slot_addr: [None; MAX_SLOTS],
            frame: 0,
            wire_frame: 0,
            started: Instant::now(),
            arena: Arena::new(min_per_team),
            touch: HashMap::new(),
            touch_seq: [0; MAX_SLOTS],
            parked_loadouts: Vec::new(),
            roster_dirty: false,
            fleet_out: Vec::new(),
            udp_out: Vec::new(),
            last_activity: Instant::now(),
            ever_joined: false,
        }
    }

    pub fn player_count(&self) -> usize {
        self.peers.values().filter(|p| p.slot.is_some()).count()
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    fn now_us(&self) -> u64 {
        clock_us()
    }

    /// Keys for an entrant the backend placed here.
    pub fn add_keys(&mut self, s: &pb::SnsLobbySessionSuccessV5Message) {
        let crypto = SessionCrypto {
            out_flags: s.server_encoder_flags,
            out_aes: key32(&s.server_enc_key),
            out_mac: key32(&s.server_mac_key),
            out_rnd: key32(&s.server_random_key),
            out_seq: s.server_sequence_id & 0x7fff_ffff_ffff_ffff,
            in_flags: s.client_encoder_flags,
            in_aes: key32(&s.client_enc_key),
            in_mac: key32(&s.client_mac_key),
            in_rnd: key32(&s.client_random_key),
            in_seq: s.client_sequence_id & 0x7fff_ffff_ffff_ffff,
        };
        self.pending.retain(|p| p.created.elapsed() < Duration::from_secs(120));
        self.pending.push(PendingKeys { crypto, created: Instant::now() });
        self.last_activity = Instant::now();
    }

    // ── Inbound ────────────────────────────────────────────────────────────

    pub fn on_packet(&mut self, from: SocketAddr, data: &[u8]) {
        if !self.peers.contains_key(&from) {
            // Claim: whichever pending key set MACs this packet is this peer's.
            let Some(i) = self.pending.iter().position(|p| mac_matches(p.crypto.in_flags, &p.crypto.in_mac, data)) else {
                return;
            };
            let keys = self.pending.swap_remove(i);
            debug!(%from, "peer claimed its session keys");
            self.peers.insert(from, Peer::new(Transport::new(&keys.crypto)));
        }
        let Some(peer) = self.peers.get_mut(&from) else { return };
        let Some(rx) = peer.t.receive(data) else { return };
        peer.last_seen = Instant::now();
        self.last_activity = Instant::now();
        for m in rx.reliable {
            self.on_message(from, m, true);
        }
        for m in rx.mailbox {
            self.on_message(from, m, false);
        }
    }

    fn reliable_to(&mut self, to: SocketAddr, ty: u64, data: &[u8]) {
        if let Some(p) = self.peers.get_mut(&to) {
            p.t.queue_reliable(ty, data);
        }
    }

    fn joined(&self) -> Vec<SocketAddr> {
        self.peers.iter().filter(|(_, p)| p.stage == Stage::Joined).map(|(a, _)| *a).collect()
    }

    fn relay_reliable(&mut self, from: SocketAddr, ty: u64, data: &[u8], include_sender: bool) {
        for (a, p) in self.peers.iter_mut() {
            if p.stage == Stage::Joined && (include_sender || *a != from) {
                p.t.queue_reliable(ty, data);
            }
        }
    }

    fn relay_mailbox(&mut self, from: SocketAddr, ty: u64, key: u64, data: &[u8]) {
        for (a, p) in self.peers.iter_mut() {
            if *a != from && p.stage == Stage::Joined {
                p.t.queue_mailbox(ty, key, data);
            }
        }
    }

    fn on_message(&mut self, from: SocketAddr, m: DecodedMessage, reliable: bool) {
        let ty = m.type_;
        let d = m.data.as_slice();
        if let Some(p) = self.peers.get_mut(&from) {
            let obj = if d.len() >= 8 { u32::from_le_bytes([d[4], d[5], d[6], d[7]]) } else { 0 };
            let obj = if ty == PHYSICS { obj } else { 0 };
            *p.seen_types.entry((ty, reliable, d.len(), obj)).or_default() += 1;
        }
        let stage = self.peers.get(&from).map(|p| p.stage).unwrap_or(Stage::Handshake);
        match ty {
            CONNECT if reliable => {
                // Reply with the client's own view of its address + broadcast mask.
                let mut payload = [0u8; 16];
                if d.len() >= 16 {
                    payload[..8].copy_from_slice(&d[8..16]);
                }
                payload[8..].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0]);
                self.reliable_to(from, CONNECT, &payload);
                self.reliable_to(from, ENCODING_SUPPORT, &[]);
            }
            ENCODING_SUPPORT | KEEPALIVE if reliable => {}
            ENABLE_COMPRESSION if reliable => self.reliable_to(from, ENABLE_COMPRESSION, &[]),
            DISCONNECT if reliable => self.remove_peer(from, pb::lobby_entrant_removed_message::Code::Disconnected),
            PING_UPDATE if !reliable => {
                let mirror = self.peers.get(&from).is_some_and(|p| !p.join_complete);
                if mirror && let Some(p) = self.peers.get_mut(&from) {
                    p.t.queue_mailbox(PING_UPDATE, PING_UPDATE, &[]);
                }
            }
            JOIN_REQUEST => self.on_join_request(from, d),
            CLIENT_PING_RESPONSE => self.on_ping_response(from, d),
            REPLICATED_ACK => {
                if let Some(p) = self.peers.get_mut(&from) {
                    p.rep_acked = true;
                }
            }
            ENCODING_FAIL => debug!(%from, "client reported a decode failure"),
            _ if stage != Stage::Joined => {}
            IDS_REQUEST => {
                let level = d.get(..8).map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8]))).unwrap_or(0);
                match captured().ids.iter().find(|(l, _)| *l == level) {
                    Some((_, body)) => self.reliable_to(from, IDS_RESPONSE, body),
                    None => debug!(%from, level = format_args!("{level:#018x}"), "no ids response for level"),
                }
            }
            BALANCE_READY => match captured().balance.iter().find(|(l, _)| *l == self.info.level) {
                Some((_, body)) => self.reliable_to(from, BALANCE_UPDATE, body),
                None => warn!(level = format_args!("{:#018x}", self.info.level), "no balance settings for this level; client will wait"),
            },
            REPLICATED_READY => {
                if baseline(self.info.level).is_none() {
                    warn!(level = format_args!("{:#018x}", self.info.level), "no replicated baseline for this level; client will wait");
                } else if let Some(p) = self.peers.get_mut(&from) {
                    p.rep_ready = true;
                }
            }
            ENTRANT_STATE if reliable => self.on_entrant_state(from, d.to_vec()),
            LOADOUT_REQUEST if d.len() == 4 => {
                let want = usize::from(u16_at(d, 0));
                let body = self.slot_addr.get(want).copied().flatten().and_then(|a| self.peers.get(&a)).and_then(|p| p.loadout.clone());
                match body {
                    Some(b) => self.reliable_to(from, LOADOUT_RESPONSE, &b),
                    None => self.parked_loadouts.push((from, want)),
                }
            }
            LOADOUT_RESPONSE if d.len() == 452 => {
                let slot = self.peers.get(&from).and_then(|p| p.slot);
                if let Some(p) = self.peers.get_mut(&from) {
                    p.loadout = Some(d.to_vec());
                }
                if let Some(slot) = slot {
                    let waiting: Vec<SocketAddr> = self.parked_loadouts.iter().filter(|(_, s)| *s == slot).map(|(a, _)| *a).collect();
                    self.parked_loadouts.retain(|(_, s)| *s != slot);
                    for a in waiting {
                        self.reliable_to(a, LOADOUT_RESPONSE, d);
                    }
                    // Everyone else gets the change too.
                    self.relay_reliable(from, LOADOUT_RESPONSE, d, false);
                }
            }
            TOUCH_XFER_RELIABLE | TOUCH_XFER_MAILBOX => self.on_touch(from, d),
            COMPONENT_EVENT => {} // server-originated only
            STUN | LOADOUT_NUMBER if reliable => self.relay_reliable(from, ty, d, false),
            EQUIPMENT if reliable && d.len() == 56 => self.relay_reliable(from, ty, d, false),
            EMOTE | ACTOR_CREATE | ACTOR_DESTROY | FORCE_PUSH if reliable => self.relay_reliable(from, ty, d, true),
            _ if reliable => debug!(%from, ty = format_args!("{ty:#018x}"), "unhandled reliable message"),
            _ => self.on_mailbox(from, ty, m.mailbox, d),
        }
    }

    fn on_mailbox(&mut self, from: SocketAddr, ty: u64, key: u64, d: &[u8]) {
        if ty == PLAYER_STATE || ty == PHYSICS {
            if d.len() >= 4 {
                let (slot, generation) = (u16_at(d, 0), u16_at(d, 2));
                if let Some(p) = self.peers.get_mut(&from) {
                    if p.slot == Some(usize::from(slot)) {
                        p.generation = Some(generation);
                    }
                }
            }
            if ty == PHYSICS && self.arena_level {
                self.observe_disc(from, d);
            }
        }
        let key = if ty == VOIP {
            let slot = self.peers.get(&from).and_then(|p| p.slot).unwrap_or(0) as u64;
            key + (slot + 1) * VOICE_SLOT_STRIDE
        } else {
            key
        };
        self.relay_mailbox(from, ty, key, d);
    }

    fn on_ping_response(&mut self, from: SocketAddr, d: &[u8]) {
        if d.len() < 24 {
            return;
        }
        let echoed_us = u64::from_le_bytes(d[8..16].try_into().expect("8 bytes"));
        let now = self.now_us();
        let Some(p) = self.peers.get_mut(&from) else { return };
        if p.echo_logged.elapsed() > Duration::from_secs(2) {
            p.echo_logged = Instant::now();
            debug!(%from, echo_age_us = now.saturating_sub(echoed_us), "server-frame echo");
        }
        p.ping_token = u64::from_le_bytes(d[16..24].try_into().expect("8 bytes"));
        p.ping_dirty = true;
    }

    fn on_join_request(&mut self, from: SocketAddr, d: &[u8]) {
        if d.len() < 33 {
            return;
        }
        let Some(p) = self.peers.get(&from) else { return };
        if p.stage != Stage::Handshake {
            // Second join on one connection: refuse, like native.
            let mut rej = [0u8; 20];
            rej[16] = 2;
            self.reliable_to(from, JOIN_REJECTED, &rej);
            self.reliable_to(from, DISCONNECT, &[]);
            return;
        }
        let Some(slot) = (0..MAX_SLOTS).find(|s| self.slot_addr[*s].is_none()) else {
            let mut rej = [0u8; 20];
            rej[16] = 4;
            self.reliable_to(from, JOIN_REJECTED, &rej);
            self.reliable_to(from, DISCONNECT, &[]);
            warn!(%from, "lobby full: join refused");
            return;
        };
        self.slot_gen[slot] = self.slot_gen[slot].wrapping_add(1);
        self.slot_addr[slot] = Some(from);
        self.touch_seq[slot] = 0;
        let raw: [u8; 16] = d[17..33].try_into().expect("16 bytes");
        let requested = d.get(37..41).map(|b| u32::from_le_bytes(b.try_into().unwrap_or([0xff; 4]))).filter(|t| *t <= 4).map(|t| t as u8);
        let genidx = self.slot_gen[slot];
        let p = self.peers.get_mut(&from).expect("present");
        p.slot = Some(slot);
        p.genidx = genidx;
        p.session_raw = raw;
        p.entrant_id = swizzle(raw);
        p.requested_team = requested;
        p.stage = Stage::AwaitingAccept;
        let eid = p.entrant_id;
        info!(%from, slot, entrant = %eid, "join request");
        self.fleet_out.push(env(Em::LobbyEntrantConnected(pb::LobbyEntrantsConnectedMessage {
            lobby_session_id: self.info.lobby.to_string(),
            entrant_ids: vec![eid.to_string()],
        })));
    }

    /// The backend's verdict on entrants this server reported.
    pub fn on_accept(&mut self, ids: &[String], accepted: bool) {
        for id in ids {
            let Ok(eid) = id.parse::<Uuid>() else { continue };
            let Some(addr) = self.peers.iter().find(|(_, p)| p.entrant_id == eid && p.stage == Stage::AwaitingAccept).map(|(a, _)| *a) else {
                continue;
            };
            if accepted {
                self.assign_team(addr);
                self.send_join_sequence(addr);
            } else {
                warn!(%addr, entrant = %eid, "backend rejected entrant");
                let mut rej = [0u8; 20];
                rej[16] = 2;
                self.reliable_to(addr, JOIN_REJECTED, &rej);
                self.reliable_to(addr, DISCONNECT, &[]);
                self.drop_peer(addr, None);
            }
        }
    }

    fn assign_team(&mut self, addr: SocketAddr) {
        let social = self.social;
        let count = |s: &Self, t: u8| s.peers.values().filter(|p| p.stage == Stage::Joined && p.team == t).count();
        let (b, o) = (count(self, TEAM_BLUE), count(self, TEAM_ORANGE));
        let Some(p) = self.peers.get(&addr) else { return };
        let team = if social {
            TEAM_SOCIAL
        } else {
            match p.requested_team {
                Some(t @ (TEAM_BLUE | TEAM_ORANGE)) if (if t == TEAM_BLUE { b } else { o }) < 8 => t,
                Some(t @ (TEAM_SPECTATOR | TEAM_MODERATOR)) => t,
                _ if b <= o => TEAM_BLUE,
                _ => TEAM_ORANGE,
            }
        };
        let used: Vec<u16> = self.peers.values().filter(|q| q.stage == Stage::Joined && q.team == team).map(|q| q.team_slot).collect();
        let team_slot = (0..).find(|s| !used.contains(s)).unwrap_or(0);
        let p = self.peers.get_mut(&addr).expect("present");
        p.team = team;
        p.team_slot = team_slot;
    }

    fn join_setup_body(&self) -> [u8; 101] {
        let mut b: [u8; 101] = [
            0x10, 0, 0, 0, 0, 0, 0, 0, 0x03, 0, 0, 0, 0, 0, 0, 0, 0x63, 0x94, 0xe6, 0x3d, 0x12, 0x99, 0xec, 0x31, 0x01, 0, 0, 0, 0,
            0, 0, 0, 0x76, 0xcf, 0xdd, 0xcf, 0xf9, 0x9c, 0x2d, 0x04, 0xbc, 0x41, 0x99, 0xdc, 0xd6, 0xc7, 0x2d, 0xe3, 0x01, 0, 0, 0,
            0, 0, 0, 0, 0x04, 0x5c, 0xc7, 0xb1, 0x15, 0xfd, 0x9a, 0xd0, 0xb4, 0x4d, 0x9f, 0xc9, 0xdb, 0xc7, 0x2d, 0xe3, 0x04, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0, 0, 0, 0, 0, 0, 0, 0x01, 0, 0, 0, 0x00,
        ];
        b[32..40].copy_from_slice(&self.info.mode.to_le_bytes());
        let level: i64 = if self.social { self.info.level as i64 } else { -1 };
        b[56..64].copy_from_slice(&level.to_le_bytes());
        b
    }

    fn owner_state(&self) -> Vec<u8> {
        let mut by_slot: Vec<serde_json::Value> = Vec::new();
        let max = self.peers.values().filter(|p| p.stage == Stage::Joined).filter_map(|p| p.slot).max();
        if let Some(max) = max {
            by_slot = vec![serde_json::Value::Null; max + 1];
            for p in self.peers.values().filter(|p| p.stage == Stage::Joined) {
                if let Some(s) = p.slot {
                    by_slot[s] = serde_json::json!({"team": p.team, "teamslot": p.team_slot});
                }
            }
        }
        if by_slot.is_empty() {
            by_slot.push(serde_json::json!({"team": if self.social { TEAM_SOCIAL } else { TEAM_BLUE }, "teamslot": 0}));
        }
        let start = self.started.elapsed().as_secs();
        let start_time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) - start;
        let json = serde_json::json!({
            "battlepassstatgroup": "",
            // Signed like native's own JSON: echo_arena's hash is above i64::MAX.
            "gametype": self.info.mode as i64,
            "ingame": true,
            "level": self.info.level as i64,
            "starttime": start_time,
            "team": [{"alignment": 0}, {"alignment": 1}],
            "user": by_slot,
        });
        let mut out = vec![0u8];
        out.extend_from_slice(json.to_string().as_bytes());
        out.push(0);
        out
    }

    /// An earlier entrant's state, restated for a newcomer's join batch.
    fn batched_entrant_state(p: &Peer) -> Option<Vec<u8>> {
        let header = p.entrant_header.as_ref()?;
        let latest = p.entrant_latest.as_ref()?;
        let slot = p.slot? as u16;
        let json = latest.get(ENTRANT_HEADER..).and_then(|r| r.iter().position(|&b| b == 0).map(|n| &r[..=n])).unwrap_or(&[]);
        let mut b = Vec::with_capacity(ENTRANT_HEADER + json.len());
        b.extend_from_slice(&header[..0x60]);
        b.extend_from_slice(&u16::from(header[0x60] & 0b11).to_le_bytes());
        b.extend_from_slice(&slot.to_le_bytes());
        b.extend_from_slice(&header[0x64..ENTRANT_HEADER]);
        b.extend_from_slice(json);
        Some(b)
    }

    fn send_join_sequence(&mut self, addr: SocketAddr) {
        let setup = self.join_setup_body();
        let earlier: Vec<Vec<u8>> = self
            .peers
            .iter()
            .filter(|(a, p)| **a != addr && p.stage == Stage::Joined)
            .filter_map(|(_, p)| Self::batched_entrant_state(p))
            .collect();
        let Some(p) = self.peers.get_mut(&addr) else { return };
        p.stage = Stage::Joined;
        let mut accepted = Vec::with_capacity(33);
        accepted.push(1u8);
        accepted.extend_from_slice(&(p.slot.unwrap_or(0) as u64).to_le_bytes());
        accepted.extend_from_slice(&u64::from(p.genidx).to_le_bytes());
        accepted.extend_from_slice(&p.session_raw);
        let owner = self.owner_state();
        let mut entries: Vec<(u64, &[u8])> = vec![(JOIN_SETUP, &setup)];
        for e in &earlier {
            entries.push((ENTRANT_STATE, e));
        }
        entries.push((OWNER_STATE, &owner));
        entries.push((HOST_STATE, &[0x00, 0x7b, 0x7d, 0x00]));
        entries.push((JOIN_ACCEPTED, &accepted));
        let batch = reliable_batch(&entries);
        let p = self.peers.get_mut(&addr).expect("present");
        p.t.queue_reliable(RELIABLE_BATCH, &batch);
        let (slot, team) = (p.slot, p.team);
        self.roster_dirty = true;
        self.ever_joined = true;
        info!(%addr, slot, team, players = self.player_count(), "player joined");
    }

    fn on_entrant_state(&mut self, from: SocketAddr, d: Vec<u8>) {
        if d.len() < ENTRANT_HEADER {
            return;
        }
        self.relay_reliable(from, ENTRANT_STATE, &d, false);
        let Some(p) = self.peers.get_mut(&from) else { return };
        if p.entrant_header.is_none() {
            p.entrant_header = Some(d[..ENTRANT_HEADER].to_vec());
        }
        p.entrant_latest = Some(d);
        p.entrant_states += 1;
        if p.entrant_states == 2 {
            if let Some(slot) = p.slot {
                let mut req = [0u8; 4];
                req[..2].copy_from_slice(&(slot as u16).to_le_bytes());
                req[2..].copy_from_slice(&p.generation.unwrap_or(0).to_le_bytes());
                p.deferred.push((2, LOADOUT_REQUEST, req.to_vec()));
            }
            p.deferred.push((2, NEW_UNLOCKS, captured().new_unlocks.clone()));
            p.deferred.push((2, REFRESH_PROFILE, captured().refresh_profile.clone()));
            p.join_tail_in = Some(3);
            p.owner_resend_in = Some(3);
        }
    }

    fn on_touch(&mut self, from: SocketAddr, d: &[u8]) {
        if d.len() < 0x30 {
            return;
        }
        let u32_at = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
        let seq = u16_at(d, 4);
        let acked = u16_at(d, 6);
        let sender = u32_at(8);
        let new_owner = u32_at(0x0c);
        let believed = u32_at(0x10);
        let entry_id = u32_at(0x14);
        let actor = &d[0x18..0x20];
        let stamp = u64::from_le_bytes(d[0x20..0x28].try_into().expect("8 bytes"));
        debug!(%from, seq, sender, new_owner, believed, entry_id, "touch request");
        let exclusive = d[0x28] & 1 != 0;
        let idx = (sender & 0xffff) as usize;
        if idx >= MAX_SLOTS || (sender >> 16) == 0xffff {
            return;
        }
        let want = ((u32::from(self.touch_seq[idx]) + 1) % 0xffff) as u16;
        if seq != want {
            return; // duplicate or out of order
        }
        self.touch_seq[idx] = seq;
        let e = *self.touch.entry(entry_id).or_insert(TouchEntry { owner: u32::MAX, stamp: 0, generation: 0 });
        let release = new_owner == u32::MAX;
        let target_ok = release || ((new_owner & 0xffff) < MAX_SLOTS as u32 && (new_owner >> 16) != 0xffff);
        // Before play (and between points) the disc is the server's: native
        // keeps it parked at centre and nobody can take it.
        let locked = self.arena_level && !release && self.arena.phase != Phase::Playing;
        let grant = target_ok
            && !locked
            && if release {
                sender == e.owner
            } else if believed == u32::MAX {
                e.owner == u32::MAX && (e.generation & 0xf) != 0xf
            } else {
                believed == e.owner && stamp >= e.stamp
            };
        let mut reply = [0u8; 0x20];
        reply[0..8].copy_from_slice(&self.info.level.to_le_bytes());
        reply[0x0c..0x10].copy_from_slice(&entry_id.to_le_bytes());
        reply[0x10..0x18].copy_from_slice(actor);
        reply[0x1a..0x1c].copy_from_slice(&seq.to_le_bytes());
        if grant {
            let prev_owner = e.owner;
            let ent = self.touch.get_mut(&entry_id).expect("inserted");
            ent.generation = (ent.generation + 1) % 15;
            ent.owner = new_owner;
            ent.stamp = stamp;
            reply[0x08..0x0c].copy_from_slice(&new_owner.to_le_bytes());
            // 0x10: exclusive grab. 0x20: the requester already held it
            // (native sets it on a re-grab; without it the client plays the
            // steal sound).
            let regrab = !release && prev_owner == new_owner;
            let flags = (u16::from(exclusive) << 4) | (u16::from(regrab) << 5) | u16::from(ent.generation & 0xf);
            reply[0x18..0x1a].copy_from_slice(&flags.to_le_bytes());
            reply[0x1c..0x20].copy_from_slice(&u32::from(acked).to_le_bytes());
            self.reliable_to(from, TOUCH_GRANTED, &reply);
        } else {
            reply[0x08..0x0c].copy_from_slice(&e.owner.to_le_bytes());
            reply[0x18..0x1a].copy_from_slice(&u16::from(e.generation & 0xf).to_le_bytes());
            self.reliable_to(from, TOUCH_DECLINED, &reply);
        }
    }

    fn observe_disc(&mut self, from: SocketAddr, d: &[u8]) {
        let Some((owner, x, y, depth)) = msg::disc_transform(d) else { return };
        if let Some(p) = self.peers.get_mut(&from) {
            p.disc_seen += 1;
            p.disc_max_depth = p.disc_max_depth.max(depth.abs());
            if p.disc_seen % 200 == 1 {
                debug!(%from, owner, x, y, depth, seen = p.disc_seen, max_depth = p.disc_max_depth, armed = self.arena.goals_armed, phase = ?self.arena.phase, "disc sample");
            }
        }
        if depth.abs() >= 30_000 {
            debug!(owner, x, y, depth, armed = self.arena.goals_armed, phase = ?self.arena.phase, "disc near a goal plane");
        }
        // While the server holds the disc (goal, relaunch) clients' copies
        // are ignored.
        if self.arena.frozen.is_some() || self.arena.parked_for.is_some() {
            return;
        }
        let owner_gen = self.peers.get(&from).and_then(|p| p.generation).unwrap_or(0);
        if usize::from(owner) < MAX_SLOTS {
            self.arena.disc_owner = (u32::from(owner_gen) << 16) | u32::from(owner);
        }
        if !self.arena.goals_armed || self.arena.phase != Phase::Playing {
            return;
        }
        if depth.abs() < 36_000 || i64::from(x) * i64::from(x) + i64::from(y) * i64::from(y) > 1_400 * 1_400 {
            return;
        }
        let blue_scored = depth > 0;
        let mut rec = [0u8; 28];
        rec[0..2].copy_from_slice(&u16::from(!blue_scored).to_le_bytes());
        rec[2..4].copy_from_slice(&2u16.to_le_bytes());
        rec[4..6].copy_from_slice(&1u16.to_le_bytes()); // native: 1 on a 2-point goal, 7 on a 3-pointer
        rec[6..10].copy_from_slice(&self.arena.disc_owner.to_le_bytes());
        rec[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
        rec[24..28].copy_from_slice(&(ROUND_SECS - self.arena.clock).to_le_bytes());
        for a in self.joined() {
            self.reliable_to(a, POST_SCORE, &rec);
        }
        // Native: the server takes the disc and pins it on the goal plane
        // where it crossed, bumping the reset count so clients snap to it.
        self.arena.frozen = Some((x, y, if blue_scored { 36_000 } else { -36_000 }));
        self.arena.disc_resets = (self.arena.disc_resets + 3) & 0xf;
        for e in self.touch.values_mut() {
            e.owner = u32::MAX;
            e.generation = (e.generation + 1) % 15;
        }
        let scorer = (self.arena.disc_owner & 0xffff) as usize;
        if let Some(p) = self.peers.values_mut().find(|p| p.slot == Some(scorer)) {
            p.stats.points += 2;
            p.stats.goals += 1;
            p.stats.shots += 1;
            p.stats.two_pt += 1;
        }
        self.arena.goal(self.frame, blue_scored, 2);
        info!(blue = self.arena.blue, orange = self.arena.red, "goal");
    }

    // ── Departures ─────────────────────────────────────────────────────────

    fn remove_peer(&mut self, addr: SocketAddr, code: pb::lobby_entrant_removed_message::Code) {
        self.drop_peer(addr, Some(code));
    }

    fn drop_peer(&mut self, addr: SocketAddr, code: Option<pb::lobby_entrant_removed_message::Code>) {
        let Some(p) = self.peers.remove(&addr) else { return };
        if let Some(slot) = p.slot {
            self.slot_addr[slot] = None;
            let mut removed = [0u8; 16];
            removed[..8].copy_from_slice(&(slot as u64).to_le_bytes());
            removed[8..].copy_from_slice(&8u64.to_le_bytes());
            for a in self.joined() {
                self.reliable_to(a, ENTRANT_REMOVED, &removed);
            }
            self.parked_loadouts.retain(|(a, s)| *a != addr && *s != slot);
            self.roster_dirty = true;
        }
        if p.stage != Stage::Handshake {
            self.fleet_out.push(env(Em::LobbyEntrantRemoved(pb::LobbyEntrantRemovedMessage {
                lobby_session_id: self.info.lobby.to_string(),
                entrant_id: p.entrant_id.to_string(),
                code: code.unwrap_or(pb::lobby_entrant_removed_message::Code::Disconnected) as i32,
            })));
        }
        info!(%addr, slot = p.slot, players = self.player_count(), "player left");
        self.last_activity = Instant::now();
    }

    pub fn kick(&mut self, entrant: &str) {
        let Ok(eid) = entrant.parse::<Uuid>() else { return };
        if let Some(a) = self.peers.iter().find(|(_, p)| p.entrant_id == eid).map(|(a, _)| *a) {
            self.reliable_to(a, DISCONNECT, &[]);
            self.flush_peer(a);
            self.drop_peer(a, Some(pb::lobby_entrant_removed_message::Code::Kicked));
        }
    }

    /// Tell every peer goodbye (session over).
    pub fn close(&mut self) {
        let all: Vec<SocketAddr> = self.peers.keys().copied().collect();
        for a in all {
            self.reliable_to(a, DISCONNECT, &[]);
            self.flush_peer(a);
        }
        self.peers.clear();
    }

    // ── Outbound ───────────────────────────────────────────────────────────

    fn flush_peer(&mut self, addr: SocketAddr) {
        if let Some(p) = self.peers.get_mut(&addr) {
            let mut pkts = Vec::new();
            p.t.flush(&mut pkts);
            self.udp_out.extend(pkts.into_iter().map(|d| (addr, d)));
        }
    }

    /// Emit everything queued (called right after inbound processing and on
    /// every frame, so relayed traffic is never held back).
    pub fn flush(&mut self) {
        let mut pkts = Vec::new();
        for (a, p) in self.peers.iter_mut() {
            p.t.flush(&mut pkts);
            self.udp_out.extend(pkts.drain(..).map(|d| (*a, d)));
        }
    }

    /// Resend timers only (cheap; runs between frames).
    pub fn service(&mut self) {
        self.flush();
    }

    fn publish_owner_state(&mut self) {
        let owner = self.owner_state();
        for a in self.joined() {
            self.reliable_to(a, OWNER_STATE, &owner);
        }
    }

    /// 20 Hz: server frame message, ping estimates, replication, rules.
    pub fn server_frame(&mut self) {
        self.frame += 1;
        // Wire frame word for this server frame (also the mailbox key bucket,
        // as native keys by it); `self.frame` stays the 20 Hz rules counter.
        let (frame, now_us) = wire_clock();
        if frame <= self.wire_frame {
            return; // two server frames inside one tick: nothing new to publish
        }
        self.wire_frame = frame;

        // Peers that went silent or stopped acking.
        let dead: Vec<SocketAddr> =
            self.peers.iter().filter(|(_, p)| p.last_seen.elapsed() > PEER_TIMEOUT || p.t.stalled()).map(|(a, _)| *a).collect();
        for a in dead {
            self.remove_peer(a, pb::lobby_entrant_removed_message::Code::Timeout);
        }

        if self.roster_dirty {
            self.roster_dirty = false;
            self.publish_owner_state();
        }

        let mut sf = [0u8; 16];
        sf[..8].copy_from_slice(&frame.to_le_bytes());
        sf[8..].copy_from_slice(&now_us.to_le_bytes());

        // Ping estimates measured from each peer's echoes: no fixed value, no floor.
        let mut estimates = Vec::new();
        for p in self.peers.values_mut() {
            let fresh = p.t.take_rtt_fresh();
            if (p.ping_dirty || (fresh && self.frame % 10 == 0)) && let (Some(slot), Some(rtt)) = (p.slot, p.rtt_us()) {
                let mut e = [0u8; 24];
                e[0..2].copy_from_slice(&(slot as u16).to_le_bytes());
                e[4..8].copy_from_slice(&(rtt as f32).to_le_bytes());
                e[8..16].copy_from_slice(&p.ping_token.to_le_bytes());
                estimates.push((slot as u64, e));
            }
            p.ping_dirty = false;
        }
        let pings = (self.frame % (2 * FRAME_HZ) == 0).then(|| {
            let mut b = [0u8; 33];
            b[0] = 1;
            for p in self.peers.values_mut() {
                if p.stage == Stage::Joined && p.t.rtt_us().is_none() {
                    debug!(slot = ?p.slot, "no ping samples yet; client shows 0 until one arrives");
                }
                if let (Some(s), Some(rtt)) = (p.slot, p.rtt_us()) {
                    let ms = ((rtt + 500) / 1000).min(999) as u16;
                    b[1 + s * 2..3 + s * 2].copy_from_slice(&ms.to_le_bytes());
                    debug!(slot = s, rtt_us = rtt, raw_rtt_us = ?p.t.raw_rtt_us(), client_frame_us = p.t.client_frame_us(), samples = p.t.rtt_sample_count(), "measured ping");
                    if !p.seen_types.is_empty() {
                        let mut v: Vec<_> = p.seen_types.drain().collect();
                        v.sort_by(|a, b| b.1.cmp(&a.1));
                        let list: Vec<String> = v.iter().take(16).map(|((t, r, l, o), n)| format!("{t:#x}/{}/{l}B/obj{o:#x}x{n}", if *r { "rel" } else { "mb" })).collect();
                        debug!(slot = s, "client sent: {}", list.join(" "));
                    }
                }
            }
            b
        });

        let mut tails = Vec::new();
        let mut owner_resend = false;
        for (a, p) in self.peers.iter_mut() {
            if p.stage != Stage::Joined {
                continue;
            }
            p.t.queue_mailbox(SERVER_FRAME, key_server_frame(frame), &sf);
            for (slot, e) in &estimates {
                p.t.queue_mailbox(USER_PING_EST, key_ping_est(frame, *slot), e);
            }
            if let Some(b) = &pings {
                p.t.queue_mailbox(UPDATE_PINGS, KEY_UPDATE_PINGS, b);
            }
            let mut due = Vec::new();
            p.deferred.retain_mut(|(n, ty, data)| {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    due.push((*ty, std::mem::take(data)));
                    false
                } else {
                    true
                }
            });
            for (ty, data) in due {
                p.t.queue_reliable(ty, &data);
            }
            if let Some(n) = p.join_tail_in {
                if n <= 1 {
                    p.join_tail_in = None;
                    tails.push(*a);
                } else {
                    p.join_tail_in = Some(n - 1);
                }
            }
            if let Some(n) = p.owner_resend_in {
                if n <= 1 {
                    p.owner_resend_in = None;
                    owner_resend = true;
                } else {
                    p.owner_resend_in = Some(n - 1);
                }
            }
        }
        for a in tails {
            let mut info = [0u8; 96];
            info[0] = b'0';
            info[32..42].copy_from_slice(b"RAD office");
            info[64..71].copy_from_slice(b"US West");
            self.reliable_to(a, JOIN_COMPLETE, &[0]);
            self.reliable_to(a, msg::SERVER_INFO, &info);
            if let Some(p) = self.peers.get_mut(&a) {
                p.join_complete = true;
            }
        }
        if owner_resend {
            self.publish_owner_state();
        }

        if self.arena_level {
            self.arena_rules();
        }
        self.replicate();
        self.flush();
    }

    // ── Arena ──────────────────────────────────────────────────────────────

    fn team_counts(&self) -> (usize, usize) {
        let joined = self.peers.values().filter(|p| p.stage == Stage::Joined);
        joined.fold((0, 0), |(b, o), p| match p.team {
            TEAM_BLUE => (b + 1, o),
            TEAM_ORANGE => (b, o + 1),
            _ => (b, o),
        })
    }

    fn arena_rules(&mut self) {
        let f = self.frame;
        let wf = self.wire_frame;
        let dt = 1.0 / FRAME_HZ as f32;
        if self.arena.phase == Phase::Waiting {
            let (b, o) = self.team_counts();
            let need = self.arena.min_per_team;
            if b >= need && o >= need && b + o > 0 {
                info!(blue = b, orange = o, "both teams present: pre-match countdown started");
                self.arena.start_round(f);
            }
        }
        // Due actions, in schedule order.
        let mut due = Vec::new();
        self.arena.schedule.retain(|(at, act)| {
            if *at <= f {
                due.push(*act);
                false
            } else {
                true
            }
        });
        let joined = self.joined();
        for act in due {
            match act {
                Act::State(s) => {
                    debug!(state = format_args!("{s:#018x}"), countdown = self.arena.countdown, "arena state");
                    self.arena.state = s;
                }
                Act::Event(sym, module, filter) => {
                    let ev = component_event(sym, module, filter);
                    for a in &joined {
                        if let Some(p) = self.peers.get_mut(a) {
                            p.t.queue_reliable(COMPONENT_EVENT, &ev);
                        }
                    }
                }
                Act::Countdown(secs) => {
                    self.arena.countdown = secs;
                    self.arena.countdown_running = secs > 0.0;
                }
                Act::Phase(ph) => {
                    info!(phase = ?ph, "arena phase");
                    self.arena.phase = ph;
                }
                Act::Park(side) => {
                    self.arena.parked_for = side;
                    self.arena.frozen = None;
                    if side.is_none() {
                        // Native republishes "no owner" at main_loop_enter.
                        self.arena.disc_owner = u32::MAX;
                    }
                }
                Act::ArmGoals => self.arena.goals_armed = true,
                Act::LaunchFx(fx) => self.arena.launch_fx = fx,
                Act::Flag(h, v) => self.arena.set_flag(h, v),
            }
        }
        if let Some((light, fx)) = self.arena.launch_fx {
            for (sym, module) in [(EV_RESTART_DISC, MOD_MATCH), (light, MOD_DISC_FX), (fx, MOD_DISC_FX)] {
                let ev = component_event(sym, module, FILTER_ALL);
                for a in &joined {
                    if let Some(p) = self.peers.get_mut(a) {
                        p.t.queue_reliable(COMPONENT_EVENT, &ev);
                    }
                }
            }
        }
        if self.arena.phase == Phase::Playing && self.arena.disc_owner != u32::MAX {
            let holder = (self.arena.disc_owner & 0xffff) as usize;
            if let Some(p) = self.peers.values_mut().find(|p| p.slot == Some(holder)) {
                p.stats.possession += 1;
            }
        }
        if matches!(self.arena.phase, Phase::Playing | Phase::Goal) && f % (3 * FRAME_HZ) == 0 {
            self.send_stats();
        }
        if std::mem::take(&mut self.arena.awards_due) {
            self.send_stats();
            self.send_top_awards();
        }
        if self.arena.countdown_running {
            self.arena.countdown = (self.arena.countdown - dt).max(0.0);
            self.arena.countdown_running = self.arena.countdown > 0.0;
        }
        if self.arena.clock_runs() && self.arena.phase == Phase::Playing {
            self.arena.clock = (self.arena.clock - dt).max(0.0);
            if self.arena.clock <= 0.0 {
                info!(blue = self.arena.blue, orange = self.arena.red, "match over");
                self.arena.end_match(f);
            }
        }
        // Back to waiting if a match never started and a team emptied.
        if self.arena.phase == Phase::Countdown {
            let (b, o) = self.team_counts();
            if b < self.arena.min_per_team || o < self.arena.min_per_team {
                self.arena.schedule.clear();
                self.arena.phase = Phase::Waiting;
                self.arena.state = ST_PRE_MATCH;
                self.arena.countdown = PRE_MATCH_SECS;
                self.arena.countdown_running = false;
                info!("a team emptied before launch: waiting again");
            }
        }
        // Server-side disc state every 3 frames.
        self.arena.disc_us += 1_000_000 / FRAME_HZ;
        if f % 3 == 0 {
            let mut d = captured().disc_physics;
            d[8..12].copy_from_slice(&(self.arena.disc_us as u32).to_le_bytes());
            // High nibble: a running counter (native's steps about 1.5 per
            // 25 ms packet); low nibble: the reset count.
            let st = ((((wf / 2) & 0xf) as u32) << 4) | (self.arena.disc_resets & 0xf);
            d[24..28].copy_from_slice(&st.to_le_bytes());
            if let Some((x, y, depth)) = self.arena.frozen {
                msg::write_disc_transform(&mut d, x, y, depth);
            } else if let Some(blue) = self.arena.parked_for {
                msg::write_disc_transform(&mut d, 0, 4536, if blue { 27_500 } else { -27_500 });
            }
            let key = key_world_physics(wf, 0x10);
            for a in &joined {
                if let Some(p) = self.peers.get_mut(a) {
                    p.t.queue_mailbox(PHYSICS, key, &d);
                }
            }
        }
    }

    // ── Match stats ────────────────────────────────────────────────────────

    /// Native, every 3 s of play: one `SR15NetReliableStatUpdate` per player
    /// (`[slot u32]` + JSON of `add` deltas since the last one, `{}` when
    /// nothing changed) and one `SR15NetReliableTeamStatUpdate` per team
    /// (running totals). These fill the scoreboard.
    fn send_stats(&mut self) {
        let joined = self.joined();
        let mut msgs: Vec<Vec<u8>> = Vec::new();
        let mut team = [Stats::default(); 2];
        for p in self.peers.values_mut() {
            let Some(slot) = p.slot else { continue };
            let (s, o) = (p.stats, p.stats_sent);
            if let Some(t) = team.get_mut(usize::from(p.team)) {
                t.points += s.points;
                t.goals += s.goals;
                t.shots += s.shots;
                t.possession += s.possession;
            }
            let mut fields = Vec::new();
            let mut add = |name: &str, v: String| fields.push(format!("\"{name}\":{{\"op\":\"add\",\"val\":{v}}}"));
            if s.points > o.points {
                add("Points", (s.points - o.points).to_string());
            }
            if s.possession > o.possession {
                add("PossessionTime", format!("{:.6}", (s.possession - o.possession) as f32 / FRAME_HZ as f32));
            }
            if s.shots > o.shots {
                add("ShotsOnGoal", (s.shots - o.shots).to_string());
            }
            if s.goals > o.goals {
                add("Goals", (s.goals - o.goals).to_string());
            }
            // Only the keys native sends mid-match; TwoPointGoals and the like
            // are profile stats (SNSRefreshProfileFromServer) and an unknown
            // key here leaves the scoreboard at 0.
            p.stats_sent = s;
            let mut m = (slot as u32).to_le_bytes().to_vec();
            m.extend_from_slice(format!("{{{}}}", fields.join(",")).as_bytes());
            m.push(0);
            msgs.push(m);
        }
        let mut team_msgs = Vec::new();
        for (i, t) in team.iter().enumerate() {
            let mut m = (i as u16).to_le_bytes().to_vec();
            for (k, h) in TEAM_STATS.iter().enumerate() {
                let (mant, exp) = match k {
                    TS_POINTS => (t.points as i32, 0),
                    TS_POSSESSION => ((t.possession as u64 * 64 / FRAME_HZ) as i32, -6),
                    TS_SHOTS => (t.shots as i32, 0),
                    TS_GOALS => (t.goals as i32, 0),
                    _ => (0, 0),
                };
                m.extend_from_slice(&h.to_le_bytes());
                m.extend_from_slice(&mant.to_le_bytes());
                m.extend_from_slice(&if mant == 0 { 0i32 } else { exp }.to_le_bytes());
                m.extend_from_slice(&u32::from(mant != 0).to_le_bytes());
            }
            team_msgs.push(m);
        }
        for a in &joined {
            for m in &msgs {
                self.reliable_to(*a, STAT_UPDATE, m);
            }
            for m in &team_msgs {
                self.reliable_to(*a, TEAM_STAT_UPDATE, m);
            }
        }
    }

    /// Native, at match over: `SR15NetTopAwardsMsg` = `[count u8]` + 72-byte
    /// records `[award u64][stat u64][value u64][14 zero bytes][name 30]
    /// [f32]`. The end-of-match board reads it (the MVP).
    fn send_top_awards(&mut self) {
        let best = self
            .peers
            .values()
            .filter(|p| p.slot.is_some())
            .max_by_key(|p| (p.stats.points, p.stats.possession));
        let Some(best) = best else { return };
        let name = best
            .entrant_header
            .as_deref()
            .and_then(|h| h.get(60..96))
            .map(|n| n.split(|&b| b == 0).next().unwrap_or(&[]).to_vec())
            .unwrap_or_default();
        let mut rec = [0u8; 72];
        rec[0..8].copy_from_slice(&AWARD_MVP_ID.to_le_bytes());
        rec[8..16].copy_from_slice(&AWARD_MVP_STAT.to_le_bytes());
        rec[16..24].copy_from_slice(&u64::from(best.stats.points.max(1)).to_le_bytes());
        let n = name.len().min(29);
        rec[38..38 + n].copy_from_slice(&name[..n]);
        rec[68..72].copy_from_slice(&(best.stats.possession as f32 / FRAME_HZ as f32).to_le_bytes());
        let mut m = vec![1u8];
        m.extend_from_slice(&rec);
        info!(mvp = %String::from_utf8_lossy(&name), points = best.stats.points, "top awards");
        for a in self.joined() {
            self.reliable_to(a, TOP_AWARDS, &m);
        }
    }

    // ── Replicated variables ───────────────────────────────────────────────

    fn replicate(&mut self) {
        let Some(base) = baseline(self.info.level) else { return };
        let frame = self.wire_frame;
        let arena = self.arena_level;
        let a = &self.arena;
        let tracked: Vec<(u64, u64)> = if arena {
            let raw = |h| var_raw(&base[0], h).unwrap_or(0);
            let mut v = vec![
                (VAR_GAME_STATE, a.state),
                (VAR_BLUE_SCORE, (raw(VAR_BLUE_SCORE) & 0xffff_ffff_0000_0000) | u64::from(a.blue)),
                (VAR_RED_SCORE, (raw(VAR_RED_SCORE) & 0xffff_ffff_0000_0000) | u64::from(a.red)),
                if a.clock_runs() {
                    (VAR_GAME_CLOCK, with_f32(raw(VAR_GAME_CLOCK), a.clock))
                } else {
                    (VAR_COUNTDOWN, with_f32(raw(VAR_COUNTDOWN), a.countdown))
                },
            ];
            // Native publishes the disc owner only once play has begun.
            if matches!(a.phase, Phase::Playing | Phase::Goal | Phase::Over) {
                v.push((VAR_DISC_OWNER, u64::from(a.disc_owner)));
            }
            for &(h, b) in &a.flags {
                v.push((h, (raw(h) & !0xff) | u64::from(b)));
            }
            v
        } else {
            Vec::new()
        };
        let fresh = |payload: &[u8], a: &Arena| -> Vec<u8> {
            let mut p = payload.to_vec();
            if !arena || p.len() < 30 {
                return p;
            }
            let n = u16::from_le_bytes([p[28], p[29]]) as usize;
            for i in 0..n {
                let o = 30 + i * 16;
                if o + 16 > p.len() {
                    break;
                }
                let h = u64::from_le_bytes(p[o..o + 8].try_into().expect("8"));
                let old = u64::from_le_bytes(p[o + 8..o + 16].try_into().expect("8"));
                let new = match h {
                    VAR_GAME_STATE => Some(a.state),
                    VAR_GAME_CLOCK => Some(with_f32(old, a.clock)),
                    VAR_COUNTDOWN => Some(with_f32(old, a.countdown)),
                    VAR_BLUE_SCORE => Some((old & 0xffff_ffff_0000_0000) | u64::from(a.blue)),
                    VAR_RED_SCORE => Some((old & 0xffff_ffff_0000_0000) | u64::from(a.red)),
                    VAR_TOTAL_ROUNDS => Some(1),
                    VAR_DISC_OWNER => Some(u64::from(a.disc_owner)),
                    VAR_BLUE_ROUND_SCORE | VAR_RED_ROUND_SCORE | VAR_ROUND_COUNTER | VAR_OVERTIME => Some(old & 0xffff_ffff_0000_0000),
                    _ => None,
                };
                let new = match a.flags.iter().find(|(k, _)| *k == h) {
                    Some(&(_, b)) => Some((old & !0xff) | u64::from(b)),
                    None => new,
                };
                if let Some(v) = new {
                    p[o + 8..o + 16].copy_from_slice(&v.to_le_bytes());
                }
            }
            p
        };
        let stamp = |p: &mut Vec<u8>, seq_off: u64| {
            if p.len() >= 28 {
                let seq = u64::from_le_bytes(p[16..24].try_into().expect("8")).wrapping_add(seq_off);
                p[16..24].copy_from_slice(&seq.to_le_bytes());
                p[24..28].copy_from_slice(&(seq as u32).to_le_bytes());
            }
        };
        for p in self.peers.values_mut() {
            if p.stage != Stage::Joined || !p.rep_ready {
                continue;
            }
            let slot = p.slot.unwrap_or(0) as u64;
            let full = !p.rep_sent || (!arena && !p.rep_acked && self.frame % 10 == 0);
            if full {
                for (stream, b) in base.iter().enumerate() {
                    let mut body = fresh(b, &self.arena);
                    stamp(&mut body, p.rep_seq);
                    p.t.queue_mailbox(REPLICATED_VARS, key_replicated(stream, frame, slot), &body);
                }
                p.rep_sent = true;
                p.rep_seq += 1;
            } else if arena {
                // Every frame: the clock/countdown plus all tracked vars (tiny).
                let mut body = Vec::with_capacity(30 + tracked.len() * 16);
                body.extend_from_slice(&base[0][..24]);
                body.extend_from_slice(&[0; 4]);
                body.extend_from_slice(&(tracked.len() as u16).to_le_bytes());
                for (h, v) in &tracked {
                    body.extend_from_slice(&h.to_le_bytes());
                    body.extend_from_slice(&v.to_le_bytes());
                }
                stamp(&mut body, p.rep_seq);
                p.t.queue_mailbox(REPLICATED_VARS, key_replicated(0, frame, slot), &body);
                p.rep_seq += 1;
            }
        }
    }
}
