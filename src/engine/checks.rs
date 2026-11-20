//! Server-side checks on what clients claim.
//!
//! Each client reports its own shots (`GameFireGun`), its hits
//! (`DamageableBulletCollision`, with the damage it says the shot did) and its
//! body's position. A modified client could claim anything, so the server
//! checks each against the game's own rules:
//!
//! - **Damage**: a hit's damage must be one its gun can do (the weapon
//!   settings' `projectile_damage`, or COMET's charge range).
//! - **Fire rate**: no more shots in 2 s than the gun's fire and refire delays
//!   (and the rifle's 4-round bursts) allow, with room for network bunching.
//! - **Hit position**: a hit on a player must land near that player's body,
//!   and the level's own walls must not stand between the shooter and it.
//! - **Speed**: a body moving faster than anyone can, for a sustained stretch
//!   (one jump, such as a respawn or teleport, is no flag).
//!
//! Modes (`--cheat-checks`): `log` (the default) writes a warning per flag and
//! changes nothing; `enforce` also caps impossible damage at the gun's most
//! and drops shots over the rate and hits far from their victim; `off`.
//! Hits through walls and speed are only ever logged: latency and peeking
//! round corners make them less certain.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use tracing::{info, warn};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    Log,
    Enforce,
}

static MODE: AtomicU8 = AtomicU8::new(1);

pub fn set_mode(m: Mode) {
    MODE.store(m as u8, Ordering::Relaxed);
}

pub fn mode() -> Mode {
    match MODE.load(Ordering::Relaxed) {
        0 => Mode::Off,
        2 => Mode::Enforce,
        _ => Mode::Log,
    }
}

pub fn parse_mode(s: &str) -> Result<Mode, String> {
    match s.to_ascii_lowercase().as_str() {
        "off" => Ok(Mode::Off),
        "log" => Ok(Mode::Log),
        "enforce" => Ok(Mode::Enforce),
        _ => Err(format!("{s}: expected off, log or enforce")),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Flag {
    Damage,
    FireRate,
    FarFromVictim,
    ThroughWall,
    Speed,
    ThrowSpeed,
}

impl Flag {
    const ALL: [Flag; 6] = [Flag::Damage, Flag::FireRate, Flag::FarFromVictim, Flag::ThroughWall, Flag::Speed, Flag::ThrowSpeed];
    #[cfg(test)]
    pub const ALL_FOR_TESTS: [Flag; 6] = Self::ALL;

    fn name(self) -> &'static str {
        match self {
            Flag::Damage => "impossible damage",
            Flag::FireRate => "fire rate",
            Flag::FarFromVictim => "hit far from its victim",
            Flag::ThroughWall => "hit through a wall",
            Flag::Speed => "speed",
            Flag::ThrowSpeed => "disc throw speed",
        }
    }
}

/// The window shots are counted over.
const RATE_WINDOW: Duration = Duration::from_secs(2);
/// A hit this far (m) from the victim's body is not on them. Players move
/// about 7 m/s at most in combat; the shooter's view is up to ~200 ms old.
pub const FAR_FROM_VICTIM: f32 = 4.0;
/// Faster than this (m/s) over every window of a sustained stretch is a flag.
/// The game's own cap on a player's speed. A launcher boost is about 10 m/s,
/// but players throwing each other (regrabs) go further, and moderators'
/// force mod pushes players at up to 100 m/s. (Native matches, all four
/// maps: nobody kept up more than 11.5 m/s over a second; one-off jumps such
/// as respawns, of 1000+, never last.)
pub const MAX_SPEED: f32 = 100.0;
/// The game's fastest throw: anything over it is flagged (headbutts and slam
/// dunks aside: a headbutt isn't timed, a slam dunk is never thrown). Honest
/// throws in our pubs topped out at 18.52.
pub const MAX_THROW_SPEED: f32 = 18.95;
const SPEED_WINDOW: Duration = Duration::from_millis(250);
/// Windows in a row over `MAX_SPEED` before it counts.
const SPEED_RUN: u32 = 4;
/// Log the first few flags of a kind, then every this many.
const LOG_FIRST: u32 = 3;
const LOG_EVERY: u32 = 25;

#[derive(Default)]
struct Moves {
    at: Option<(Instant, [f32; 3])>,
    fast: u32,
    /// Every position in the last `TRAIL`, for hits that saw the body a
    /// moment before it moved (a teleport, TRANSLOCATOR, a respawn).
    trail: VecDeque<(Instant, [f32; 3])>,
}

/// How far back a hit may have seen its victim.
const TRAIL: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct Checks {
    shots: HashMap<(SocketAddr, u8), VecDeque<Instant>>,
    moves: HashMap<SocketAddr, Moves>,
    counts: HashMap<(SocketAddr, Flag), u32>,
}

impl Checks {
    /// Counts a flag and logs it (the first few, then every `LOG_EVERY`).
    pub fn flag(&mut self, who: SocketAddr, f: Flag, detail: &str) {
        if mode() == Mode::Off {
            return;
        }
        let n = self.counts.entry((who, f)).or_default();
        *n += 1;
        if *n <= LOG_FIRST || *n % LOG_EVERY == 0 {
            warn!(player = %who, check = f.name(), count = *n, enforce = mode() == Mode::Enforce, "cheat check: {detail}");
        }
    }

    pub fn count(&self, who: SocketAddr, f: Flag) -> u32 {
        self.counts.get(&(who, f)).copied().unwrap_or(0)
    }

    /// A shot fired: whether it is within its gun's rate.
    pub fn fired(&mut self, w: &Value, who: SocketAddr, instance: u8, damage: f32, now: Instant) -> bool {
        if mode() == Mode::Off {
            return true;
        }
        let limit = shots_allowed(w, damage);
        let q = self.shots.entry((who, instance)).or_default();
        while q.front().is_some_and(|t| now.saturating_duration_since(*t) > RATE_WINDOW) {
            q.pop_front();
        }
        q.push_back(now);
        let n = q.len() as u32;
        if n > limit {
            self.flag(who, Flag::FireRate, &format!("{n} shots in 2 s from one gun (damage {damage}), most allowed {limit}"));
            return false;
        }
        true
    }

    /// A body position: flags a sustained impossible speed.
    pub fn moved(&mut self, who: SocketAddr, pos: [f32; 3], now: Instant) {
        if mode() == Mode::Off {
            return;
        }
        let m = self.moves.entry(who).or_default();
        m.trail.push_back((now, pos));
        while m.trail.front().is_some_and(|(t, _)| now.saturating_duration_since(*t) > TRAIL) {
            m.trail.pop_front();
        }
        let Some((t, p)) = m.at else {
            m.at = Some((now, pos));
            return;
        };
        let dt = now.saturating_duration_since(t);
        if dt < SPEED_WINDOW {
            return;
        }
        m.at = Some((now, pos));
        if dt > SPEED_WINDOW * 8 {
            m.fast = 0;
            return;
        }
        let d = ((pos[0] - p[0]).powi(2) + (pos[1] - p[1]).powi(2) + (pos[2] - p[2]).powi(2)).sqrt();
        let v = d / dt.as_secs_f32();
        if v > MAX_SPEED {
            m.fast += 1;
            if m.fast == SPEED_RUN {
                self.flag(who, Flag::Speed, &format!("moving {v:.0} m/s for {} s", (SPEED_WINDOW * SPEED_RUN).as_secs_f32()));
            }
        } else {
            m.fast = 0;
        }
    }

    /// Where a body was over the last second (latest last).
    pub fn trail(&self, who: SocketAddr) -> impl Iterator<Item = [f32; 3]> + '_ {
        self.moves.get(&who).into_iter().flat_map(|m| m.trail.iter().map(|(_, p)| *p))
    }

    /// A player gone: their flags summed up, and forgotten.
    pub fn forget(&mut self, who: SocketAddr) {
        let flags: Vec<String> = Flag::ALL.iter().filter_map(|f| self.counts.remove(&(who, *f)).map(|n| format!("{} x{n}", f.name()))).collect();
        if !flags.is_empty() {
            info!(player = %who, flags = %flags.join(", "), "cheat check summary");
        }
        self.shots.retain(|(a, _), _| *a != who);
        self.moves.remove(&who);
    }
}

