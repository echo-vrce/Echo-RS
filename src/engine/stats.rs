//! Arena match stats: per-player running totals and the disc tracker that
//! credits them.
//!
//! **Native sends running match totals, not deltas.** In the recorded native
//! game (`captures/native-20261111`) every `SR15NetReliableStatUpdate` repeats
//! the whole total each 3 s (`"Points":{"op":"add","val":2}` again and again,
//! then 5 after a 3-pointer). nevr-server-rs reached the same conclusion from
//! the binary (`OnScoreCB` accumulates, nothing clears on send).
//!
//! The twelve screen stats, in native's list order. The team update's twelve
//! record hashes follow this same order (`msg::TEAM_STATS`): the recorded
//! team records carry Points at 0, PossessionTime at 4, ShotsOnGoal at 6 and
//! Goals at 8, which is exactly this list.
//!
//! What native's server derives itself (stuns, saves, steals...) has no
//! readable producer, so these are credited from what the server sees:
//! - **Possession, passes, catches, steals, interceptions, saves, shots,
//!   assists, 2/3 points**: from the disc transform, but only the copy sent by
//!   the client that the disc names as its owner (that client simulates it).
//!   A zero-g disc in free flight moves in a straight line at constant speed;
//!   a held one follows a hand. That tells a throw from a carry.
//! - **Stuns**: one per `SR15StunPlayerMsg` the stunner's client sends.
//!
//! Distances are in the disc transform's units (millimetres).

use std::collections::VecDeque;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stat {
    Points = 0,
    Assists,
    Saves,
    Stuns,
    PossessionTime,
    Steals,
    ShotsOnGoal,
    Interceptions,
    Goals,
    /// No rule credits it yet; it keeps its slot in the list.
    #[allow(dead_code)]
    Blocks,
    Catches,
    Passes,
}

pub const STAT_NAMES: [&str; 12] = [
    "Points",
    "Assists",
    "Saves",
    "Stuns",
    "PossessionTime",
    "Steals",
    "ShotsOnGoal",
    "Interceptions",
    "Goals",
    "Blocks",
    "Catches",
    "Passes",
];

/// One player's running totals. Possession is counted in server frames.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Stats {
    pub v: [u32; 12],
}

impl Stats {
    pub fn add(&mut self, s: Stat, n: u32) {
        self.v[s as usize] += n;
    }
    pub fn get(&self, s: Stat) -> u32 {
        self.v[s as usize]
    }

    /// The `SR15NetReliableStatUpdate` JSON: every non-zero total, as native
    /// writes them (possession as a real in seconds, the rest integers).
    pub fn to_json(&self, frame_hz: u64) -> String {
        let mut fields = Vec::new();
        for (i, name) in STAT_NAMES.iter().enumerate() {
            let n = self.v[i];
            if n == 0 {
                continue;
            }
            let val = if i == Stat::PossessionTime as usize { format!("{:.6}", n as f32 / frame_hz as f32) } else { n.to_string() };
            fields.push(format!("\"{name}\":{{\"op\":\"add\",\"val\":{val}}}"));
        }
        format!("{{{}}}", fields.join(","))
    }
}

/// Goal types, 1-based in the client's list: slam dunk, inside shot, long
/// shot, bounce shot, long bounce shot, headbutt, long headbutt, bumper
/// shot, long bumper shot, self goal. Native's carried-in goal sent 1.
pub const GOAL_SLAM_DUNK: u16 = 1;
pub const GOAL_INSIDE_SHOT: u16 = 2;
pub const GOAL_LONG_SHOT: u16 = 3;
pub const GOAL_SELF: u16 = 10;

