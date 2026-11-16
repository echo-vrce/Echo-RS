//! One lobby/match: entrants, the join sequence, message relay, replicated
//! game state, real per-peer ping, disc ownership and the arena rules.
//!
//! Everything here is driven by two calls: [`Session::on_packet`] for each
//! datagram and [`Session::server_frame`] at `--frame-hz` (default 20). Relayed player traffic is
//! forwarded as soon as it is decoded; nothing waits for the next frame.

use std::collections::{HashMap, VecDeque};
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
use super::stats::{DiscTracker, Stat, Stats};
use super::transport::{SessionCrypto, Transport, mac_matches, reliable_batch};

pub const MAX_SLOTS: usize = 16;
/// Server frames per second (frame message, replication, rules), set once
/// at startup (`--frame-hz`). Every timing in the rules is in seconds and
/// converted with this, so any rate in 10..=120 runs the same match.
static FRAME_HZ_V: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(20);

pub fn frame_hz() -> u64 {
    FRAME_HZ_V.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_frame_hz(hz: u64) {
    FRAME_HZ_V.store(hz.clamp(10, 200), std::sync::atomic::Ordering::Relaxed);
}
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
    loadout: Option<Vec<u8>>,
    /// The player's platform id (platform, account), from their entrant state.
    xpid: Option<[u64; 2]>,
    /// Jersey number, from their loadout number notifications.
    jersey: Option<u32>,
    /// When to report a changed loadout to the backend (changes come in bursts).
    loadout_save_at: Option<Instant>,
    /// The player changed their loadout here: it is saved once more as they leave.
    loadout_changed: bool,
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
    /// When this peer last sent a stun (one credit per stun burst).
    last_stun: Option<Instant>,
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
            xpid: None,
            jersey: None,
            loadout_save_at: None,
            loadout_changed: false,
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
            last_stun: None,
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

/// The server clock every session publishes, built like native's: the frame
/// word counts 120 Hz ticks and `server_us` is exactly frame × 8333 µs, both
/// measured from machine uptime. That keeps them small (native's are ~4e6
/// frames) and still never going backwards when this process restarts, which
/// matters because the client drops any frame not above the last one it saw
/// and keeps that memory from one server to the next. The client's own match
/// timers (the tube countdown, the doors) run on this clock.
pub const TICK_US: u64 = 8333;
/// Echo ages kept per player for the ping estimate (native keeps 450).
const ECHO_SAMPLES: usize = 450;

/// Public arena: a lead this large ends the match (the mercy rule); the
/// host can change it (mp_arena_rules).
fn public_mercy_margin() -> u32 {
    super::msg::arena_public().mercy_margin
}

/// How far the server's disc timestamp runs ahead of session time (the
/// clients' clocks start within a few seconds of it).
const DISC_STAMP_LEAD_US: u64 = 60_000_000;

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
            awards_due: false,
            disc_resets: 0,
            granted: u32::MAX,
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
            rounds_played: 0,
            goal_touch: [false; 2],
        }
    }

    fn at(&mut self, frame: u64, act: Act) {
        self.schedule.push((frame, act));
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
            self.frozen = self.last_disc;
        }
        self.server_disc = true;
        self.disc_live = false;
        self.last_disc = None;
    }

    /// The server takes the disc back to its spawn; the reset count bump
    /// makes clients snap their copy to it.
    fn take_disc(&mut self) {
        self.server_disc = true;
        self.disc_live = false;
        self.frozen = None;
        self.last_disc = None;
        self.bump_resets();
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
        for _ in 0..2 {
            self.at(g, ev13(EV_GOAL_A, MOD_MATCH));
            self.at(g, ev13(EV_GOAL_B, MOD_MATCH));
        }
        self.at(g, ev(EV_GAME_SCORE));
        self.at(g, Act::State(ST_GOAL[0]));
        self.at(g, Act::Flag(VAR_LAUNCH_GATES, 0));
        self.at(g + 1, Act::State(ST_GOAL[1]));
        self.at(g + 1, Act::Flag(VAR_HOLD, 1));
        let celebrate = self.celebration_secs();
        self.at(g + 1, Act::Countdown(celebrate));
        let call = if points >= 3 { EV_ANNOUNCER_GOAL_3 } else { EV_ANNOUNCER_GOAL_2 };
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

    /// A private round ended (time up, mercy or an overtime goal): credit the
    /// winner, then the next round, or the match end once it is decided.
    fn end_round(&mut self, f: u64, mercy: bool) {
        self.round_live = false;
        self.overtime = false;
        match self.blue.cmp(&self.red) {
            std::cmp::Ordering::Greater => self.blue_rounds += 1,
            std::cmp::Ordering::Less => self.red_rounds += 1,
            std::cmp::Ordering::Equal => {}
        }
        let need = self.total_rounds / 2 + 1;
        let majority = self.rules.best_of() && (self.blue_rounds >= need || self.red_rounds >= need);
        let decided = majority || self.rounds_played + 1 >= self.total_rounds;
        self.set_flag(VAR_MERCY_WIN, u8::from(mercy));
        if decided {
            // The round counter (the HUD shows it + 1) stays on the last
            // round: "round 2 of 2", not "3 of 2".
            self.end_match(f);
        } else {
            self.rounds_played += 1;
            self.next_round(f);
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
        // or overtime goal) until the next round's walk takes it to spawn.
        self.pin_disc();
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
        // deciding goal); the spawn flag below only matters for a restart.
        self.pin_disc();
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
    /// Component handles from the IdsResponse this session sends.
    handles: Handles,
    peers: HashMap<SocketAddr, Peer>,
    pending: Vec<PendingKeys>,
    slot_gen: [u16; MAX_SLOTS],
    slot_addr: [Option<SocketAddr>; MAX_SLOTS],
    frame: u64,
    /// Last wire frame word published (120 Hz ticks of the server clock).
    wire_frame: u64,
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

/// How long after the last loadout change to save it.
const LOADOUT_SAVE_DELAY: Duration = Duration::from_millis(1500);

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
        Self {
            info,
            social,
            arena_level,
            handles,
            peers: HashMap::new(),
            pending: Vec::new(),
            slot_gen: [0; MAX_SLOTS],
            slot_addr: [None; MAX_SLOTS],
            frame: 0,
            wire_frame: 0,
            started: Instant::now(),
            arena,
            touch: HashMap::new(),
            gear: super::gear::Gear::new(),
            props: super::props::Props::new(),
            server_disc_was: false,
            touch_seq: HashMap::new(),
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

    fn send_gear(&mut self, from: SocketAddr, o: super::gear::Out) {
        use super::gear::To;
        match o.to {
            To::Sender => self.reliable_to(from, o.ty, &o.body),
            To::Others => self.relay_reliable(from, o.ty, &o.body, false),
            To::All => self.relay_reliable(from, o.ty, &o.body, true),
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
            STUN if reliable && self.stun_victim_is_spectator(d) => {
                debug!(%from, body = %hex_str(d), "stun on a spectator (not relayed)");
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
                // Social lobby: sent on walking out of the combat area.
                if self.social {
                    for o in self.gear.left_area(from) {
                        self.send_gear(from, o);
                    }
                }
            }
            EQUIPMENT if reliable && d.len() == 56 => self.relay_reliable(from, ty, d, false),
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
                self.relay_reliable(from, ty, d, true);
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
                    if holder && self.arena.frozen.is_none() {
                        let mut f = d.to_vec();
                        f[24] = (f[24] & 0xf0) | (self.arena.disc_resets & 0xf) as u8;
                        self.relay_mailbox(from, ty, key, &f);
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
            p.echo_ages.push_back((frames * TICK_US) as u32);
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
        let genidx = self.slot_gen[slot];
        self.slot_gen[slot] = genidx.wrapping_add(1);
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
        let locked = self.arena_level && !release && self.arena.phase != Phase::Playing;
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
                believed == e.owner
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
                self.props.on_grant(object, entry_id);
            }
        } else {
            reply[0x08..0x0c].copy_from_slice(&e.owner.to_le_bytes());
            reply[0x18..0x1a].copy_from_slice(&u16::from(e.generation & 0xf).to_le_bytes());
            self.reliable_to(from, TOUCH_DECLINED, &reply);
        }
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
        let free_play = self.arena.private && self.arena.phase == Phase::Waiting;
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
        // A player who moved the disc without a grab (a headbutt, a punch)
        // now simulates it, but the reset count has not moved, so the others
        // would ignore that player's disc: move it past the player's own.
        if self.arena.phase == Phase::Playing && !self.arena.server_disc && handle != self.arena.disc_owner && handle != self.arena.granted {
            self.arena.disc_resets = (client_count + 1) % DISC_COUNTS;
            self.arena.granted = handle;
            info!(slot = owner, resets = self.arena.disc_resets, "disc taken without a grab (headbutt?): others snap to it");
        }
        self.arena.client_resets = Some(client_count);
        self.arena.disc_owner = handle;
        if self.arena.server_disc && (self.arena.phase == Phase::Playing || free_play) {
            self.arena.server_disc = false;
            info!(slot = owner, "player took the disc; server stops publishing it");
        }
        if self.arena.phase != Phase::Playing {
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
        // Client disc state word: second byte 2 while held, 4 once free.
        let st = u32::from_le_bytes([d[24], d[25], d[26], d[27]]);
        let held = match (st >> 8) & 0xf {
            2 => Some(true),
            4 => Some(false),
            _ => None,
        };
        if owner_team == TEAM_BLUE || owner_team == TEAM_ORANGE {
            let client_us = u32::from_le_bytes([d[8], d[9], d[10], d[11]]);
            self.arena.tracker.sample(now, Some(client_us), owner, owner_team, [x, y, depth], held);
        }
        self.apply_credits();
        if !self.arena.goals_armed {
            return;
        }
        let Some((goal_idx, hit)) = entered else { return };
        // Orange's goal (index 1) is where blue scores.
        let blue_scored = goal_idx == 1;
        let own_goal = (owner_team == TEAM_BLUE) != blue_scored;
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
        rec[6..10].copy_from_slice(&self.arena.disc_owner.to_le_bytes());
        let assister = g
            .assister
            .and_then(|s| self.slot_addr.get(usize::from(s)).copied().flatten())
            .and_then(|a| self.peers.get(&a))
            .map(|p| (u32::from(p.generation.unwrap_or(0)) << 16) | p.slot.unwrap_or(0) as u32)
            .unwrap_or(u32::MAX);
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
        self.arena.bump_resets();
        self.arena.server_disc = true;
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
            if self.arena.overtime || mercy.is_some() {
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
            for n in first..frame {
                let mut sf = [0u8; 16];
                sf[..8].copy_from_slice(&n.to_le_bytes());
                sf[8..].copy_from_slice(&(n * TICK_US).to_le_bytes());
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
            self.arena_rules();
        }
        self.replicate();
        // The lobby objects nobody owns, ten times a second (native: 45; they
        // do not move here, so less is enough).
        // The Fury, while it flies, goes out 40 times a second.
        if self.social {
            self.props.step(Instant::now());
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

    fn team_counts(&self) -> (usize, usize) {
        let joined = self.peers.values().filter(|p| p.stage == Stage::Joined);
        joined.fold((0, 0), |(b, o), p| match p.team {
            TEAM_BLUE => (b + 1, o),
            TEAM_ORANGE => (b, o + 1),
            _ => (b, o),
        })
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
                    // Out of the goal and onto the spawn for the walk.
                    a.frozen = None;
                    a.bump_resets();
                    a.tracker.reset();
                    info!(round = a.rounds_played + 1, "next round");
                }
            }
        }
        if let Some((light, fx)) = self.arena.launch_fx {
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
                if !a.private {
                    info!(blue = a.blue, orange = a.red, "match over");
                    a.end_match(f);
                } else if a.blue == a.red && a.rules.overtime() {
                    // Tied: play on; the next goal wins the round.
                    a.overtime = true;
                    let ot = crate::sns::symbol("MP_ANNOUNCER_001A_OVERTIME");
                    a.soon.push(Act::Event(ot, MOD_ANNOUNCER, FILTER_ALL_ALT));
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
        self.arena.disc_us = self.started.elapsed().as_micros() as u64 + DISC_STAMP_LEAD_US;
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
            let (x, y, depth) = match self.arena.frozen {
                Some(p) => p,
                None => {
                    let base = baseline(self.info.level, self.info.mode).and_then(|b| var_raw(&b[0], VAR_DISC_SPAWN)).unwrap_or(2);
                    disc_spawn_pos(self.arena.flag(VAR_DISC_SPAWN).unwrap_or(base as u8))
                }
            };
            msg::write_disc_transform(&mut d, x, y, depth);
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

    /// Native, at match over: `SR15NetTopAwardsMsg` = `[count u8]` + 72-byte
    /// records `[award u64][stat u64][value u64][14 zero bytes][name 30]
    /// [f32]`. The end-of-match board reads it (the MVP).
    fn send_top_awards(&mut self) {
        let best = self
            .peers
            .values()
            .filter(|p| p.slot.is_some())
            .max_by_key(|p| (p.stats.get(Stat::Points), p.stats.get(Stat::PossessionTime)));
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
        rec[16..24].copy_from_slice(&u64::from(best.stats.get(Stat::Points).max(1)).to_le_bytes());
        let n = name.len().min(29);
        rec[38..38 + n].copy_from_slice(&name[..n]);
        rec[68..72].copy_from_slice(&(best.stats.get(Stat::PossessionTime) as f32 / frame_hz() as f32).to_le_bytes());
        let mut m = vec![1u8];
        m.extend_from_slice(&rec);
        info!(mvp = %String::from_utf8_lossy(&name), points = best.stats.get(Stat::Points), "top awards");
        for a in self.joined() {
            self.reliable_to(a, TOP_AWARDS, &m);
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
            let (v, c) = self.props.vars();
            (v.to_vec(), c)
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
            hs.into_iter().filter_map(|h| value(0, h, raw(h)).map(|v| (h, v))).collect()
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
        let stamp = |p: &mut Vec<u8>, seq_off: u64| {
            if p.len() >= 28 {
                let seq = u64::from_le_bytes(p[16..24].try_into().expect("8")).wrapping_add(seq_off);
                p[16..24].copy_from_slice(&seq.to_le_bytes());
                p[24..28].copy_from_slice(&(seq as u32).to_le_bytes());
            }
        };
        let fulls: Vec<Vec<u8>> = base.iter().enumerate().map(|(i, b)| fresh(b, i)).collect();
        for p in self.peers.values_mut() {
            if p.stage != Stage::Joined || !p.rep_ready {
                continue;
            }
            let slot = p.slot.unwrap_or(0) as u64;
            let full = !p.rep_sent || (!arena && !p.rep_acked && server_frame % (frame_hz() / 2) == 0);
            if full {
                for (stream, b) in fulls.iter().enumerate() {
                    let mut body = b.clone();
                    stamp(&mut body, p.rep_seq);
                    p.t.queue_mailbox(REPLICATED_VARS, key_replicated(stream, frame, slot), &body);
                }
                p.rep_sent = true;
                p.rep_seq += 1;
            } else if arena {
                // Every frame: the clock/countdown plus all tracked vars (tiny).
                let mut body = delta(&base[0], &tracked);
                stamp(&mut body, p.rep_seq);
                p.t.queue_mailbox(REPLICATED_VARS, key_replicated(0, frame, slot), &body);
                if !tracked1.is_empty() && base.len() > 1 {
                    let mut body = delta(&base[1], &tracked1);
                    stamp(&mut body, p.rep_seq);
                    p.t.queue_mailbox(REPLICATED_VARS, key_replicated(1, frame, slot), &body);
                }
                p.rep_seq += 1;
            } else if lobby_due {
                let mut body = delta(&base[0], &lobby_vars);
                stamp(&mut body, p.rep_seq);
                p.t.queue_mailbox(REPLICATED_VARS, key_replicated(0, frame, slot), &body);
                p.rep_seq += 1;
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
        if self.social {
            // The lobby's toys: the Fury controller's trigger.
            self.props.on_event(from, sym);
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
