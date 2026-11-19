//! The social lobby's loose objects: its three discs, the toys, the Fury.
//!
//! From a native recording (`captures/native-match-20261116`). A
//! `SR15NetPhysicsSimStateEvent` for one of the lobby's object sets is
//! `[owner u32][set id u32][time µs u32][0 u32][records * 80 u32][0 u32]`
//! then one 284-bit record per object, bit-packed, padded to 8 bytes:
//!
//! | bits | |
//! |---|---|
//! | 0..8 | touch generation (the grant's low nibble) |
//! | 8 | asleep |
//! | 9 | held in a hand |
//! | 10 | held exclusively |
//! | 11..18 | entry number (the touch request's entry id) |
//! | 75, 94, 113 | position, 19-bit millimetres + 250000 |
//!
//! The native server sends every object no client owns, 45 times a second
//! (owner `ffffffff`): 18 in set `b1c75c04`, 9 in `d33d44b7`. A client that
//! is granted an object sends its record itself from then on, and keeps
//! sending it after letting go.
//!
//! This server cannot simulate them, but it can say where they are: the
//! recorded resting places until somebody takes an object, then, once that
//! player has left, the last record they sent, put to sleep. Without it a
//! player who joined later saw every object where it started, and an object
//! whose owner had left was in a different place for everyone.
//!
//! More of the record (same recording):
//!
//! | bits | |
//! |---|---|
//! | 132..164 | rotation: 2-bit index of the largest of x, y, z, w (left out, positive), then the other three, 10 bits each over ±1/√2 |
//! | 164, 176, 188 | velocity, 12 bits each: `(raw - 1536) / 32` m/s |
//! | 200, 209, 218 | spin, 9 bits each: `raw - 144`, about rad/s, world axes |
//!
//! ## The Fury
//!
//! Entry 2 of `b1c75c04` is the toy ship, entry 3 its controller. The native
//! server flies the ship itself. While the controller's trigger is held
//! (the client sends ComponentEvent `88678f45e3fb8c0e`, then
//! `4879bc8a220df32f` on letting go) the ship thrusts along its own nose:
//! the recorded flight fits `a = 1.40 * nose - 1.31 * v` (top speed about
//! 1.07 m/s). That part is reproduced here.
//!
//! Steering is not native's. In the recording the ship tumbled at 40 to
//! 190 degrees a second and its heading followed neither the controller's
//! orientation nor its turning, so what turns it could not be worked out
//! from one flight. Here the ship turns to face the way the controller
//! faces. It also has no walls to bounce off (this server knows no level
//! geometry): it slows to a stop when the trigger is released, and stops at
//! a set distance from where it rests.

use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const HEAD: usize = 24;
const RECORD_BITS: usize = 284;
const RECORD_BYTES: usize = RECORD_BITS.div_ceil(8);
/// The count word is 80 per record.
const COUNT_UNIT: u32 = 80;
const ASLEEP: usize = 8;
const HELD: usize = 9;
const EXCLUSIVE: usize = 10;
const ENTRY_BIT: usize = 11;
const ENTRY_BITS: usize = 7;

const POS_BIT: usize = 75;
const ROT_BIT: usize = 132;
const VEL_BIT: usize = 164;
const SPIN_BIT: usize = 200;

/// The Fury's set, its entry and its controller's.
const FURY_SET: u32 = 0xb1c7_5c04;
const FURY: u8 = 2;
const FURY_CONTROLLER: u8 = 3;
/// The controller's trigger, pressed and let go (ComponentEvent symbols).
pub const FURY_ENGINE_ON: u64 = 0x0e8c_fbe3_458f_6788;
pub const FURY_ENGINE_OFF: u64 = 0x2ff3_0d22_8abc_7948;
/// `a = THRUST * nose - DRAG * v` while the trigger is held, fitted to the
/// recorded flight.
const FURY_THRUST: f64 = 1.40;
const FURY_DRAG: f64 = 1.31;
/// Engine off it coasts: native's velocity stays constant for seconds after
/// the release and is still 0.07 m/s fifteen seconds on (1/s).
const FURY_COAST_DRAG: f64 = 0.12;
/// How fast the ship turns to the held controller's facing (1/s). Not native's.
const FURY_TURN: f64 = 4.0;
/// Let go, it keeps turning the way it was, slowing (1/s).
const FURY_SPIN_DRAG: f64 = 0.4;
/// How far from its resting place the ship may go (m).
const FURY_RANGE: f64 = 30.0;
const FURY_STOPPED: f64 = 0.02;
const FURY_SPIN_STOPPED: f64 = 0.05;

/// The lobby's replicated vars native's server changes (all on the lobby's
/// object, `d09afd15b1c75c04`). The Fury's engine: 1 from the trigger's
/// press to its release; the clients play the engine sound and flames from it.
pub const VAR_FURY_ENGINE: u64 = 0xbec4_d8ed_f6ac_f8f3;
/// Set when a lobby disc is first grabbed: (set, entry, vars).
const GRAB_VARS: [(u32, u32, &[(u64, u64)]); 3] = [
    // The main disc.
    (0xb1c7_5c04, 0, &[(0x3261_fa72_2679_4279, 1)]),
    // The quarter-court disc (its holder var, `VAR_QC_HOLDER`, is set on every grant).
    (0xd33d_44b7, 0, &[(0xa22d_90ef_70f6_fa83, 1)]),
    // The mini-arena disc.
    (0xd33d_44b7, 1, &[(0xd279_7da9_9ab3_17a5, 1)]),
];