/// Goal plane depth and ring radius, as the goal detector uses them.
pub const GOAL_DEPTH: i32 = 36_000;
pub const GOAL_RADIUS: i32 = 1_400;
/// A throw released at least this far from the goal it scores in is worth 3.
/// Native's one recorded 3-pointer was thrown from 8.86 m and its 2-pointer
/// from 0.53 m; the true line is unrecorded, so this is an estimate.
pub const THREE_POINT_MM: f32 = 7_000.0;
/// An assist: the scorer's teammate handed off to the scorer this recently.
pub const ASSIST_WINDOW_US: u64 = 10_000_000;
/// Free flight: at least this fast (mm/ms == m/s)...
const FLIGHT_MIN_SPEED: f32 = 0.6;
/// ...with consecutive velocities this close (m/s).
const FLIGHT_TOLERANCE: f32 = 0.15;
const HISTORY: usize = 48;

#[derive(Clone, Copy, Debug)]
struct Sample {
    t_us: u64,
    /// The sender's own clock (µs, wraps), from the transform: free of
    /// network jitter, so velocities use it when present.
    client_us: Option<u32>,
    p: [i32; 3],
}

#[derive(Clone, Copy, Debug)]
struct Flight {
    thrower: u16,
    start: [i32; 3],
    /// Heading into the goal the thrower attacks.
    on_goal: bool,
}

/// What the tracker reports about a goal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoalInfo {
    pub points: u32,
    /// The client's goal type list, 1-based: see `GOAL_*`.
    pub goal_type: u16,
    pub assister: Option<u16>,
    /// Disc speed at the goal (m/s) and release-to-goal distance (m).
    pub speed: f32,
    pub distance: f32,
}

#[derive(Default)]
pub struct DiscTracker {
    hist: VecDeque<Sample>,
    owner: Option<(u16, u8)>,
    /// The previous owner, its team and when the current owner took over.
    handoff: Option<(u16, u8, u64)>,
    flight: Option<Flight>,
    /// The current flight already credited a shot on goal.
    shot_credited: bool,
    pub credits: Vec<(u16, Stat)>,
}