fn num(v: &Value, k: &str) -> Option<f32> {
    v.get(k).and_then(Value::as_f64).map(|x| x as f32)
}

/// The weapons whose shots do `damage` (exact, or within COMET's charge range).
fn guns_doing(w: &Value, damage: f32) -> Vec<&Value> {
    let Some(props) = w.get("weapon_properties").and_then(Value::as_object) else { return Vec::new() };
    props
        .values()
        .filter(|p| match (num(p, "projectile_damage"), num(p, "projectile_damage_min"), num(p, "projectile_damage_max")) {
            (Some(d), _, _) => (d - damage).abs() < 0.01,
            (None, Some(lo), Some(hi)) => damage >= lo - 0.01 && damage <= hi + 0.01,
            _ => false,
        })
        .collect()
}

/// Projectiles one trigger pull sends: the drone's `blaster` fires 9 at
/// once (native: 18 a second, in bursts with no gap).
fn pellets(p: &Value) -> u32 {
    if num(p, "volume_damage").is_some() { 9 } else { 1 }
}

/// The most shots a gun fires in `RATE_WINDOW`, with room for bunching.
fn shots_for(p: &Value) -> u32 {
    let burst = num(p, "fire_count").unwrap_or(1.0).max(1.0);
    let fire = num(p, "fire_delay").or_else(|| num(p, "fire_delay_end")).unwrap_or(0.0);
    let refire = num(p, "refire_delay").unwrap_or(0.0);
    let cycle = if burst > 1.0 { refire + (burst - 1.0) * fire } else { fire.max(refire) }.max(0.02);
    let most = pellets(p) as f32 * burst * ((RATE_WINDOW.as_secs_f32() / cycle).floor() + 1.0);
    (most * 1.25).ceil() as u32 + 2 * pellets(p)
}

