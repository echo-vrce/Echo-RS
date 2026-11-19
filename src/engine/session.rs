//! One lobby/match: entrants, the join sequence, message relay, replicated
//! game state, real per-peer ping, disc ownership and the arena rules.
//!
//! Everything here is driven by two calls: [`Session::on_packet`] for each
//! datagram and [`Session::server_frame`] at `--frame-hz` (default 20). Relayed player traffic is
//! forwarded as soon as it is decoded; nothing waits for the next frame.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use nevr_proto::rtapi::v1 as pb;
use nevr_proto::rtapi::v1::envelope::Message as Em;
use rad_protocol::codec::decoder::DecodedMessage;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::goal;
use super::msg::{self, *};
use super::rules::{self, RULES, Rules, Target};
use super::stats::{CombatStats, DiscTracker, Stat, Stats};
use super::transport::{SessionCrypto, Transport, mac_matches, reliable_batch};

pub const MAX_SLOTS: usize = 16;
/// Server frames per second (frame message, replication, rules): `--frame-hz`
/// at startup, `frame-hz N` on stdin while running. Every timing in the rules
/// is in seconds and converted with this, so any rate runs the same match.
static FRAME_HZ_V: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(20);

pub fn frame_hz() -> u64 {
    FRAME_HZ_V.load(std::sync::atomic::Ordering::Relaxed)
}

pub const MIN_FRAME_HZ: u64 = 10;
pub const MAX_FRAME_HZ: u64 = 240;
/// Native public servers' wire rate; comp servers run 180. Below this the
/// wire clock still ticks at 120 Hz (every frame message is sent).
pub const BASE_WIRE_HZ: u64 = 120;

/// The wire clock: (frame at the last rate change, its server µs, µs per
/// frame). Frames count on from there at the current rate, so a change in
/// a running process never repeats or skips a frame number.
static WIRE: std::sync::Mutex<Option<(u64, u64, u64)>> = std::sync::Mutex::new(None);

fn wire_tick_for(hz: u64) -> u64 {
    1_000_000 / hz.max(BASE_WIRE_HZ)
}

/// Sets the rate (clamped to 10..=240); safe while games run. Returns the
/// rate in force.
pub fn set_frame_hz(hz: u64) -> u64 {
    let hz = hz.clamp(MIN_FRAME_HZ, MAX_FRAME_HZ);
    FRAME_HZ_V.store(hz, std::sync::atomic::Ordering::Relaxed);
    let tick = wire_tick_for(hz);
    let mut w = WIRE.lock().expect("wire clock");
    let now = clock_us();
    *w = Some(match *w {
        // Start on a 240 Hz scale of uptime: whatever rate an earlier run of
        // this machine used, its frames stayed below that, so a restart at a
        // lower rate never goes backwards for a client that remembers them.
        None => (now * MAX_FRAME_HZ / 1_000_000, now, tick),
        Some(c) if c.2 == tick => c,
        Some((f0, t0, old)) => {
            let f = f0 + now.saturating_sub(t0) / old;
            (f + 1, t0 + (f + 1 - f0) * old, tick)
        }
    });
    hz
}

fn wire_state() -> (u64, u64, u64) {
    let w = *WIRE.lock().expect("wire clock");
    w.unwrap_or_else(|| {
        set_frame_hz(frame_hz());
        WIRE.lock().expect("wire clock").expect("set")
    })
}

/// Server µs of wire frame `n`.
fn wire_time(n: u64) -> u64 {
    let (f0, t0, tick) = wire_state();
    if n >= f0 { t0 + (n - f0) * tick } else { t0.saturating_sub((f0 - n) * tick) }
}

/// µs per wire frame now.
pub fn wire_tick_us() -> u64 {
    wire_state().2
}
/// Public arena: no new players in the last minute of the match.
const JOIN_LOCK_SECS: f32 = 60.0;
const PEER_TIMEOUT: Duration = Duration::from_secs(12);
/// The arena disc's reset count (low nibble of its state word, and the touch
/// grant's generation) runs 0..=0xe and wraps to 0: native, public match
/// 2026-11-16, never sent 0xf in 260k disc states or 70 grants. At 0xf the
/// clients, which wrap at 0xe themselves, no longer matched the server.
const DISC_COUNTS: u32 = 15;
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
    /// This entrant's lobby data as everyone gets it (key, server -> client body).
    entrant_data: Vec<(u64, Vec<u8>)>,
    loadout: Option<Vec<u8>>,
    /// The player's platform id (platform, account), from their entrant state.
    xpid: Option<[u64; 2]>,
    /// Jersey number, from their loadout number notifications.
    jersey: Option<u32>,
    /// When to report a changed loadout to the backend (changes come in bursts).
    loadout_save_at: Option<Instant>,
    /// The player changed their loadout here: it is saved once more as they leave.
    loadout_changed: bool,
    /// When the player last changed team (the team orbs in the lobby's combat
    /// area): the client sends a new loadout number for it.
    team_changed_at: Option<Instant>,
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
    /// Ages (µs) of the server frames this client echoed, when the echo came
    /// back: what native's ping estimate is built from.
    echo_ages: VecDeque<u32>,
    echo_logged: Instant,
    disc_seen: u32,
    disc_max_depth: i32,
    /// (type, reliable, len, object id) -> count, logged and cleared every 2 s.
    seen_types: HashMap<(u64, bool, usize, u32), u32>,
    /// Running match totals (native sends totals, never deltas).
    stats: Stats,
    /// Combat match totals.
    cstats: CombatStats,
    /// A hand on the payload (the client's touch state).
    on_payload: bool,
    /// When this peer last sent a stun (one credit per stun burst).
    last_stun: Option<Instant>,
    /// Lobby: the cease fire last sent to this player (true = lifted).
    fight_sent: Option<bool>,
    /// Lobby: every entrant's cease fire var as last sent to this player.
    holds_sent: Option<Vec<(u64, u64)>>,
    /// This player's latest `SR15NetPlayerEquipmentUpdate` (guns, tac mod,
    /// ordnance, weapon arm), for players who join later.
    equipment: Option<Vec<u8>>,
    /// When the player joined: other players' bodies wait for its spawn.
    joined_at: Option<Instant>,
    /// Replicated-var acks logged so far (join trace).
    acks_logged: u32,
}

const SPAWN_WAIT_MAX: Duration = Duration::from_secs(45);

/// The other players' body traffic, held back until a newcomer spawns.
fn is_body_state(ty: u64) -> bool {
    ty == PHYSICS || ty == PLAYER_STATE || ty == super::combat::TOUCH_STATE
}

impl Peer {
    /// Native (netrec-lobby-native-20261118, a late joiner): no other
    /// player's equipment, physics, state or touch reaches a newcomer before
    /// its own first equipment update (its spawn); they start 50-150 ms
    /// after. Ours sent them from the join, before its own body existed, and
    /// a late joiner's client then let it stun and block anywhere in the
    /// lobby and during arena goals. A client that never sends one (a
    /// spectator, if any) gets them after `SPAWN_WAIT_MAX`.
    fn spawned(&self) -> bool {
        self.equipment.is_some() || self.joined_at.is_some_and(|t| t.elapsed() > SPAWN_WAIT_MAX)
    }

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
            entrant_data: Vec::new(),
            loadout: None,
            xpid: None,
            jersey: None,
            loadout_save_at: None,
            loadout_changed: false,
            team_changed_at: None,
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
            cstats: CombatStats::default(),
            on_payload: false,
            last_stun: None,
            fight_sent: None,
            holds_sent: None,
            equipment: None,
            joined_at: None,
            acks_logged: 0,
            ping_dirty: false,
            echo_ages: VecDeque::with_capacity(ECHO_SAMPLES),
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

    /// The estimate native publishes for a player (`SR15NetUserPingEstMsg`):
    /// how old the server frame a client echoes is when the echo arrives,
    /// averaged (native keeps 450 samples; `CR15NetGame::ClientPingResponseCB`
    /// stores `server time - echoed time`). Two frames on a LAN.
    fn echo_est_us(&self) -> Option<f32> {
        if self.echo_ages.is_empty() {
            return None;
        }
        Some(self.echo_ages.iter().map(|a| f64::from(*a)).sum::<f64>() as f32 / self.echo_ages.len() as f32)
    }
}

struct PendingKeys {
    crypto: SessionCrypto,
    created: Instant,
}

/// The full id a touch reply names, from the low half a request carries
/// (native's grants: `d09afd15b1c75c04` for the lobby, `6daa00a6d33d44b7`).
fn touch_object(low: u32, level: u64) -> u64 {
    const KNOWN: [u64; 3] = [0x6daa_00a6_d33d_44b7, 0xcb99_77f7_fc2b_4526, msg::LEVEL_SOCIAL];
    if level as u32 == low {
        return level;
    }
    KNOWN.into_iter().find(|k| *k as u32 == low).unwrap_or(level)
}

#[derive(Clone, Copy, Default)]
struct TouchEntry {
    owner: u32,
    stamp: u64,
    generation: u8,
    /// The owner has it in hand (an exclusive grab), not merely thrown.
    held: bool,
}

// The server clock every session publishes (`wire_clock`), built like
// native's from machine uptime: small numbers (native's are ~4e6 frames)
// that never go backwards when this process restarts, which matters because
// the client drops any frame not above the last one it saw. The frame word
// ticks at the frame rate (at least 120 Hz) and the client's own match timers
// (the tube countdown, the doors) run on this clock.
/// Echo ages kept per player for the ping estimate (native keeps 450).
const ECHO_SAMPLES: usize = 450;

/// Public arena: a lead this large ends the match (the mercy rule); the
/// host can change it (mp_arena_rules).
fn public_mercy_margin() -> u32 {
    super::msg::arena_public().mercy_margin
}

/// `sudden_death_active` (the arena mode graph's overtime var; low byte 0/1).
/// Native's arena baseline has it at 0. `VAR_OVERTIME` is combat's.
const VAR_SUDDEN_DEATH: u64 = 0xee2e_44f7_38a3_0c35;
/// The arena graph's sudden death states (CSymbol64 of the state names, as
/// every other `game_state`): `pre_sudden_death_enter/_update`,
/// `sudden_death_new_round_enter/_update`, `post_sudden_death_enter/_update`.
const ST_PRE_SUDDEN_DEATH: [u64; 2] = [0xe1c6_8408_2c60_875c, 0xfc23_4970_54cb_cbcf];
const ST_SD_NEW_ROUND: [u64; 2] = [0xb0e1_b685_58db_a498, 0x9393_13b4_1be2_7085];
const ST_POST_SUDDEN_DEATH: [u64; 2] = [0x9d35_6995_6c5e_79a0, 0xf09c_446f_72b8_6057];
/// Match events native sends at a tied time up (names unknown;
/// captures/netrec-pub-ot-20261119): two with round_over_enter, one with
/// pre_sudden_death_enter, one after the draw call.
const EV_SD_ROUND_OVER: [u64; 2] = [0x086e_4889_749a_9aa4, 0x46dd_ece9_1562_f5d4];
const EV_SD_ENTER: u64 = 0xe283_ccf7_d47e_142b;
const EV_SD_CALLED: u64 = 0x0ae9_dc18_2d5a_c591;
/// Native sits in sudden_death_new_round this long before the team-room walk.
const SD_NEW_ROUND_SECS: f32 = 5.0;

/// How far the server's disc timestamp runs ahead of session time (the
/// clients' clocks start within a few seconds of it).
const DISC_STAMP_LEAD_US: u64 = 60_000_000;

/// In play, a holder that has sent no disc state for this long has lost the
/// disc: the server takes it and holds it where it last was. Disc states are
/// unreliable and never resent, and nobody else simulates the holder's disc,
/// so without this every client flew its own copy until the next grab (up
/// to the 12 s peer timeout). Native holders never paused over 0.2 s
/// (two-player capture).
const DISC_SILENCE: Duration = Duration::from_millis(1000);

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

/// Current wire frame word and its server time.
fn wire_clock() -> (u64, u64) {
    let (f0, t0, tick) = wire_state();
    let frame = f0 + clock_us().saturating_sub(t0) / tick;
    (frame, t0 + (frame - f0) * tick)
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
    /// Play begins: the disc state word goes live (a player may take it).
    Live,
    ArmGoals,
    /// Per-frame disc reset + light/FX during the launch count (None: off).
    LaunchFx(Option<(u64, u64)>),
    /// Set the low byte of a baseline flag var.
    Flag(u64, u8),
    /// Override a stream-0 var relative to its baseline value.
    Var(u64, Val),
    /// Private restart: every var back to the baseline; the rules too
    /// unless the flag is set (Rematch keeps them).
    Reset(bool),
    /// Round over (mercy, or an overtime goal), after the goal celebration.
    EndRound,
    /// The next round of a private match begins: scores and clock reset.
    NewRound,
}

// Public timings: the mp_arena_rules clients get (the host can change them).
fn pre_match_secs() -> f32 {
    super::msg::arena_public().pre_match_secs
}
fn launch_secs() -> f32 {
    super::msg::arena_public().launch_secs
}
/// The count native runs between round_start and pre_launch.
const HOLD_SECS: f32 = 4.5;
fn round_secs() -> f32 {
    super::msg::arena_public().round_secs
}
/// Private: the round_over count before the next round's walk.
const ROUND_OVER_SECS: f32 = 10.0;

struct Arena {
    phase: Phase,
    state: u64,
    blue: u32,
    red: u32,
    clock: f32,
    countdown: f32,
    countdown_running: bool,
    disc_owner: u32,
    /// The server owns the disc and publishes it. Native stops as soon as a
    /// player holds it and takes it back on a goal.
    server_disc: bool,
    /// After the launch: the disc state word's high nibble reads 2.
    disc_live: bool,
    /// Last trusted disc position, for the goal-plane crossing.
    last_disc: Option<(i32, i32, i32)>,
    /// Where the disc stopped in the goal; the server holds it there until
    /// the relaunch park.
    frozen: Option<(i32, i32, i32)>,
    /// Where the server holds the disc in play after its holder went silent
    /// or left (else its spawn). Unlike `frozen`, a player may take it.
    parked: Option<(i32, i32, i32)>,
    /// When the holder's last disc state (or its grant) came in.
    disc_heard: Instant,
    /// The disc's touch entry, from the last grab of it: (object, entry,
    /// actor bytes). What a grant the server makes on its own must name.
    disc_touch: Option<(u32, u32, [u8; 8])>,
    /// The server handed an abandoned disc to another player's client and
    /// has not heard a disc state from it yet.
    delegated: bool,
    /// Match over: send the top awards on the next tick.
    awards_due: bool,
    /// Low nibble of the disc state word. Clients count it up themselves
    /// (grabs, throws) and snap to the server's disc only when the server's
    /// count is ahead of theirs, so native publishes the holder's count + 1.
    disc_resets: u32,
    /// The disc holder's own reset count, from its last transform.
    client_resets: Option<u32>,
    /// The last player the server granted the disc to (handle).
    granted: u32,
    /// The holder's last forwarded disc state (64 bytes, still moving).
    holder_state: Option<Vec<u8>>,
    /// Frames left in which the server publishes `holder_state` (owner
    /// none, new count) instead of the pinned disc, right after a goal.
    goal_flight: u32,
    goals_armed: bool,
    schedule: Vec<(u64, Act)>,
    min_per_team: usize,
    disc_us: u64,
    launch_fx: Option<(u64, u64)>,
    /// Flag vars changed from the baseline: (hash, low byte).
    flags: Vec<(u64, u8)>,
    /// Possession, throws and shots, for the player stats.
    tracker: DiscTracker,
    /// Game clock when this round's play began (the score message reports
    /// seconds of play since then, as native does).
    round_clock: f32,
    /// Private match (echo_arena_private): terminal rules, Start, pause.
    private: bool,
    /// Rules clock: the schedule runs on it; it stops while paused.
    tick: u64,
    /// Stream-0 / stream-1 vars changed from the baseline.
    vars: Vec<(u64, Val)>,
    vars1: Vec<(u64, Val)>,
    /// Send stream-1 deltas up to this server frame (after a rules change).
    s1_until: u64,
    rules: Rules,
    total_rounds: u32,
    blue_rounds: u32,
    red_rounds: u32,
    paused: bool,
    /// Seconds paused (counts up), and the post-unpause count (14.5 -> 0).
    pause_time: f32,
    resume: f32,
    /// Flags as they were when the pause began, restored at unpause.
    pre_pause_flags: Vec<(u64, u8)>,
    /// Acts for the next frame, run even while paused.
    soon: Vec<Act>,
    /// Team requests from the terminal: [blue, orange].
    ready: [bool; 2],
    restart_req: [bool; 2],
    /// The round is under way (from the first launch to its end): the clock
    /// may run.
    round_live: bool,
    /// Tied at time up with overtime on: the next goal ends the round.
    overtime: bool,
    /// Overtime rounds so far (the announcer's "overtime round N").
    ot_rounds: u32,
    /// Time ran out with the disc out of the goals: it stays in play (thrown,
    /// held, grabbed) until the next round's walk or the end of the match.
    disc_loose: bool,
    rounds_played: u32,
    /// The disc's last move touched each goal box ([blue goal, orange
    /// goal]); a goal is the move that starts touching it.
    goal_touch: [bool; 2],
}

impl Arena {
    fn new(min_per_team: usize, private: bool) -> Self {
        let rules = Rules::default();
        let clock = if private { rules.round_secs() } else { round_secs() };
        Self {
            phase: Phase::Waiting,
            state: ST_PRE_MATCH,
            blue: 0,
            red: 0,
            clock,
            countdown: if private { 0.0 } else { pre_match_secs() },
            countdown_running: false,
            disc_owner: u32::MAX,
            // Private: the disc is free to play with until the match starts.
            server_disc: !private,
            disc_live: false,
            last_disc: None,
            frozen: None,
            parked: None,
            disc_heard: Instant::now(),
            disc_touch: None,
            delegated: false,
            awards_due: false,
            disc_resets: 0,
            granted: u32::MAX,
            holder_state: None,
            goal_flight: 0,
            client_resets: None,
            goals_armed: false,
            schedule: Vec::new(),
            min_per_team,
            disc_us: 0,
            launch_fx: None,
            flags: Vec::new(),
            tracker: DiscTracker::new(),
            round_clock: clock,
            private,
            tick: 0,
            vars: Vec::new(),
            vars1: Vec::new(),
            s1_until: 0,
            total_rounds: if private { rules.v[8] as u32 } else { 1 },
            rules,
            blue_rounds: 0,
            red_rounds: 0,
            paused: false,
            pause_time: 0.0,
            resume: 0.0,
            pre_pause_flags: Vec::new(),
            soon: Vec::new(),
            ready: [false; 2],
            restart_req: [false; 2],
            round_live: false,
            overtime: false,
            ot_rounds: 0,
            disc_loose: false,
            rounds_played: 0,
            goal_touch: [false; 2],
        }
    }

    fn at(&mut self, frame: u64, act: Act) {
        self.schedule.push((frame, act));
    }

    /// The stock announcer's call for a save, interception or steal, as
    /// `mp_announcer_server_state` answers the engine's event (see
    /// `announcer`): a save at once; an interception 0.75 s later from the
    /// interceptor's team's table; a steal's `steal_sound` at once and its
    /// call 0.75 s later.
    fn announce_stat(&mut self, stat: Stat, team: u8) {
        use super::announcer as an;
        let say = |s| Act::Event(s, MOD_ANNOUNCER, FILTER_ALL_ALT);
        let later = self.tick + (an::CALL_DELAY_S * frame_hz() as f32).round() as u64;
        match stat {
            Stat::Saves => self.soon.push(say(an::pick(&an::SAVE))),
            Stat::Interceptions => {
                let table: &[u64] = match team {
                    TEAM_BLUE => &an::INTERCEPTION_BLUE,
                    TEAM_ORANGE => &an::INTERCEPTION_ORANGE,
                    _ => return,
                };
                self.at(later, say(an::pick(table)));
            }
            Stat::Steals => {
                self.soon.push(say(an::STEAL_SOUND));
                self.at(later, say(an::pick(&an::STEAL)));
            }
            _ => {}
        }
    }

    /// Players may take the disc: in play, and in a private match before
    /// Start (the disc is free to play with until the match starts).
    fn disc_free(&self) -> bool {
        self.phase == Phase::Playing || (self.private && self.phase == Phase::Waiting) || self.disc_loose
    }

    /// A round or match ended: after a goal the disc stays pinned in the
    /// goal; at time up it stays free (user, 2026-11-19: native lets it fly
    /// on, and players can still grab and throw it).
    fn end_disc(&mut self) {
        if self.frozen.is_some() {
            self.pin_disc();
        } else {
            self.disc_loose = true;
        }
    }

    /// Paused, or counting back in after an unpause.
    fn time_frozen(&self) -> bool {
        self.paused || self.resume > 0.0
    }

    fn flag(&self, h: u64) -> Option<u8> {
        self.flags.iter().find(|(k, _)| *k == h).map(|e| e.1)
    }

    fn set_var(&mut self, h: u64, v: Val) {
        match self.vars.iter_mut().find(|(k, _)| *k == h) {
            Some(e) => e.1 = v,
            None => self.vars.push((h, v)),
        }
    }

    fn set_var1(&mut self, h: u64, v: Val) {
        match self.vars1.iter_mut().find(|(k, _)| *k == h) {
            Some(e) => e.1 = v,
            None => self.vars1.push((h, v)),
        }
    }

    /// Private Start: native's sequence after the terminal's Start button.
    /// What follows the start count reaching 0 is unrecorded; it continues
    /// the way a public round does (round_start walk, then the launch).
    fn private_start(&mut self, f: u64) {
        let ev = |s, m| Act::Event(s, m, FILTER_ALL);
        self.phase = Phase::Countdown;
        // Round spawn from the rules (a rematch left it at the centre).
        self.set_flag(VAR_DISC_SPAWN, rules::disc_spawn(self.rules.v[rules::ROW_DISC_SPAWN]));
        self.take_disc();
        self.at(f, ev(EV_ROUND_RESTART, MOD_MATCH));
        self.at(f, ev(EV_REALLY_RESTART, u64::MAX));
        let l = f + 4 * frame_hz();
        self.at(l, ev(EV_START_LAUNCH, MOD_MATCH));
        self.at(l, Act::Var(VAR_START_LAUNCHED, Val::Lo8(1)));
        self.at(l, ev(EV_DISABLE_LOCAL_DISC, MOD_MATCH));
        self.at(l, ev(EV_ROUND_START, MOD_MATCH));
        self.at(l + 1, Act::State(ST_GOAL[4]));
        self.at(l + 1, Act::Flag(VAR_ROUND_BEGUN, 0));
        self.at(l + 1, Act::Flag(VAR_LAUNCH_GATES, 0));
        self.at(l + 2, Act::State(ST_GOAL[5]));
        let secs = self.rules.start_count();
        self.at(l + 2, Act::Countdown(secs));
        let e = l + 2 + (secs * frame_hz() as f32) as u64;
        // Start cleared round_begun (and shut the gates); public sets the
        // doors and round_begun at this point, and the tubes stay shut
        // without them.
        self.at(e, Act::Phase(Phase::Launch));
        self.at(e, Act::State(ST_GOAL[6]));
        self.at(e, Act::Flag(VAR_TUNNEL_DOORS, 1));
        self.at(e + 1, Act::State(ST_GOAL[7]));
        self.at(e + 1, Act::Flag(VAR_ROUND_BEGUN, 1));
        self.at(e + 2, Act::State(ST_PRE_LAUNCH_ENTER));
        self.at(e + 2, ev(EV_RESTART_DISC, MOD_MATCH));
        self.schedule_launch(e + 3, (EV_LIGHT_WHITE, EV_FX_WHITE));
    }