fn sub(a: [i32; 3], b: [i32; 3]) -> [f32; 3] {
    [(a[0] - b[0]) as f32, (a[1] - b[1]) as f32, (a[2] - b[2]) as f32]
}
fn len(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Blue attacks +depth (the goal detector: `blue_scored = depth > 0`).
fn attack_sign(team: u8) -> f32 {
    if team == 0 { 1.0 } else { -1.0 }
}

impl DiscTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything (a goal, a relaunch: the server holds the disc).
    pub fn reset(&mut self) {
        self.hist.clear();
        self.owner = None;
        self.handoff = None;
        self.flight = None;
        self.shot_credited = false;
    }

    pub fn owner(&self) -> Option<u16> {
        self.owner.map(|(s, _)| s)
    }

    /// Velocity (mm/ms) between the last two distinct samples.
    fn velocity(&self, back: usize) -> Option<[f32; 3]> {
        let n = self.hist.len();
        if n < back + 2 {
            return None;
        }
        let (a, b) = (self.hist[n - 2 - back], self.hist[n - 1 - back]);
        let dt_us = match (a.client_us, b.client_us) {
            (Some(x), Some(y)) => u64::from(y.wrapping_sub(x)),
            _ => b.t_us.saturating_sub(a.t_us),
        };
        let dt = dt_us as f32 / 1000.0;
        if dt <= 0.0 {
            return None;
        }
        let d = sub(b.p, a.p);
        Some([d[0] / dt, d[1] / dt, d[2] / dt])
    }

    /// Straight, steady and fast over the last three intervals.
    fn in_free_flight(&self) -> bool {
        let (Some(v0), Some(v1), Some(v2)) = (self.velocity(0), self.velocity(1), self.velocity(2)) else { return false };
        len(v0) >= FLIGHT_MIN_SPEED && len(sub_f(v0, v1)) <= FLIGHT_TOLERANCE && len(sub_f(v1, v2)) <= FLIGHT_TOLERANCE
    }

    /// Does the current straight line carry the disc into `team`'s target goal?
    fn heading_into_goal(&self, team: u8) -> bool {
        let (Some(v), Some(last)) = (self.velocity(0), self.hist.back()) else { return false };
        let sign = attack_sign(team);
        if v[2] * sign <= 0.0 {
            return false;
        }
        let plane = GOAL_DEPTH as f32 * sign;
        let t = (plane - last.p[2] as f32) / v[2];
        if t < 0.0 {
            return false;
        }
        let (x, y) = (last.p[0] as f32 + v[0] * t, last.p[1] as f32 + v[1] * t);
        x * x + y * y <= (GOAL_RADIUS as f32 * 1.1).powi(2)
    }

    /// A disc transform from the client the disc names as its owner.
    /// `team` is that owner's team.
    /// `held`: the client's own word on whether the disc is in a hand
    /// (None: unknown, judged from the motion instead).
    pub fn sample(&mut self, t_us: u64, client_us: Option<u32>, owner: u16, team: u8, p: [i32; 3], held: Option<bool>) {
        if self.hist.back().is_some_and(|s| s.p == p) {
            return; // the client repeats an unchanged transform
        }
        if self.owner.map(|(s, _)| s) != Some(owner) {
            self.change_owner(t_us, owner, team);
        }
        let release = self.hist.back().map(|s| s.p);
        self.hist.push_back(Sample { t_us, client_us, p });
        if self.hist.len() > HISTORY {
            self.hist.pop_front();
        }
        if held == Some(true) {
            self.flight = None; // carried, however fast
            return;
        }
        if held == Some(false) && self.flight.is_none() {
            // Just released: the flight starts at the last held position.
            self.flight = Some(Flight { thrower: owner, start: release.unwrap_or(p), on_goal: false });
            self.shot_credited = false;
        }
        if held == Some(false) || self.in_free_flight() {
            if self.flight.is_none() {
                // Released: the flight starts where the straight line began.
                let n = self.hist.len();
                let start = self.hist[n.saturating_sub(4)].p;
                self.flight = Some(Flight { thrower: owner, start, on_goal: false });
                self.shot_credited = false;
            }
            let on_goal = self.flight.as_ref().is_some_and(|f| f.thrower == owner) && self.heading_into_goal(team);
            if let Some(f) = self.flight.as_mut() {
                f.on_goal = on_goal;
            }
            if on_goal && !self.shot_credited {
                self.shot_credited = true;
                self.credits.push((owner, Stat::ShotsOnGoal));
            }
        } else if self.flight.is_some_and(|f| f.thrower == owner) && self.velocity(0).is_some_and(|v| len(v) < FLIGHT_MIN_SPEED * 0.5) {
            // Caught again by the thrower, or stopped dead.
            self.flight = None;
        }
    }

    fn change_owner(&mut self, t_us: u64, to: u16, to_team: u8) {
        let flight = self.flight.take();
        if let Some((from, from_team)) = self.owner {
            match (from_team == to_team, flight) {
                (true, Some(_)) => {
                    self.credits.push((from, Stat::Passes));
                    self.credits.push((to, Stat::Catches));
                }
                (true, None) => {}
                (false, Some(f)) if f.on_goal => self.credits.push((to, Stat::Saves)),
                (false, Some(_)) => self.credits.push((to, Stat::Interceptions)),
                (false, None) => self.credits.push((to, Stat::Steals)),
            }
            self.handoff = Some((from, from_team, t_us));
        }
        self.owner = Some((to, to_team));
        self.shot_credited = false;
        self.hist.clear();
    }

    /// The owner possesses the disc this frame (not while it flies free).
    pub fn holding(&self) -> Option<u16> {
        if self.flight.is_some() { None } else { self.owner() }
    }

    /// A goal by the current owner, crossing at `at`, for `scoring_team`.
    /// Credits the scorer's stats and returns what the score messages carry.
    pub fn goal(&mut self, t_us: u64, at: [i32; 3], scoring_team: u8) -> GoalInfo {
        let speed = self.velocity(0).map(len).unwrap_or(0.0);
        let mut info = GoalInfo { points: 2, goal_type: GOAL_SLAM_DUNK, assister: None, speed, distance: 0.0 };
        let Some((scorer, team)) = self.owner else { return info };
        if team != scoring_team {
            info.goal_type = GOAL_SELF;
            return info; // an own goal: the team scores, nobody is credited
        }
        if let Some(f) = self.flight.filter(|f| f.thrower == scorer) {
            info.distance = len(sub(at, f.start)) / 1000.0;
            if info.distance * 1000.0 >= THREE_POINT_MM {
                info.points = 3;
                info.goal_type = GOAL_LONG_SHOT;
            } else {
                info.goal_type = GOAL_INSIDE_SHOT;
            }
        }
        if !self.shot_credited {
            self.credits.push((scorer, Stat::ShotsOnGoal));
        }
        self.credits.push((scorer, Stat::Goals));
        for _ in 0..info.points {
            self.credits.push((scorer, Stat::Points));
        }
        if let Some((from, from_team, at_us)) = self.handoff
            && from_team == team
            && from != scorer
            && t_us.saturating_sub(at_us) <= ASSIST_WINDOW_US
        {
            self.credits.push((from, Stat::Assists));
            info.assister = Some(from);
        }
        info
    }
}