/// The lobby combat room's level scripts, as native's server runs them
/// (`captures/netrec-native-srv-1201`, 2026-11-18): a client's server-only
/// event (sym, net handle) sets a var's low word (the high one stays the
/// baseline's, see [`LO32_VARS`]). Area enter/exit on handles 0xa8 and 0xb7
/// (1.0 / 0.0), the ability station on 0xc6 (used 0, ready 1), and 0xb7's
/// second trigger (exit 0.0; enter 1.0 by its pair, not seen changing).
const LOBBY_REFLEXES: [(u64, u16, u64, u32); 8] = [
    (0xfbc0_427a_c211_919e, 0xa8, 0xec48_aa3f_f874_8bae, 0),
    (0xfbc0_427a_c211_919f, 0xa8, 0xec48_aa3f_f874_8bae, 0x3f80_0000),
    (0xfbc0_427a_c211_919e, 0xb7, 0xab54_4d4b_2113_0c68, 0),
    (0xfbc0_427a_c211_919f, 0xb7, 0xab54_4d4b_2113_0c68, 0x3f80_0000),
    (0xea7a_6e23_ed5c_ace2, 0xc6, 0x01d5_d534_863e_cb0e, 0),
    (0x8a4a_9239_b3d3_06ac, 0xc6, 0x01d5_d534_863e_cb0e, 1),
    (0x635d_09de_ff71_64c0, 0xb7, 0xcb3e_710e_2dd8_e5c2, 0),
    (0x635d_09de_ff71_64c1, 0xb7, 0xcb3e_710e_2dd8_e5c2, 0x3f80_0000),
];
/// The lobby combat room's barriers (net handle, var): the var is 0 while the
/// barrier is down, 1 once it respawns. 64/65/66 were seen breaking in the same
/// capture; the rest follow from them by `tools/gen_combat_vars.py`'s
/// `barrier_var` (the handle in decimal), and all 11 are native's baseline vars at 1.
pub const LOBBY_BARRIER_VARS: [(u16, u64); 11] = [
    (58, 0x707e_2294_ceb4_9f36),
    (59, 0x379e_4d91_6ec6_4c78),
    (60, 0x29e6_4cf2_bb83_bb90),
    (61, 0x46a6_d317_a29d_3db0),
    (62, 0x58de_d274_77d8_ca58),
    (63, 0x1f3e_bd71_d7aa_1916),
    (64, 0x7452_cb20_8066_2e3a),
    (65, 0x6a2a_ca43_5523_d9d2),
    (66, 0x2dca_a546_f551_0a9c),
    (67, 0x33b2_a425_2014_fd74),
    (68, 0x5cf2_3bc0_390a_7b54),
];
/// Vars whose high word is a per-level version: only their low word is ours.
pub const LO32_VARS: [u64; 4] = [0xec48_aa3f_f874_8bae, 0xab54_4d4b_2113_0c68, 0x01d5_d534_863e_cb0e, 0xcb3e_710e_2dd8_e5c2];

/// An object whose owner has sent no record of it for this long is the
/// server's again. A client stops sending what it threw once it comes to
/// rest (native's server simulates it from there); still counted as that
/// client's, nobody sent it at all, and a player who joined later saw it
/// where it started.
const SILENCE: Duration = Duration::from_secs(2);

// -- The quarter court's goal ------------------------------------------------
//
// The goal is `mpl_arena_goal_di_i705` (lobby.fbx): a ring of radius 1.555 m
// at (-53.540, 1.482, -30.910), turned 60 degrees about +Y, so its axis is
// `QC_NORMAL`. Native's server (`captures/netrec-info-20261118`, the session
// of the 14:40:19 "EmitOnFlagBaseScore team(1), point value(2) ... INSIDE
// SHOT" log) took a goal for each pass of the disc (set `d33d44b7`, entry 0)
// through the ring from the front (+normal) to the back: all six goals were
// such passes, 0.09 to 1.02 m from the centre; every other front-to-back pass
// was 1.42 m out or more, and a back-to-front one at 0.34 m did nothing. The
// disc's owner keeps simulating it through the goal (no stop, no reset).
//
// On a goal native:
// - sets `VAR_GAME_STATE` to `LOBBY_ST_GOAL` for 3 s, then back to playing,
//   with `EV_ENABLE_SCORING` on the lobby's scoring module (`OnRoundStartCB`);
// - sent everyone `SR15NetReliablePostScoreUpdate`: team 1, 2 points, goal
//   type 2 (INSIDE SHOT), the owner's handle, no assist, speed (m/s),
//   release-to-goal distance (m), the lobby's clock (s). For three of the six
//   it did not (released from 6.4 to 11.1 m out, 5.6 to 10.9 m to the side;
//   the scoring ones 7.0 to 8.0 m out, 0.8 to 6.1 m to the side): the rule is
//   not known, so here every goal scores.

/// The quarter-court disc: set and entry.
pub const QC_SET: u32 = 0xd33d_44b7;
const QC_ENTRY: u8 = 0;
/// The disc's current simulator, `gen << 16 | slot` (`ffffffff`: the server).
pub const VAR_QC_HOLDER: u64 = 0x1b10_bdc9_62b4_7fcc;
/// The ring's centre, its front (unit, horizontal) and the radius a goal
/// passes within (m; the ring itself is 1.555).
pub const QC_GOAL: [f64; 3] = [-53.540, 1.482, -30.910];
const QC_NORMAL: [f64; 3] = [0.866_025, 0.0, 0.5];
const QC_RADIUS: f64 = 1.2;
/// How long the lobby stays in its goal state.
pub const QC_GOAL_HOLD: Duration = Duration::from_secs(3);
/// The lobby's game state during a goal (native; `ST_PLAYING` otherwise).
pub const LOBBY_ST_GOAL: u64 = 0xb059_068e_f815_8682;

/// The 3-point bubble, `mpl_arena_barrier_3pt_bubble_blue_i4364` (lobby.fbx,
/// material `mpl_score_volume_blue`): a sphere cap of radius 7.97 m (its 2726
/// vertices within 0.2 m) centred 0.7 m behind the ring. A shot let go
/// outside it is a 3-pointer (LONG SHOT), inside a 2-pointer (INSIDE SHOT).
/// Where it was let go is the disc's first free-flight position. Native's
/// six recorded goals were all 2-pointers, let go 6.2 to 8.2 m from the
/// centre (the 8.2 m one at about 15 samples a second), hence the slack.
pub const QC_BUBBLE: [f64; 3] = [-54.106, 1.466, -31.237];
const QC_BUBBLE_RADIUS: f64 = 7.97;
const QC_BUBBLE_SLACK: f64 = 0.3;

/// Points and goal type (`stats::GOAL_*`) for a shot let go at `release`.
fn qc_points(release: Option<[f64; 3]>) -> (u16, u16) {
    let out = release.is_some_and(|a| (0..3).map(|k| (a[k] - QC_BUBBLE[k]).powi(2)).sum::<f64>().sqrt() > QC_BUBBLE_RADIUS + QC_BUBBLE_SLACK);
    if out { (3, super::stats::GOAL_LONG_SHOT) } else { (2, super::stats::GOAL_INSIDE_SHOT) }
}