/// The most shots in `RATE_WINDOW` for a shot of `damage` (the most
/// permissive gun it could be; unknown damage: no limit, the damage check
/// flags it).
pub fn shots_allowed(w: &Value, damage: f32) -> u32 {
    guns_doing(w, damage).into_iter().map(shots_for).max().unwrap_or(u32::MAX)
}

/// Whether `gun` is a player's own gun by name (not a drone's shot or its
/// cone, which fire from the drone).
pub fn hand_gun(w: &Value, gun: u64) -> bool {
    w.get("weapon_properties").and_then(Value::as_object).is_some_and(|m| m.iter().any(|(k, p)| crate::sns::symbol(k) == gun && num(p, "volume_damage").is_none()))
}

/// The most damage a hit from `gun` (its name's symbol, or ff.. for a drone's
/// shot) can claim: its weapon's damage, else the most any weapon does.
pub fn most_damage(w: &Value, gun: u64) -> f32 {
    let Some(props) = w.get("weapon_properties").and_then(Value::as_object) else { return f32::MAX };
    let top = |p: &Value| num(p, "projectile_damage").or_else(|| num(p, "projectile_damage_max"));
    let named = if gun == u64::MAX { props.get("blaster") } else { props.iter().find(|(k, _)| crate::sns::symbol(k) == gun).map(|(_, v)| v) };
    match named.and_then(top) {
        Some(d) => d,
        None => props.values().filter_map(top).fold(0.0, f32::max),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w() -> &'static Value {
        super::super::gear::combat_weapons_for_tests()
    }

    #[test]
    fn rate_limits_follow_the_guns() {
        // Native recordings (2 s windows): PULSAR 16, SMG 29, rifle 12,
        // COMET 3, drone 36. Each limit sits above, but not far.
        for (damage, seen, most) in [(9.0, 16, 24), (6.0, 29, 46), (11.0, 12, 17), (45.0, 4, 7), (2.0, 36, 75)] {
            let n = shots_allowed(w(), damage);
            assert!(n >= seen && n <= most, "damage {damage}: {n}");
        }
        // COMET's damage range overlaps the rocket's 30: the more permissive.
        assert_eq!(shots_allowed(w(), 30.0), shots_allowed(w(), 30.0).max(6));
        assert!(shots_allowed(w(), 37.5) <= 6);
        assert_eq!(shots_allowed(w(), 999.0), u32::MAX);
    }

    #[test]
    fn damage_caps_follow_the_gun() {
        let gun = |n: &str| crate::sns::symbol(n);
        assert_eq!(most_damage(w(), gun("assault")), 9.0);
        assert_eq!(most_damage(w(), gun("scout")), 60.0);
        assert_eq!(most_damage(w(), u64::MAX), 2.0);
        assert_eq!(most_damage(w(), 12345), 60.0);
    }

    #[test]
    fn rapid_fire_is_flagged_and_steady_fire_is_not() {
        let me: SocketAddr = "127.0.0.1:7000".parse().unwrap();
        let mut c = Checks::default();
        let t0 = Instant::now();
        // PULSAR at its own pace, 8 a second, for 5 s.
        for i in 0..40 {
            assert!(c.fired(w(), me, 0, 9.0, t0 + Duration::from_millis(125 * i)));
        }
        assert_eq!(c.count(me, Flag::FireRate), 0);
        // Twice that.
        let t1 = t0 + Duration::from_secs(10);
        let ok = (0..40).filter(|i| c.fired(w(), me, 0, 9.0, t1 + Duration::from_micros(62_500 * i))).count();
        assert!(ok < 40);
        assert!(c.count(me, Flag::FireRate) > 0);
    }

    #[test]
    fn one_jump_is_no_speed_flag_but_a_run_is() {
        let me: SocketAddr = "127.0.0.1:7000".parse().unwrap();
        let mut c = Checks::default();
        let t0 = Instant::now();
        let at = |i: u32| t0 + SPEED_WINDOW * i;
        c.moved(me, [0.0; 3], at(0));
        // A respawn: 60 m at once, then still.
        c.moved(me, [60.0, 0.0, 0.0], at(1));
        for i in 2..10 {
            c.moved(me, [60.0, 0.0, 0.0], at(i));
        }
        assert_eq!(c.count(me, Flag::Speed), 0);
        // A force push at the cap, 100 m/s, for 2 s: allowed.
        for i in 10..18 {
            c.moved(me, [60.0 + 25.0 * (i - 9) as f32, 0.0, 0.0], at(i));
        }
        assert_eq!(c.count(me, Flag::Speed), 0);
        // 150 m/s for 2 s.
        for i in 18..26 {
            c.moved(me, [260.0 + 37.5 * (i - 17) as f32, 0.0, 0.0], at(i));
        }
        assert_eq!(c.count(me, Flag::Speed), 1);
    }
}