fn sub_f(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1000;

    /// Feed a straight flight: `n` samples, 33 ms apart, `v` mm per sample.
    fn fly(t: &mut DiscTracker, t0: u64, owner: u16, team: u8, from: [i32; 3], v: [i32; 3], n: usize) -> (u64, [i32; 3]) {
        let mut p = from;
        let mut at = t0;
        for _ in 0..n {
            t.sample(at, None, owner, team, p, None);
            p = [p[0] + v[0], p[1] + v[1], p[2] + v[2]];
            at += 33 * MS;
        }
        (at, p)
    }

    /// Hand jitter: a held disc wobbles.
    fn hold(t: &mut DiscTracker, t0: u64, owner: u16, team: u8, at: [i32; 3], n: usize) -> u64 {
        let mut ts = t0;
        for i in 0..n {
            let w = if i % 2 == 0 { 40 } else { -25 };
            t.sample(ts, None, owner, team, [at[0] + w, at[1] - w, at[2] + (i as i32 % 3) * 15], None);
            ts += 33 * MS;
        }
        ts
    }

    /// The team-record hashes are a length-keyed symbol hash of these
    /// names: same-length names share the top 12 bits (Points/Steals/Blocks/
    /// Passes, Saves/Stuns/Goals, Assists/Catches). That pins the order.
    #[test]
    fn team_hashes_follow_stat_names() {
        let ts = crate::engine::msg::TEAM_STATS;
        for i in 0..12 {
            for j in 0..12 {
                let same_len = STAT_NAMES[i].len() == STAT_NAMES[j].len();
                assert_eq!(ts[i] >> 52 == ts[j] >> 52, same_len, "{} vs {}", STAT_NAMES[i], STAT_NAMES[j]);
            }
        }
    }

    /// A straight 13 m/s throw whose packets reach the server 10..12.5 ms
    /// apart (network jitter) but are stamped 11 ms apart on the client's
    /// clock: still a flight, timed by the client clock.
    #[test]
    fn jittered_arrivals_still_a_flight() {
        let mut t = DiscTracker::new();
        let at = hold(&mut t, 0, 0, 0, [0, 0, 24_000], 10);
        let (mut ts, mut cs) = (at, 5_000_000u32);
        let mut p = [0, 0, 24_000];
        for i in 0..90 {
            t.sample(ts, Some(cs), 0, 0, p, None);
            p = [p[0], p[1], p[2] + 143];
            ts += if i % 2 == 0 { 10_000 } else { 12_500 };
            cs = cs.wrapping_add(11_000);
        }
        let g = t.goal(ts, [0, 0, 36_000], 0);
        assert_eq!(g.points, 3, "{g:?}");
        assert!((g.distance - 12.0).abs() < 0.5, "{}", g.distance);
    }

    #[test]
    fn json_is_running_totals() {
        let mut s = Stats::default();
        s.add(Stat::Points, 2);
        s.add(Stat::Stuns, 1);
        s.add(Stat::PossessionTime, 30);
        assert_eq!(
            s.to_json(20),
            r#"{"Points":{"op":"add","val":2},"Stuns":{"op":"add","val":1},"PossessionTime":{"op":"add","val":1.500000}}"#
        );
        assert_eq!(Stats::default().to_json(20), "{}");
    }

    #[test]
    fn native_three_pointer_scores_three() {
        // The recorded native 3-pointer: released near (418,3652,27732) at
        // ~2.4 m/s, scored at (-378,672,36000), reported 8.855 m.
        let mut t = DiscTracker::new();
        let ts = hold(&mut t, 0, 0, 0, [400, 3650, 27700], 10);
        let (ts, _) = fly(&mut t, ts, 0, 0, [418, 3652, 27732], [-10, -30, 80], 60);
        let g = t.goal(ts, [-378, 672, 36_000], 0);
        assert_eq!((g.points, g.goal_type), (3, GOAL_LONG_SHOT));
        assert!((g.distance - 8.8).abs() < 0.3, "{}", g.distance);
        let pts = t.credits.iter().filter(|c| c.1 == Stat::Points).count();
        assert_eq!(pts, 3);
        assert_eq!(t.credits.iter().filter(|c| c.1 == Stat::ShotsOnGoal).count(), 1);
    }

    #[test]
    fn carried_in_is_two_points() {
        let mut t = DiscTracker::new();
        hold(&mut t, 0, 0, 0, [0, 0, 35_500], 20);
        let g = t.goal(1_000 * MS, [0, 0, 36_000], 0);
        assert_eq!((g.points, g.goal_type), (2, GOAL_SLAM_DUNK));
    }

    #[test]
    fn pass_catch_and_assist() {
        let mut t = DiscTracker::new();
        let ts = hold(&mut t, 0, 1, 0, [0, 0, 10_000], 10);
        let (ts, p) = fly(&mut t, ts, 1, 0, [0, 0, 10_000], [0, 100, 0], 10);
        let ts = hold(&mut t, ts, 2, 0, p, 10);
        t.goal(ts, [0, 0, 36_000], 0);
        assert!(t.credits.contains(&(1, Stat::Passes)));
        assert!(t.credits.contains(&(2, Stat::Catches)));
        assert!(t.credits.contains(&(1, Stat::Assists)));
    }

    #[test]
    fn defender_touching_a_shot_is_a_save() {
        let mut t = DiscTracker::new();
        let ts = hold(&mut t, 0, 1, 0, [0, 0, 20_000], 10);
        let (ts, p) = fly(&mut t, ts, 1, 0, [0, 0, 20_000], [0, 0, 150], 10);
        assert!(t.credits.contains(&(1, Stat::ShotsOnGoal)));
        hold(&mut t, ts, 5, 1, p, 5);
        assert!(t.credits.contains(&(5, Stat::Saves)));
    }

    #[test]
    fn steal_and_interception() {
        let mut t = DiscTracker::new();
        let ts = hold(&mut t, 0, 1, 0, [0, 0, 0], 10);
        let ts = hold(&mut t, ts, 5, 1, [0, 0, 0], 10);
        assert!(t.credits.contains(&(5, Stat::Steals)));
        // A pass sideways (not on goal) taken by the other team.
        let (ts, p) = fly(&mut t, ts, 5, 1, [0, 0, 0], [120, 0, 0], 10);
        hold(&mut t, ts, 2, 0, p, 5);
        assert!(t.credits.contains(&(2, Stat::Interceptions)));
    }

    #[test]
    fn possession_only_while_held() {
        let mut t = DiscTracker::new();
        let ts = hold(&mut t, 0, 1, 0, [0, 0, 0], 10);
        assert_eq!(t.holding(), Some(1));
        fly(&mut t, ts, 1, 0, [0, 0, 0], [0, 100, 0], 10);
        assert_eq!(t.holding(), None);
    }

    #[test]
    fn own_goal_credits_nobody() {
        let mut t = DiscTracker::new();
        hold(&mut t, 0, 5, 1, [0, 0, 35_000], 10);
        t.goal(1_000 * MS, [0, 0, 36_000], 0);
        assert!(t.credits.is_empty());
    }
}