/// A disc through the quarter court's ring.
#[derive(Clone, Debug, PartialEq)]
pub struct QcGoal {
    /// The client simulating the disc (the shooter).
    pub by: SocketAddr,
    /// 2 or 3 (the bubble).
    pub points: u16,
    pub goal_type: u16,
    pub speed: f32,
    /// From where it was let go to the ring (m), 0 when not seen.
    pub distance: f32,
}

#[derive(Default)]
struct QuarterCourt {
    /// The disc's last reported position.
    last: Option<[f64; 3]>,
    /// Held in the last report.
    held: bool,
    /// Where it was let go: its first free position after a hand.
    release: Option<[f64; 3]>,
    /// In the goal state until then.
    hold_until: Option<Instant>,
    goal: Option<QcGoal>,
    round_start: bool,
}

/// Whether the move a -> b goes through the ring from its front.
fn through_the_ring(a: [f64; 3], b: [f64; 3]) -> bool {
    let side = |p: [f64; 3]| (0..3).map(|k| (p[k] - QC_GOAL[k]) * QC_NORMAL[k]).sum::<f64>();
    let (sa, sb) = (side(a), side(b));
    if !(sa > 0.0 && sb <= 0.0) {
        return false;
    }
    let u = sa / (sa - sb);
    (0..3).map(|k| (a[k] + u * (b[k] - a[k]) - QC_GOAL[k]).powi(2)).sum::<f64>() <= QC_RADIUS * QC_RADIUS
}

fn velocity(r: &Record) -> [f64; 3] {
    [0, 1, 2].map(|k| (f64::from(field(r, VEL_BIT + 12 * k, 12)) - 1536.0) / 32.0)
}

type Record = [u8; RECORD_BYTES];

fn field(r: &[u8], at: usize, n: usize) -> u32 {
    (0..n).fold(0, |v, k| v | u32::from(bit(r, at + k)) << k)
}

fn set_field(r: &mut [u8], at: usize, n: usize, v: u32) {
    for k in 0..n {
        set_bit(r, at + k, v >> k & 1 == 1);
    }
}

fn position(r: &Record) -> [f64; 3] {
    [0, 1, 2].map(|k| (f64::from(field(r, POS_BIT + 19 * k, 19)) - 250_000.0) / 1000.0)
}

fn set_position(r: &mut Record, p: [f64; 3]) {
    for (k, x) in p.iter().enumerate() {
        set_field(r, POS_BIT + 19 * k, 19, (x * 1000.0 + 250_000.0).round().clamp(0.0, 524_287.0) as u32);
    }
}

fn set_velocity(r: &mut Record, v: [f64; 3]) {
    for (k, x) in v.iter().enumerate() {
        set_field(r, VEL_BIT + 12 * k, 12, (x * 32.0 + 1536.0).round().clamp(0.0, 4095.0) as u32);
    }
}

/// Rotation as (x, y, z, w).
fn rotation(r: &Record) -> [f64; 4] {
    let raw = field(r, ROT_BIT, 32);
    let largest = (raw & 3) as usize;
    let mut rest = [0, 1, 2].map(|i| (f64::from(raw >> (2 + 10 * i) & 1023) / 1023.0 * 2.0 - 1.0) / std::f64::consts::SQRT_2).into_iter();
    let mut q = [0.0; 4];
    let mut sum = 0.0;
    for (i, c) in q.iter_mut().enumerate() {
        if i != largest {
            *c = rest.next().unwrap_or(0.0);
            sum += *c * *c;
        }
    }
    q[largest] = (1.0 - sum).max(0.0).sqrt();
    q
}

fn set_rotation(r: &mut Record, q: [f64; 4]) {
    let largest = (0..4).max_by(|a, b| q[*a].abs().total_cmp(&q[*b].abs())).unwrap_or(3);
    let sign = if q[largest] < 0.0 { -1.0 } else { 1.0 };
    let mut raw = largest as u32;
    for (i, c) in (0..4).filter(|i| *i != largest).enumerate() {
        let v = ((q[c] * sign * std::f64::consts::SQRT_2 + 1.0) / 2.0 * 1023.0).round().clamp(0.0, 1023.0) as u32;
        raw |= v << (2 + 10 * i);
    }
    set_field(r, ROT_BIT, 32, raw);
}

/// Where a rotation points the object's own +Z (its nose).
fn nose(q: [f64; 4]) -> [f64; 3] {
    let [x, y, z, w] = q;
    [2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)]
}

fn mul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn normalised(q: [f64; 4]) -> [f64; 4] {
    let n = q.iter().map(|x| x * x).sum::<f64>().sqrt();
    if n < 1e-9 { [0.0, 0.0, 0.0, 1.0] } else { q.map(|x| x / n) }
}

/// World angular velocity (rad/s) that turns `a` into `b` in `dt`.
fn angular_velocity(a: [f64; 4], b: [f64; 4], dt: f64) -> [f64; 3] {
    if dt <= 0.0 {
        return [0.0; 3];
    }
    let mut d = mul(b, [-a[0], -a[1], -a[2], a[3]]);
    if d[3] < 0.0 {
        d = d.map(|x| -x);
    }
    let s = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if s < 1e-9 {
        return [0.0; 3];
    }
    let angle = 2.0 * s.atan2(d[3]);
    [0, 1, 2].map(|i| d[i] / s * angle / dt)
}

/// `q` turned by world angular velocity `w` for `dt`.
fn spun(q: [f64; 4], w: [f64; 3], dt: f64) -> [f64; 4] {
    let n = w.iter().map(|x| x * x).sum::<f64>().sqrt();
    if n * dt < 1e-9 {
        return q;
    }
    let half = n * dt / 2.0;
    let s = half.sin() / n;
    normalised(mul([w[0] * s, w[1] * s, w[2] * s, half.cos()], q))
}

/// A step from `a` towards `b` (normalised lerp, the short way round).
fn turn(a: [f64; 4], b: [f64; 4], k: f64) -> [f64; 4] {
    let dot: f64 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
    let s = if dot < 0.0 { -1.0 } else { 1.0 };
    let mut q = [0, 1, 2, 3].map(|i| a[i] + (b[i] * s - a[i]) * k);
    let n = q.iter().map(|x| x * x).sum::<f64>().sqrt();
    if n < 1e-9 {
        return a;
    }
    for c in &mut q {
        *c /= n;
    }
    q
}