    /// Private restart button, as native sequenced it.
    fn private_restart(&mut self, f: u64, keep_rules: bool) {
        let ev = |s| Act::Event(s, MOD_MATCH, FILTER_ALL);
        self.schedule.clear();
        self.goals_armed = false;
        for i in 0..2 {
            if i == 1 {
                self.at(f + i, ev(EV_ENABLE_SCORING));
            }
            for s in [EV_RESTART_A, EV_ROUND_RESTART, EV_RESTART_B, EV_RESTART_C] {
                self.at(f + i, ev(s));
            }
        }
        self.at(f + 2, Act::Reset(keep_rules));
        self.at(f + 2, Act::State(ST_RESTART_A));
        self.at(f + 2, ev(EV_ENABLE_SCORING));
        self.at(f + 2, Act::Event(EV_REALLY_RESTART, u64::MAX, FILTER_ALL));
        for s in [EV_RESTART_CELEBRATION, EV_RESTART_GOALS, EV_MATCH_NOT_READY] {
            self.at(f + 3, ev(s));
        }
        self.at(f + 3, Act::Event(EV_REALLY_RESTART, u64::MAX, FILTER_ALL));
        self.at(f + 4, Act::State(ST_RESTART_B));
        self.at(f + 5, Act::State(ST_PRE_MATCH));
        // The disc back to the middle of the arena, free to grab.
        self.at(f + 5, Act::Flag(VAR_DISC_SPAWN, 2));
        self.at(f + 6, ev(EV_RESTART_DISC));
        self.at(f + 6, Act::Event(EV_LIGHT_WHITE, MOD_DISC_FX, FILTER_ALL));
        self.at(f + 6, Act::Event(EV_FX_WHITE, MOD_DISC_FX, FILTER_ALL));
        self.at(f + 5 + 3 * frame_hz(), Act::Event(EV_REALLY_RESTART, u64::MAX, FILTER_ALL));
    }

    /// The server holds the disc where it is: on the goal plane after a
    /// goal, else where a player last had it (spawn if never touched).
    fn pin_disc(&mut self) {
        if self.frozen.is_none() {
            self.frozen = self.last_disc.or(self.parked);
        }
        // A player's client was simulating the disc (time up mid-play): move
        // the reset count past theirs so every client, the holder too, snaps
        // to the pinned disc. Without it the holder kept flying its own copy
        // while everyone else saw the pin, frozen in the air.
        if !self.server_disc {
            self.bump_resets();
            self.disc_owner = u32::MAX;
            self.holder_state = None;
            self.tracker.reset();
        }
        self.parked = None;
        self.server_disc = true;
        self.disc_live = false;
        self.last_disc = None;
    }

    /// The server takes the disc back to its spawn; the reset count bump
    /// makes clients snap their copy to it.
    fn take_disc(&mut self) {
        self.disc_loose = false;
        self.server_disc = true;
        self.disc_live = false;
        self.frozen = None;
        self.parked = None;
        self.last_disc = None;
        self.bump_resets();
    }

    /// The holder went silent or left in play: the server takes the disc
    /// and holds it where the holder last had it, free to grab. The reset
    /// count moves past the holder's, so every client snaps to that place.
    fn park_disc(&mut self) {
        if let Some(p) = self.last_disc.take() {
            self.parked = Some(p);
        }
        self.server_disc = true;
        self.bump_resets();
        self.disc_owner = u32::MAX;
        self.holder_state = None;
        self.tracker.reset();
    }

    /// Move the reset count past every client's, so they snap to the
    /// server's disc. Native goals: client at 2 -> server 3, client at 5 ->
    /// server 6. Without a client count, +3 as before.
    fn bump_resets(&mut self) {
        self.disc_resets = match self.client_resets.take() {
            Some(c) => c + 1,
            None => self.disc_resets + 3,
        } % DISC_COUNTS;
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
        self.at(f, Act::Countdown(pre_match_secs()));
        // When the pre-match count hits 0 (recorded native game): the
        // pre-match-done state with the round-start events, the tunnel doors
        // open, the graph walks round_start into a 4.5 s count, then
        // pre_launch runs the 15 s count in the tubes.
        let r = f + (pre_match_secs() as u64) * frame_hz();
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
        let e = r + 4 + (HOLD_SECS * frame_hz() as f32) as u64;
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
        let secs = if self.private { self.rules.catapult_secs() } else { launch_secs() };
        self.at(l, Act::Countdown(secs));
        self.at(l, Act::LaunchFx(Some(fx)));
        // Count over: scoring on, launch gates open, main loop, play.
        let p = l + (secs * frame_hz() as f32) as u64;
        self.at(p, Act::LaunchFx(None));
        self.at(p, Act::Live);
        self.at(p, ev(EV_ENABLE_SCORING, MOD_MATCH, FILTER_ALL));
        self.at(p + 1, Act::Flag(VAR_LAUNCH_GATES, 1));
        self.at(p + 1, ev(EV_NEW_ROUND_STARTED, MOD_MATCH, FILTER_ALL));
        self.at(p + 2, Act::State(ST_MAIN_LOOP_ENTER));
        self.at(p + 2, Act::Flag(VAR_NOT_PLAYING, 0));
        self.at(p + 2, Act::Phase(Phase::Playing));
        self.at(p + 3, Act::State(ST_PLAYING));
        self.at(p + 3, Act::ArmGoals);
        // Native: "Teams have cleared the launchers!" 2 s into play.
        self.at(p + 2 + 2 * frame_hz(), ev(EV_ANNOUNCER_LAUNCHERS_CLEARED, MOD_ANNOUNCER, FILTER_ALL_ALT));
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
        // Native: the first event names the scoring team (it picks the
        // celebration); ours always sent blue's, so orange got blue's.
        let goal_a = if blue_scored { EV_GOAL_A } else { EV_GOAL_A_ORANGE };
        for _ in 0..2 {
            self.at(g, ev13(goal_a, MOD_MATCH));
            self.at(g, ev13(EV_GOAL_B, MOD_MATCH));
        }
        self.at(g, ev(EV_GAME_SCORE));
        self.at(g, Act::State(ST_GOAL[0]));
        self.at(g, Act::Flag(VAR_LAUNCH_GATES, 0));
        self.at(g + 1, Act::State(ST_GOAL[1]));
        self.at(g + 1, Act::Flag(VAR_HOLD, 1));
        let celebrate = self.celebration_secs();
        self.at(g + 1, Act::Countdown(celebrate));
        // Native (mp_announcer_server_state, nevt_game_score): one line at
        // random from the points' table.
        let call = super::announcer::pick(super::announcer::goal_table(points));
        self.at(g + 1, ev13(call, MOD_ANNOUNCER));
        let c = g + 1 + (celebrate * frame_hz() as f32) as u64;
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
        let e = c + 5 + (HOLD_SECS * frame_hz() as f32) as u64;
        self.at(e - frame_hz() / 2, Act::Flag(VAR_HOLD, 0));
        self.at(e, Act::State(ST_PRE_LAUNCH_ENTER));
        self.at(e, ev(EV_RESTART_DISC));
        // The disc goes to the team that conceded; native lit it orange after
        // blue scored. The blue pair is unrecorded, so white stands in.
        let fx = if blue_scored { (EV_LIGHT_ORANGE, EV_FX_ORANGE) } else { (EV_LIGHT_WHITE, EV_FX_WHITE) };
        self.schedule_launch(e + 1, fx);
    }

    /// After a goal: the score celebration before the team-room walk. In a
    /// private match it is the respawn time rule.
    fn celebration_secs(&self) -> f32 {
        if self.private { self.rules.respawn_secs().max(1.0) } else { launch_secs() }
    }

    /// Frame a goal scheduled at `g` hands over to the team-room walk.
    fn celebration_end(&self, g: u64) -> u64 {
        g + 1 + (self.celebration_secs() * frame_hz() as f32) as u64
    }

    /// Time up, tied, with overtime on: sudden death, as native sequenced
    /// it in a public match (captures/netrec-pub-ot-20261119, t = time up):
    ///   0      round_over_enter, not playing, disc spawn centre, two match events
    ///   +1 fr  pre_sudden_death_enter/_update, sudden_death_active 1, hold,
    ///          "draw" (MP_ANNOUNCER_0100?_DRAW), clock to 2:00 (stopped)
    ///   +2 fr  sudden_death_new_round_enter/_update, 5 s
    ///   +5 s   round restart + round start, gates shut, transition_to_launch
    ///          (team-room cue, again 3 s later), 4.5 s count, hold off at 4 s
    ///   +9.5 s pre_launch, "overtime round N" (MP_ANNOUNCER_OT_ROUND_N)
    ///   +10.5 s disc restart, round_start, the usual 15 s launch, play.
    /// The next goal wins (`end_round` then goes through post_sudden_death).
    /// A scoreless overtime round (public: the 2:00 clock) goes again with
    /// the next "overtime round" call. Private has no clock: it stays at 0:00.
    fn start_sudden_death(&mut self, f: u64) {
        let rules = super::msg::arena_public();
        let ev = |s| Act::Event(s, MOD_MATCH, FILTER_ALL);
        let say = |s| Act::Event(s, MOD_ANNOUNCER, FILTER_ALL_ALT);
        self.schedule.clear();
        self.soon.clear();
        self.overtime = true;
        self.ot_rounds += 1;
        self.round_live = true;
        self.goals_armed = false;
        self.phase = Phase::Goal;
        // Whoever holds the disc drops it; the server brings it to centre.
        self.take_disc();
        self.disc_owner = u32::MAX;
        self.holder_state = None;
        self.tracker.reset();
        self.clock = if self.private { 0.0 } else { rules.sd_round_secs };
        if !self.private {
            self.round_clock = self.clock;
        }
        self.at(f, Act::State(ST_MATCH_END[0]));
        self.at(f, Act::Flag(VAR_NOT_PLAYING, 1));
        self.at(f, Act::Flag(VAR_DISC_SPAWN, 2));
        self.at(f, Act::Flag(VAR_LAUNCH_GATES, 0));
        for e in EV_SD_ROUND_OVER {
            self.at(f, ev(e));
        }
        self.at(f + 1, Act::State(ST_PRE_SUDDEN_DEATH[0]));
        self.at(f + 1, Act::Flag(VAR_SUDDEN_DEATH, 1));
        self.at(f + 1, Act::Flag(VAR_HOLD, 1));
        self.at(f + 1, ev(EV_SD_ENTER));
        self.at(f + 1, Act::State(ST_PRE_SUDDEN_DEATH[1]));
        let draw = ["A", "B", "C", "D", "E", "F"][rand::random::<usize>() % 6];
        self.at(f + 1, say(crate::sns::symbol(&format!("MP_ANNOUNCER_0100{draw}_DRAW"))));
        self.at(f + 1, ev(EV_SD_CALLED));
        self.at(f + 2, Act::State(ST_SD_NEW_ROUND[0]));
        self.at(f + 3, Act::State(ST_SD_NEW_ROUND[1]));
        self.at(f + 3, Act::Countdown(SD_NEW_ROUND_SECS));
        // Then everyone goes back to the launchers, as after a goal.
        let c = f + 3 + (SD_NEW_ROUND_SECS * frame_hz() as f32) as u64;
        self.at(c, ev(EV_ROUND_RESTART));
        self.at(c, ev(EV_ROUND_START));
        self.at(c + 1, Act::Flag(VAR_LAUNCH_GATES, 0));
        self.at(c + 2, Act::State(ST_GOAL[6])); // transition_to_launch_enter
        self.at(c + 2, Act::Event(EV_REALLY_RESTART, u64::MAX, FILTER_ALL));
        self.at(c + 2, Act::State(ST_GOAL[7])); // transition_to_launch_update
        self.at(c + 2, Act::Countdown(HOLD_SECS));
        self.at(c + 2 + 3 * frame_hz(), Act::Event(EV_REALLY_RESTART, u64::MAX, FILTER_ALL));
        let e = c + 2 + (HOLD_SECS * frame_hz() as f32) as u64;
        self.at(e - frame_hz() / 2, Act::Flag(VAR_HOLD, 0));
        self.at(e, Act::State(ST_PRE_LAUNCH_ENTER));
        let n = self.ot_rounds.clamp(1, 15);
        self.at(e, say(crate::sns::symbol(&format!("MP_ANNOUNCER_OT_ROUND_{n}"))));
        let l = e + frame_hz();
        self.at(l, ev(EV_RESTART_DISC));
        self.schedule_launch(l, (EV_LIGHT_WHITE, EV_FX_WHITE));
    }

    /// After the overtime goal's celebration: native's goal tail
    /// (states 2-3, restart B), then post_sudden_death with
    /// sudden_death_active off, then whatever ends the round. Returns the
    /// frame that comes next.
    fn leave_sudden_death(&mut self, f: u64) -> u64 {
        self.at(f, Act::State(ST_GOAL[2]));
        self.at(f, Act::State(ST_GOAL[3]));
        self.at(f, Act::Event(EV_RESTART_B, MOD_MATCH, FILTER_ALL));
        self.at(f + 1, Act::State(ST_POST_SUDDEN_DEATH[0]));
        self.at(f + 2, Act::State(ST_POST_SUDDEN_DEATH[1]));
        self.at(f + 2, Act::Flag(VAR_SUDDEN_DEATH, 0));
        f + 3
    }

    /// A private round ended (time up, mercy or an overtime goal): credit the
    /// winner, then the next round, or the match end once it is decided.
    fn end_round(&mut self, f: u64, mercy: bool) {
        self.round_live = false;
        let was_overtime = std::mem::replace(&mut self.overtime, false);
        self.ot_rounds = 0;
        if !was_overtime {
            self.set_flag(VAR_SUDDEN_DEATH, 0);
        }
        match self.blue.cmp(&self.red) {
            std::cmp::Ordering::Greater => self.blue_rounds += 1,
            std::cmp::Ordering::Less => self.red_rounds += 1,
            std::cmp::Ordering::Equal => {}
        }
        let need = self.total_rounds / 2 + 1;
        let majority = self.rules.best_of() && (self.blue_rounds >= need || self.red_rounds >= need);
        let decided = majority || self.rounds_played + 1 >= self.total_rounds;
        self.set_flag(VAR_MERCY_WIN, u8::from(mercy));
        // Out of overtime: native's post_sudden_death comes first.
        let start = if was_overtime { f + 3 } else { f };
        if decided {
            // The round counter (the HUD shows it + 1) stays on the last
            // round: "round 2 of 2", not "3 of 2".
            self.end_match(start);
        } else {
            self.rounds_played += 1;
            self.next_round(start);
        }
        if was_overtime {
            self.leave_sudden_death(f);
        }
    }

    /// Between rounds: round_over, then the same team-room walk a goal uses,
    /// with the disc on the spawn the rules give, and a fresh launch.
    fn next_round(&mut self, f: u64) {
        let ev = |s| Act::Event(s, MOD_MATCH, FILTER_ALL);
        self.schedule.clear();
        self.goals_armed = false;
        self.phase = Phase::Goal;
        // The disc stays where the round ended (in the goal after a mercy
        // or overtime goal; free at time up) until the next round's walk
        // takes it to spawn.
        self.end_disc();
        self.at(f, Act::State(ST_MATCH_END[0]));
        self.at(f, ev(EV_ROUND_OVER));
        self.at(f, Act::Flag(VAR_LAUNCH_GATES, 0));
        self.announce_result(f, self.blue.cmp(&self.red));
        self.at(f + 1, Act::State(ST_MATCH_END[1]));
        self.at(f + 1, Act::Flag(VAR_HOLD, 1));
        self.at(f + 1, Act::Countdown(ROUND_OVER_SECS));
        let r = f + 1 + (ROUND_OVER_SECS as u64) * frame_hz();
        // Back to the team rooms with the tunnel doors shut, then the same
        // start sequence the match began with: start_launch, round_begun
        // cleared, the ROUND WAIT TIME count, doors open, pre_launch.
        self.at(r, Act::NewRound);
        self.at(r, Act::Flag(VAR_TUNNEL_DOORS, 0));
        self.at(r, Act::Var(VAR_START_LAUNCHED, Val::Lo8(0)));
        self.at(r, Act::Flag(VAR_DISC_SPAWN, rules::disc_spawn(self.rules.v[rules::ROW_DISC_SPAWN])));
        for s in [EV_ROUND_RESTART, EV_RESTART_GOALS] {
            self.at(r, ev(s));
        }
        self.at(r, Act::Event(EV_REALLY_RESTART, u64::MAX, FILTER_ALL));
        self.at(r + 1, Act::Flag(VAR_HOLD, 0));
        let l = r + 1;
        self.at(l, ev(EV_START_LAUNCH));
        self.at(l, Act::Var(VAR_START_LAUNCHED, Val::Lo8(1)));
        self.at(l, ev(EV_DISABLE_LOCAL_DISC));
        self.at(l, ev(EV_ROUND_START));
        self.at(l + 1, Act::State(ST_GOAL[4]));
        self.at(l + 1, Act::Flag(VAR_ROUND_BEGUN, 0));
        self.at(l + 1, Act::Flag(VAR_LAUNCH_GATES, 0));
        self.at(l + 2, Act::State(ST_GOAL[5]));
        // ROUND WAIT TIME: the count in the team rooms before the launch.
        let secs = self.rules.start_count();
        self.at(l + 2, Act::Countdown(secs));
        let e = l + 2 + (secs * frame_hz() as f32) as u64;
        self.at(e, Act::Phase(Phase::Launch));
        self.at(e, Act::State(ST_GOAL[6]));
        self.at(e, Act::Flag(VAR_TUNNEL_DOORS, 1));
        self.at(e + 1, Act::State(ST_GOAL[7]));
        self.at(e + 1, Act::Flag(VAR_ROUND_BEGUN, 1));
        self.at(e + 2, Act::State(ST_PRE_LAUNCH_ENTER));
        self.at(e + 2, ev(EV_RESTART_DISC));
        self.schedule_launch(e + 3, (EV_LIGHT_WHITE, EV_FX_WHITE));
    }

    /// Native, after round_over_enter: "That wraps up the round..." at
    /// 2.5 s, then the winner (or a draw) at 4.5 s.
    fn announce_result(&mut self, f: u64, blue_vs_orange: std::cmp::Ordering) {
        let say = |s| Act::Event(s, MOD_ANNOUNCER, FILTER_ALL_ALT);
        // Mercy ends too: "That wraps up the round..." (user-confirmed).
        self.at(f + frame_hz() * 5 / 2, say(EV_ANNOUNCER_ROUND_OVER));
        let line = match blue_vs_orange {
            std::cmp::Ordering::Greater => EV_ANNOUNCER_BLUE_WINS,
            std::cmp::Ordering::Less => crate::sns::symbol("MP_ANNOUNCER_0070A_RED_TEAM_WINS"),
            std::cmp::Ordering::Equal => crate::sns::symbol("MP_ANNOUNCER_0100A_DRAW"),
        };
        self.at(f + frame_hz() * 9 / 2, say(line));
    }