#[cfg(test)]
mod held_tests {
    use super::*;

    #[test]
    fn carried_in_fast_is_a_slam_dunk() {
        let mut t = DiscTracker::new();
        // Carried at 8 m/s straight at the goal: still held.
        for i in 0..40u64 {
            t.sample(i * 16_000, None, 0, 0, [0, 0, 20_000 + i as i32 * 128], Some(true));
        }
        let g = t.goal(700_000, [0, 0, 36_000], 0);
        assert_eq!((g.points, g.goal_type), (2, GOAL_SLAM_DUNK));
    }

    #[test]
    fn released_long_throw_is_a_long_shot() {
        let mut t = DiscTracker::new();
        t.sample(0, None, 0, 0, [0, 0, 20_000], Some(true));
        for i in 1..40u64 {
            t.sample(i * 16_000, None, 0, 0, [0, 0, 20_000 + i as i32 * 200], Some(false));
        }
        let g = t.goal(700_000, [0, 0, 36_000], 0);
        assert_eq!((g.points, g.goal_type), (3, GOAL_LONG_SHOT));
    }
}

// ── Combat ────────────────────────────────────────────────────────────────

/// A combat player's match totals, as native's combat server keeps them
/// (server recordings, 2026-11-16): `SR15NetReliableStatUpdate` every 3 s
/// with the running totals, `{}` for a player with none.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct CombatStats {
    pub objective_damage: f32,
    pub eliminations: u32,
    pub objective_eliminations: u32,
    pub objective_time: f32,
    pub kills: u32,
    pub assists: u32,
    pub deaths: u32,
    pub damage: f32,
    pub damage_taken: f32,
    /// Seconds of play on a team (not sent: it decides the XP).
    pub played: f32,
}