/// The Fury while it is awake.
struct Flight {
    pos: [f64; 3],
    vel: [f64; 3],
    rot: [f64; 4],
    /// World angular velocity (rad/s).
    spin: [f64; 3],
    stepped: Instant,
}

fn bit(d: &[u8], i: usize) -> bool {
    d[i >> 3] >> (i & 7) & 1 == 1
}

fn set_bit(d: &mut [u8], i: usize, v: bool) {
    if v {
        d[i >> 3] |= 1 << (i & 7);
    } else {
        d[i >> 3] &= !(1 << (i & 7));
    }
}

fn entry_of(r: &Record) -> u8 {
    (0..ENTRY_BITS).fold(0, |v, k| v | u8::from(bit(r, ENTRY_BIT + k)) << k)
}

/// The records of one message (any owner), or none if it is not well formed.
fn records(d: &[u8]) -> Vec<Record> {
    if d.len() < HEAD {
        return Vec::new();
    }
    let n = (u32::from_le_bytes([d[16], d[17], d[18], d[19]]) / COUNT_UNIT) as usize;
    if n == 0 || n > 64 || HEAD * 8 + n * RECORD_BITS > d.len() * 8 {
        return Vec::new();
    }
    (0..n)
        .map(|i| {
            let mut r = [0u8; RECORD_BYTES];
            for k in 0..RECORD_BITS {
                set_bit(&mut r, k, bit(d, HEAD * 8 + i * RECORD_BITS + k));
            }
            r
        })
        .collect()
}

struct Entry {
    record: Record,
    /// The client sending this object's state, if any.
    owner: Option<SocketAddr>,
    /// When that client last sent it.
    heard: Option<Instant>,
}

impl Entry {
    /// No client's any more: at rest where it was last reported.
    fn release(&mut self) {
        self.owner = None;
        set_bit(&mut self.record, ASLEEP, true);
        set_bit(&mut self.record, HELD, false);
        set_bit(&mut self.record, EXCLUSIVE, false);
    }
}

struct Set {
    id: u32,
    key_base: u64,
    entries: Vec<Entry>,
}

pub struct Props {
    sets: Vec<Set>,
    /// Who holds the Fury's trigger down.
    pilot: Option<SocketAddr>,
    flight: Option<Flight>,
    /// Where the Fury rests.
    fury_home: [f64; 3],
    /// Lobby replicated vars set since the server started (sent on top of
    /// the captured full state), and whether one changed since last sent.
    vars: Vec<(u64, u64)>,
    vars_changed: bool,
    qc: QuarterCourt,
    /// Who the quarter-court disc was last granted to.
    qc_holder: Option<SocketAddr>,
}

fn unhex(t: &str) -> Vec<u8> {
    let t = t.trim();
    (0..t.len() / 2).map(|i| u8::from_str_radix(&t[2 * i..2 * i + 2], 16).unwrap_or(0)).collect()
}

/// The native server's first message for each set: every object at rest.
fn baselines() -> &'static [(u32, u64, Vec<u8>)] {
    static B: OnceLock<Vec<(u32, u64, Vec<u8>)>> = OnceLock::new();
    B.get_or_init(|| {
        vec![
            (0xb1c7_5c04, 0x2100, unhex(include_str!("data/props_b1c75c04.hex"))),
            (0xd33d_44b7, 0x324b, unhex(include_str!("data/props_d33d44b7.hex"))),
        ]
    })
}

impl Default for Props {
    fn default() -> Self {
        Self::new()
    }
}

impl Props {
    pub fn new() -> Self {
        let sets = baselines()
            .iter()
            .map(|(id, key_base, d)| Set { id: *id, key_base: *key_base, entries: records(d).into_iter().map(|record| Entry { record, owner: None, heard: None }).collect() })
            .collect();
        let mut p = Self { sets, pilot: None, flight: None, fury_home: [0.0; 3], vars: Vec::new(), vars_changed: false, qc: QuarterCourt::default(), qc_holder: None };
        p.fury_home = p.entry(FURY).map_or([0.0; 3], |e| position(&e.record));
        p
    }

    fn entry(&self, n: u8) -> Option<&Entry> {
        self.sets.iter().find(|s| s.id == FURY_SET)?.entries.iter().find(|e| entry_of(&e.record) == n)
    }

    fn entry_mut(&mut self, n: u8) -> Option<&mut Entry> {
        self.sets.iter_mut().find(|s| s.id == FURY_SET)?.entries.iter_mut().find(|e| entry_of(&e.record) == n)
    }

    /// A client's ComponentEvent: the Fury controller's trigger.
    pub fn on_event(&mut self, from: SocketAddr, sym: u64, handle: u16) {
        for (s, h, var, v) in LOBBY_REFLEXES {
            if s == sym && h == handle {
                self.set_var(var, u64::from(v));
            }
        }
        match sym {
            FURY_ENGINE_ON if self.entry(FURY_CONTROLLER).is_some_and(|c| c.owner == Some(from)) => {
                tracing::info!(%from, "Fury: engine on");
                self.pilot = Some(from);
                self.set_var(VAR_FURY_ENGINE, 1);
            }
            FURY_ENGINE_OFF if self.pilot == Some(from) => {
                tracing::info!(%from, "Fury: engine off");
                self.engine_off();
            }
            _ => {}
        }
    }

    fn engine_off(&mut self) {
        self.pilot = None;
        self.set_var(VAR_FURY_ENGINE, 0);
    }

    fn set_var(&mut self, h: u64, v: u64) {
        match self.vars.iter_mut().find(|e| e.0 == h) {
            Some(e) if e.1 == v => return,
            Some(e) => e.1 = v,
            None => self.vars.push((h, v)),
        }
        self.vars_changed = true;
    }

    /// The combat room's barriers that are down now (net handles).
    pub fn set_barriers_down(&mut self, down: &[u16]) {
        for (h, var) in LOBBY_BARRIER_VARS {
            let up = !down.contains(&h);
            // Untouched barriers stay as the baseline has them (up).
            if up && !self.vars.iter().any(|e| e.0 == var) {
                continue;
            }
            self.set_var(var, u64::from(up));
        }
    }