    fn end_match(&mut self, f: u64) {
        self.schedule.clear();
        self.goals_armed = false;
        self.round_live = false;
        self.phase = Phase::Over;
        self.awards_due = true;
        // The disc stays where the match ended (in the goal after the
        // deciding goal; free at time up); the spawn flag below only matters
        // for a restart.
        self.end_disc();
        let ev = |s| Act::Event(s, MOD_MATCH, FILTER_ALL);
        let hold = 15 * frame_hz();
        let steps = [0, 1, hold, hold + 3 * frame_hz(), hold + 13 * frame_hz(), hold + 13 * frame_hz() + 1, hold + 28 * frame_hz()];
        let winner = if self.blue >= self.red { VAR_BLUE_ROUND_SCORE } else { VAR_RED_ROUND_SCORE };
        let result = if self.private && self.blue_rounds != self.red_rounds {
            self.blue_rounds.cmp(&self.red_rounds)
        } else {
            self.blue.cmp(&self.red)
        };
        self.announce_result(f, result);
        self.at(f, Act::Flag(VAR_END_DOORS, 1));
        self.at(f, Act::Flag(VAR_DISC_SPAWN, 2));
        self.at(f, Act::Flag(VAR_NOT_PLAYING, 1));
        self.at(f, Act::Flag(winner, 1));
        self.at(f, Act::Flag(VAR_LAUNCH_GATES, 0));
        self.at(f + 1, Act::Flag(VAR_HOLD, 1));
        self.at(f + 1, Act::Countdown(15.0));
        self.at(f + hold, Act::Flag(VAR_CELEBRATION, 1));
        self.at(f + hold + 3 * frame_hz(), Act::Countdown(10.0));
        self.at(f + hold + 13 * frame_hz() + 1, Act::Countdown(15.0));
        for (i, (at, st)) in steps.iter().zip(ST_MATCH_END).enumerate() {
            self.at(f + at, Act::State(st));
            let e = match i {
                0 => Some(EV_MATCH_OVER),
                2 => Some(EV_TELEPORT_CELEBRATION),
                4 => Some(EV_PERSONAL_STATS),
                // Private matches stay until Rematch or Lobby is pressed.
                6 if !self.private => Some(EV_RETURN_TO_LOBBY),
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
    /// A combat match (Dyson, Combustion, Fission, Gauss): its round flow.
    combat: Option<super::combat::Combat>,
    combat_at: Option<Instant>,
    /// The combat vars last sent (only changes go out, plus all twice a second).
    combat_sent: HashMap<u64, u64>,
    /// Seconds since the last combat stat update.
    combat_stat_clock: f32,
    /// Who hurt whom lately (assists): victim -> (attacker, when).
    recent_hits: HashMap<SocketAddr, Vec<(SocketAddr, Instant)>>,
    /// Component handles from the IdsResponse this session sends.
    handles: Handles,
    peers: HashMap<SocketAddr, Peer>,
    pending: Vec<PendingKeys>,
    slot_gen: [u16; MAX_SLOTS],
    slot_addr: [Option<SocketAddr>; MAX_SLOTS],
    frame: u64,
    /// Last wire frame word published (120 Hz ticks of the server clock).
    wire_frame: u64,
    /// Rules frames on real time: the rate they were counted at, when that
    /// count started, the frames due at that start, and the frames run.
    /// A server loop that falls behind (Windows timers) runs the missing
    /// rules frames next time instead of slowing the match.
    rules_hz: u64,
    rules_t0: Option<Instant>,
    rules_base: u64,
    rules_done: u64,
    /// The matchmaker was told this public match takes no more players.
    join_locked: bool,
    started: Instant,
    arena: Arena,
    /// Touch (ownership) state per (object, entry): entry ids are numbered
    /// per object (the lobby's level props `b1c75c04` and `d33d44b7` both
    /// have an entry 0, one of them the disc).
    touch: HashMap<(u64, u32), TouchEntry>,
    /// Combat gear in the social lobby's combat area.
    gear: super::gear::Gear,
    /// The social lobby's loose objects (discs, toys).
    props: super::props::Props,
    /// `arena.server_disc` last frame: when the server takes the disc back
    /// (goal, match start, new round, restart) nobody owns it any more.
    server_disc_was: bool,
    /// Last touch request seq per (slot, object): each client counts its
    /// requests per object.
    touch_seq: HashMap<(usize, u64), u16>,
    parked_loadouts: Vec<(SocketAddr, usize)>,
    roster_dirty: bool,
    pub fleet_out: Vec<pb::Envelope>,
    pub udp_out: Vec<(SocketAddr, Vec<u8>)>,
    pub last_activity: Instant,
    pub ever_joined: bool,
    /// Entrants the backend server-muted (atlas's /mute-player): their voice
    /// is not relayed to anyone. Kept by entrant, so a mute can arrive before
    /// the player's connection does; a rejoin gets a new entrant id, and the
    /// backend mutes that one again.
    muted: HashSet<Uuid>,
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

/// A save-loadout report for the backend, from a peer's latest loadout.
fn loadout_save(lobby: &str, p: &Peer) -> Option<pb::Envelope> {
    let slot = p.slot?;
    let Some(inst) = msg::parse_loadout(p.loadout.as_deref()?) else {
        warn!(slot, "could not read a changed loadout");
        return None;
    };
    info!(slot, entrant = %p.entrant_id, items = inst.items.len(), "loadout changed; saving to the backend");
    Some(env(Em::GameServerSaveLoadout(pb::GameServerSaveLoadoutMessage {
        lobby_session_id: lobby.to_string(),
        entrant_id: p.entrant_id.to_string(),
        loadout_slot: slot as i32,
        // -1: not known here; nakama then keeps the number it has (0 overwrote it).
        jersey_number: p.jersey.map_or(-1, |n| n as i32),
        loadout_instances: vec![inst],
        loadout_entry: None,
    })))
}

/// Native combat's stat updates: every 3 s of the match (Dyson: 103.5, 106.5, ...).
const COMBAT_STATS_EVERY: f32 = 3.0;
/// Damage this recent counts toward an assist (not recorded: native's Dyson
/// had none).
const COMBAT_ASSIST_WINDOW: Duration = Duration::from_secs(10);

/// A profile's period stat groups: `daily_YYYY_MM_DD` and `weekly_` + its
/// Monday (native on 2026-11-16, a Monday: both that date).
/// One player's arena match as a server profile update: `{"stats":{"arena":
/// {name: {"op","val","cnt"}}}}`, the shape nakama's `ArenaStatistics` reads.
fn arena_profile_delta(s: &Stats, blue: bool, result: std::cmp::Ordering) -> serde_json::Value {
    use std::cmp::Ordering::*;
    let mut st = serde_json::Map::new();
    let mut put = |k: &str, op: &str, v: serde_json::Value| {
        st.insert(k.to_string(), serde_json::json!({"op": op, "val": v, "cnt": 1}));
    };
    for (i, name) in super::stats::STAT_NAMES.iter().enumerate() {
        let n = s.v[i];
        if n == 0 {
            continue;
        }
        if i == Stat::PossessionTime as usize {
            put(name, "add", serde_json::json!(n as f64 / frame_hz() as f64));
        } else {
            put(name, "add", serde_json::json!(n));
        }
    }
    for (name, stat) in [("HighestPoints", Stat::Points), ("HighestSaves", Stat::Saves), ("HighestStuns", Stat::Stuns)] {
        if s.get(stat) > 0 {
            put(name, "max", serde_json::json!(s.get(stat)));
        }
    }
    let outcome = match (result, blue) {
        (Equal, _) => "ArenaTies",
        (Greater, true) | (Less, false) => "ArenaWins",
        _ => "ArenaLosses",
    };
    put(outcome, "add", serde_json::json!(1));
    put("GamesPlayed", "add", serde_json::json!(1));
    serde_json::json!({ "stats": { "arena": st } })
}

fn profile_periods(unix: u64) -> (String, String) {
    let days = (unix / 86_400) as i64;
    let date = |d: i64| {
        // The civil date of a day count since 1970-01-01.
        let z = d + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let dd = doy - (153 * mp + 2) / 5 + 1;
        let mm = if mp < 10 { mp + 3 } else { mp - 9 };
        let yy = yoe + era * 400 + i64::from(mm <= 2);
        format!("{yy:04}_{mm:02}_{dd:02}")
    };
    // 1970-01-01 was a Thursday: that week's Monday is 3 days before.
    let monday = days - (days + 3).rem_euclid(7);
    (format!("daily_{}", date(days)), format!("weekly_{}", date(monday)))
}

/// How long after the last loadout change to save it.
const LOADOUT_SAVE_DELAY: Duration = Duration::from_millis(1500);
/// A loadout number this soon after a team change belongs to the change.
const TEAM_LOADOUT_WINDOW: Duration = Duration::from_secs(2);

fn swizzle(raw: [u8; 16]) -> Uuid {
    let mut b = raw;
    b[0..4].reverse();
    b[4..6].reverse();
    b[6..8].reverse();
    Uuid::from_bytes(b)
}

fn hex_str(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Native originates one `SR15NetPooledActorDestroy` for every pooled actor
/// a client creates, its body bytes 40..56 of the create (nevr-server-rs's
/// measurement of a native 4v4, issue #490). Without it pooled actors are
/// never returned to their pool.
const POOLED_DESTROY_WINDOW: std::ops::Range<usize> = 40..56;

/// The combat flow for a combat map, with the settings its clients get.
fn combat_for(level: u64) -> Option<super::combat::Combat> {
    if !super::combat::LEVELS.contains(&level) {
        return None;
    }
    let block = captured().balance.iter().find(|(l, _)| *l == level).map(|(_, b)| b.as_slice())?;
    let name = match level {
        super::combat::LEVEL_DYSON => "mpl_combat_dyson_map_settings",
        super::combat::LEVEL_COMBUSTION => "mpl_combat_combustion_map_settings",
        super::combat::LEVEL_FISSION => "mpl_combat_fission_map_settings",
        _ => "mpl_combat_gauss_map_settings",
    };
    let settings = super::balance::document(block, name).unwrap_or_default();
    let rules = super::balance::document(block, "mp_combat_rules").unwrap_or_default();
    let components = ids_response(level, level, 0).map_or(1, |b| b.len().saturating_sub(8) / 10);
    super::combat::Combat::new(level, components, settings, rules)
}

fn u16_at(p: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([p[o], p[o + 1]])
}

impl Session {
    pub fn new(info: SessionInfo, min_per_team: usize) -> Self {
        let social = SOCIAL_MODES.contains(&info.mode) || info.level == LEVEL_SOCIAL;
        let arena_level = info.level == LEVEL_ARENA;
        let private = arena_level && info.mode == MODE_ARENA_PRIVATE;
        let handles = ids_response(info.level, info.level, info.mode).and_then(Handles::from_ids).unwrap_or(Handles::PUBLIC_ARENA);
        let mut arena = Arena::new(min_per_team, private);
        if private && let Some(c) = baseline(info.level, info.mode).and_then(|b| var_raw(&b[0], VAR_COUNTDOWN)) {
            arena.countdown = f32::from_bits(c as u32);
        }
        let mut combat = combat_for(info.level);
        if info.mode == MODE_COMBAT_PRIVATE
            && let Some(c) = combat.as_mut()
        {
            c.set_private();
        }
        let mut gear = super::gear::Gear::new();
        if combat.is_some() {
            gear.set_match(info.level);
        }
        Self {
            info,
            social,
            arena_level,
            combat,
            combat_at: None,
            combat_sent: HashMap::new(),
            combat_stat_clock: 0.0,
            recent_hits: HashMap::new(),
            handles,
            peers: HashMap::new(),
            pending: Vec::new(),
            slot_gen: [0; MAX_SLOTS],
            slot_addr: [None; MAX_SLOTS],
            frame: 0,
            wire_frame: 0,
            rules_hz: frame_hz(),
            rules_t0: None,
            rules_base: 0,
            rules_done: 0,
            join_locked: false,
            started: Instant::now(),
            arena,
            touch: HashMap::new(),
            gear,
            props: super::props::Props::new(),
            server_disc_was: false,
            touch_seq: HashMap::new(),
            parked_loadouts: Vec::new(),
            roster_dirty: false,
            fleet_out: Vec::new(),
            udp_out: Vec::new(),
            last_activity: Instant::now(),
            ever_joined: false,
            muted: HashSet::new(),
        }
    }

    /// Mutes or unmutes the entrants' voice; returns how many changed.
    pub fn set_muted(&mut self, entrants: &[Uuid], muted: bool) -> usize {
        entrants
            .iter()
            .filter(|e| !e.is_nil())
            .filter(|e| if muted { self.muted.insert(**e) } else { self.muted.remove(*e) })
            .count()
    }

    /// Whether the peer at this address is server-muted.
    fn is_muted(&self, from: SocketAddr) -> bool {
        !self.muted.is_empty() && self.peers.get(&from).is_some_and(|p| self.muted.contains(&p.entrant_id))
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

    fn send_gear(&mut self, from: SocketAddr, o: super::gear::Out) {
        use super::gear::To;
        match o.to {
            To::Sender => self.reliable_to(from, o.ty, &o.body),
            To::Others => self.relay_reliable(from, o.ty, &o.body, false),
            To::All => self.relay_reliable(from, o.ty, &o.body, true),
        }
    }

    fn relay_mailbox(&mut self, from: SocketAddr, ty: u64, key: u64, data: &[u8]) {
        let body_state = is_body_state(ty);
        for (a, p) in self.peers.iter_mut() {
            if *a != from && p.stage == Stage::Joined && (!body_state || p.spawned()) {
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
            if tracing::enabled!(tracing::Level::DEBUG) {
                *p.seen_types.entry((ty, reliable, d.len(), obj)).or_default() += 1;
            }
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
                    if p.acks_logged < 12 || p.acks_logged % 200 == 0 {
                        info!(%from, slot = ?p.slot, n = p.acks_logged, body = %hex_str(d), "replicated ack");
                    }
                    p.acks_logged += 1;
                    p.rep_acked = true;
                }
            }
            ENCODING_FAIL => debug!(%from, "client reported a decode failure"),
            _ if stage != Stage::Joined => {}
            IDS_REQUEST => {
                let level = d.get(..8).map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8]))).unwrap_or(0);
                match ids_response(level, self.info.level, self.info.mode) {
                    Some(body) => self.reliable_to(from, IDS_RESPONSE, body),
                    None => debug!(%from, level = format_args!("{level:#018x}"), "no ids response for level"),
                }
            }
            BALANCE_READY => match captured().balance.iter().find(|(l, _)| *l == self.info.level) {
                Some((_, body)) => self.reliable_to(from, BALANCE_UPDATE, body),
                None => warn!(level = format_args!("{:#018x}", self.info.level), "no balance settings for this level; client will wait"),
            },
            REPLICATED_READY => {
                if baseline(self.info.level, self.info.mode).is_none() {
                    warn!(level = format_args!("{:#018x}", self.info.level), "no replicated baseline for this level; client will wait");
                } else if let Some(p) = self.peers.get_mut(&from) {
                    p.rep_ready = true;
                }
            }
            ENTRANT_STATE if reliable => self.on_entrant_state(from, d.to_vec()),
            ENTRANT_DATA => self.on_entrant_data(from, d),
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
                    // A change after the first one is the player editing their cosmetics.
                    if p.loadout.as_deref().is_some_and(|old| old != d) {
                        p.loadout_save_at = Some(Instant::now() + LOADOUT_SAVE_DELAY);
                        p.loadout_changed = true;
                    }
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
            COMPONENT_EVENT => self.on_client_event(from, d),
            UI_INC_SETTING | UI_DEC_SETTING if reliable => self.on_rules_press(from, d, ty == UI_INC_SETTING),
            API_SET_RULE if reliable => self.on_api_rule(from, d),
            ASSIGN_TEAM_REQUEST if reliable => self.on_assign_team(from, d),
            // Native (`SR15StunPlayerMsg` handler, 0x140d53da0) checks only
            // that the victim (`+0` slot | gen << 16) is not a spectator, then
            // passes it on; where punching works is the clients' own call.
            // (A combat-area filter here dropped every punch in the brawler
            // room, where no gear-station message is ever sent. Punching
            // outside it came from clients left in combat mode by stations
            // that were never freed.)
            STUN if { info!(%from, reliable, body = %hex_str(d), "stun message"); false } => {}
            STUN if reliable && self.stun_victim_is_spectator(d) => {
                debug!(%from, body = %hex_str(d), "stun on a spectator (not relayed)");
            }
            // The cease fire is the clients' to honour, and a client that
            // missed it (players after the first in the lobby, everyone
            // during a goal) still sent stuns: dropped here, where native's
            // cease fire would have stopped them.
            STUN if reliable && self.cease_fire_for(from) => {
                info!(%from, body = %hex_str(d), "stun during a cease fire (not relayed)");
            }
            STUN if reliable => {
                self.on_stun(from, d);
                self.relay_reliable(from, ty, d, false);
            }
            ITEM_EQUIPPED if reliable && d.len() == 32 => {
                let q = |o: usize| u64::from_le_bytes(d[o..o + 8].try_into().expect("8"));
                if let Some(l) = self.peers.get_mut(&from).and_then(|p| p.loadout.as_mut()) {
                    msg::equip_in_loadout(l, q(16), q(24));
                }
            }
            SAVE_LOADOUT_REQUEST if reliable => {
                // Everyone else re-dresses this player; the backend gets it shortly.
                let body = self.peers.get_mut(&from).and_then(|p| {
                    p.loadout_save_at = Some(Instant::now() + LOADOUT_SAVE_DELAY);
                        p.loadout_changed = true;
                    p.loadout.clone()
                });
                if let Some(b) = body {
                    self.relay_reliable(from, LOADOUT_RESPONSE, &b, false);
                }
            }
            LOADOUT_NUMBER if reliable => {
                if let (Some(p), Some(n)) = (self.peers.get_mut(&from), d.get(8..12)) {
                    let n = u32::from_le_bytes(n.try_into().expect("4 bytes"));
                    if p.jersey.is_some_and(|old| old != n) {
                        p.loadout_save_at = Some(Instant::now() + LOADOUT_SAVE_DELAY);
                        p.loadout_changed = true;
                    }
                    p.jersey = Some(n);
                }
                self.relay_reliable(from, ty, d, false);
                // Social lobby: older clients sent it on walking out of the
                // combat area. It also follows a team change (the team orbs,
                // inside the area), and clients with the area-leave event
                // report walking out with that: freeing the stations here took
                // the player's gun as they picked a team.
                let team_change = self.peers.get(&from).and_then(|p| p.team_changed_at).is_some_and(|t| t.elapsed() < TEAM_LOADOUT_WINDOW);
                if self.social && !team_change && !self.gear.sends_area_events(from) {
                    for o in self.gear.left_area(from) {
                        self.send_gear(from, o);
                    }
                }
            }
            EQUIPMENT if reliable && d.len() == 56 => self.on_equipment(from, d),
            // Only the 160-byte creates (projectiles) get native's destroy.
            // Others are tac mod effects placed in the world (NEUTRON's 96-byte
            // field, likely UMBRAL VEIL's cloud too): native neither echoes
            // them to the sender nor destroys them. NEUTRON's bytes 40..56 are
            // the area id + its own actor, so a destroy killed the effect
            // (stock barrier flashed, the player froze).
            ACTOR_CREATE if reliable && d.len() != 160 => {
                debug!(%from, len = d.len(), "tac mod effect actor (relayed, not destroyed)");
                self.relay_reliable(from, ty, d, false);
            }
            ACTOR_CREATE if reliable => {
                // Native (public match + lobby, 2026-11-16): the create goes to
                // everyone but its sender, the destroy to everyone.
                self.relay_reliable(from, ty, d, false);
                if let Some(body) = d.get(POOLED_DESTROY_WINDOW).map(<[u8]>::to_vec) {
                    self.relay_reliable(from, ACTOR_DESTROY, &body, true);
                }
            }
            // Native relays none of the clients' own destroys: the server's
            // destroy above is the only one anyone receives.
            ACTOR_DESTROY if reliable => debug!(%from, body = %hex_str(d), "client pooled-actor destroy (not relayed)"),
            EMOTE | FORCE_PUSH if reliable => self.relay_reliable(from, ty, d, true),
            _ if reliable && super::gear::handles(ty) => {
                let outs = self.gear.on_message(from, ty, d, Instant::now());
                for o in outs {
                    self.send_gear(from, o);
                }
            }
            _ if reliable => debug!(%from, ty = format_args!("{ty:#018x}"), len = d.len(), body = %hex_str(&d[..d.len().min(96)]), "unhandled reliable message"),
            _ => self.on_mailbox(from, ty, m.mailbox, d),
        }
    }

    fn on_mailbox(&mut self, from: SocketAddr, ty: u64, key: u64, d: &[u8]) {
        if ty == VOIP && self.is_muted(from) {
            return; // server-muted: nobody hears them
        }
        if ty == PLAYER_STATE || ty == PHYSICS {
            if d.len() >= 4 {
                let (slot, generation) = (u16_at(d, 0), u16_at(d, 2));
                if let Some(p) = self.peers.get_mut(&from) {
                    if p.slot == Some(usize::from(slot)) {
                        p.generation = Some(generation);
                    }
                }
            }
            if ty == PHYSICS && self.social {
                self.gear.observe_physics(from, d);
                self.props.observe(from, d);
            }
            if ty == PHYSICS && self.combat.is_some() {
                self.gear.observe_physics(from, d);
            }
        }
        if ty == super::combat::TOUCH_STATE && self.combat.is_some() {
            // A hand on the payload claims it (native).
            if let Some((_, on)) = super::combat::payload_touch(d)
                && let Some(p) = self.peers.get_mut(&from)
            {
                p.on_payload = on;
            }
        }
        if ty == PLAYER_STATE || ty == PHYSICS {
            if ty == PHYSICS && self.arena_level {
                // Native (public match, 2026-11-16): once the disc is granted to
                // someone, the previous holder's states (it keeps sending them
                // for 150-200 ms, more on high ping, until the grant reaches it)
                // are neither used nor forwarded. Forwarded with the new reset
                // count, they told every client the stale disc was the current
                // one: a caught or headbutted disc snapped back.
                if msg::disc_transform(d).is_some() && self.arena.granted != u32::MAX {
                    let handle = self.peers.get(&from).map(|p| (u32::from(p.generation.unwrap_or(0)) << 16) | p.slot.unwrap_or(usize::MAX) as u32);
                    if handle != Some(self.arena.granted) {
                        return;
                    }
                }
                self.observe_disc(from, d);
                if let Some((owner, ..)) = msg::disc_transform(d) {
                    // Native forwards the holder's disc under the holder's own
                    // key, with the reset count (low nibble of [24]) set to the
                    // server's: one more after each grant, so the others snap
                    // to the new holder's disc. Unchanged, they ignored it
                    // until they had grabbed it themselves. Non-holders send
                    // none in native; while the server holds it, nothing.
                    let holder = self.peers.get(&from).is_some_and(|p| p.slot == Some(usize::from(owner)));
                    // A state the server did not take the disc from (a stale
                    // holder's, after the server took it back) is not
                    // forwarded: with the server's count it would pull
                    // everyone off the server's disc.
                    if holder && self.arena.frozen.is_none() && !self.arena.server_disc {
                        self.arena.disc_heard = Instant::now();
                        if std::mem::take(&mut self.arena.delegated) {
                            info!(slot = owner, "abandoned disc: the player's client took it over and simulates it");
                        }
                        let mut f = d.to_vec();
                        f[24] = (f[24] & 0xf0) | (self.arena.disc_resets & 0xf) as u8;
                        // In flight it goes out at once, in a packet of its
                        // own: waiting for the next server frame made every
                        // disc state up to a frame late (4 ms on average at
                        // 120 Hz). In a hand (state byte 2) it stays with the
                        // frame, beside the holder's own state.
                        if f[25] & 0xf == 2 {
                            self.relay_mailbox(from, ty, key, &f);
                        } else {
                            let mut pkts = Vec::new();
                            for (a, p) in self.peers.iter_mut() {
                                if *a != from && p.stage == Stage::Joined && p.spawned() {
                                    p.t.mailbox_now(ty, key, &f, &mut pkts);
                                    self.udp_out.extend(pkts.drain(..).map(|d| (*a, d)));
                                }
                            }
                        }
                        self.arena.holder_state = Some(f);
                    }
                    return;
                }
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
        // Native drops a response whose client time is not newer than the last.
        let token = u64::from_le_bytes(d[16..24].try_into().expect("8 bytes"));
        if token <= p.ping_token {
            return;
        }
        // Age in whole server frames, counted to the frame the estimate goes
        // out in (native: 16666 µs for a LAN client, never less).
        let echoed_frame = u64::from_le_bytes(d[0..8].try_into().expect("8 bytes"));
        let frames = (wire_clock().0 + 1).saturating_sub(echoed_frame);
        if (1..=240).contains(&frames) {
            if p.echo_ages.len() >= ECHO_SAMPLES {
                p.echo_ages.pop_front();
            }
            p.echo_ages.push_back((frames * wire_tick_us()) as u32);
        }
        p.ping_token = token;
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
        self.slot_addr[slot] = Some(from);
        self.touch_seq.retain(|(s, _), _| *s != slot);
        let raw: [u8; 16] = d[17..33].try_into().expect("16 bytes");
        let requested = d.get(37..41).map(|b| u32::from_le_bytes(b.try_into().unwrap_or([0xff; 4]))).filter(|t| *t <= 4).map(|t| t as u8);
        // Native gives a slot's first player generation 0, and the next one 1.
        // Kept to one digit: the entrant's cease fire var name has it as one
        // (`msg::entrant_hold_var`), and a client reads only its own.
        let genidx = self.slot_gen[slot] % 10;
        self.slot_gen[slot] = (genidx + 1) % 10;
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
        self.send_entrant_data_on_join(addr);
        if let Some(p) = self.peers.get_mut(&addr) {
            p.joined_at = Some(Instant::now());
            p.t.trace = true;
        }
        info!(%addr, slot, team, players = self.player_count(), "player joined");
    }

    /// A client's own lobby data (native): stored, and given to everyone,
    /// the sender included, under the sender's slot.
    fn on_entrant_data(&mut self, from: SocketAddr, d: &[u8]) {
        if d.len() < 18 {
            return;
        }
        let Some(p) = self.peers.get_mut(&from) else { return };
        let Some(slot) = p.slot else { return };
        let key = u64::from_le_bytes(d[8..16].try_into().expect("8"));
        let mut body = (slot as u64).to_le_bytes().to_vec();
        body.extend_from_slice(&d[8..d.len() - 2]);
        match p.entrant_data.iter_mut().find(|e| e.0 == key) {
            Some(e) => e.1 = body.clone(),
            None => p.entrant_data.push((key, body.clone())),
        }
        debug!(%from, slot, key = format_args!("{key:#018x}"), len = body.len(), "entrant data");
        self.relay_reliable(from, ENTRANT_DATA, &body, true);
    }

    /// Native's entrant data at a join: the newcomer's own header (and, with
    /// others present, its empty slot entry), then every earlier entrant's
    /// slot entry and data; the others hear the newcomer's slot entry.
    fn send_entrant_data_on_join(&mut self, addr: SocketAddr) {
        let Some(slot) = self.peers.get(&addr).and_then(|p| p.slot) else { return };
        let mut header = (slot as u64).to_le_bytes().to_vec();
        header.extend_from_slice(&ENTRANT_DATA_HEADER);
        let mut to_new = vec![header];
        let mut to_others = Vec::new();
        let earlier: Vec<(usize, Vec<Vec<u8>>)> = self
            .peers
            .iter()
            .filter(|(a, p)| **a != addr && p.stage == Stage::Joined)
            .filter_map(|(_, p)| Some((p.slot?, p.entrant_data.iter().map(|e| e.1.clone()).collect())))
            .collect();
        let slot_entry = |s: usize, with_value: bool| {
            let mut b = (s as u64).to_le_bytes().to_vec();
            b.extend_from_slice(&ENTRANT_DATA_SLOT_KEY.to_le_bytes());
            if with_value {
                b.extend_from_slice(&(s as u16).to_le_bytes());
            }
            b
        };
        if !earlier.is_empty() {
            to_new.push(slot_entry(slot, false));
            to_others.push(slot_entry(slot, true));
        }
        // Native's server sets the newcomer's `afk` itself, right after.
        let mut afk = (slot as u64).to_le_bytes().to_vec();
        afk.extend_from_slice(&ENTRANT_KEY_AFK.to_le_bytes());
        afk.extend_from_slice(&ENTRANT_AFK_VALUE);
        to_new.push(afk.clone());
        to_others.push(afk);
        for (s, data) in earlier {
            to_new.push(slot_entry(s, true));
            to_new.extend(data);
        }
        for b in &to_new {
            self.reliable_to(addr, ENTRANT_DATA, b);
        }
        for b in &to_others {
            self.relay_reliable(addr, ENTRANT_DATA, b, false);
        }
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
        if p.xpid.is_none() && d.len() >= 16 {
            let x = [u64::from_le_bytes(d[..8].try_into().expect("8")), u64::from_le_bytes(d[8..16].try_into().expect("8"))];
            p.xpid = Some(x);
            crate::upstream::note_entrant(p.entrant_id, x);
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
            // No SR15NetNewUnlocks on join. The native server sends it only for unlocks the player
            // is missing ("grant missing unlocks"); replaying the captured 227-item list marked every
            // one of them as a new unlock on each join, so clearing notifications never stuck.
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
        debug!(%from, seq, acked, sender, new_owner, believed, entry_id, "touch request");
        let exclusive = d[0x28] & 1 != 0;
        let idx = (sender & 0xffff) as usize;
        if idx >= MAX_SLOTS || (sender >> 16) == 0xffff {
            return;
        }
        // The object the entry belongs to is the request's first word (the
        // low half of its id): the lobby's props and disc (`b1c75c04`), its
        // second set (`d33d44b7`), the combat area. Sequence numbers and
        // entries are per object; the bytes at 0x18 are ff.. in every lobby
        // request, so keyed by them two objects' entry 0 were one entry, a
        // second object's requests were dropped as out of order, and every
        // grant was addressed to the level's object.
        let object = u32_at(0);
        let actor_id = u64::from(object);
        let last = self.touch_seq.get(&(idx, actor_id)).copied().unwrap_or(0);
        let want = ((u32::from(last) + 1) % 0xffff) as u16;
        if seq != want {
            debug!(%from, seq, want, actor = format_args!("{actor_id:016x}"), "touch request out of order (dropped)");
            return; // duplicate or out of order
        }
        self.touch_seq.insert((idx, actor_id), seq);
        let key = (actor_id, entry_id);
        let mut e = *self.touch.entry(key).or_insert(TouchEntry { owner: u32::MAX, stamp: 0, generation: 0, held: false });
        // Arena (native, two clients): the disc's touch generation IS the
        // disc reset count. Each grant moves it on by one, a goal by one;
        // the grant carries it and the server stamps it into the holder's
        // forwarded transforms, which is how the other clients switch to
        // the new holder's disc.
        if self.arena_level {
            e.generation = (self.arena.disc_resets & 0xf) as u8;
        }
        let release = new_owner == u32::MAX;
        let target_ok = release || ((new_owner & 0xffff) < MAX_SLOTS as u32 && (new_owner >> 16) != 0xffff);
        // Before play (and between points) the disc is the server's: native
        // keeps it parked at centre and nobody can take it.
        // A private match before Start is free play: declined there, every
        // grab went by the disc states alone, whose reset count never moved,
        // so a second player's disc was ignored by the others.
        let locked = self.arena_level && !release && !self.arena.disc_free();
        // An object whose owner has left (props in a lobby, mostly) would stay
        // owned by a ghost: nobody's view of the owner or stamp could ever
        // match again. The server takes it back for whoever asks next.
        let owner_gone = e.owner != u32::MAX && !self.handle_present(e.owner);
        let grant = target_ok
            && !locked
            && if release {
                sender == e.owner
            } else if owner_gone || (e.owner == sender && new_owner == sender) {
                // The owner's own request (a re-grab or a throw) always passes.
                true
            } else if !self.arena_level && !e.held {
                // Lobby props and discs: a client stops owning what it threw
                // once it comes to rest, without telling anyone (native's
                // server simulates it from there; this one cannot). The entry
                // then still named the thrower, and everyone else, who
                // rightly believed nobody owned it, was declined until the
                // thrower touched it again. Only something in a hand is kept.
                true
            } else if believed == u32::MAX {
                e.owner == u32::MAX && (self.arena_level || (e.generation & 0xf) != 0xf)
            } else {
                // The requester's current view of the owner decides. The
                // stamp is that client's own clock (u32 at 0x20; 0x24.. holds
                // other bits), so it can't order two clients' requests, and
                // the owner's own requests (a throw: still its, no longer
                // exclusive) must always pass, as native grants them.
                // Arena: when nobody owns the disc (the server took it
                // back, or its holder left) a requester that still believes
                // in the last holder is granted too; declined, it showed the
                // disc in its hand and then lost it again.
                believed == e.owner || (self.arena_level && e.owner == u32::MAX)
            };
        debug!(%from, grant, actor = format_args!("{actor_id:016x}"), entry_id, owner = e.owner, new_owner, believed, stamp, owner_stamp = e.stamp, exclusive, locked, "touch decision");
        let mut reply = [0u8; 0x20];
        reply[0..8].copy_from_slice(&touch_object(object, self.info.level).to_le_bytes());
        reply[0x0c..0x10].copy_from_slice(&entry_id.to_le_bytes());
        reply[0x10..0x18].copy_from_slice(actor);
        reply[0x1a..0x1c].copy_from_slice(&seq.to_le_bytes());
        if grant {
            let prev_owner = e.owner;
            let ent = self.touch.get_mut(&key).expect("inserted");
            ent.generation = if self.arena_level {
                let g = ((u32::from(e.generation) + 1) % DISC_COUNTS) as u8;
                self.arena.disc_resets = u32::from(g);
                g
            } else {
                (ent.generation + 1) % 15
            };
            if self.arena_level && !release {
                self.arena.granted = new_owner;
                self.arena.disc_heard = Instant::now();
                self.arena.disc_touch = Some((object, entry_id, actor.try_into().expect("8 bytes")));
                self.arena.delegated = false;
                // Native (headbutt of the parked disc, 2026-11-16): the server
                // stops sending its own disc at the grant; the next disc state
                // the others get is the grantee's, under the new count. Ours
                // kept sending the parked disc, now with the new count, until
                // the grantee's state caught up: the others took that as the
                // current disc and ignored the grantee's (same count), so a
                // headbutted disc never moved for them until the next grab.
                if self.arena.server_disc && self.arena.frozen.is_none() {
                    self.arena.server_disc = false;
                    info!(handle = new_owner, resets = self.arena.disc_resets, "disc granted: server stops publishing it");
                }
            }
            let ent = self.touch.get_mut(&key).expect("inserted");
            ent.owner = new_owner;
            ent.stamp = stamp;
            ent.held = exclusive && !release;
            reply[0x08..0x0c].copy_from_slice(&new_owner.to_le_bytes());
            // 0x10: exclusive grab. 0x20: the requester already held it
            // (native sets it on a re-grab; without it the client plays the
            // steal sound).
            let regrab = !release && prev_owner == new_owner;
            let flags = (u16::from(exclusive) << 4) | (u16::from(regrab) << 5) | u16::from(ent.generation & 0xf);
            reply[0x18..0x1a].copy_from_slice(&flags.to_le_bytes());
            // 0x1c..0x20 stays 0: native's 77 grants and declines in a public
            // match (2026-11-16 recording) all had 0 there.
            // Native (two-client capture): only the requester gets the
            // grant; the others learn from the holder's forwarded disc,
            // whose reset count the server moves on.
            self.reliable_to(from, TOUCH_GRANTED, &reply);
            if self.social && !release {
                self.props.on_grant(from, new_owner, object, entry_id);
            }
        } else {
            reply[0x08..0x0c].copy_from_slice(&e.owner.to_le_bytes());
            reply[0x18..0x1a].copy_from_slice(&u16::from(e.generation & 0xf).to_le_bytes());
            self.reliable_to(from, TOUCH_DECLINED, &reply);
        }
    }

    /// The disc's holder left or went silent while the disc was still in
    /// play. This server cannot simulate it (no level geometry), but every
    /// client can: hand it to another player's client with a grant that
    /// player did not ask for, not held, so it keeps flying and bouncing
    /// there and everyone else follows that client's disc.
    ///
    /// Whether a client takes a grant it did not request is NOT known from
    /// any recording (native never needs to: its server simulates). If the
    /// chosen client sends no disc state within `DISC_SILENCE`, the server
    /// parks the disc as before. False: nobody to hand it to.
    fn delegate_disc(&mut self, holder: u32) -> bool {
        let Some((object, entry_id, actor)) = self.arena.disc_touch else { return false };
        // A player on a team (not a spectator), the best connection first.
        let Some((to, handle)) = self
            .peers
            .iter()
            .filter(|(_, p)| p.stage == Stage::Joined && (p.team == TEAM_BLUE || p.team == TEAM_ORANGE))
            .filter_map(|(a, p)| Some((*a, (u32::from(p.generation.unwrap_or(0)) << 16) | p.slot? as u32, p.t.stalled())))
            .filter(|(_, h, stalled)| *h != holder && !stalled)
            .min_by_key(|(_, h, _)| h & 0xffff)
            .map(|(a, h, _)| (a, h))
        else {
            return false;
        };
        let key = (u64::from(object), entry_id);
        let generation = ((self.arena.disc_resets + 1) % DISC_COUNTS) as u8;
        self.arena.disc_resets = u32::from(generation);
        self.arena.granted = handle;
        self.arena.disc_heard = Instant::now();
        self.arena.delegated = true;
        self.arena.tracker.reset();
        let e = self.touch.entry(key).or_insert(TouchEntry { owner: u32::MAX, stamp: 0, generation: 0, held: false });
        e.owner = handle;
        e.generation = generation;
        e.held = false;
        let seq = self.touch_seq.get(&((handle & 0xffff) as usize, u64::from(object))).copied().unwrap_or(0);
        let mut reply = [0u8; 0x20];
        reply[0..8].copy_from_slice(&touch_object(object, self.info.level).to_le_bytes());
        reply[0x08..0x0c].copy_from_slice(&handle.to_le_bytes());
        reply[0x0c..0x10].copy_from_slice(&entry_id.to_le_bytes());
        reply[0x10..0x18].copy_from_slice(&actor);
        reply[0x18..0x1a].copy_from_slice(&u16::from(generation & 0xf).to_le_bytes());
        reply[0x1a..0x1c].copy_from_slice(&seq.to_le_bytes());
        self.reliable_to(to, TOUCH_GRANTED, &reply);
        info!(from = holder, to = handle, resets = self.arena.disc_resets, "abandoned disc: handed to another player's client to simulate");
        true
    }

    /// A player handle (`gen << 16 | slot`) still names a joined player.
    fn handle_present(&self, h: u32) -> bool {
        let slot = (h & 0xffff) as usize;
        let Some(addr) = self.slot_addr.get(slot).copied().flatten() else { return false };
        self.peers.get(&addr).is_some_and(|p| p.stage == Stage::Joined && p.generation.is_none_or(|g| u32::from(g) == h >> 16))
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
        // are ignored; in play, the first player to hold it takes it over.
        let free_play = (self.arena.private && self.arena.phase == Phase::Waiting) || self.arena.disc_loose;
        if self.arena.frozen.is_some() || (self.arena.server_disc && self.arena.phase != Phase::Playing && !free_play) {
            return;
        }
        // The disc's owner simulates it; every other client's copy is a
        // stale prediction (it still names the last owner it knew), so only
        // the owner's own transform counts. This also keeps two clients from
        // flipping the owner back and forth.
        let Some(sender) = self.peers.get(&from) else { return };
        if sender.slot != Some(usize::from(owner)) {
            return;
        }
        let (owner_gen, owner_team) = (sender.generation.unwrap_or(0), sender.team);
        let client_count = u32::from(d[24]) & 0xf;
        // A client that has not yet caught up to the server's reset count
        // still shows its old disc: it can't take the server's over.
        if self.arena.server_disc && (client_count % DISC_COUNTS + DISC_COUNTS - self.arena.disc_resets % DISC_COUNTS) % DISC_COUNTS >= 8 {
            return;
        }
        let handle = (u32::from(owner_gen) << 16) | u32::from(owner);
        // A player who moved the disc without a grab (a headbutt)
        // now simulates it, but the reset count has not moved, so the others
        // would ignore that player's disc: move it past the player's own.
        // The same when it is the server's disc that was taken (parked in
        // play): the others hold the server's disc at the server's count.
        // The touch table follows, or the others' grabs from this player
        // (whom they now see as the owner) would all be declined.
        if self.arena.disc_free() && handle != self.arena.granted && (self.arena.server_disc || handle != self.arena.disc_owner) {
            self.arena.disc_resets = (client_count + 1) % DISC_COUNTS;
            self.arena.granted = handle;
            self.arena.disc_heard = Instant::now();
            for e in self.touch.values_mut() {
                e.owner = handle;
                e.held = false;
            }
            info!(slot = owner, resets = self.arena.disc_resets, "disc taken without a grab (headbutt?): others snap to it");
        }
        self.arena.client_resets = Some(client_count);
        self.arena.disc_owner = handle;
        if self.arena.server_disc && (self.arena.phase == Phase::Playing || free_play) {
            self.arena.server_disc = false;
            info!(slot = owner, "player took the disc; server stops publishing it");
        }
        if self.arena.phase != Phase::Playing {
            // Free play: where the disc is, should its holder leave.
            self.arena.last_disc = Some((x, y, depth));
            return;
        }
        let prev = self.arena.last_disc.replace((x, y, depth));
        // The game's goal test (see goal.rs): this move against each goal's
        // trigger box, counted on the move that starts touching it.
        let mut entered = None;
        for (i, g) in [&goal::BLUE_GOAL, &goal::ORANGE_GOAL].into_iter().enumerate() {
            let hit = prev.and_then(|(px, py, pd)| {
                let m = |v: i32| v as f32 / 1000.0;
                g.hit([m(px), m(py), m(pd)], [m(x), m(y), m(depth)])
            });
            if hit.is_some() && !self.arena.goal_touch[i] && entered.is_none() {
                entered = hit.map(|h| (i, h.map(|c| (c * 1000.0).round() as i32)));
            }
            self.arena.goal_touch[i] = hit.is_some();
        }
        let now = self.now_us();
        // Client disc state word, second byte (native 2026-11-11): 2 in a
        // hand, 0 after a throw, 4 after a hit (a headbutt; from
        // the grant of a request with the hit bit to the next grab).
        let st = u32::from_le_bytes([d[24], d[25], d[26], d[27]]);
        let (held, hit) = match (st >> 8) & 0xf {
            2 => (Some(true), false),
            0 => (Some(false), false),
            4 => (Some(false), true),
            _ => (None, false),
        };
        if owner_team == TEAM_BLUE || owner_team == TEAM_ORANGE {
            let client_us = u32::from_le_bytes([d[8], d[9], d[10], d[11]]);
            self.arena.tracker.sample(now, Some(client_us), owner, owner_team, [x, y, depth], held, hit);
        }
        self.apply_credits();
        if !self.arena.goals_armed {
            return;
        }
        let Some((goal_idx, hit)) = entered else { return };
        // Orange's goal (index 1) is where blue scores.
        let blue_scored = goal_idx == 1;
        // A bumper shot (thrown at an opponent who headbutts it into their
        // own goal) is the thrower's goal, not an own goal.
        let scoring_team = if blue_scored { TEAM_BLUE } else { TEAM_ORANGE };
        let own_goal = (owner_team == TEAM_BLUE) != blue_scored && !self.arena.tracker.bumper(scoring_team);
        // Own goals: public matches follow mp_arena_rules (stock: never), private ones the RULES screen.
        let self_goals = if self.arena.private { self.arena.rules.self_goaling() } else { super::msg::arena_public().self_goals };
        if own_goal && !self_goals {
            info!(slot = owner, private = self.arena.private, "own goal with self goaling off: not counted");
            return;
        }
        let g = self.arena.tracker.goal(now, hit, if blue_scored { TEAM_BLUE } else { TEAM_ORANGE });
        self.apply_credits();
        let mut rec = [0u8; 28];
        rec[0..2].copy_from_slice(&u16::from(!blue_scored).to_le_bytes());
        rec[2..4].copy_from_slice(&(g.points as u16).to_le_bytes());
        rec[4..6].copy_from_slice(&g.goal_type.to_le_bytes());
        let handle_of = |s: Option<u16>| {
            s.and_then(|s| self.slot_addr.get(usize::from(s)).copied().flatten())
                .and_then(|a| self.peers.get(&a))
                .map(|p| (u32::from(p.generation.unwrap_or(0)) << 16) | p.slot.unwrap_or(0) as u32)
        };
        let scorer = handle_of(g.scorer).unwrap_or(self.arena.disc_owner);
        rec[6..10].copy_from_slice(&scorer.to_le_bytes());
        let assister = handle_of(g.assister).unwrap_or(u32::MAX);
        rec[10..14].copy_from_slice(&assister.to_le_bytes());
        // Native: disc speed (m/s), release-to-goal distance (m), seconds of
        // play this round.
        rec[16..20].copy_from_slice(&g.speed.to_le_bytes());
        rec[20..24].copy_from_slice(&g.distance.to_le_bytes());
        rec[24..28].copy_from_slice(&(self.arena.round_clock - self.arena.clock).to_le_bytes());
        for a in self.joined() {
            self.reliable_to(a, POST_SCORE, &rec);
        }
        // Native: the server takes the disc and pins it where it entered the
        // goal box, bumping the reset count so clients snap to it.
        self.arena.frozen = Some((hit[0], hit[1], hit[2]));
        self.arena.parked = None;
        self.arena.bump_resets();
        self.arena.server_disc = true;
        // Native (hoster match, goal at 324.257 s): the server's first disc
        // state after the goal is still the flying disc (the scorer's last
        // state, owner none, the new count, high nibble 0); the pinned one
        // follows ~25 ms later. The spectator HUD's disc speed reads that
        // first state: ours sent the pinned disc at once and it showed 0 m/s.
        self.arena.goal_flight = if self.arena.holder_state.is_some() { (frame_hz() / 40).max(1) as u32 } else { 0 };
        self.arena.disc_live = false;
        self.arena.last_disc = None;
        // Native bumps the touch generation once per goal (grants 2 -> 4
        // across goal 1); the clients bump theirs too.
        for e in self.touch.values_mut() {
            e.owner = u32::MAX;
            e.generation = (e.generation + 1) % 15;
        }
        self.arena.tracker.reset();
        self.arena.goal(self.arena.tick, blue_scored, g.points);
        // An overtime goal (private), or the lead reaching the mercy margin
        // (private: the rule; public: always 12), ends the round once the
        // celebration has played. Public has one round, so that ends the match.
        {
            let margin = if self.arena.private { self.arena.rules.mercy_margin() } else { Some(public_mercy_margin()) };
            let mercy = margin.filter(|&m| self.arena.blue.abs_diff(self.arena.red) >= m);
            if mercy.is_some() && !self.arena.overtime {
                // Mercy ends the match at once: the score events of this
                // goal go out, then match over (boards and clock read
                // MERCY), with no 15 s goal celebration first.
                let g = self.arena.tick;
                self.arena.schedule.retain(|(at, _)| *at <= g);
                self.arena.at(g + 1, Act::EndRound);
                self.arena.round_live = false;
                info!(margin = ?mercy, "mercy: match ends now");
            } else if self.arena.overtime {
                let end = self.arena.celebration_end(self.arena.tick);
                self.arena.schedule.retain(|(at, _)| *at < end);
                self.arena.at(end, Act::EndRound);
                self.arena.round_live = false;
                info!(margin = ?mercy, overtime = self.arena.overtime, "round decided by this goal");
            }
        }
        info!(blue = self.arena.blue, orange = self.arena.red, points = g.points, scorer = owner, assister = ?g.assister, speed = g.speed, distance = g.distance, "goal");
    }

    /// Move the tracker's stat credits onto the players.
    fn apply_credits(&mut self) {
        for (slot, stat) in std::mem::take(&mut self.arena.tracker.credits) {
            if let Some(p) = self.peers.values_mut().find(|p| p.slot == Some(usize::from(slot))) {
                p.stats.add(stat, 1);
                debug!(slot, ?stat, total = p.stats.get(stat), "stat");
                let team = p.team;
                self.arena.announce_stat(stat, team);
            }
        }
    }

    /// `SR15StunPlayerMsg` from the stunner's client: one stun. The body
    /// layout is unrecorded, so a burst within 0.5 s counts once.
    /// The stun's victim (`u16` slot at 0) is watching, not playing.
    fn stun_victim_is_spectator(&self, d: &[u8]) -> bool {
        let Some(b) = d.get(0..2) else { return false };
        let slot = usize::from(u16::from_le_bytes([b[0], b[1]]));
        self.peers.values().any(|p| p.slot == Some(slot) && matches!(p.team, TEAM_SPECTATOR | TEAM_MODERATOR))
    }

    /// `SR15NetPlayerEquipmentUpdate`: relayed to the others, as native. A
    /// newcomer's first one also gets everyone else's latest: native (all four
    /// combat server recordings, 2026-11-16) answers it within 0.1-0.3 s with
    /// each other player's update, so players who joined later saw no guns,
    /// tac mods or weapon arm (hand swap) until the owner changed something.
    fn on_equipment(&mut self, from: SocketAddr, d: &[u8]) {
        for (a, p) in self.peers.iter_mut() {
            if *a != from && p.stage == Stage::Joined && p.spawned() {
                p.t.queue_reliable(EQUIPMENT, d);
            }
        }
        let Some(p) = self.peers.get_mut(&from) else { return };
        let first = p.equipment.replace(d.to_vec()).is_none();
        if !first {
            return;
        }
        // Native's server marks the player spawned (entrant data, to everyone).
        if let Some(slot) = p.slot {
            let mut spawned = (slot as u64).to_le_bytes().to_vec();
            spawned.extend_from_slice(&ENTRANT_KEY_SPAWNED.to_le_bytes());
            let mut status = (slot as u64).to_le_bytes().to_vec();
            status.extend_from_slice(&ENTRANT_KEY_MM_STATUS.to_le_bytes());
            status.extend_from_slice(&ENTRANT_MM_STATUS_VALUE);
            self.relay_reliable(from, ENTRANT_DATA, &spawned, true);
            self.relay_reliable(from, ENTRANT_DATA, &status, true);
        }
        info!(%from, "spawned: other players' bodies now sent to it");
        let others: Vec<Vec<u8>> = self
            .peers
            .iter()
            .filter(|(a, p)| **a != from && p.stage == Stage::Joined)
            .filter_map(|(_, p)| p.equipment.clone())
            .collect();
        debug!(%from, players = others.len(), "equipment of the others sent to a newcomer");
        for e in others {
            self.reliable_to(from, EQUIPMENT, &e);
        }
    }

    /// Whether a stun from `who` is dropped: in the social lobby outside the
    /// brawler room (arena stun and block) and the combat area (its stun
    /// ordnance); in the arena while the hold is on (goal celebrations, round
    /// and match ends). The client's own cease fire (`VAR_HOLD`) is lifted in
    /// the brawler room only.
    fn cease_fire_for(&self, who: SocketAddr) -> bool {
        if self.social {
            return !self.gear.may_fight(who) && !self.gear.stun_allowed(who, &[]);
        }
        self.arena_level && self.arena.flags.iter().any(|&(h, v)| h == VAR_HOLD && v == 1)
    }

    fn on_stun(&mut self, from: SocketAddr, d: &[u8]) {
        if !self.arena_level || self.arena.phase != Phase::Playing {
            return;
        }
        let Some(p) = self.peers.get_mut(&from) else { return };
        if p.last_stun.is_some_and(|t| t.elapsed() < Duration::from_millis(500)) {
            return;
        }
        p.last_stun = Some(Instant::now());
        p.stats.add(Stat::Stuns, 1);
        info!(%from, slot = ?p.slot, stuns = p.stats.get(Stat::Stuns), len = d.len(), body = %hex_str(d), "stun");
    }

    // ── Departures ─────────────────────────────────────────────────────────

    fn remove_peer(&mut self, addr: SocketAddr, code: pb::lobby_entrant_removed_message::Code) {
        self.drop_peer(addr, Some(code));
    }

    fn drop_peer(&mut self, addr: SocketAddr, code: Option<pb::lobby_entrant_removed_message::Code>) {
        let Some(p) = self.peers.remove(&addr) else { return };
        // Their gear stations are freed for everyone still here.
        for o in self.gear.forget(addr) {
            self.send_gear(addr, o);
        }
        self.props.forget(addr);
        // Saved again on the way out even when already saved: the backend may
        // have written its own older copy of the profile since.
        if p.loadout_save_at.is_some() || p.loadout_changed {
            let lobby = self.info.lobby.to_string();
            self.fleet_out.extend(loadout_save(&lobby, &p));
        }
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

    /// Reliable frames and resends only (between server frames); mailbox
    /// state goes out once per frame from `server_frame`.
    pub fn flush_reliable(&mut self) {
        let mut pkts = Vec::new();
        for (a, p) in self.peers.iter_mut() {
            p.t.flush_reliable(&mut pkts);
            self.udp_out.extend(pkts.drain(..).map(|d| (*a, d)));
        }
    }

    /// Resend timers only (cheap; runs between frames).
    pub fn service(&mut self) {
        self.save_loadouts();
        for (from, o) in self.gear.due(Instant::now()) {
            self.send_gear(from, o);
        }
        self.flush_reliable();
    }

    /// Report loadouts players changed to the backend, which keeps them in
    /// their profile (EchoRelay and nakama each do it their own way; the
    /// bridge in `upstream` handles that).
    fn save_loadouts(&mut self) {
        let now = Instant::now();
        let lobby = self.info.lobby.to_string();
        let mut out = Vec::new();
        for p in self.peers.values_mut() {
            if p.loadout_save_at.is_none_or(|t| t > now) {
                continue;
            }
            p.loadout_save_at = None;
            out.extend(loadout_save(&lobby, p));
        }
        self.fleet_out.extend(out);
    }

    fn publish_owner_state(&mut self) {
        let owner = self.owner_state();
        for a in self.joined() {
            self.reliable_to(a, OWNER_STATE, &owner);
        }
    }

    /// Every server frame (`frame_hz()`): server frame message, ping estimates, replication, rules.
    pub fn server_frame(&mut self) {
        self.frame += 1;
        // Wire frame word for this server frame (also the mailbox key bucket,
        // as native keys by it); `self.frame` stays the rules frame counter.
        let (frame, now_us) = wire_clock();
        if frame <= self.wire_frame {
            return; // two server frames inside one tick: nothing new to publish
        }
        // Native sends a server frame message for every 120 Hz frame; the
        // client counts a gap in the numbers as lost packets (it showed 83%
        // loss when only every 6th went out). Send each one since the last
        // tick (capped, after a stall).
        let first = if self.wire_frame == 0 { frame } else { (self.wire_frame + 1).max(frame.saturating_sub(23)) };
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
        // Who may hurt whom (splash, shots, stun fields) follows the teams.
        self.gear.teams.clear();
        self.gear.teams.extend(self.peers.iter().map(|(a, p)| (*a, p.team)));

        let mut sf = [0u8; 16];
        sf[..8].copy_from_slice(&frame.to_le_bytes());
        sf[8..].copy_from_slice(&now_us.to_le_bytes());

        // Ping estimates measured from each peer's echoes: no fixed value, no floor.
        let mut estimates = Vec::new();
        for p in self.peers.values_mut() {
            p.t.take_rtt_fresh();
            // One estimate per echo, as native: `[slot u16][generation u16]
            // [age f32][client time u64][0 u64]`. Clients drop an estimate
            // whose generation is not the slot's (`ClientsPingEstCB`), and
            // without estimates they have no clock for that player: everyone
            // in a reused slot stuttered while this was always 0.
            if p.ping_dirty && let (Some(slot), Some(est)) = (p.slot, p.echo_est_us()) {
                let mut e = [0u8; 24];
                e[0..2].copy_from_slice(&(slot as u16).to_le_bytes());
                e[2..4].copy_from_slice(&p.generation.unwrap_or(p.genidx).to_le_bytes());
                e[4..8].copy_from_slice(&est.to_le_bytes());
                e[8..16].copy_from_slice(&p.ping_token.to_le_bytes());
                estimates.push((slot as u64, e));
            }
            p.ping_dirty = false;
        }
        let pings = (self.frame % (2 * frame_hz()) == 0).then(|| {
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
                        let list: Vec<String> = v.iter().take(64).map(|((t, r, l, o), n)| format!("{t:#x}/{}/{l}B/obj{o:#x}x{n}", if *r { "rel" } else { "mb" })).collect();
                        debug!(slot = s, "client sent: {}", list.join(" "));
                    }
                    if !p.t.sent_types.is_empty() {
                        let mut v: Vec<_> = p.t.sent_types.drain().collect();
                        v.sort_by(|a, b| b.1.cmp(&a.1));
                        let list: Vec<String> = v.iter().take(96).map(|((t, m, l), n)| format!("{t:#x}/{}/{l}B x{n}", if *m { "mb" } else { "rel" })).collect();
                        info!(slot = s, "server sent: {}", list.join(" "));
                    }
                    if p.joined_at.is_some_and(|t| t.elapsed() > Duration::from_secs(90)) {
                        p.t.trace = false;
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
            for n in first..frame {
                let mut sf = [0u8; 16];
                sf[..8].copy_from_slice(&n.to_le_bytes());
                sf[8..].copy_from_slice(&wire_time(n).to_le_bytes());
                p.t.queue_mailbox(SERVER_FRAME, key_server_frame(n), &sf);
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
            // Native follows JoinComplete with the entrant's team, old 0xffff.
            if let Some((slot, team)) = self.peers.get(&a).and_then(|p| Some((p.slot?, p.team))) {
                let tc = team_changed(slot, team, None);
                for b in self.joined() {
                    self.reliable_to(b, ENTRANT_TEAM_CHANGED, &tc);
                }
            }
            self.reliable_to(a, msg::SERVER_INFO, &info);
            if let Some(p) = self.peers.get_mut(&a) {
                p.join_complete = true;
            }
        }
        if owner_resend {
            self.publish_owner_state();
        }

        if self.arena_level {
            self.run_rules();
        }
        if self.combat.is_some() {
            self.run_combat();
        }
        self.replicate();
        // The lobby objects nobody owns, ten times a second (native: 45; they
        // do not move here, so less is enough).
        // The Fury, while it flies, goes out 40 times a second.
        if self.social {
            self.props.step(Instant::now());
            self.props.expire(Instant::now());
            self.lobby_goal();
        }
        let every = if self.props.active() { frame_hz() / 40 } else { frame_hz() / 10 };
        if self.social && self.frame % every.max(1) == 0 {
            for (key, body) in self.props.broadcast(frame, now_us) {
                for p in self.peers.values_mut().filter(|p| p.stage == Stage::Joined) {
                    p.t.queue_mailbox(PHYSICS, key, &body);
                }
            }
        }
        self.flush();
    }

    // ── Arena ──────────────────────────────────────────────────────────────

    /// The combat match's flow (`combat.rs`), on wall time.
    fn run_combat(&mut self) {
        let now = Instant::now();
        let dt = self.combat_at.map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(1.0));
        self.combat_at = Some(now);
        let players: Vec<super::combat::Player> = self
            .peers
            .iter()
            .filter(|(_, p)| p.stage == Stage::Joined && matches!(p.team, TEAM_BLUE | TEAM_ORANGE))
            .map(|(a, p)| {
                let slot = self.gear.slot_of(a) as u8;
                (u8::from(p.team == TEAM_ORANGE), self.gear.alive(*a), self.gear.body_pos(*a), slot)
            })
            .collect();
        let down = self.gear.barriers_down();
        let holders: Vec<u8> = self.peers.iter().filter(|(_, p)| p.on_payload && p.stage == Stage::Joined).map(|(a, _)| self.gear.slot_of(a) as u8).collect();
        let Some(c) = self.combat.as_mut() else { return };
        c.set_barriers_down(down);
        c.set_payload_holders(holders);
        let outs = c.step(dt, &players);
        self.combat_stats(dt, now);
        for o in outs {
            match o {
                super::combat::Out::Reliable(ty, body) => {
                    for a in self.joined() {
                        self.reliable_to(a, ty, &body);
                    }
                }
                super::combat::Out::Event(sym, module, handle, filter) => {
                    let ev = component_event(sym, module, handle, filter);
                    for a in self.joined() {
                        self.reliable_to(a, COMPONENT_EVENT, &ev);
                    }
                }
                super::combat::Out::Awards => {
                    self.send_combat_stats();
                    self.send_combat_awards();
                }
                super::combat::Out::RestoreBarriers => {
                    for (from, o) in self.gear.restore_barriers() {
                        self.send_gear(from, o);
                    }
                }
                super::combat::Out::SwapTeams => self.swap_teams(),
                super::combat::Out::Unlock => {
                    if self.join_locked {
                        self.join_locked = false;
                        info!("join unlock (next combat round)");
                        self.fleet_out.push(env(Em::LobbySessionEvent(pb::LobbySessionEventMessage {
                            lobby_session_id: self.info.lobby.to_string(),
                            code: pb::lobby_session_event_message::Code::Unlocked as i32,
                        })));
                    }
                }
                super::combat::Out::Lock => {
                    // Native: the totals at once when a round or the match is won.
                    self.send_combat_stats();
                    if !self.join_locked {
                        self.join_locked = true;
                        info!("join lock (combat match over)");
                        self.fleet_out.push(env(Em::LobbySessionEvent(pb::LobbySessionEventMessage {
                            lobby_session_id: self.info.lobby.to_string(),
                            code: pb::lobby_session_event_message::Code::Locked as i32,
                        })));
                    }
                }
            }
        }
    }

    /// Half time in a private payload match: blue and orange trade places
    /// (each keeps its team slot), as an `EntrantTeamChanged` each.
    fn swap_teams(&mut self) {
        let mut changed = Vec::new();
        for p in self.peers.values_mut().filter(|p| p.stage == Stage::Joined) {
            let (Some(slot), old) = (p.slot, p.team) else { continue };
            let team = match old {
                TEAM_BLUE => TEAM_ORANGE,
                TEAM_ORANGE => TEAM_BLUE,
                _ => continue,
            };
            p.team = team;
            p.requested_team = Some(team);
            p.team_changed_at = Some(Instant::now());
            changed.push(team_changed(slot, team, Some(old)));
        }
        info!(players = changed.len(), "private combat match: teams swap sides");
        for tc in &changed {
            for a in self.joined() {
                self.reliable_to(a, ENTRANT_TEAM_CHANGED, tc);
            }
        }
        if !changed.is_empty() {
            self.roster_dirty = true;
            self.publish_owner_state();
        }
    }

    fn team_counts(&self) -> (usize, usize) {
        let joined = self.peers.values().filter(|p| p.stage == Stage::Joined);
        joined.fold((0, 0), |(b, o), p| match p.team {
            TEAM_BLUE => (b + 1, o),
            TEAM_ORANGE => (b, o + 1),
            _ => (b, o),
        })
    }

    /// The rules frames due by the clock since the last call (at most a
    /// second's worth after a stall).
    fn run_rules(&mut self) {
        let hz = frame_hz();
        if hz != self.rules_hz {
            self.rescale_rules(self.rules_hz, hz);
        }
        let now = Instant::now();
        let t0 = *self.rules_t0.get_or_insert(now);
        let due = self.rules_base + now.duration_since(t0).as_micros() as u64 * hz / 1_000_000;
        if due.saturating_sub(self.rules_done) > hz {
            self.rules_done = due - hz;
        }
        while self.rules_done < due {
            self.rules_done += 1;
            self.arena_rules();
        }
        self.update_join_lock();
    }

    /// A public match stops taking players (no backfill) with a minute left
    /// and once it is over: the server tells the matchmaker with a lobby
    /// session event, Locked, and Unlocked if a new match starts after all.
    fn update_join_lock(&mut self) {
        if self.arena.private {
            return;
        }
        let a = &self.arena;
        let lock = a.phase == Phase::Over || (matches!(a.phase, Phase::Playing | Phase::Goal) && a.clock <= JOIN_LOCK_SECS);
        if lock != self.join_locked {
            self.join_locked = lock;
            let code = if lock { pb::lobby_session_event_message::Code::Locked } else { pb::lobby_session_event_message::Code::Unlocked };
            info!(clock = a.clock, phase = ?a.phase, locked = lock, "join lock");
            self.fleet_out.push(env(Em::LobbySessionEvent(pb::LobbySessionEventMessage { lobby_session_id: self.info.lobby.to_string(), code: code as i32 })));
        }
    }

    /// The rate changed mid-game: everything counted in rules frames (the
    /// rules clock, its schedule, possession) moves to the new rate, so a
    /// countdown or pin keeps its remaining seconds.
    fn rescale_rules(&mut self, old: u64, new: u64) {
        let scale = |v: u64| v * new / old.max(1);
        let a = &mut self.arena;
        let tick = a.tick;
        let new_tick = scale(tick);
        for (at, _) in &mut a.schedule {
            *at = if *at >= tick { new_tick + scale(*at - tick) } else { scale(*at) };
        }
        a.tick = new_tick;
        for p in self.peers.values_mut() {
            let v = &mut p.stats.v[Stat::PossessionTime as usize];
            *v = (u64::from(*v) * new / old.max(1)) as u32;
        }
        self.rules_hz = new;
        self.rules_t0 = Some(Instant::now());
        self.rules_base = 0;
        self.rules_done = 0;
        info!(lobby = %self.info.lobby, from = old, to = new, "frame rate changed");
    }

    fn arena_rules(&mut self) {
        let dt = 1.0 / frame_hz() as f32;
        let frozen = self.arena.time_frozen();
        if !frozen {
            self.arena.tick += 1;
        }
        let f = self.arena.tick;
        let wf = self.wire_frame;
        let handles = self.handles;
        if self.arena.paused {
            self.arena.pause_time += dt;
            self.arena.set_var(VAR_PAUSE_TIME, Val::F32(self.arena.pause_time));
        } else if self.arena.resume > 0.0 {
            self.arena.resume = (self.arena.resume - dt).max(0.0);
            self.arena.set_var(VAR_RESUME_COUNT, Val::F32(self.arena.resume));
        }
        if self.arena.private && (self.arena.ready.contains(&true) || self.arena.restart_req.contains(&true)) {
            self.check_team_requests();
        }
        if self.arena.phase == Phase::Waiting && !self.arena.private {
            let (b, o) = self.team_counts();
            let need = self.arena.min_per_team;
            if b >= need && o >= need && b + o > 0 {
                info!(blue = b, orange = o, "both teams present: pre-match countdown started");
                self.arena.start_round(f);
            }
        }
        // Due actions, in schedule order.
        let mut due = std::mem::take(&mut self.arena.soon);
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
                    let ev = component_event(sym, module, handles.for_module(module), filter);
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
                Act::Live => {
                    self.arena.disc_live = true;
                    self.arena.last_disc = None;
                    self.arena.tracker.reset();
                    // Native republishes "no owner" at main_loop_enter.
                    self.arena.disc_owner = u32::MAX;
                }
                Act::ArmGoals => {
                    self.arena.goals_armed = true;
                    self.arena.round_live = true;
                    self.arena.round_clock = self.arena.clock;
                }
                Act::LaunchFx(fx) => {
                    // Launch count: the server releases the goal pin and
                    // holds the disc on its spawn.
                    if fx.is_some() {
                        // Out of the goal: clients that took the pinned disc
                        // only move to the spawn if the count moves on too.
                        if self.arena.frozen.is_some() {
                            self.arena.bump_resets();
                            info!(resets = self.arena.disc_resets, "goal pin released: disc to its spawn");
                        }
                        self.arena.frozen = None;
                        self.arena.parked = None;
                        self.arena.server_disc = true;
                        self.arena.disc_live = false;
                    }
                    self.arena.launch_fx = fx;
                }
                Act::Flag(h, v) => self.arena.set_flag(h, v),
                Act::Var(h, v) => self.arena.set_var(h, v),
                Act::Reset(keep) => self.private_reset(keep),
                Act::EndRound => {
                    let mercy = !self.arena.overtime;
                    info!(blue = self.arena.blue, orange = self.arena.red, mercy, "round over");
                    self.arena.end_round(f, mercy);
                    info!(blue_rounds = self.arena.blue_rounds, orange_rounds = self.arena.red_rounds, over = self.arena.phase == Phase::Over, "rounds");
                }
                Act::NewRound => {
                    let a = &mut self.arena;
                    if !a.rules.carry_points() {
                        [a.blue, a.red] = a.rules.start_scores();
                    }
                    a.clock = a.rules.round_secs();
                    a.round_clock = a.clock;
                    a.set_flag(VAR_MERCY_WIN, 0);
                    // A disc left loose at time up: the server takes it back.
                    if std::mem::take(&mut a.disc_loose) {
                        a.server_disc = true;
                        a.disc_live = false;
                        a.disc_owner = u32::MAX;
                        a.holder_state = None;
                        a.last_disc = None;
                    }
                    // Out of the goal and onto the spawn for the walk.
                    a.frozen = None;
                    a.parked = None;
                    a.bump_resets();
                    a.tracker.reset();
                    info!(round = a.rounds_played + 1, "next round");
                }
            }
        }
        // Native sends these every other 120 Hz frame (60/s); at 180 or 200 Hz
        // one per rules frame would be 3x that.
        let fx_due = (f * 60 / frame_hz()) != (f.saturating_sub(1) * 60 / frame_hz());
        if let Some((light, fx)) = self.arena.launch_fx.filter(|_| fx_due) {
            for (sym, module) in [(EV_RESTART_DISC, MOD_MATCH), (light, MOD_DISC_FX), (fx, MOD_DISC_FX)] {
                let ev = component_event(sym, module, handles.for_module(module), FILTER_ALL);
                for a in &joined {
                    if let Some(p) = self.peers.get_mut(a) {
                        p.t.queue_reliable(COMPONENT_EVENT, &ev);
                    }
                }
            }
        }
        // Possession: frames the disc is in someone's hand (not in flight).
        if self.arena.phase == Phase::Playing && let Some(holder) = self.arena.tracker.holding() {
            if let Some(p) = self.peers.values_mut().find(|p| p.slot == Some(usize::from(holder))) {
                p.stats.add(Stat::PossessionTime, 1);
            }
        }
        if matches!(self.arena.phase, Phase::Playing | Phase::Goal) && f % (3 * frame_hz()) == 0 {
            self.send_stats();
        }
        if std::mem::take(&mut self.arena.awards_due) {
            self.send_stats();
            self.send_top_awards();
            self.send_arena_profile_updates();
        }
        if self.arena.countdown_running && !frozen {
            self.arena.countdown = (self.arena.countdown - dt).max(0.0);
            self.arena.countdown_running = self.arena.countdown > 0.0;
        }
        let a = &self.arena;
        let in_play = a.clock_runs() && a.phase == Phase::Playing;
        let running = if a.private {
            // Goal stops time off: the clock keeps going through the goal
            // celebration and relaunch.
            a.round_live && !a.overtime && (in_play || (!a.rules.goal_stops_time() && matches!(a.phase, Phase::Goal | Phase::Launch)))
        } else {
            in_play
        };
        if running && !frozen {
            self.arena.clock = (self.arena.clock - dt).max(0.0);
            if self.arena.clock <= 0.0 {
                let a = &mut self.arena;
                if !a.private && a.blue == a.red && super::msg::arena_public().sudden_death {
                    let again = a.overtime;
                    a.start_sudden_death(f);
                    info!(score = a.blue, again, "time up, tied: sudden death");
                } else if !a.private {
                    info!(blue = a.blue, orange = a.red, "match over");
                    a.end_match(f);
                } else if a.blue == a.red && a.rules.overtime() {
                    // Tied: the draw and overtime calls, everyone back to the
                    // launchers, then a launch; the next goal wins the round.
                    a.start_sudden_death(f);
                    info!(score = a.blue, "time up, tied: overtime");
                } else {
                    info!(blue = a.blue, orange = a.red, "time up: round over");
                    a.end_round(f, false);
                    info!(blue_rounds = a.blue_rounds, orange_rounds = a.red_rounds, over = a.phase == Phase::Over, "rounds");
                }
            }
        }
        // Back to waiting if a match never started and a team emptied.
        if self.arena.phase == Phase::Countdown && !self.arena.private {
            let (b, o) = self.team_counts();
            if b < self.arena.min_per_team || o < self.arena.min_per_team {
                self.arena.schedule.clear();
                self.arena.phase = Phase::Waiting;
                self.arena.state = ST_PRE_MATCH;
                self.arena.countdown = pre_match_secs();
                self.arena.countdown_running = false;
                info!("a team emptied before launch: waiting again");
            }
        }
        // Server-side disc, only while the server owns it (native: not
        // while a player holds it).
        // The disc state's timestamp: real µs since the session began, the
        // base the clients' own disc stamps use (theirs run ~2.7 s ahead of
        // it; native's server ran ~2.6 s ahead of its client). Counting ticks
        // drifted 30 s behind in 2 minutes and restarted on a rematch, and
        // a client then ignored the holder's disc until it grabbed it itself.
        // Native (two-client capture): the server's stamp runs well ahead
        // of every client's (93 s vs 57 s / 40 s), so a client always takes
        // the server's disc as newer than its own last one. Ours ran ~2.8 s
        // behind, and after a goal the last holder ignored the server's disc
        // (pin and relaunch) and kept flying its own.
        // Kept to 32 bits, as the clients' own stamps are (native's high
        // word was 0 in every capture): about 70 minutes into a session the
        // u64 passed 2^32, and a non-zero high word makes every holder's
        // disc look older than the server's last one for good.
        self.arena.disc_us = (self.started.elapsed().as_micros() as u64 + DISC_STAMP_LEAD_US) & 0xffff_ffff;
        // A holder that left or went silent in play loses the disc to the
        // server (not during a pause: time stands still for the holder too).
        if self.arena.disc_free() && !self.arena.server_disc {
            if frozen {
                self.arena.disc_heard = Instant::now();
            } else {
                let holder = if self.arena.granted != u32::MAX { self.arena.granted } else { self.arena.disc_owner };
                let gone = holder != u32::MAX && !self.handle_present(holder);
                let silent = self.arena.disc_heard.elapsed();
                // Free play nobody has touched yet: leave the disc alone.
                let untouched = holder == u32::MAX && self.arena.phase != Phase::Playing;
                if !untouched && (gone || silent > DISC_SILENCE) && !self.arena.delegated && self.delegate_disc(holder) {
                    // Another client simulates it now; if that client
                    // says nothing either, the server parks it next time.
                } else if !untouched && (gone || silent > DISC_SILENCE) {
                    if std::mem::take(&mut self.arena.delegated) {
                        info!("abandoned disc: the client it was handed to sent nothing");
                    }
                    self.arena.park_disc();
                    info!(holder, gone, silent_ms = silent.as_millis() as u64, at = ?self.arena.parked, resets = self.arena.disc_resets, "disc holder gone or silent: server holds the disc there");
                }
            }
        }
        if self.arena.server_disc && !self.server_disc_was {
            // The server took the disc back: clients treat it as unowned
            // (their next grab says believed = none), so the table must too,
            // or every grab is declined and plays the steal sound. The
            // generation stays: clients only move theirs on a goal, and a
            // grant whose generation they don't expect is ignored by the
            // other clients.
            for e in self.touch.values_mut() {
                e.owner = u32::MAX;
            }
            // Nobody holds it now: the next grant names the next holder.
            self.arena.granted = u32::MAX;
        }
        self.server_disc_was = self.arena.server_disc;
        if self.arena.server_disc {
            let mut d = captured().disc_physics;
            // [8..16]: the state's timestamp (µs, the clients' shared clock).
            // The captured template has 0x12 in the high word; native sends
            // 0 there. With it, every client transform looks older than the
            // server's last one, so clients drop the holder's disc for good.
            d[8..16].copy_from_slice(&self.arena.disc_us.to_le_bytes());
            // High nibble (native): 1 before play and on a goal, a running
            // counter through the launch count, 2 once play starts. Low
            // nibble: the reset count (+3 per goal).
            let hi = if self.arena.launch_fx.is_some() {
                ((wf / 2) & 0xf) as u32
            } else if self.arena.disc_live {
                2
            } else {
                1
            };
            d[24..28].copy_from_slice(&((hi << 4) | (self.arena.disc_resets & 0xf)).to_le_bytes());
            let (x, y, depth) = match self.arena.frozen.or(self.arena.parked) {
                Some(p) => p,
                None => {
                    let base = baseline(self.info.level, self.info.mode).and_then(|b| var_raw(&b[0], VAR_DISC_SPAWN)).unwrap_or(2);
                    disc_spawn_pos(self.arena.flag(VAR_DISC_SPAWN).unwrap_or(base as u8))
                }
            };
            msg::write_disc_transform(&mut d, x, y, depth);
            if self.arena.goal_flight > 0 {
                self.arena.goal_flight -= 1;
                if let Some(h) = self.arena.holder_state.as_deref().filter(|h| h.len() == d.len()) {
                    d.copy_from_slice(h);
                    d[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
                    d[8..16].copy_from_slice(&self.arena.disc_us.to_le_bytes());
                    d[24..28].copy_from_slice(&(self.arena.disc_resets & 0xf).to_le_bytes());
                }
                if self.arena.goal_flight == 0 {
                    self.arena.holder_state = None;
                }
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
        for p in self.peers.values() {
            let Some(slot) = p.slot else { continue };
            if let Some(t) = team.get_mut(usize::from(p.team)) {
                for (k, v) in p.stats.v.iter().enumerate() {
                    t.v[k] += v;
                }
            }
            // Running totals, every key native's screen list names (only
            // non-zero ones, as native omits uncredited stats).
            // The client drops it unless the high half is the player's
            // generation for that slot.
            let generation = p.generation.unwrap_or(p.genidx);
            let mut m = ((u32::from(generation) << 16) | slot as u32).to_le_bytes().to_vec();
            m.extend_from_slice(p.stats.to_json(frame_hz()).as_bytes());
            m.push(0);
            msgs.push(m);
        }
        let mut team_msgs = Vec::new();
        for (i, t) in team.iter().enumerate() {
            let mut m = (i as u16).to_le_bytes().to_vec();
            // The twelve records follow the screen-stat order (`stats::STAT_NAMES`).
            for (k, h) in TEAM_STATS.iter().enumerate() {
                let (mant, exp) = if k == TS_POSSESSION {
                    ((t.v[k] as u64 * 64 / frame_hz()) as i32, -6)
                } else {
                    (t.v[k] as i32, 0)
                };
                m.extend_from_slice(&h.to_le_bytes());
                m.extend_from_slice(&mant.to_le_bytes());
                m.extend_from_slice(&if mant == 0 { 0i32 } else { exp }.to_le_bytes());
                m.extend_from_slice(&u32::from(mant != 0).to_le_bytes());
            }
            team_msgs.push(m);
        }
        if let Some(m) = msgs.first() {
            debug!(players = msgs.len(), to = joined.len(), first = %String::from_utf8_lossy(&m[4..]), "stat update");
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

    /// The combat match's stats: the gear's damage and kills, time on the
    /// objective, and native's update every 3 s.
    fn combat_stats(&mut self, dt: f32, now: Instant) {
        let Some(c) = self.combat.as_ref() else { return };
        if c.phase == super::combat::Phase::Waiting {
            self.gear.credits.clear();
            self.recent_hits.clear();
            self.combat_stat_clock = 0.0;
            for p in self.peers.values_mut() {
                p.cstats = CombatStats::default();
            }
            return;
        }
        let playing = c.phase == super::combat::Phase::Playing;
        let counting = c.counting();
        let mut objective: HashMap<SocketAddr, bool> = HashMap::new();
        for (a, p) in &self.peers {
            if p.stage == Stage::Joined && matches!(p.team, TEAM_BLUE | TEAM_ORANGE) {
                objective.insert(*a, self.gear.body_pos(*a).is_some_and(|q| c.on_objective(q)));
            }
        }
        let credits: Vec<super::gear::Credit> = self.gear.credits.drain(..).collect();
        if playing {
            for (a, o) in &objective {
                let alive = self.gear.alive(*a);
                if let Some(p) = self.peers.get_mut(a) {
                    p.cstats.played += dt;
                    if *o && alive {
                        p.cstats.objective_time += dt;
                    }
                }
            }
        }
        let on_obj = |a: &SocketAddr| objective.get(a).copied().unwrap_or(false);
        for cr in credits {
            match cr {
                super::gear::Credit::Damage { by, victim, amount } => {
                    if let Some(p) = self.peers.get_mut(&victim) {
                        p.cstats.damage_taken += amount;
                    }
                    let Some(by) = by else { continue };
                    let obj = on_obj(&by) || on_obj(&victim);
                    if let Some(p) = self.peers.get_mut(&by) {
                        p.cstats.damage += amount;
                        if obj {
                            p.cstats.objective_damage += amount;
                        }
                    }
                    let hits = self.recent_hits.entry(victim).or_default();
                    hits.retain(|(a, _)| *a != by);
                    hits.push((by, now));
                }
                super::gear::Credit::Kill { by, victim } => {
                    let obj = by.is_some_and(|b| on_obj(&b)) || on_obj(&victim);
                    if let Some(p) = self.peers.get_mut(&victim) {
                        p.cstats.deaths += 1;
                    }
                    if let Some(p) = by.and_then(|b| self.peers.get_mut(&b)) {
                        p.cstats.kills += 1;
                        p.cstats.eliminations += 1;
                        if obj {
                            p.cstats.objective_eliminations += 1;
                        }
                    }
                    // Others who hurt the victim in the last 10 s assisted.
                    let helpers: Vec<SocketAddr> = self
                        .recent_hits
                        .remove(&victim)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|(a, t)| Some(*a) != by && now.duration_since(*t) <= COMBAT_ASSIST_WINDOW)
                        .map(|(a, _)| a)
                        .collect();
                    for a in helpers {
                        if let Some(p) = self.peers.get_mut(&a) {
                            p.cstats.assists += 1;
                        }
                    }
                }
            }
        }
        if counting {
            self.combat_stat_clock += dt;
            if self.combat_stat_clock >= COMBAT_STATS_EVERY {
                self.combat_stat_clock = 0.0;
                self.send_combat_stats();
            }
        }
    }

    /// Native combat: `SR15NetReliableStatUpdate` per player (`[slot u32]` +
    /// the running totals' JSON) and `SR15NetReliableTeamStatUpdate` per team
    /// (`[team u16]` + eight `[stat sym][m i32][e i32][players i32]` records,
    /// value `m * 2^e`, players = how many on the team have any).
    fn send_combat_stats(&mut self) {
        let mut msgs = Vec::new();
        let mut teams: [Vec<CombatStats>; 2] = [Vec::new(), Vec::new()];
        let mut slots: Vec<(usize, &Peer)> = self.peers.values().filter_map(|p| p.slot.map(|s| (s, p))).collect();
        slots.sort_by_key(|(s, _)| *s);
        for (slot, p) in slots {
            if matches!(p.team, TEAM_BLUE | TEAM_ORANGE) {
                teams[usize::from(p.team)].push(p.cstats);
            }
            let generation = p.generation.unwrap_or(p.genidx);
            let mut m = ((u32::from(generation) << 16) | slot as u32).to_le_bytes().to_vec();
            m.extend_from_slice(p.cstats.to_json().as_bytes());
            m.push(0);
            msgs.push(m);
        }
        let mut team_msgs = Vec::new();
        for (i, members) in teams.iter().enumerate() {
            let mut m = (i as u16).to_le_bytes().to_vec();
            for name in super::stats::COMBAT_TEAM_STATS {
                let mut total = 0.0;
                let mut real = false;
                let mut have = 0i32;
                for s in members {
                    let (v, r) = s.get(name);
                    real = r;
                    total += v;
                    have += i32::from(v != 0.0);
                }
                let (mant, exp) = super::stats::mantissa_exp(total, real);
                m.extend_from_slice(&crate::sns::symbol(name).to_le_bytes());
                m.extend_from_slice(&mant.to_le_bytes());
                m.extend_from_slice(&exp.to_le_bytes());
                m.extend_from_slice(&have.to_le_bytes());
            }
            team_msgs.push(m);
        }
        for a in self.joined() {
            for m in &msgs {
                self.reliable_to(a, STAT_UPDATE, m);
            }
            for m in &team_msgs {
                self.reliable_to(a, TEAM_STAT_UPDATE, m);
            }
        }
    }

    /// Native combat at match over: the MVP (`SR15NetTopAwardsMsg`, one
    /// award, from the winning team), one tier reward per player (gold, then
    /// silver, then bronze, in eliminations, objective eliminations and
    /// objective time: the best player without one yet), and each player's
    /// profile change (`SNSRefreshProfileFromServer`: their stats and XP),
    /// which the backend also keeps.
    fn send_combat_awards(&mut self) {
        let Some(c) = self.combat.as_ref() else { return };
        let winner = c.winner.map(|w| match w {
            super::combat::Winner::Blue => Some(TEAM_BLUE),
            super::combat::Winner::Orange => Some(TEAM_ORANGE),
            super::combat::Winner::Draw => None,
        });
        let winner = winner.flatten();
        let capture = c.is_capture();
        struct P {
            addr: SocketAddr,
            slot: usize,
            handle: u32,
            team: u8,
            name: Vec<u8>,
            s: CombatStats,
            xpid: Option<[u64; 2]>,
        }
        let mut players: Vec<P> = self
            .peers
            .iter()
            .filter(|(_, p)| matches!(p.team, TEAM_BLUE | TEAM_ORANGE))
            .filter_map(|(a, p)| {
                let slot = p.slot?;
                let name = p
                    .entrant_header
                    .as_deref()
                    .and_then(|h| h.get(60..96))
                    .map(|n| n.split(|&b| b == 0).next().unwrap_or(&[]).to_vec())
                    .unwrap_or_default();
                let handle = (u32::from(p.generation.unwrap_or(0)) << 16) | slot as u32;
                Some(P { addr: *a, slot, handle, team: p.team, name, s: p.cstats, xpid: p.xpid })
            })
            .collect();
        players.sort_by_key(|p| p.slot);
        if players.is_empty() {
            return;
        }
        let won = |p: &P| winner == Some(p.team);
        let mvp = players
            .iter()
            .filter(|p| winner.is_none() || won(p))
            .max_by_key(|p| (p.s.score(), std::cmp::Reverse(p.slot)))
            .map(|p| p.slot);
        if let Some(m) = mvp.and_then(|s| players.iter().find(|p| p.slot == s)) {
            let mut b = vec![1u8];
            b.extend_from_slice(&AWARD_MVP_ID.to_le_bytes());
            b.extend_from_slice(&AWARD_MVP_STAT.to_le_bytes());
            b.extend_from_slice(&m.s.score().max(1).to_le_bytes());
            b.extend_from_slice(&(m.slot as u64).to_le_bytes());
            b.extend_from_slice(&m.handle.to_le_bytes());
            b.extend_from_slice(&u16::from(m.team == TEAM_ORANGE).to_le_bytes());
            let mut n = [0u8; 32];
            let k = m.name.len().min(31);
            n[..k].copy_from_slice(&m.name[..k]);
            b.extend_from_slice(&n);
            b.extend_from_slice(&[0, 0]);
            info!(mvp = %String::from_utf8_lossy(&m.name), "combat top awards");
            for a in self.joined() {
                self.reliable_to(a, TOP_AWARDS, &b);
            }
        }
        // Tier rewards.
        let tier_stats: [(&str, fn(&CombatStats) -> f32); 3] = [
            ("CombatEliminations", |s| s.eliminations as f32),
            ("CombatObjectiveEliminations", |s| s.objective_eliminations as f32),
            ("CombatObjectiveTime", |s| s.objective_time),
        ];
        let mut tier_xp: HashMap<usize, u64> = HashMap::new();
        let mut rewards = Vec::new();
        for (tier, xp) in [(TIER_GOLD, 150u64), (TIER_SILVER, 100), (TIER_BRONZE, 50)] {
            for (name, get) in &tier_stats {
                let best = players
                    .iter()
                    // One reward a player: not the MVP, not twice.
                    .filter(|p| mvp != Some(p.slot) && !tier_xp.contains_key(&p.slot) && get(&p.s) > 0.0)
                    .max_by(|a, b| get(&a.s).total_cmp(&get(&b.s)).then(b.slot.cmp(&a.slot)));
                let Some(p) = best else { continue };
                let mut b = [0u8; 32];
                b[0..8].copy_from_slice(&crate::sns::symbol(name).to_le_bytes());
                b[8..16].copy_from_slice(&tier.to_le_bytes());
                b[16..24].copy_from_slice(&(p.slot as u64).to_le_bytes());
                b[24..32].copy_from_slice(&xp.to_le_bytes());
                tier_xp.insert(p.slot, xp);
                rewards.push((p.addr, b));
            }
        }
        for (to, b) in rewards {
            self.reliable_to(to, TIER_REWARD, &b);
        }
        // XP and the profile.
        let longest = players.iter().map(|p| p.s.played).fold(0.0f32, f32::max);
        let (day, week) = profile_periods(crate::store::now_secs());
        for p in &players {
            let stayed = longest > 0.0 && p.s.played >= 0.5 * longest;
            let is_mvp = mvp == Some(p.slot);
            let w = won(p);
            let xp = super::stats::combat_xp(w, stayed, is_mvp, tier_xp.get(&p.slot).copied().unwrap_or(0));
            let mut st = serde_json::Map::new();
            let mut add = |k: &str, v: serde_json::Value| {
                st.insert(k.to_string(), serde_json::json!({"op": "add", "val": v, "cnt": 1}));
            };
            let s = p.s;
            for (k, v) in [
                ("CombatObjectiveDamage", s.objective_damage),
                ("CombatObjectiveTime", s.objective_time),
                ("CombatDamage", s.damage),
                ("CombatDamageTaken", s.damage_taken),
            ] {
                if v > 0.0 {
                    add(k, serde_json::json!(v));
                }
            }
            for (k, v) in [
                ("CombatEliminations", s.eliminations),
                ("CombatObjectiveEliminations", s.objective_eliminations),
                ("CombatKills", s.kills),
                ("CombatAssists", s.assists),
                ("CombatDeaths", s.deaths),
            ] {
                if v > 0 {
                    add(k, serde_json::json!(v));
                }
            }
            if winner.is_some() && stayed {
                add(if w { "CombatWins" } else { "CombatLosses" }, serde_json::json!(1));
                if w {
                    add(if capture { "CombatPointCaptureWins" } else { "CombatPayloadWins" }, serde_json::json!(1));
                }
                add(if capture { "CombatPointCaptureGamesPlayed" } else { "CombatPayloadGamesPlayed" }, serde_json::json!(1));
            }
            if is_mvp {
                add("CombatMVPs", serde_json::json!(1));
            }
            if xp > 0 {
                add("XP", serde_json::json!(xp));
            }
            let stats = serde_json::Value::Object(st);
            let mut groups = serde_json::Map::new();
            groups.insert("combat".into(), stats.clone());
            groups.insert(day.clone(), stats.clone());
            groups.insert(week.clone(), stats);
            let delta = serde_json::json!({ "stats": groups });
            info!(slot = p.slot, xp, won = w, mvp = is_mvp, "combat profile update");
            if let Some([platform, account]) = p.xpid {
                let mut b = platform.to_le_bytes().to_vec();
                b.extend_from_slice(&account.to_le_bytes());
                b.extend_from_slice(delta.to_string().as_bytes());
                b.push(0);
                self.reliable_to(p.addr, REFRESH_PROFILE, &b);
                let id = crate::sns::XpId { platform, account }.token();
                self.fleet_out.push(env(Em::UserServerProfileUpdateRequest(pb::SnsUserServerProfileUpdateRequestMessage {
                    user_id: Some(pb::XPlatformId { id }),
                    // nakama keeps these (its statistics boards) when the
                    // match type is a combat one and the session is its match.
                    payload: serde_json::json!({
                        "matchtype": crate::sns::symbol("echo_combat") as i64,
                        "sessionid": self.info.lobby.to_string(),
                        "update": delta,
                    })
                    .to_string(),
                })));
            }
        }
    }

    /// An arena match is over: each player's totals, win or loss, to the
    /// backend as an `SNSUserServerProfileUpdateRequest` (nakama adds them to
    /// its statistics boards, as it does a combat match's). Native's game
    /// server reports arena stats in post-match remote logs over a login
    /// session of its own, which this server does not have.
    fn send_arena_profile_updates(&mut self) {
        let a = &self.arena;
        let result = if a.private && a.blue_rounds != a.red_rounds { a.blue_rounds.cmp(&a.red_rounds) } else { a.blue.cmp(&a.red) };
        let mut out = Vec::new();
        for p in self.peers.values().filter(|p| p.stage == Stage::Joined && matches!(p.team, TEAM_BLUE | TEAM_ORANGE)) {
            let Some([platform, account]) = p.xpid else { continue };
            let delta = arena_profile_delta(&p.stats, p.team == TEAM_BLUE, result);
            out.push(env(Em::UserServerProfileUpdateRequest(pb::SnsUserServerProfileUpdateRequestMessage {
                user_id: Some(pb::XPlatformId { id: crate::sns::XpId { platform, account }.token() }),
                payload: serde_json::json!({
                    "matchtype": crate::sns::symbol("echo_arena") as i64,
                    "sessionid": self.info.lobby.to_string(),
                    "update": delta,
                })
                .to_string(),
            })));
        }
        info!(players = out.len(), "arena profile updates");
        self.fleet_out.extend(out);
    }

    /// Native, at match over: `SR15NetTopAwardsMsg` = `[count u8]`, then
    /// arrays of `count`: award ids, stat hashes, values (u64 each), slots
    /// (u64), handles (u32), teams (u16), names (32 bytes), and a u16 each.
    /// Native sends four (MVP and three stat awards); ours sent only the MVP
    /// and the end-of-match board showed only that. Then the tier rewards.
    fn send_top_awards(&mut self) {
        struct Pick {
            id: u64,
            stat: u64,
            value: u64,
            slot: usize,
            handle: u32,
            team: u16,
            name: Vec<u8>,
        }
        let players: Vec<(usize, u32, u16, Vec<u8>, Stats)> = self
            .peers
            .values()
            .filter(|p| matches!(p.team, TEAM_BLUE | TEAM_ORANGE))
            .filter_map(|p| {
                let slot = p.slot?;
                let name = p
                    .entrant_header
                    .as_deref()
                    .and_then(|h| h.get(60..96))
                    .map(|n| n.split(|&b| b == 0).next().unwrap_or(&[]).to_vec())
                    .unwrap_or_default();
                let handle = (u32::from(p.generation.unwrap_or(0)) << 16) | slot as u32;
                Some((slot, handle, u16::from(p.team == TEAM_ORANGE), name, p.stats))
            })
            .collect();
        // One award a player (native: every recorded award list names each
        // slot once): the MVP first, then each stat's best among those
        // still without one.
        let mut picks: Vec<Pick> = Vec::new();
        if let Some(p) = players.iter().max_by_key(|p| (p.4.get(Stat::Points), p.4.get(Stat::PossessionTime))) {
            picks.push(Pick { id: AWARD_MVP_ID, stat: AWARD_MVP_STAT, value: u64::from(p.4.get(Stat::Points).max(1)), slot: p.0, handle: p.1, team: p.2, name: p.3.clone() });
        }
        for (id, s) in [(AWARD_GOALS_ID, Stat::Goals), (AWARD_PASSES_ID, Stat::Passes), (AWARD_CATCHES_ID, Stat::Catches)] {
            let best = players.iter().filter(|p| p.4.get(s) > 0 && picks.iter().all(|k| k.slot != p.0)).max_by_key(|p| (p.4.get(s), p.4.get(Stat::Points)));
            if let Some(p) = best {
                picks.push(Pick { id, stat: TEAM_STATS[s as usize], value: u64::from(p.4.get(s)), slot: p.0, handle: p.1, team: p.2, name: p.3.clone() });
            }
        }
        picks.truncate(TOP_AWARDS_MAX);
        if picks.is_empty() {
            return;
        }
        let mut m = vec![picks.len() as u8];
        for p in &picks {
            m.extend_from_slice(&p.id.to_le_bytes());
        }
        for p in &picks {
            m.extend_from_slice(&p.stat.to_le_bytes());
        }
        for p in &picks {
            m.extend_from_slice(&p.value.to_le_bytes());
        }
        for p in &picks {
            m.extend_from_slice(&(p.slot as u64).to_le_bytes());
        }
        for p in &picks {
            m.extend_from_slice(&p.handle.to_le_bytes());
        }
        for p in &picks {
            m.extend_from_slice(&p.team.to_le_bytes());
        }
        for p in &picks {
            let mut n = [0u8; 32];
            let k = p.name.len().min(31);
            n[..k].copy_from_slice(&p.name[..k]);
            m.extend_from_slice(&n);
        }
        m.extend(std::iter::repeat_n(0u8, 2 * picks.len()));
        info!(mvp = %String::from_utf8_lossy(&picks[0].name), awards = picks.len(), "top awards");
        for a in self.joined() {
            self.reliable_to(a, TOP_AWARDS, &m);
        }
        // Tier rewards: gold, silver, bronze (150/100/50 XP) in Goals,
        // Assists and Saves, golds first. One reward a player: nobody who
        // already has a top award, and nobody twice.
        let mut tiers = Vec::new();
        let mut given: Vec<usize> = picks.iter().map(|p| p.slot).collect();
        for (tier, xp) in [(TIER_GOLD, 150u64), (TIER_SILVER, 100), (TIER_BRONZE, 50)] {
            for s in [Stat::Goals, Stat::Assists, Stat::Saves] {
                let best = players.iter().filter(|p| p.4.get(s) > 0 && !given.contains(&p.0)).max_by_key(|p| (p.4.get(s), p.4.get(Stat::Points)));
                let Some(p) = best else { continue };
                let mut b = [0u8; 32];
                b[0..8].copy_from_slice(&TEAM_STATS[s as usize].to_le_bytes());
                b[8..16].copy_from_slice(&tier.to_le_bytes());
                b[16..24].copy_from_slice(&(p.0 as u64).to_le_bytes());
                b[24..32].copy_from_slice(&xp.to_le_bytes());
                given.push(p.0);
                tiers.push((p.0, b));
            }
        }
        for (slot, b) in tiers {
            if let Some(a) = self.slot_addr.get(slot).copied().flatten() {
                self.reliable_to(a, TIER_REWARD, &b);
            }
        }
    }

    // ── Replicated variables ───────────────────────────────────────────────

    fn replicate(&mut self) {
        let Some(base) = baseline(self.info.level, self.info.mode) else { return };
        let frame = self.wire_frame;
        let server_frame = self.frame;
        let arena = self.arena_level;
        // The lobby's vars this server changed (the Fury's engine, the discs'
        // first grabs): on a change, and every second for those who joined since.
        let (lobby_vars, lobby_changed) = if self.social {
            // The combat room's barriers: their vars follow them (native).
            let down = self.gear.barriers_down();
            self.props.set_barriers_down(&down);
            let (v, c) = self.props.vars();
            // Versioned vars keep the baseline's high word; ours is the low one.
            let v = v
                .iter()
                .map(|&(h, x)| {
                    if super::props::LO32_VARS.contains(&h) {
                        (h, (var_raw(&base[0], h).unwrap_or(0) & !0xffff_ffff) | (x & 0xffff_ffff))
                    } else {
                        (h, x)
                    }
                })
                .collect();
            (v, c)
        } else {
            (Vec::new(), false)
        };
        let lobby_due = !lobby_vars.is_empty() && (lobby_changed || server_frame % frame_hz() == 0);
        let a = &self.arena;
        // The arena's own vars; everything in `a.vars` is applied first, so
        // these win (none of them are ever put in `a.vars`).
        let dynamic = |h: u64, old: u64| -> Option<u64> {
            Some(match h {
                VAR_GAME_STATE => a.state,
                VAR_GAME_CLOCK => with_f32(old, a.clock),
                VAR_COUNTDOWN => with_f32(old, a.countdown),
                VAR_BLUE_SCORE => Val::Lo32(a.blue).resolve(old),
                VAR_RED_SCORE => Val::Lo32(a.red).resolve(old),
                VAR_TOTAL_ROUNDS => Val::Lo32(a.total_rounds).resolve(old),
                VAR_BLUE_ROUND_SCORE => Val::Lo32(a.blue_rounds).resolve(old),
                VAR_RED_ROUND_SCORE => Val::Lo32(a.red_rounds).resolve(old),
                VAR_ROUND_COUNTER => Val::Lo32(a.rounds_played).resolve(old),
                VAR_OVERTIME => old & 0xffff_ffff_0000_0000,
                VAR_DISC_OWNER => u64::from(a.disc_owner),
                _ => return None,
            })
        };
        let value = |stream: usize, h: u64, old: u64| -> Option<u64> {
            if stream == 1 {
                return a.vars1.iter().find(|(k, _)| *k == h).map(|(_, v)| v.resolve(old));
            }
            let mut new = a.vars.iter().find(|(k, _)| *k == h).map(|(_, v)| v.resolve(old));
            if let Some(v) = dynamic(h, old) {
                new = Some(v);
            }
            if let Some(&(_, b)) = a.flags.iter().find(|(k, _)| *k == h) {
                new = Some((new.unwrap_or(old) & !0xff) | u64::from(b));
            }
            new
        };
        // Every entrant's own cease fire var (`msg::entrant_hold_var`): a client
        // obeys only its own entrant's, and native sends all of them to every
        // client. Ours sent only `VAR_HOLD` (entrant 0/0's), so every late
        // joiner's client never had a cease fire: stun and block anywhere in
        // the lobby and through arena goals. Lifted (false) only in the
        // brawler room (lobby); the arena's goal hold is everyone's.
        let entrants: Vec<(u64, bool)> = self
            .peers
            .iter()
            .filter(|(_, p)| p.stage == Stage::Joined)
            .filter_map(|(addr, p)| {
                let slot = p.slot?;
                let var = msg::entrant_hold_var(p.genidx, slot);
                if var.is_none() {
                    debug!(%addr, slot, genidx = p.genidx, "no cease fire var name for this entrant");
                }
                Some((var?, !self.social || self.gear.may_fight(*addr)))
            })
            .collect();
        let tracked: Vec<(u64, u64)> = if arena {
            let raw = |h| var_raw(&base[0], h).unwrap_or(0);
            let mut hs = vec![VAR_GAME_STATE, VAR_BLUE_SCORE, VAR_RED_SCORE];
            if a.private {
                hs.extend([VAR_GAME_CLOCK, VAR_COUNTDOWN, VAR_TOTAL_ROUNDS, VAR_BLUE_ROUND_SCORE, VAR_RED_ROUND_SCORE, VAR_ROUND_COUNTER]);
            } else if a.clock_runs() {
                hs.push(VAR_GAME_CLOCK);
            } else {
                hs.push(VAR_COUNTDOWN);
            }
            // Native publishes the disc owner only once play has begun.
            if matches!(a.phase, Phase::Playing | Phase::Goal | Phase::Over) {
                hs.push(VAR_DISC_OWNER);
            }
            for h in a.flags.iter().map(|e| e.0).chain(a.vars.iter().map(|e| e.0)) {
                if !hs.contains(&h) {
                    hs.push(h);
                }
            }
            let mut t: Vec<(u64, u64)> = hs.into_iter().filter_map(|h| value(0, h, raw(h)).map(|v| (h, v))).collect();
            if let Some(&(_, hold)) = t.iter().find(|(h, _)| *h == VAR_HOLD) {
                for &(var, _) in &entrants {
                    if var != VAR_HOLD {
                        t.push((var, hold));
                    }
                }
            }
            t
        } else {
            Vec::new()
        };
        let tracked1: Vec<(u64, u64)> = if arena && server_frame <= a.s1_until {
            a.vars1.iter().map(|&(h, v)| (h, v.resolve(var_raw(&base[1], h).unwrap_or(0)))).collect()
        } else {
            Vec::new()
        };
        let fresh = |payload: &[u8], stream: usize| -> Vec<u8> {
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
                if let Some(v) = value(stream, h, old) {
                    p[o + 8..o + 16].copy_from_slice(&v.to_le_bytes());
                }
            }
            p
        };
        let delta = |header: &[u8], vars: &[(u64, u64)]| -> Vec<u8> {
            let mut body = Vec::with_capacity(30 + vars.len() * 16);
            body.extend_from_slice(&header[..24]);
            body.extend_from_slice(&[0; 4]);
            body.extend_from_slice(&(vars.len() as u16).to_le_bytes());
            for (h, v) in vars {
                body.extend_from_slice(&h.to_le_bytes());
                body.extend_from_slice(&v.to_le_bytes());
            }
            body
        };
        // The sequence is one lobby-wide clock (native: frames since the
        // session began, the same for every player; a late joiner's first
        // snapshot came at 0xbdf, not at the first player's 0x13a). Ours
        // counted per player from the captured start, so a late joiner's
        // updates looked as old as the lobby's first ones and its client
        // ignored them: the cease fire (`VAR_HOLD`) never reached it, in the
        // lobby or on an arena goal, and it could stun and block anywhere.
        let stamp = |p: &mut Vec<u8>, seq_off: u64| {
            if p.len() >= 24 {
                let seq = u64::from_le_bytes(p[16..24].try_into().expect("8")).wrapping_add(seq_off);
                p[16..24].copy_from_slice(&seq.to_le_bytes());
                if p.len() >= 28 {
                    p[24..28].copy_from_slice(&(seq as u32).to_le_bytes());
                }
            }
        };
        let mut fulls: Vec<Vec<u8>> = base.iter().enumerate().map(|(i, b)| fresh(b, i)).collect();
        // The lobby's cease fire (`VAR_HOLD`), per player: lifted (0) only in
        // the brawler room, where the client then lets them stun and block.
        // Native's version in the high word (`LOBBY_HOLD_VERSION`), baseline included.
        let holds: Vec<(u64, u64)> = if self.social {
            entrants.iter().map(|&(var, fight)| (var, msg::LOBBY_HOLD_VERSION | u64::from(!fight))).collect()
        } else {
            Vec::new()
        };
        if let Some(f) = fulls.first_mut() {
            if self.social {
                *f = msg::with_var(f, VAR_HOLD, msg::LOBBY_HOLD_VERSION | 1);
                for &(var, v) in &holds {
                    *f = msg::with_var_added(f, var, v);
                }
            } else if arena {
                // The goal hold, for every entrant (`tracked` has them).
                for &(var, _) in &entrants {
                    if let Some(&(_, v)) = tracked.iter().find(|(h, _)| *h == var) {
                        *f = msg::with_var_added(f, var, v);
                    }
                }
            }
        }
        let gear = &self.gear;
        // The combat mode's vars: those that changed every frame, all of them
        // twice a second (late joiners, lost packets).
        let combat_vars = self.combat.as_ref().map(|c| {
            let all: Vec<(u64, u64)> = c.vars().into_iter().map(|(h, v)| (h, v.resolve(var_raw(&base[0], h).unwrap_or(0)))).collect();
            let every = server_frame % (frame_hz() / 2).max(1) == 0;
            let changed: Vec<(u64, u64)> = all.iter().copied().filter(|(h, v)| every || self.combat_sent.get(h) != Some(v)).collect();
            (all, changed)
        });
        if let Some((all, _)) = &combat_vars {
            self.combat_sent = all.iter().copied().collect();
        }
        let second = server_frame % frame_hz() == 0;
        for (addr, p) in self.peers.iter_mut() {
            if p.stage != Stage::Joined || !p.rep_ready {
                continue;
            }
            let slot = p.slot.unwrap_or(0) as u64;
            let full = !p.rep_sent || (!arena && !p.rep_acked && server_frame % (frame_hz() / 2) == 0);
            if full {
                for (stream, b) in fulls.iter().enumerate() {
                    let mut body = b.clone();
                    stamp(&mut body, server_frame);
                    p.t.queue_mailbox(REPLICATED_VARS, key_replicated(stream, frame, slot), &body);
                }
                p.rep_sent = true;
                p.rep_seq += 1;
                // The baseline has the cease fire on: send ours again next frame.
                p.fight_sent = None;
                p.holds_sent = None;
            } else if arena {
                // Every frame: the clock/countdown plus all tracked vars (tiny).
                let mut body = delta(&base[0], &tracked);
                stamp(&mut body, server_frame);
                p.t.queue_mailbox(REPLICATED_VARS, key_replicated(0, frame, slot), &body);
                if !tracked1.is_empty() && base.len() > 1 {
                    let mut body = delta(&base[1], &tracked1);
                    stamp(&mut body, server_frame);
                    p.t.queue_mailbox(REPLICATED_VARS, key_replicated(1, frame, slot), &body);
                }
                p.rep_seq += 1;
            } else if let Some((_, vars)) = combat_vars.as_ref() {
                let mut body = delta(&base[0], vars);
                stamp(&mut body, server_frame);
                p.t.queue_mailbox(REPLICATED_VARS, key_replicated(0, frame, slot), &body);
                p.rep_seq += 1;
            } else if self.social {
                let fight = gear.may_fight(*addr);
                if p.fight_sent != Some(fight) {
                    info!(%addr, fight, slot = ?p.slot, genidx = p.genidx, pos = ?gear.body_pos(*addr), "lobby cease fire {}", if fight { "lifted" } else { "on" });
                }
                let changed = p.holds_sent.as_ref() != Some(&holds);
                if lobby_due || changed || second {
                    let mut vars = lobby_vars.clone();
                    vars.extend_from_slice(&holds);
                    let mut body = delta(&base[0], &vars);
                    stamp(&mut body, server_frame);
                    p.t.queue_mailbox(REPLICATED_VARS, key_replicated(0, frame, slot), &body);
                    p.rep_seq += 1;
                    p.fight_sent = Some(fight);
                    p.holds_sent = Some(holds.clone());
                } else if base[0].len() >= 24 {
                    // Native (netrec 2026-11-18 21:05, both players): one every
                    // frame, a 24-byte header when nothing changed, so the
                    // sequence and the mailbox keys run on without gaps.
                    let mut body = base[0][..24].to_vec();
                    stamp(&mut body, server_frame);
                    p.t.queue_mailbox(REPLICATED_VARS, key_replicated(0, frame, slot), &body);
                    p.rep_seq += 1;
                }
            }
        }
    }

    /// The quarter court (`props`): a goal's score update to everyone, and
    /// the round start 3 s on, as native's server sends them.
    fn lobby_goal(&mut self) {
        if let Some(g) = self.props.take_goal() {
            let handle = self.peers.get(&g.by).and_then(|p| Some((u32::from(p.generation.unwrap_or(p.genidx)) << 16) | p.slot? as u32));
            let mut rec = [0u8; 28];
            rec[0..2].copy_from_slice(&1u16.to_le_bytes());
            rec[2..4].copy_from_slice(&g.points.to_le_bytes());
            rec[4..6].copy_from_slice(&g.goal_type.to_le_bytes());
            rec[6..10].copy_from_slice(&handle.unwrap_or(u32::MAX).to_le_bytes());
            rec[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
            rec[16..20].copy_from_slice(&g.speed.to_le_bytes());
            rec[20..24].copy_from_slice(&g.distance.to_le_bytes());
            rec[24..28].copy_from_slice(&self.started.elapsed().as_secs_f32().to_le_bytes());
            info!(by = %g.by, handle = ?handle, points = g.points, speed = g.speed, distance = g.distance, "quarter court goal: team 1");
            for a in self.joined() {
                self.reliable_to(a, POST_SCORE, &rec);
            }
        }
        if self.props.take_round_start() {
            let ev = component_event(msg::EV_ENABLE_SCORING, msg::MOD_LOBBY_SCORING, msg::LOBBY_SCORING_HANDLE, FILTER_ALL);
            for a in self.joined() {
                self.reliable_to(a, COMPONENT_EVENT, &ev);
            }
        }
    }

    // ── Private match: terminal rules, buttons, teams ──────────────────────

    fn send_event(&mut self, sym: u64, module: u64) {
        let ev = component_event(sym, module, self.handles.for_module(module), FILTER_ALL);
        for a in self.joined() {
            self.reliable_to(a, COMPONENT_EVENT, &ev);
        }
    }

    /// `SR15NetUIIncSettingMsg` / `SR15NetUIDecSettingMsg` from the terminal.
    fn on_rules_press(&mut self, from: SocketAddr, d: &[u8], inc: bool) {
        if !self.arena.private {
            debug!(%from, "rules press outside a private match");
            return;
        }
        let Some((page, row)) = rules::parse_press(d) else { return };
        // Native echoes every press, clamped or not, stamped with its clock.
        let mut echo = d[..24].to_vec();
        echo.extend_from_slice(&wire_clock().1.to_le_bytes());
        for a in self.joined() {
            self.reliable_to(a, UI_CHANGED_SETTINGS, &echo);
        }
        let Some(i) = Rules::index(page, row) else {
            debug!(page, row, "unknown rules row");
            return;
        };
        let changed = self.arena.rules.press(i, inc);
        if changed {
            self.apply_rule(i);
        }
        self.send_event(EV_SETTING_CHANGED, MOD_MATCH);
        if (page, row) == (1, 2) {
            self.send_event(EV_RULES_PANEL_1_2, MOD_RULES_PANEL);
        }
        info!(page, row, inc, value = self.arena.rules.v[i], changed, "rules press");
    }

    /// A rule set through the client's HTTP API. The client sends its whole
    /// rule set, repeatedly, stepping changed values one at a time; each
    /// one is applied outright.
    fn on_api_rule(&mut self, from: SocketAddr, d: &[u8]) {
        if !self.arena.private {
            return;
        }
        let Some((i, v)) = rules::parse_api(d) else { return };
        if !self.arena.rules.set(i, v) {
            return;
        }
        self.apply_rule(i);
        self.send_event(EV_SETTING_CHANGED, MOD_MATCH);
        if i == rules::ROW_RESPAWN_TIME {
            self.send_event(EV_RULES_PANEL_1_2, MOD_RULES_PANEL);
        }
        info!(%from, rule = rules::API_NAMES[i], value = self.arena.rules.v[i], "rules set (API)");
    }

    /// Publish rule `i` (setting var and what it drives).
    fn apply_rule(&mut self, i: usize) {
        let a = &mut self.arena;
        let v = a.rules.v[i];
        a.set_var1(RULES[i].var, a.rules.s1_value(i));
        for (h, val) in a.rules.s0_values(i) {
            a.set_var(h, val);
        }
        match RULES[i].target {
            Target::RoundTime if a.phase == Phase::Waiting => {
                a.clock = a.rules.round_secs();
                a.round_clock = a.clock;
            }
            Target::Score(true, _) => a.blue = v as u32,
            Target::Score(false, _) => a.red = v as u32,
            Target::DiscSpawn => a.set_flag(VAR_DISC_SPAWN, rules::disc_spawn(v)),
            Target::TotalRounds => a.total_rounds = v as u32,
            Target::RoundsWon(true) => a.blue_rounds = v as u32,
            Target::RoundsWon(false) => a.red_rounds = v as u32,
            _ => {}
        }
        a.s1_until = self.frame + frame_hz();
    }

    /// Component events a client sends: the private terminal's buttons.
    fn on_client_event(&mut self, from: SocketAddr, d: &[u8]) {
        if d.len() < 18 {
            return;
        }
        let sym = u64::from_le_bytes(d[0..8].try_into().expect("8"));
        let module = u64::from_le_bytes(d[8..16].try_into().expect("8"));
        if let Some(c) = self.combat.as_mut()
            && c.is_private()
            && matches!(sym, BTN_READY_BLUE | BTN_READY_ORANGE)
        {
            // A private combat match's terminal: the team is ready.
            let team = usize::from(sym == BTN_READY_ORANGE);
            if c.team_ready(team) {
                info!(%from, team, "private combat match: team ready");
            }
            return;
        }
        if self.combat.is_some() {
            // Native relays a client's filter-13 events (the war room, doors,
            // barriers: the level scripts each client runs) to the others,
            // byte for byte; filter-5 ones are for the server alone.
            let filter = if d.len() >= 20 { u16::from_le_bytes([d[18], d[19]]) } else { 0 };
            if d.len() == 24 && filter == FILTER_ALL_ALT {
                self.relay_reliable(from, COMPONENT_EVENT, d, false);
            }
            // The level scripts' reaction to the server-only ones (native's
            // doors, the war room's objects, area triggers set level vars).
            if filter == FILTER_ALL
                && let Some(c) = self.combat.as_mut()
            {
                c.client_event(sym, module, u16::from_le_bytes([d[16], d[17]]));
            }
            return;
        }
        if self.social {
            // Walking into / out of the brawler room.
            for o in self.gear.area_event(from, sym, Instant::now()) {
                self.send_gear(from, o);
            }
            // The lobby's toys: the Fury controller's trigger.
            self.props.on_event(from, sym, u16::from_le_bytes([d[16], d[17]]));
            // Like combat (native relays a client's filter-13 events, the
            // level scripts each client runs, byte for byte to the others):
            // the lobby's scripts (punch zone, cease fire) sync through them
            // too. Ours dropped every one in the lobby, so a client's script
            // never told the others, and only the first player behaved.
            let filter = if d.len() >= 20 { u16::from_le_bytes([d[18], d[19]]) } else { 0 };
            info!(%from, sym = format_args!("{sym:#018x}"), module = format_args!("{module:#018x}"), handle = u16::from_le_bytes([d[16], d[17]]), filter, "lobby client event");
            if d.len() == 24 && filter == FILTER_ALL_ALT {
                self.relay_reliable(from, COMPONENT_EVENT, d, false);
            }
            return;
        }
        if !self.arena.private {
            return;
        }
        // The terminal sends these on its own module; the client's HTTP API
        // may use another, so any module is accepted (and logged).
        if module != MOD_RULES_PANEL {
            info!(%from, sym = format_args!("{sym:#018x}"), module = format_args!("{module:#018x}"), "client event off the terminal module");
        }
        // Each button names its team (orange's or blue's terminal, or the
        // API on a team's behalf), so a moderator or spectator can use them.
        let side = match sym {
            BTN_READY_BLUE | BTN_RESTART_BLUE | BTN_PAUSE_BLUE => Some(0),
            BTN_READY_ORANGE | BTN_RESTART_ORANGE | BTN_PAUSE => Some(1),
            _ => None,
        };
        let team = match side {
            Some(0) => TEAM_BLUE,
            Some(_) => TEAM_ORANGE,
            None => self.peers.get(&from).map(|p| p.team).unwrap_or(TEAM_SPECTATOR),
        };
        match sym {
            BTN_READY_ORANGE | BTN_READY_BLUE => {
                let Some(i) = side else { return };
                if self.arena.phase != Phase::Waiting || self.arena.ready[i] {
                    return;
                }
                self.arena.ready[i] = true;
                self.arena.set_var(if i == 0 { VAR_BLUE_READY } else { VAR_ORANGE_READY }, Val::Lo8(1));
                info!(%from, team, button = format_args!("{sym:#018x}"), "private match: team ready");
                self.check_team_requests();
            }
            BTN_PAUSE | BTN_PAUSE_BLUE => self.toggle_pause(team),
            // Rematch: the whole private match again, in place.
            BTN_REMATCH if self.arena.phase == Phase::Over => {
                info!(%from, "private match: rematch");
                self.arena.private_restart(self.arena.tick, true);
            }
            EV_CLIENT_AT_CELEBRATION => debug!(%from, "client reached the post-match celebration"),
            // Lobby: only this player leaves (`nevt_return_to_lobby`).
            BTN_LOBBY => {
                info!(%from, "private match: player returns to the lobby");
                let ev = component_event(EV_RETURN_TO_LOBBY, MOD_MATCH, self.handles.for_module(MOD_MATCH), FILTER_ALL);
                self.reliable_to(from, COMPONENT_EVENT, &ev);
            }
            BTN_RESTART_ORANGE | BTN_RESTART_BLUE => {
                let Some(i) = side else { return };
                self.arena.restart_req[i] = true;
                self.arena.set_var(if i == 0 { VAR_BLUE_RESTART_REQUEST } else { VAR_ORANGE_RESTART_REQUEST }, Val::Lo8(1));
                info!(%from, team, button = format_args!("{sym:#018x}"), "private match: restart requested");
                self.check_team_requests();
            }
            _ => info!(%from, sym = format_args!("{sym:#018x}"), "unknown terminal button"),
        }
    }

    /// Run a ready or restart once every team with players has asked (an
    /// empty team doesn't hold the other up).
    fn check_team_requests(&mut self) {
        if !self.arena.private {
            return;
        }
        let (b, o) = self.team_counts();
        let all = |req: [bool; 2]| (b > 0 || o > 0) && (b == 0 || req[0]) && (o == 0 || req[1]);
        let f = self.arena.tick;
        if all(self.arena.restart_req) {
            info!("private match: restart");
            self.arena.restart_req = [false; 2];
            self.arena.paused = false;
            self.arena.resume = 0.0;
            self.arena.private_restart(f, false);
        } else if self.arena.phase == Phase::Waiting && all(self.arena.ready) {
            info!("private match: all teams ready, start");
            self.arena.private_start(f);
        }
    }

    fn toggle_pause(&mut self, team: u8) {
        let who = match team {
            TEAM_BLUE => SYM_BLUE,
            TEAM_ORANGE => SYM_ORANGE,
            _ => SYM_NONE,
        };
        let a = &mut self.arena;
        if !a.paused {
            info!(team, "private match: pause");
            a.paused = true;
            a.resume = 0.0;
            a.pause_time = 0.0;
            a.pre_pause_flags = a.flags.clone();
            a.set_var(VAR_PAUSED_BY, Val::Raw(who));
            a.set_var(VAR_PAUSE_STATE, Val::Raw(SYM_PAUSING));
            a.soon.push(Act::Var(VAR_PAUSE_STATE, Val::Raw(SYM_PAUSED)));
            a.soon.push(Act::Var(VAR_RESUME_COUNT, Val::F32(RESUME_SECS)));
            a.soon.push(Act::Flag(VAR_ROUND_BEGUN, 1));
            a.soon.push(Act::Flag(VAR_LAUNCH_GATES, 1));
            self.send_event(EV_PAUSED, MOD_MATCH);
        } else {
            info!(team, "private match: unpause");
            a.paused = false;
            a.resume = RESUME_SECS;
            a.set_var(VAR_UNPAUSED_BY, Val::Raw(who));
            a.set_var(VAR_PAUSE_STATE, Val::Raw(SYM_UNPAUSED));
            let restore = |flags: &[(u64, u8)], h| flags.iter().find(|(k, _)| *k == h).map(|e| e.1);
            let begun = restore(&a.pre_pause_flags, VAR_ROUND_BEGUN).unwrap_or(1);
            let gates = restore(&a.pre_pause_flags, VAR_LAUNCH_GATES).unwrap_or(1);
            a.set_flag(VAR_ROUND_BEGUN, begun);
            a.soon.push(Act::Flag(VAR_LAUNCH_GATES, gates));
            a.soon.push(Act::Event(EV_REALLY_RESTART, u64::MAX, FILTER_ALL));
            self.send_event(EV_ROUND_RESTART, MOD_MATCH);
        }
    }

    /// Restart: rules, scores and vars back to the baseline, everyone gets
    /// the full var set again.
    fn private_reset(&mut self, keep_rules: bool) {
        let a = &mut self.arena;
        let schedule = std::mem::take(&mut a.schedule);
        let mut fresh = Arena::new(a.min_per_team, true);
        fresh.schedule = schedule;
        fresh.tick = a.tick;
        fresh.countdown = a.countdown;
        // The disc goes free, as at the first start; the restart walk
        // respawns it in the centre (`evt_restart_disc`, spawn 2).
        a.bump_resets();
        fresh.disc_resets = a.disc_resets;
        // The server publishes the disc (at the centre, with the bumped
        // count) until a client has snapped to it and grabs it; otherwise
        // the client keeps its old disc wherever the match ended.
        fresh.server_disc = true;
        let rules = a.rules.clone();
        *a = fresh;
        for p in self.peers.values_mut() {
            p.rep_sent = false;
            p.stats = Stats::default();
        }
        if keep_rules {
            // Rematch: the same rules again (re-derive what each one sets).
            self.arena.rules = rules;
            for i in 0..RULES.len() {
                if self.arena.rules.v[i] != RULES[i].default {
                    self.apply_rule(i);
                }
            }
        }
    }

    /// `SR15NetUserAssignTeamRequest` `[slot u32][team u16]`.
    fn on_assign_team(&mut self, from: SocketAddr, d: &[u8]) {
        if d.len() < 6 {
            return;
        }
        let want = u8::try_from(u16_at(d, 4)).unwrap_or(u8::MAX);
        let Some(p) = self.peers.get(&from) else { return };
        let (Some(slot), old) = (p.slot, p.team) else { return };
        let full = |s: &Self, t: u8| s.peers.values().filter(|q| q.stage == Stage::Joined && q.team == t).count() >= 8;
        let ok = match want {
            TEAM_BLUE | TEAM_ORANGE => !full(self, want),
            TEAM_SPECTATOR | TEAM_MODERATOR => true,
            // The social lobby's combat area: back to no team.
            TEAM_SOCIAL => self.social,
            _ => false,
        };
        let team = if ok && want != old {
            let used: Vec<u16> = self.peers.values().filter(|q| q.stage == Stage::Joined && q.team == want).map(|q| q.team_slot).collect();
            let team_slot = (0..).find(|s| !used.contains(s)).unwrap_or(0);
            let p = self.peers.get_mut(&from).expect("present");
            p.team = want;
            p.team_slot = team_slot;
            p.requested_team = Some(want);
            p.team_changed_at = Some(Instant::now());
            let tc = team_changed(slot, want, Some(old));
            for a in self.joined() {
                self.reliable_to(a, ENTRANT_TEAM_CHANGED, &tc);
            }
            self.roster_dirty = true;
            info!(%from, slot, old, team = want, "team change");
            want
        } else {
            debug!(%from, slot, old, want, "team change refused");
            old
        };
        self.reliable_to(from, ASSIGN_TEAM_RESPONSE, &[team, 2]);
        if team != old {
            self.publish_owner_state();
        }
    }
}

const RESUME_SECS: f32 = 14.5;

/// Where the server holds the disc for a `VAR_DISC_SPAWN` value. Native: 2 at
/// the centre, 1 (after blue scored) at (0, 4536, 27500) on orange's side;
/// 0 is taken as the mirror on blue's side.
fn disc_spawn_pos(spawn: u8) -> (i32, i32, i32) {
    match spawn {
        1 => (0, 4536, 27_500),
        0 => (0, 4536, -27_500),
        _ => (0, 0, 0),
    }
}

/// `SNSLobbyEntrantTeamChanged`: `[slot u64][new u16][old u16][u32 0]`.
fn team_changed(slot: usize, team: u8, old: Option<u8>) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..8].copy_from_slice(&(slot as u64).to_le_bytes());
    b[8..10].copy_from_slice(&u16::from(team).to_le_bytes());
    b[10..12].copy_from_slice(&old.map_or(u16::MAX, u16::from).to_le_bytes());
    b
}

#[cfg(test)]
mod server_mute_tests {
    use super::*;
    use crate::engine::transport::SessionCrypto;

    fn joined_peer(entrant: Uuid) -> Peer {
        let c = SessionCrypto {
            out_flags: 0,
            out_aes: [0; 32],
            out_mac: [0; 32],
            out_rnd: [0; 32],
            out_seq: 0,
            in_flags: 0,
            in_aes: [0; 32],
            in_mac: [0; 32],
            in_rnd: [0; 32],
            in_seq: 0,
        };
        let mut p = Peer::new(Transport::new(&c));
        p.stage = Stage::Joined;
        p.entrant_id = entrant;
        p
    }

    /// A server-muted peer's voice reaches nobody; others still hear each
    /// other, and lifting the mute lets it through again.
    #[test]
    fn a_muted_peers_voice_is_not_relayed() {
        let info = SessionInfo { lobby: Uuid::nil(), mode: 0, level: msg::LEVEL_SOCIAL };
        let mut s = Session::new(info, 1);
        let (a, b): (SocketAddr, SocketAddr) = ("10.0.0.1:1".parse().unwrap(), "10.0.0.2:1".parse().unwrap());
        let (ea, eb) = (Uuid::from_u128(1), Uuid::from_u128(2));
        s.peers.insert(a, joined_peer(ea));
        s.peers.insert(b, joined_peer(eb));
        let heard = |s: &Session, at: SocketAddr| s.peers[&at].t.queued_mailbox.iter().filter(|m| m.0 == VOIP).count();

        assert_eq!(s.set_muted(&[ea], true), 1);
        assert_eq!(s.set_muted(&[ea], true), 0, "muting twice changes nothing");
        s.on_mailbox(a, VOIP, 0, &[1, 2, 3]);
        assert_eq!(heard(&s, b), 0, "muted voice dropped");
        s.on_mailbox(b, VOIP, 0, &[1, 2, 3]);
        assert_eq!(heard(&s, a), 1, "the muted player still hears others");

        assert_eq!(s.set_muted(&[ea], false), 1);
        s.on_mailbox(a, VOIP, 0, &[1, 2, 3]);
        assert_eq!(heard(&s, b), 1, "lifted mute relays again");
    }

    /// A mute can arrive before the entrant's peer exists; it applies once
    /// that entrant joins.
    /// Lobby: a client's filter-13 event (its level scripts) reaches the
    /// other players byte for byte, not the sender; filter 5 stays here.
    #[test]
    fn lobby_relays_filter_13_client_events_to_the_others() {
        let info = SessionInfo { lobby: Uuid::nil(), mode: 0, level: msg::LEVEL_SOCIAL };
        let mut s = Session::new(info, 1);
        let (a, b): (SocketAddr, SocketAddr) = ("10.0.0.1:1".parse().unwrap(), "10.0.0.2:1".parse().unwrap());
        s.peers.insert(a, joined_peer(Uuid::from_u128(1)));
        s.peers.insert(b, joined_peer(Uuid::from_u128(2)));
        let events = |s: &Session, at: SocketAddr| s.peers[&at].t.queued_reliable.iter().filter(|m| m.0 == COMPONENT_EVENT).map(|m| m.1.clone()).collect::<Vec<_>>();
        let ev13 = hex_bytes("7a5aa1f228cc059cc4b6bab2ee99ea6a6c000d0000000000");
        let ev5 = hex_bytes("c06471ffde095d63c4b6bab2ee99ea6a6c00050000000000");
        s.on_client_event(a, &ev13);
        s.on_client_event(a, &ev5);
        assert_eq!(events(&s, b), vec![ev13.clone()], "filter 13 relayed as is, filter 5 not");
        assert!(events(&s, a).is_empty(), "not back to the sender");
    }

    /// Native's lobby entrant data (netrec-lobby-native-20261118, slot-4
    /// joiner; netrec-native-srv-1201): the newcomer gets its own header, its
    /// empty slot entry and each earlier entrant's slot entry and data; a
    /// client's own data goes to everyone, the sender too, under its slot,
    /// without the client's 2-byte trailer.
    #[test]
    fn lobby_entrant_data_matches_native() {
        let info = SessionInfo { lobby: Uuid::nil(), mode: 0, level: msg::LEVEL_SOCIAL };
        let mut s = Session::new(info, 1);
        let (a, b): (SocketAddr, SocketAddr) = ("10.0.0.1:1".parse().unwrap(), "10.0.0.2:1".parse().unwrap());
        let mut pa = joined_peer(Uuid::from_u128(1));
        pa.slot = Some(0);
        s.peers.insert(a, pa);
        let data = |s: &Session, at: SocketAddr| s.peers[&at].t.queued_reliable.iter().filter(|m| m.0 == ENTRANT_DATA).map(|m| m.1.clone()).collect::<Vec<_>>();
        s.on_entrant_data(a, &hex_bytes("0000000000000000e1e491a8b1d0e8c87d061f480cf31c191528a176ce52e9700000"));
        let echo = hex_bytes("0000000000000000e1e491a8b1d0e8c87d061f480cf31c191528a176ce52e970");
        assert_eq!(data(&s, a), vec![echo.clone()], "the sender hears its own, trailer dropped");
        let mut pb = joined_peer(Uuid::from_u128(2));
        pb.slot = Some(4);
        s.peers.insert(b, pb);
        s.send_entrant_data_on_join(b);
        let got = data(&s, b);
        assert_eq!(ENTRANT_DATA, 0x8f05_b673_5e91_6658);
        assert_eq!(got[0].len(), 80);
        assert_eq!(&got[0][..16], &hex_bytes("0400000000000000cc24f15e6e1a0fb1")[..]);
        assert_eq!(got[1], hex_bytes("0400000000000000c332fe567a1e19a0"));
        assert_eq!(got[2], hex_bytes("0400000000000000e1e491a8b1d0e8c87d061f480cf31c191528a176ce52e970"), "native's own afk");
        assert_eq!(got[3], hex_bytes("0000000000000000c332fe567a1e19a00000"));
        assert_eq!(got[4], echo);
        let to_a = data(&s, a);
        assert_eq!(to_a[to_a.len() - 2], hex_bytes("0400000000000000c332fe567a1e19a00400"));
    }

    /// Native: a newcomer hears nothing of the others' bodies or gear until
    /// its own first equipment update; then their equipment, `spawned`.
    #[test]
    fn late_joiner_gets_bodies_only_after_its_spawn() {
        let info = SessionInfo { lobby: Uuid::nil(), mode: 0, level: msg::LEVEL_SOCIAL };
        let mut s = Session::new(info, 1);
        let (a, b): (SocketAddr, SocketAddr) = ("10.0.0.1:1".parse().unwrap(), "10.0.0.2:1".parse().unwrap());
        let mut pa = joined_peer(Uuid::from_u128(1));
        pa.slot = Some(0);
        pa.equipment = Some(vec![1; 56]);
        let mut pb = joined_peer(Uuid::from_u128(2));
        pb.slot = Some(1);
        pb.joined_at = Some(Instant::now());
        s.peers.insert(a, pa);
        s.peers.insert(b, pb);
        s.relay_mailbox(a, PLAYER_STATE, 5, &[0; 64]);
        s.relay_mailbox(a, PHYSICS, 5, &[0; 64]);
        assert!(s.peers[&b].t.queued_mailbox.is_empty(), "no bodies before the spawn");
        s.on_equipment(a, &[2; 56]);
        assert!(!s.peers[&b].t.queued_reliable.iter().any(|m| m.0 == EQUIPMENT), "no gear before the spawn");
        s.on_equipment(b, &[3; 56]);
        let rel = &s.peers[&b].t.queued_reliable;
        assert!(rel.iter().any(|m| m.0 == EQUIPMENT && m.1 == vec![2; 56]), "the others' gear at the spawn");
        assert!(rel.iter().any(|m| m.0 == ENTRANT_DATA && m.1 == hex_bytes("01000000000000008e527a499e9fc42f")));
        assert!(rel.iter().any(|m| m.0 == ENTRANT_DATA && m.1 == hex_bytes("01000000000000007d061f480cf31c19c4bedaed35d9f32d")));
        s.relay_mailbox(a, PLAYER_STATE, 5, &[0; 64]);
        assert_eq!(s.peers[&b].t.queued_mailbox.len(), 1);
    }

    fn hex_bytes(h: &str) -> Vec<u8> {
        (0..h.len() / 2).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn a_mute_before_join_applies_on_join() {
        let info = SessionInfo { lobby: Uuid::nil(), mode: 0, level: msg::LEVEL_SOCIAL };
        let mut s = Session::new(info, 1);
        let ea = Uuid::from_u128(7);
        s.set_muted(&[ea], true);
        let (a, b): (SocketAddr, SocketAddr) = ("10.0.0.1:1".parse().unwrap(), "10.0.0.2:1".parse().unwrap());
        s.peers.insert(a, joined_peer(ea));
        s.peers.insert(b, joined_peer(Uuid::from_u128(8)));
        s.on_mailbox(a, VOIP, 0, &[9]);
        assert!(s.peers[&b].t.queued_mailbox.is_empty());
    }
}

#[cfg(test)]
mod arena_profile_tests {
    use super::*;

    /// The arena update is what nakama's `ArenaStatistics` reads: each stat an
    /// `{op, val, cnt}`, the result, a game played.
    #[test]
    fn arena_profile_delta_has_natives_stat_shape() {
        let mut s = Stats::default();
        s.add(Stat::Goals, 2);
        s.add(Stat::Saves, 3);
        let d = arena_profile_delta(&s, true, std::cmp::Ordering::Greater);
        let a = &d["stats"]["arena"];
        assert_eq!(a["Goals"], serde_json::json!({"op": "add", "val": 2, "cnt": 1}));
        assert_eq!(a["HighestSaves"]["op"], "max");
        assert_eq!(a["ArenaWins"]["val"], 1);
        assert_eq!(a["GamesPlayed"]["val"], 1);
        assert!(a.get("Stuns").is_none(), "zeros left out");
        let lost = arena_profile_delta(&s, false, std::cmp::Ordering::Greater);
        assert_eq!(lost["stats"]["arena"]["ArenaLosses"]["val"], 1);
        let tie = arena_profile_delta(&s, false, std::cmp::Ordering::Equal);
        assert_eq!(tie["stats"]["arena"]["ArenaTies"]["val"], 1);
    }

    #[test]
    fn nakama_url_gets_the_save_loadout_feature() {
        assert_eq!(crate::upstream::with_features_for_test("ws://h/ws?a=1"), "ws://h/ws?a=1&features=save_loadout,server_mute");
        assert_eq!(crate::upstream::with_features_for_test("ws://h/ws"), "ws://h/ws?features=save_loadout,server_mute");
        assert_eq!(crate::upstream::with_features_for_test("ws://h/ws?features=x"), "ws://h/ws?features=x");
    }
}

#[cfg(test)]
mod arena_end_tests {
    use super::*;

    /// Time up while a player's client flies the disc: the server pins it and
    /// moves the reset count on, so the holder's client snaps to the pin too.
    #[test]
    fn time_up_pin_moves_every_client_to_the_server_disc() {
        let mut a = Arena::new(1, false);
        a.server_disc = false;
        a.last_disc = Some((10, 20, 30));
        a.disc_owner = 3;
        let before = a.disc_resets;
        a.pin_disc();
        assert_eq!(a.frozen, Some((10, 20, 30)));
        assert!(a.server_disc);
        assert_ne!(a.disc_resets, before, "reset count moved past the holder's");
        assert_eq!(a.disc_owner, u32::MAX);
        // Already the server's (after a goal): no second bump.
        let r = a.disc_resets;
        a.pin_disc();
        assert_eq!(a.disc_resets, r);
    }

    /// Public, time up tied: sudden death. pre_sudden_death with the
    /// overtime call, a 2:00 clock, `sudden_death_active` on, then the launch;
    /// a scoreless one goes again through post_sudden_death; a goal ends it.
        /// Private, time up tied with OVERTIME on: the same calls and relaunch
    /// as public, but no clock: it stays at 0:00.
    #[test]
    fn private_tied_overtime_relaunches_without_a_clock() {
        let mut a = Arena::new(1, true);
        a.start_sudden_death(1000);
        assert!(a.overtime);
        assert_eq!(a.clock, 0.0);
        assert!(a.schedule.iter().any(|(_, x)| matches!(x, Act::Event(s, _, _) if *s == EV_ROUND_RESTART)));
        assert!(a.schedule.iter().any(|(_, x)| matches!(x, Act::State(s) if *s == ST_PRE_LAUNCH_ENTER)));
    }

    /// Time up with the disc out of the goals: it stays free (grabs and the
    /// holder's states go on); after a goal it stays pinned in the goal.
    #[test]
    fn time_up_leaves_the_disc_free_unless_it_is_in_a_goal() {
        let mut a = Arena::new(1, false);
        a.server_disc = false;
        a.last_disc = Some((10, 20, 30));
        a.disc_owner = 3;
        a.end_match(1000);
        assert!(a.disc_loose && a.disc_free());
        assert_eq!(a.frozen, None);
        assert!(!a.server_disc, "the holder keeps simulating it");
        let mut b = Arena::new(1, false);
        b.frozen = Some((0, 0, 36_000));
        b.end_match(1000);
        assert!(!b.disc_loose && !b.disc_free());
        assert!(b.server_disc);
    }

    /// Saves, interceptions and steals get native's calls, with its delays
    /// and the interceptor's team's table (mp_announcer_server_state).
    #[test]
    fn stat_calls_follow_the_stock_announcer() {
        use super::super::announcer as an;
        let said = |a: &Arena| -> Vec<(u64, u64)> {
            a.schedule.iter().filter_map(|(t, x)| match x { Act::Event(s, m, _) if *m == MOD_ANNOUNCER => Some((*t, *s)), _ => None }).collect()
        };
        let now = |a: &Arena| -> Vec<u64> { a.soon.iter().filter_map(|x| match x { Act::Event(s, m, _) if *m == MOD_ANNOUNCER => Some(*s), _ => None }).collect() };
        let later = (an::CALL_DELAY_S * frame_hz() as f32).round() as u64;
        let mut a = Arena::new(1, false);
        a.tick = 100;
        a.announce_stat(Stat::Saves, TEAM_BLUE);
        assert!(an::SAVE.contains(&now(&a)[0]) && a.schedule.is_empty());

        let mut a = Arena::new(1, false);
        a.tick = 100;
        a.announce_stat(Stat::Interceptions, TEAM_ORANGE);
        let s = said(&a);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].0, 100 + later);
        assert!(an::INTERCEPTION_ORANGE.contains(&s[0].1));
        a.schedule.clear();
        for _ in 0..100 {
            a.announce_stat(Stat::Interceptions, TEAM_BLUE);
        }
        assert!(said(&a).iter().all(|(_, c)| an::INTERCEPTION_BLUE.contains(c)));
        assert!(!said(&a).iter().any(|(_, c)| *c == an::INTERCEPTION_ORANGE[4]), "no red call for blue");

        let mut a = Arena::new(1, false);
        a.tick = 100;
        a.announce_stat(Stat::Steals, TEAM_BLUE);
        assert_eq!(now(&a), vec![an::STEAL_SOUND]);
        let s = said(&a);
        assert_eq!(s[0].0, 100 + later);
        assert!(an::STEAL.contains(&s[0].1));

        let mut a = Arena::new(1, false);
        a.announce_stat(Stat::Passes, TEAM_BLUE);
        a.announce_stat(Stat::Interceptions, TEAM_SPECTATOR);
        assert!(said(&a).is_empty() && now(&a).is_empty());
    }

    /// Public, time up tied: native's sudden death (captures/netrec-pub-ot-20261119).
    #[test]
    fn tied_time_up_goes_to_repeating_sudden_death() {
        let states = |a: &Arena| -> Vec<u64> { a.schedule.iter().filter_map(|(_, x)| match x { Act::State(s) => Some(*s), _ => None }).collect() };
        let said = |a: &Arena| -> Vec<(u64, u64)> { a.schedule.iter().filter_map(|(t, x)| match x { Act::Event(s, m, _) if *m == MOD_ANNOUNCER => Some((*t, *s)), _ => None }).collect() };
        let at = |a: &Arena, want: &dyn Fn(&Act) -> bool| a.schedule.iter().find(|(_, x)| want(x)).map(|(t, _)| *t).expect("scheduled");
        let draws: Vec<u64> = ["A", "B", "C", "D", "E", "F"].iter().map(|v| crate::sns::symbol(&format!("MP_ANNOUNCER_0100{v}_DRAW"))).collect();
        let hz = frame_hz();
        let mut a = Arena::new(1, false);
        a.blue = 4;
        a.red = 4;
        a.start_sudden_death(1000);
        assert!(a.overtime);
        assert_eq!(a.clock, 120.0);
        let st = states(&a);
        assert_eq!(&st[..5], &[ST_MATCH_END[0], ST_PRE_SUDDEN_DEATH[0], ST_PRE_SUDDEN_DEATH[1], ST_SD_NEW_ROUND[0], ST_SD_NEW_ROUND[1]]);
        assert!(st.contains(&ST_GOAL[6]) && st.contains(&ST_PRE_LAUNCH_ENTER) && st.contains(&ST_PLAYING));
        // The draw at once, "overtime round 1" at pre_launch.
        let s = said(&a);
        assert!(s.iter().any(|&(t, x)| t == 1001 && draws.contains(&x)));
        let pre = at(&a, &|x| matches!(x, Act::State(s) if *s == ST_PRE_LAUNCH_ENTER));
        assert!(s.contains(&(pre, crate::sns::symbol("MP_ANNOUNCER_OT_ROUND_1"))));
        // 5 s in sudden_death_new_round, then the walk; pre_launch 4.5 s on.
        let restart = at(&a, &|x| matches!(x, Act::Event(s, _, _) if *s == EV_ROUND_RESTART));
        assert_eq!(restart, 1003 + 5 * hz);
        assert_eq!(pre, restart + 2 + (HOLD_SECS * hz as f32) as u64);
        assert_eq!(at(&a, &|x| matches!(x, Act::State(s) if *s == ST_ROUND_START)), pre + hz);
        // Scoreless: again, "overtime round 2".
        a.start_sudden_death(50_000);
        assert_eq!(a.clock, 120.0);
        assert!(said(&a).iter().any(|&(_, x)| x == crate::sns::symbol("MP_ANNOUNCER_OT_ROUND_2")));
        // A goal decides it: post_sudden_death, then match over.
        a.blue = 6;
        a.end_round(90_000, false);
        assert_eq!(a.phase, Phase::Over);
        assert!(!a.overtime);
        let post0 = at(&a, &|x| matches!(x, Act::State(s) if *s == ST_POST_SUDDEN_DEATH[0]));
        let over = at(&a, &|x| matches!(x, Act::State(s) if *s == ST_MATCH_END[0]));
        assert!(post0 < over);
        assert!(a.schedule.iter().any(|(t, x)| *t < over && matches!(x, Act::Flag(k, 0) if *k == VAR_SUDDEN_DEATH)));
        assert_eq!(a.flag(VAR_MERCY_WIN), Some(0));
    }

    /// A mercy end of the match sets the mercy flag and opens with the round
    /// wrap-up, like a time-up end.
    #[test]
    fn mercy_end_announces_round_over() {
        let said = |a: &Arena| -> Vec<u64> {
            a.schedule.iter().filter_map(|(_, act)| match act { Act::Event(s, m, _) if *m == MOD_ANNOUNCER => Some(*s), _ => None }).collect()
        };
        let mut a = Arena::new(1, false);
        a.blue = 15;
        a.red = 3;
        a.end_round(100, true);
        assert_eq!(a.phase, Phase::Over);
        assert_eq!(a.flag(VAR_MERCY_WIN), Some(1));
        assert!(said(&a).contains(&EV_ANNOUNCER_ROUND_OVER));

        let mut b = Arena::new(1, false);
        b.blue = 5;
        b.red = 3;
        b.end_match(100);
        assert!(said(&b).contains(&EV_ANNOUNCER_ROUND_OVER));
    }
}