/// The team update's eight records, in native's order.
pub const COMBAT_TEAM_STATS: [&str; 8] = [
    "CombatEliminations",
    "CombatObjectiveEliminations",
    "CombatObjectiveTime",
    "CombatObjectiveDamage",
    "CombatKills",
    "CombatAssists",
    "CombatDeaths",
    "CombatDamage",
];

/// A stat value as native's team record writes it: `m * 2^e`. Counts are
/// whole (`e` 0); times and damage are reals, on a 1/64 grid, a whole one
/// with its factors of two moved into `e` (native: 110 damage = 55 * 2^1).
pub fn mantissa_exp(v: f32, real: bool) -> (i32, i32) {
    if !real || v == 0.0 {
        return (v as i32, 0);
    }
    if v.fract() == 0.0 {
        let (mut m, mut e) = (v as i64, 0);
        while m != 0 && m % 2 == 0 {
            m /= 2;
            e += 1;
        }
        return (m as i32, e);
    }
    ((v * 64.0).round() as i32, -6)
}

impl CombatStats {
    /// (name, value, real) in the order native writes a player's JSON.
    fn fields(&self) -> [(&'static str, f32, bool); 9] {
        [
            ("CombatObjectiveDamage", self.objective_damage, true),
            ("CombatEliminations", self.eliminations as f32, false),
            ("CombatObjectiveEliminations", self.objective_eliminations as f32, false),
            ("CombatObjectiveTime", self.objective_time, true),
            ("CombatKills", self.kills as f32, false),
            ("CombatAssists", self.assists as f32, false),
            ("CombatDeaths", self.deaths as f32, false),
            ("CombatDamage", self.damage, true),
            ("CombatDamageTaken", self.damage_taken, true),
        ]
    }

    /// One team record's value: by its name in `COMBAT_TEAM_STATS`.
    pub fn get(&self, name: &str) -> (f32, bool) {
        self.fields().iter().find(|f| f.0 == name).map_or((0.0, false), |f| (f.1, f.2))
    }

    /// The `SR15NetReliableStatUpdate` JSON: every non-zero total (native
    /// leaves out what a player has none of; damage taken is profile-only).
    pub fn to_json(&self) -> String {
        let mut out = Vec::new();
        for (name, v, real) in self.fields() {
            if v == 0.0 || name == "CombatDamageTaken" {
                continue;
            }
            let val = if real { format!("{v}") } else { format!("{}", v as u32) };
            out.push(format!("\"{name}\":{{\"op\":\"add\",\"val\":{val}}}"));
        }
        format!("{{{}}}", out.join(","))
    }

    /// Native's MVP: the most to show for the match (no formula recorded;
    /// it went to the player ahead on eliminations and objective time in
    /// every recording).
    pub fn score(&self) -> u64 {
        (self.eliminations as f32 * 100.0 + self.objective_eliminations as f32 * 50.0 + self.objective_time * 10.0 + self.damage + self.objective_damage) as u64
    }
}

/// XP for a match (native, all four recorded maps): 1200 for a win, 1000 for
/// a loss, for a player there for most of it (late joiners got only their
/// tier reward); +100 for the MVP; + the tier reward's XP.
pub fn combat_xp(won: bool, stayed: bool, mvp: bool, tier_xp: u64) -> u64 {
    let base = match (stayed, won) {
        (false, _) => 0,
        (true, true) => 1200,
        (true, false) => 1000,
    };
    base + if mvp { 100 } else { 0 } + tier_xp
}

#[cfg(test)]
mod combat_tests {
    use super::*;