    /// A touch grant in the lobby: native's scripts set a var the first
    /// time each disc is taken.
    pub fn on_grant(&mut self, to: SocketAddr, handle: u32, set: u32, entry: u32) {
        // Native: the quarter-court disc's holder var follows every grant
        // (`ffffffff` once the server has it again).
        if set == QC_SET && entry == u32::from(QC_ENTRY) {
            self.qc_holder = Some(to);
            self.set_var(VAR_QC_HOLDER, u64::from(handle));
        }
        for (s, e, vars) in GRAB_VARS {
            if s == set && e == entry {
                for &(h, v) in vars {
                    self.set_var(h, v);
                }
            }
        }
    }

    /// The lobby vars this server changed, and whether any changed since
    /// the last call.
    pub fn vars(&mut self) -> (&[(u64, u64)], bool) {
        let changed = std::mem::take(&mut self.vars_changed);
        (&self.vars, changed)
    }

    /// The Fury is moving: its state should go out every few frames.
    pub fn active(&self) -> bool {
        self.flight.is_some()
    }

    /// Fly the Fury (call every server frame).
    ///
    /// Whoever holds its controller steers: the ship turns to the
    /// controller's facing with or without the trigger. The trigger
    /// (`pilot`) is the engine: thrust along the nose. Let go, it coasts and
    /// keeps turning, slowing down, then sleeps.
    pub fn step(&mut self, now: Instant) {
        self.qc_step(now);
        let held = self.entry(FURY_CONTROLLER).filter(|c| c.owner.is_some() && bit(&c.record, HELD)).map(|c| (c.owner, rotation(&c.record)));
        // The engine stops with the hand that started it.
        if self.pilot.is_some() && held.map(|h| h.0) != Some(self.pilot) {
            self.engine_off();
        }
        let thrust = self.pilot.is_some();
        let aim = held.map(|h| h.1);
        // Somebody picked the ship itself up: it is theirs, not ours.
        let Some(ship) = self.entry(FURY).filter(|e| e.owner.is_none()).map(|e| e.record) else {
            self.flight = None;
            return;
        };
        if self.flight.is_none() {
            if aim.is_none() {
                return;
            }
            self.flight = Some(Flight { pos: position(&ship), vel: [0.0; 3], rot: rotation(&ship), spin: [0.0; 3], stepped: now });
        }
        let home = self.fury_home;
        let Some(f) = self.flight.as_mut() else { return };
        let dt = now.saturating_duration_since(f.stepped).as_secs_f64().min(0.1);
        f.stepped = now;
        if let Some(aim) = aim {
            let before = f.rot;
            f.rot = turn(f.rot, aim, 1.0 - (-dt * FURY_TURN).exp());
            if dt > 0.0 {
                f.spin = angular_velocity(before, f.rot, dt);
            }
        } else {
            f.rot = spun(f.rot, f.spin, dt);
            let k = (-dt * FURY_SPIN_DRAG).exp();
            f.spin = f.spin.map(|x| x * k);
        }
        let push = if thrust { nose(f.rot).map(|x| x * FURY_THRUST) } else { [0.0; 3] };
        let drag = if thrust { FURY_DRAG } else { FURY_COAST_DRAG };
        for k in 0..3 {
            f.vel[k] += (push[k] - drag * f.vel[k]) * dt;
            f.pos[k] += f.vel[k] * dt;
        }
        // No walls here: it may not leave its range.
        let away: [f64; 3] = [0, 1, 2].map(|k| f.pos[k] - home[k]);
        let far = away.iter().map(|x| x * x).sum::<f64>().sqrt();
        if far > FURY_RANGE {
            for k in 0..3 {
                f.pos[k] = home[k] + away[k] / far * FURY_RANGE;
                f.vel[k] = 0.0;
            }
        }
        let len = |v: &[f64; 3]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
        let stopped = aim.is_none() && len(&f.vel) < FURY_STOPPED && len(&f.spin) < FURY_SPIN_STOPPED;
        let (pos, vel, rot, spin) = if stopped { (f.pos, [0.0; 3], f.rot, [0.0; 3]) } else { (f.pos, f.vel, f.rot, f.spin) };
        if stopped {
            self.flight = None;
        }
        if let Some(e) = self.entry_mut(FURY) {
            set_position(&mut e.record, pos);
            set_velocity(&mut e.record, vel);
            set_rotation(&mut e.record, rot);
            for (k, w) in spin.iter().enumerate() {
                set_field(&mut e.record, SPIN_BIT + 9 * k, 9, (w + 144.0).round().clamp(0.0, 511.0) as u32);
            }
            set_bit(&mut e.record, ASLEEP, stopped);
        }
    }

    /// A client's physics message: the objects in it are that client's now.
    pub fn observe(&mut self, from: SocketAddr, d: &[u8]) {
        if d.len() < HEAD || d[..4] == [0xff; 4] {
            return;
        }
        let id = u32::from_le_bytes([d[4], d[5], d[6], d[7]]);
        if !self.sets.iter().any(|s| s.id == id) {
            return;
        }
        let now = Some(Instant::now());
        for record in records(d) {
            let entry = entry_of(&record);
            if id == QC_SET && entry == QC_ENTRY {
                self.qc_track(from, &record, Instant::now());
            }
            let Some(set) = self.sets.iter_mut().find(|s| s.id == id) else { return };
            match set.entries.iter_mut().find(|e| entry_of(&e.record) == entry) {
                Some(e) => {
                    e.record = record;
                    e.owner = Some(from);
                    e.heard = now;
                }
                None => set.entries.push(Entry { record, owner: Some(from), heard: now }),
            }
        }
    }

    /// The quarter-court disc as its owner reports it: through the ring?
    fn qc_track(&mut self, from: SocketAddr, r: &Record, now: Instant) {
        let pos = position(r);
        let prev = self.qc.last.replace(pos);
        if bit(r, HELD) {
            self.qc.release = Some(pos);
            self.qc.held = true;
            return;
        }
        if std::mem::take(&mut self.qc.held) {
            self.qc.release = Some(pos);
        }
        let Some(prev) = prev else { return };
        if self.qc.hold_until.is_some() || !through_the_ring(prev, pos) {
            return;
        }
        let v = velocity(r);
        let speed = v.iter().map(|x| x * x).sum::<f64>().sqrt() as f32;
        let release = self.qc.release.take();
        let distance = release.map_or(0.0, |a| (0..3).map(|k| (QC_GOAL[k] - a[k]).powi(2)).sum::<f64>().sqrt() as f32);
        let (points, goal_type) = qc_points(release);
        tracing::info!(%from, points, speed, distance, release = ?release, "quarter court: goal");
        self.qc.hold_until = Some(now + QC_GOAL_HOLD);
        self.qc.goal = Some(QcGoal { by: from, points, goal_type, speed, distance });
        self.set_var(super::msg::VAR_GAME_STATE, LOBBY_ST_GOAL);
    }

    /// A quarter-court goal since the last call.
    pub fn take_goal(&mut self) -> Option<QcGoal> {
        self.qc.goal.take()
    }

    /// The goal state just ended: send `EV_ENABLE_SCORING`.
    pub fn take_round_start(&mut self) -> bool {
        std::mem::take(&mut self.qc.round_start)
    }

    fn qc_step(&mut self, now: Instant) {
        if self.qc.hold_until.is_some_and(|t| now >= t) {
            self.qc.hold_until = None;
            self.qc.round_start = true;
            self.set_var(super::msg::VAR_GAME_STATE, super::msg::ST_PLAYING);
        }
    }

    /// A player left: what they owned stays where they last reported it, asleep.
    pub fn forget(&mut self, who: SocketAddr) {
        if self.qc_holder == Some(who) {
            self.qc_holder = None;
            self.set_var(VAR_QC_HOLDER, 0xffff_ffff);
        }
        if self.pilot == Some(who) {
            self.engine_off();
        }
        for e in self.sets.iter_mut().flat_map(|s| s.entries.iter_mut()).filter(|e| e.owner == Some(who)) {
            e.release();
        }
    }

    /// Objects their owners stopped sending: the server sends them from
    /// here on, asleep where they were last reported.
    pub fn expire(&mut self, now: Instant) {
        for s in &mut self.sets {
            for e in s.entries.iter_mut().filter(|e| e.owner.is_some()) {
                if e.heard.is_none_or(|t| now.saturating_duration_since(t) > SILENCE) {
                    tracing::debug!(set = format_args!("{:08x}", s.id), entry = entry_of(&e.record), "lobby object: owner silent, the server sends it");
                    e.release();
                }
            }
        }
    }