    /// Native's team records (Dyson, 181.73 s): 110 damage = 55e1, 8.97 s = 574e-6.
    #[test]
    fn team_values_encode_like_native() {
        assert_eq!(mantissa_exp(110.0, true), (55, 1));
        assert_eq!(mantissa_exp(65.0, true), (65, 0));
        assert_eq!(mantissa_exp(313.80167, true), (20083, -6));
        assert_eq!(mantissa_exp(7.0, false), (7, 0));
        assert_eq!(mantissa_exp(0.0, true), (0, 0));
    }

    /// Native Dyson's XP: the MVP winner with a gold 1450, the loser with a gold 1150.
    #[test]
    fn xp_matches_native() {
        assert_eq!(combat_xp(true, true, true, 150), 1450);
        assert_eq!(combat_xp(false, true, false, 150), 1150);
        assert_eq!(combat_xp(true, true, false, 0), 1200);
        assert_eq!(combat_xp(false, false, false, 150), 150);
    }

    /// The team records' symbols, as native's wire bytes.
    #[test]
    fn team_stat_symbols_are_natives() {
        let wire = |n: &str| crate::sns::symbol(n).to_le_bytes();
        assert_eq!(wire("CombatKills"), [0x4b, 0x69, 0xe9, 0xaa, 0x02, 0x71, 0x50, 0x11]);
        assert_eq!(wire("CombatObjectiveTime"), [0x99, 0x59, 0x4a, 0xf1, 0x72, 0xc4, 0xd2, 0xf8]);
        assert_eq!(wire("CombatEliminations"), [0x6f, 0x8d, 0xd3, 0x65, 0x48, 0x09, 0x82, 0x26]);
    }

    #[test]
    fn json_leaves_out_zeros() {
        let s = CombatStats { kills: 2, eliminations: 2, damage: 35.5, ..Default::default() };
        assert_eq!(s.to_json(), r#"{"CombatEliminations":{"op":"add","val":2},"CombatKills":{"op":"add","val":2},"CombatDamage":{"op":"add","val":35.5}}"#);
        assert_eq!(CombatStats::default().to_json(), "{}");
    }
}