    /// One message per set with the objects no client owns: `(mailbox key, body)`.
    pub fn broadcast(&self, frame: u64, now_us: u64) -> Vec<(u64, Vec<u8>)> {
        self.sets
            .iter()
            .filter_map(|s| {
                let mine: Vec<&Record> = s.entries.iter().filter(|e| e.owner.is_none()).map(|e| &e.record).collect();
                if mine.is_empty() {
                    return None;
                }
                let bits = HEAD * 8 + mine.len() * RECORD_BITS;
                let mut b = vec![0u8; bits.div_ceil(64) * 8];
                b[..4].copy_from_slice(&[0xff; 4]);
                b[4..8].copy_from_slice(&s.id.to_le_bytes());
                b[8..12].copy_from_slice(&(now_us as u32).to_le_bytes());
                b[16..20].copy_from_slice(&(mine.len() as u32 * COUNT_UNIT).to_le_bytes());
                for (i, r) in mine.iter().enumerate() {
                    for k in 0..RECORD_BITS {
                        if bit(*r, k) {
                            set_bit(&mut b, HEAD * 8 + i * RECORD_BITS + k, true);
                        }
                    }
                }
                Some((s.key_base + (frame % 180) * 0x11, b))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_object_its_owner_stopped_sending_is_the_servers_again() {
        let mut p = Props::new();
        let all = records(&p.broadcast(0, 0)[0].1).len();
        let mut r = records(&baselines()[0].2)[0];
        set_bit(&mut r, HELD, true);
        p.observe(addr(), &message(&r));
        let now = Instant::now();
        p.expire(now);
        assert_eq!(records(&p.broadcast(0, 0)[0].1).len(), all - 1);
        p.expire(now + SILENCE + Duration::from_millis(100));
        let sent = records(&p.broadcast(0, 0)[0].1);
        assert_eq!(sent.len(), all);
        let back = sent.iter().find(|x| entry_of(x) == entry_of(&r)).expect("entry");
        assert!(bit(back, ASLEEP) && !bit(back, HELD));
    }

    fn addr() -> SocketAddr {
        "127.0.0.1:6794".parse().expect("addr")
    }

    #[test]
    fn rotation_and_position_round_trip() {
        let mut r = records(&baselines()[0].2)[2];
        assert_eq!(entry_of(&r), FURY);
        // The Fury's resting place in the recording.
        let p = position(&r);
        assert!((p[0] + 15.894).abs() < 1e-3 && (p[1] - 0.753).abs() < 1e-3 && (p[2] - 10.488).abs() < 1e-3);
        let before = r;
        set_rotation(&mut r, rotation(&before));
        set_position(&mut r, position(&before));
        assert_eq!(r, before);
    }

    /// A client's one-record message.
    fn message(rec: &Record) -> Vec<u8> {
        let mut d = vec![0u8; 64];
        d[4..8].copy_from_slice(&FURY_SET.to_le_bytes());
        d[16..20].copy_from_slice(&COUNT_UNIT.to_le_bytes());
        for k in 0..RECORD_BITS {
            set_bit(&mut d, HEAD * 8 + k, bit(rec, k));
        }
        d
    }

    #[test]
    fn the_fury_flies_where_the_controller_faces_and_stops_after() {
        let mut p = Props::new();
        let start = Instant::now();
        let home = p.fury_home;
        // The controller in a hand, facing along +X (a quarter turn about Y).
        let mut c = records(&baselines()[0].2)[3];
        assert_eq!(entry_of(&c), FURY_CONTROLLER);
        set_bit(&mut c, HELD, true);
        let h = std::f64::consts::FRAC_1_SQRT_2;
        set_rotation(&mut c, [0.0, h, 0.0, h]);
        p.observe(addr(), &message(&c));
        let tick = std::time::Duration::from_micros(8333);
        let mut now = start;
        // Held without the trigger: it turns to the controller, it does not move.
        for _ in 0..(2 * 120) {
            now += tick;
            p.step(now);
        }
        assert!(p.active());
        let r = p.entry(FURY).expect("fury").record;
        let n = nose(rotation(&r));
        assert!(n[0] > 0.9, "{n:?}");
        let still = position(&r);
        assert!((0..3).all(|k| (still[k] - home[k]).abs() < 1e-3), "{still:?}");
        assert!(p.vars().0.is_empty());
        p.on_event(addr(), FURY_ENGINE_ON, 0);
        assert_eq!(p.vars(), (&[(VAR_FURY_ENGINE, 1)][..], true));
        for _ in 0..(8 * 120) {
            now += tick;
            p.step(now);
        }
        assert!(p.active());
        let at = position(&p.entry(FURY).expect("fury").record);
        // Eight seconds at up to 1.07 m/s, mostly along +X.
        assert!(at[0] - home[0] > 4.0 && at[0] - home[0] < 8.6, "{at:?}");
        assert!((at[1] - home[1]).abs() < 1.5 && (at[2] - home[2]).abs() < 1.5, "{at:?}");
        assert!(!bit(&p.entry(FURY).expect("fury").record, ASLEEP));
        // It is in what the server sends.
        assert_eq!(records(&p.broadcast(0, 0)[0].1).iter().filter(|r| entry_of(r) == FURY).count(), 1);
        // Trigger and controller let go: the engine var drops, the ship coasts on.
        p.on_event(addr(), FURY_ENGINE_OFF, 0);
        assert_eq!(p.vars(), (&[(VAR_FURY_ENGINE, 0)][..], true));
        set_bit(&mut c, HELD, false);
        p.observe(addr(), &message(&c));
        let speed = |p: &Props| p.flight.as_ref().map_or(0.0, |f| f.vel.iter().map(|x| x * x).sum::<f64>().sqrt());
        let before = speed(&p);
        for _ in 0..(3 * 120) {
            now += tick;
            p.step(now);
        }
        assert!(p.active());
        assert!(speed(&p) > before * 0.6, "{} {before}", speed(&p));
        // And sleeps in the end.
        for _ in 0..(60 * 120) {
            now += tick;
            p.step(now);
        }
        assert!(!p.active());
        assert!(bit(&p.entry(FURY).expect("fury").record, ASLEEP));
    }

    #[test]
    fn a_first_grab_of_each_lobby_disc_sets_natives_var() {
        let mut p = Props::new();
        let who: SocketAddr = "1.2.3.4:5".parse().expect("addr");
        p.on_grant(who, 7, 0xd33d_44b7, 1);
        assert_eq!(p.vars(), (&[(0xd279_7da9_9ab3_17a5, 1)][..], true));
        p.on_grant(who, 7, 0xd33d_44b7, 1);
        assert!(!p.vars().1);
        p.on_grant(who, 7, 0xb1c7_5c04, 3);
        assert_eq!(p.vars().0.len(), 1);
    }

    // Every combat-room barrier breaks out of sight, not only the three a capture saw.
    #[test]
    fn every_lobby_barrier_has_its_var() {
        let handles: Vec<u16> = include_str!("data/lobby_combat_barriers.txt")
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.split_whitespace().nth(1).unwrap().parse().unwrap())
            .collect();
        let ours: Vec<u16> = LOBBY_BARRIER_VARS.iter().map(|&(h, _)| h).collect();
        assert_eq!(handles, ours);
    }

    // The quarter-court disc's holder var follows each grant, and is the
    // server's (`ffffffff`) once the holder leaves (captures/netrec-info-20261118).
    #[test]
    fn quarter_court_holder_var_follows_grants() {
        let mut p = Props::new();
        let a: SocketAddr = "1.2.3.4:5".parse().expect("addr");
        let b: SocketAddr = "1.2.3.4:6".parse().expect("addr");
        p.on_grant(a, 0x0002_0001, QC_SET, 0);
        assert!(p.vars().0.contains(&(VAR_QC_HOLDER, 0x0002_0001)));
        p.on_grant(b, 5, QC_SET, 0);
        assert!(p.vars().0.contains(&(VAR_QC_HOLDER, 5)));
        p.forget(a);
        assert!(p.vars().0.contains(&(VAR_QC_HOLDER, 5)));
        p.forget(b);
        assert!(p.vars().0.contains(&(VAR_QC_HOLDER, 0xffff_ffff)));
    }

    // Moves of the disc from the native recording (two samples each).
    #[test]
    fn the_ring_takes_native_goals_and_only_them() {
        // Goals (146.6 s, 242.3 s, 419.9 s, 447.4 s, 601.9 s): the two
        // samples either side of the ring.
        assert!(through_the_ring([-53.543, 1.444, -30.648], [-53.571, 1.396, -30.916]));
        assert!(through_the_ring([-53.452, 1.159, -30.583], [-53.529, 1.13, -30.965]));
        assert!(through_the_ring([-53.851, 1.387, -29.97], [-54.214, 1.488, -30.078]));
        assert!(through_the_ring([-53.641, 1.647, -30.691], [-53.926, 1.435, -30.548]));
        assert!(through_the_ring([-53.003, 1.44, -31.594], [-53.302, 1.419, -31.334]));
        // The same move backwards: from behind the ring.
        assert!(!through_the_ring([-53.571, 1.396, -30.916], [-53.543, 1.444, -30.648]));
        // Past the ring's edge (463.7 s, 1.42 m from its centre): no goal.
        assert!(!through_the_ring([-54.085, 2.331, -29.903], [-54.125, 2.435, -30.062]));
    }

    // Native's goals were all 2-pointers: first free positions of three of
    // them (146.1 s, 241.9 s, 419.6 s) and one close to the ring. A shot from
    // well outside the bubble is a 3-pointer, and an unseen release 2.
    #[test]
    fn the_bubble_splits_twos_and_threes() {
        use super::super::stats::{GOAL_INSIDE_SHOT, GOAL_LONG_SHOT};
        for at in [[-52.82, 2.693, -23.675], [-52.217, 1.624, -24.457], [-47.613, -0.347, -28.12], [-53.003, 1.44, -33.594]] {
            assert_eq!(qc_points(Some(at)), (2, GOAL_INSIDE_SHOT), "{at:?}");
        }
        assert_eq!(qc_points(Some([-45.0, 1.5, -26.0])), (3, GOAL_LONG_SHOT));
        assert_eq!(qc_points(Some([-57.241, 1.975, -20.447])), (3, GOAL_LONG_SHOT));
        assert_eq!(qc_points(None), (2, GOAL_INSIDE_SHOT));
    }

    #[test]
    fn a_shot_from_outside_the_bubble_scores_three() {
        let mut p = Props::new();
        let who: SocketAddr = "1.2.3.4:5".parse().expect("addr");
        p.observe(who, &qc_record([-44.9, 1.6, -25.8], true));
        p.observe(who, &qc_record([-45.0, 1.5, -26.0], false));
        p.observe(who, &qc_record([-53.543, 1.444, -30.648], false));
        p.observe(who, &qc_record([-53.571, 1.396, -30.916], false));
        let g = p.take_goal().expect("goal");
        assert_eq!(g.points, 3);
    }

    fn qc_record(pos: [f64; 3], held: bool) -> Vec<u8> {
        let mut r = [0u8; RECORD_BYTES];
        set_field(&mut r, ENTRY_BIT, ENTRY_BITS, u32::from(QC_ENTRY));
        set_bit(&mut r, HELD, held);
        set_position(&mut r, pos);
        set_velocity(&mut r, [-1.25, -2.15, -12.06]);
        let mut d = vec![0u8; (HEAD * 8 + RECORD_BITS).div_ceil(64) * 8];
        d[..4].copy_from_slice(&2u32.to_le_bytes());
        d[4..8].copy_from_slice(&QC_SET.to_le_bytes());
        d[16..20].copy_from_slice(&COUNT_UNIT.to_le_bytes());
        for k in 0..RECORD_BITS {
            if bit(&r, k) {
                set_bit(&mut d, HEAD * 8 + k, true);
            }
        }
        d
    }

    #[test]
    fn a_goal_holds_the_lobby_three_seconds() {
        let mut p = Props::new();
        let who: SocketAddr = "1.2.3.4:5".parse().expect("addr");
        // Native 146.06 s: the last sample in the hand, the first free one.
        p.observe(who, &qc_record([-52.798, 2.743, -23.436], true));
        p.observe(who, &qc_record([-52.82, 2.693, -23.675], false));
        p.observe(who, &qc_record([-53.543, 1.444, -30.648], false));
        assert!(p.take_goal().is_none());
        p.observe(who, &qc_record([-53.571, 1.396, -30.916], false));
        let g = p.take_goal().expect("goal");
        assert_eq!(g.by, who);
        assert_eq!((g.points, g.goal_type), (2, super::super::stats::GOAL_INSIDE_SHOT));
        assert!((g.speed - 12.3).abs() < 0.1, "{}", g.speed);
        // Native reported 7.41 m.
        assert!((g.distance - 7.4).abs() < 0.2, "{}", g.distance);
        assert!(p.vars().0.contains(&(super::super::msg::VAR_GAME_STATE, LOBBY_ST_GOAL)));
        // Through again during the goal: nothing.
        p.observe(who, &qc_record([-53.543, 1.444, -30.648], false));
        p.observe(who, &qc_record([-53.571, 1.396, -30.916], false));
        assert!(p.take_goal().is_none());
        p.step(Instant::now());
        assert!(!p.take_round_start());
        p.step(Instant::now() + QC_GOAL_HOLD);
        assert!(p.take_round_start());
        assert!(p.vars().0.contains(&(super::super::msg::VAR_GAME_STATE, super::super::msg::ST_PLAYING)));
    }

    #[test]
    fn untouched_sets_are_sent_as_native_sent_them() {
        let p = Props::new();
        let out = p.broadcast(0, 0);
        assert_eq!(out.len(), 2);
        for ((_, _, native), (_, ours)) in baselines().iter().zip(&out) {
            assert_eq!(ours.len(), native.len());
            // All but the time word.
            assert_eq!(ours[..8], native[..8]);
            assert_eq!(ours[12..], native[12..]);
        }
    }

    #[test]
    fn a_taken_object_leaves_the_servers_list_and_returns_asleep() {
        let mut p = Props::new();
        let before = p.broadcast(0, 0)[0].1.len();
        // A client's message for the main disc (entry 0), held: one record, 64 bytes.
        let mut rec = records(&baselines()[0].2)[0];
        set_bit(&mut rec, ASLEEP, false);
        set_bit(&mut rec, HELD, true);
        let mut d = vec![0u8; 64];
        d[4..8].copy_from_slice(&0xb1c7_5c04u32.to_le_bytes());
        d[16..20].copy_from_slice(&COUNT_UNIT.to_le_bytes());
        for k in 0..RECORD_BITS {
            set_bit(&mut d, HEAD * 8 + k, bit(&rec, k));
        }
        p.observe(addr(), &d);
        let during = &p.broadcast(0, 0)[0].1;
        assert!(during.len() < before);
        assert_eq!(u32::from_le_bytes(during[16..20].try_into().expect("4")), 17 * COUNT_UNIT);
        p.forget(addr());
        let after = &p.broadcast(0, 0)[0].1;
        assert_eq!(after.len(), before);
        let recs = records(after);
        let disc = recs.iter().find(|r| entry_of(r) == 0).expect("entry 0");
        assert!(bit(disc, ASLEEP) && !bit(disc, HELD));
    }

    /// Native's lobby combat room (netrec-native-srv-1201): area events on a
    /// handle set that handle's var; a broken barrier's var is 0 until it
    /// respawns; untouched barriers send nothing.
    #[test]
    fn lobby_reflexes_and_barrier_vars_follow_native() {
        let mut p = Props::new();
        p.set_barriers_down(&[]);
        assert!(p.vars().0.is_empty());
        p.on_event(addr(), 0xfbc0_427a_c211_919e, 0xb7);
        assert_eq!(p.vars(), (&[(0xab54_4d4b_2113_0c68, 0)][..], true));
        // The same event on another handle is another area's var.
        p.on_event(addr(), 0xfbc0_427a_c211_919f, 0xa8);
        assert!(p.vars().0.contains(&(0xec48_aa3f_f874_8bae, 0x3f80_0000)));
        p.set_barriers_down(&[65]);
        assert!(p.vars().0.contains(&(0x6a2a_ca43_5523_d9d2, 0)));
        p.set_barriers_down(&[]);
        let (v, changed) = p.vars();
        assert!(changed && v.contains(&(0x6a2a_ca43_5523_d9d2, 1)));
        assert!(!v.iter().any(|e| e.0 == 0x7452_cb20_8066_2e3a));
    }
}
