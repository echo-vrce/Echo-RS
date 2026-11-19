//! Combat gear in the social lobby's combat area (`mpl_lobby_b_combat`,
//! id bytes `26452bfcf77799cb`): what the native server answers, from a
//! recording of the H build (`captures/native-20261116`, see its NOTES.md).
//!
//! - Orb teleports: `SR15NetGameplayTeleportPlayerRequest` is answered to the
//!   sender with `SR15NetGameplayTeleportPlayer`, same bytes.
//! - The area's bit field (gear stations): `SR15NetBitFieldRequest` gets the
//!   current 120 entries; `SR15NetSetBits` changes one and is relayed.
//! - Guns: `FireGun` is relayed; a `DamageableBulletCollision` is answered to
//!   the shooter with `DamageableLocallyTrackedDamage` (base damage x the
//!   damage table's multiplier for the part hit).
//! - Tac mods: `AbilityActivate` / `AbilityCooldown` / `HealBubbleParamUpdate`
//!   go to everyone (the sender too); `GameActivateHealBubbleMsg` to the others.
//! - Ordnance: `GameSpawnMagazine` / `GameSetMagazineState` are relayed. When
//!   a magazine is thrown the server ends it after the item's lifetime
//!   (`SetMagazineState` used + `CooldownActive` to the thrower) and destroys
//!   it 2.5 s later. `ActivateExplosion` gets `ExplosionNothingHit`.
//!
//! - Splash: players' positions come from their body physics (64-byte
//!   `PhysicsSimStateEvent`, object `8ec21d00`, 19-bit mm fields at bit 267),
//!   thrown ordnance from the thrower's area object (`26452bfc`). METEOR / COMET
//!   explosions (`ActivateExplosion`), DETONATOR, STUN FIELD and DOT FIELD hit
//!   every player in range (`ExplosionPlayerDamage`, victim id = `f234d509...`
//!   with its low byte xor the slot, as in a recorded Gauss match), and the
//!   area's training dummy (victim `ffff...`, its recorded position).
//! - Re-entering the area: the native server resets the player's tac mod
//!   (an extra `AbilityCooldown`, a full `DamageableHealthBroadcast` and the
//!   heal bubbles' last parameters).

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rad_protocol::symbol::MatSym;
use serde_json::Value;
use tracing::{debug, info};

const fn h(name: &str) -> u64 {
    MatSym::hash(name).data
}

pub const TELEPORT_REQUEST: u64 = h("SR15NetGameplayTeleportPlayerRequest");
pub const TELEPORT: u64 = h("SR15NetGameplayTeleportPlayer");
pub const BITFIELD_REQUEST: u64 = h("SR15NetBitFieldRequest");
pub const BITFIELD_RESPONSE: u64 = h("SR15NetBitFieldResponse");
pub const SET_BITS: u64 = h("SR15NetSetBits");
/// `SR15NetComponentEvent` symbols the client sends crossing the combat area's edge.
pub const AREA_ENTER_EVENT: u64 = 0x635d_09de_ff71_64c0;
pub const AREA_LEAVE_EVENT: u64 = 0x635d_09de_ff71_64c1;
/// Native freed a leaver's stations 3.3, 3.6 and 4.7 s after its leave event.
const LEAVE_DELAY: Duration = Duration::from_millis(3500);
pub const SET_OWNER: u64 = h("SR15NetSetOwner");
pub const FIRE_GUN: u64 = h("SR15NetGameFireGun");
pub const BULLET_COLLISION: u64 = h("SR15NetDamageableBulletCollision");
pub const TRACKED_DAMAGE: u64 = h("SR15NetDamageableLocallyTrackedDamage");
pub const DAMAGE_DEALT: u64 = h("SR15NetDamageableDamageDealt");
pub const ABILITY_ACTIVATE: u64 = h("SR15NetAbilityActivate");
pub const ABILITY_COOLDOWN: u64 = h("SR15NetAbilityCooldown");
pub const HEAL_BUBBLE_PARAMS: u64 = h("SR15NetHealBubbleParamUpdate");
pub const HEAL_BUBBLE_ACTIVATE: u64 = h("SR15NetGameActivateHealBubbleMsg");
pub const SPAWN_MAGAZINE: u64 = h("SR15NetGameSpawnMagazine");
pub const MAGAZINE_STATE: u64 = h("SR15NetGameSetMagazineState");
pub const DESTROY_MAGAZINE: u64 = h("SR15NetGameDestroyMagazine");
pub const COOLDOWN_ACTIVE: u64 = h("SR15NetCooldownActive");
pub const ACTIVATE_EXPLOSION: u64 = h("SR15NetActivateExplosion");
pub const EXPLOSION_NOTHING_HIT: u64 = h("SR15NetExplosionNothingHit");
pub const EXPLOSION_PLAYER_DAMAGE: u64 = 0xc9bf_5b1d_bb88_8def;
pub const STUN_MSG: u64 = 0x1feb_4959_6855_9bc3;
pub const HEALTH_BROADCAST: u64 = 0x97bb_bd24_5d46_7354;
pub const PLAYER_KILLED: u64 = h("SR15NetDamageablePlayerKilled");
pub const COOLDOWN_REFUND: u64 = h("SR15NetCooldownRefund");
pub const DEPOSIT_MAGAZINE: u64 = h("SR15NetGameDepositMagazine");
pub const PLAYER_RESPAWN: u64 = h("SR15NetDamageablePlayerRespawn");
pub const PLAYER_RESPAWN_COMPLETE: u64 = h("SR15NetDamageablePlayerRespawnComplete");
/// A non-player damageable's new health and shield, to everyone (native
/// `CR15NetDamageableCS::DealDamage` after `ProcessDamageServer`):
/// `[space u64][index u16][generation u16][kind u32 = 1][health f32][shield f32]`.
/// Each client sets them (`ProcessDamageClient`); health reaching 0 is its death
/// event, which breaks a barrier.
pub const DEAL_DAMAGE: u64 = h("SR15NetDamageableDealDamage");
/// A drone's shot: its owner's client aims and sends it, the server passes it
/// to every other peer (`CR15NetAutoTargetCS::FireBulletNetwork`).
pub const AUTO_TARGET_FIRE: u64 = h("SR15NetAutoTargetFireBullet");

/// Physics objects (bytes 4..8 of a 64-byte `PhysicsSimStateEvent`).
const BODY_OBJECT: [u8; 4] = [0x8e, 0xc2, 0x1d, 0x00];
/// The combat maps' war room targets live in their own space: native answers
/// a shot on one with `LocallyTrackedDamage` in this space to the shooter only
/// (Gauss, Dyson and Fission recordings), and with nothing else.
const WAR_ROOM_SPACE: [u8; 8] = [0x0b, 0xef, 0x8d, 0x10, 0x9e, 0xaf, 0xa1, 0x08];
/// Thrown ordnance takes bullets: native deals from 9999 (the level's
/// ordnance templates), then the owner's client sets it off.
const ORDNANCE_HEALTH: f32 = 9999.0;
/// Native's barrier break: `DealDamage` 0 in the hit's frame, the break event
/// and `DealDamage` 0 again a frame later (10 ms), and the level script's own
/// event a frame after that (Dyson and Gauss recordings).
const BREAK_EVENT_AFTER: Duration = Duration::from_millis(10);
const BREAK_ECHO_AFTER: Duration = Duration::from_millis(17);
/// A respawn: the event and full health, then the script's event a frame later.
const RESPAWN_ECHO_AFTER: Duration = Duration::from_millis(7);
#[cfg(test)]
const AREA_OBJECT: [u8; 4] = [0x26, 0x45, 0x2b, 0xfc];
/// Explosion kinds (bytes 8..16 of the explosion messages).
const EX_ROCKET: [u8; 8] = [0xdc, 0x35, 0x4e, 0x7e, 0x5c, 0x3d, 0x0a, 0xb3];
const EX_DET: [u8; 8] = [0x6c, 0x8a, 0xc2, 0x55, 0xa1, 0x24, 0x51, 0x20];
const EX_STUN: [u8; 8] = [0x10, 0x3f, 0x1d, 0xe6, 0x54, 0x5b, 0x90, 0x25];
const EX_DOT: [u8; 8] = [0x06, 0x2a, 0xbc, 0x9b, 0x8f, 0xed, 0x2c, 0xea];
/// What a shooter's hit marker names (`LocallyTrackedDamage` bytes 8..16).
const MARK_ROCKET: [u8; 8] = [0x8d, 0xa7, 0xeb, 0x67, 0xbd, 0x59, 0x0f, 0x11];
const MARK_SCOUT: [u8; 8] = [0xe7, 0xda, 0xbe, 0xe8, 0x48, 0x77, 0x82, 0x03];
/// `GameFireGun` byte 0: the weapon that fired (COMET 0x1e, METEOR 0x2d).
const GUN_SCOUT: u8 = 0x1e;
/// `DamageableHealthBroadcast` damage kinds (u32 at 20): explosions 4, ordnance 2.
const HURT_EXPLOSION: u32 = 4;
const HURT_ORDNANCE: u32 = 2;
/// DETONATOR damage (native: 65 on a dummy, killed a player at 27).
const DET_DAMAGE: f32 = 65.0;
const DET_RADIUS: f64 = 1.5;
/// How far a det reaches barriers, and its damage to one at a distance:
/// full to 1 m, then down to 40% at the edge.
const DET_BARRIER_REACH: f32 = 2.5;
fn det_barrier_falloff(d: f32) -> f32 {
    let f = if d <= 1.0 { 1.0 } else { 1.0 - 0.6 * ((d - 1.0) / (DET_BARRIER_REACH - 1.0)).clamp(0.0, 1.0) };
    DET_DAMAGE * f
}
/// Health regeneration, as native did it: 12.4 s after the last loss, about
/// 73 health a second (kind 8) up to full, then a plain full-health broadcast.
const REGEN_DELAY: Duration = Duration::from_millis(12_400);
const REGEN_PER_SEC: f32 = 73.0;
const REGEN_STEP: Duration = Duration::from_millis(50);
const HURT_REGEN: u32 = 8;
/// Ordnance items (`SpawnMagazine` bytes 24..32).
const ITEM_DET: [u8; 8] = [0xfe, 0xe7, 0x94, 0xa8, 0xb1, 0xd0, 0xe8, 0xc8];
const ITEM_STUN: [u8; 8] = [0xbe, 0x0b, 0x67, 0x29, 0x48, 0x3e, 0xc3, 0xc8];
const ITEM_DOT: [u8; 8] = [0xfe, 0xed, 0x94, 0xa8, 0xb1, 0xd0, 0xe8, 0xc8];
const ITEM_DRONE: [u8; 8] = [0xb5, 0x4a, 0x80, 0xcb, 0xde, 0xc7, 0x2d, 0xe3];
/// A bullet collision's target (bytes 32..40) is a pool handle when its kind
/// (bytes 36..40) is 1: `[index u16][generation u16]`.
const HANDLE_KIND: [u8; 4] = [1, 0, 0, 0];
/// Hit body part 6 is "barrier" in the damage table.
const PART_BARRIER: u64 = 6;
/// A barrier the level data does not list (the stock maximum).
const BARRIER_HEALTH: f32 = 230.0;
/// Native's barrier break / respawn event (`ComponentEvent` to the barrier's
/// net handle, all clients, around a `DealDamage` with its new health).
/// Wire bytes `ff278203989fcc2b` / `c4b6bab2ee99ea6a` (u64s little-endian;
/// these were written byte-reversed before 2026-11-16, so clients never saw
/// the break and kept broken barriers up).
const BARRIER_EVENT: u64 = 0x2bcc_9f98_0382_27ff;
pub const BARRIER_MODULE: u64 = 0x6aea_99ee_b2ba_b6c4;
use super::msg::COMPONENT_EVENT;

/// A level's damageable net components (`tools/gen_combat_maps.py`): pool
/// slot, net handle, max health, respawn seconds (0: stays down).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Barrier {
    pool: u16,
    handle: u16,
    max: f32,
    respawn: f32,
    /// The actor's place and rotation (x, y, z, w), and its box in that
    /// frame: half extents, centred `off_z` along its z (none: no splash).
    pos: [f32; 3],
    rot: [f32; 4],
    half: [f32; 3],
    off_z: f32,
}

impl Barrier {
    /// The point of the barrier's box nearest `at`, and how far it is.
    fn nearest(&self, at: [f32; 3]) -> Option<([f32; 3], f32)> {
        if self.half.iter().all(|h| *h <= 0.0) {
            return None;
        }
        let rel = [at[0] - self.pos[0], at[1] - self.pos[1], at[2] - self.pos[2]];
        let [x, y, z, w] = self.rot;
        let mut l = rotate([-x, -y, -z, w], rel);
        l[2] -= self.off_z;
        for i in 0..3 {
            l[i] = l[i].clamp(-self.half[i], self.half[i]);
        }
        l[2] += self.off_z;
        let r = rotate(self.rot, l);
        let p = [self.pos[0] + r[0], self.pos[1] + r[1], self.pos[2] + r[2]];
        Some((p, dist(at, p)))
    }
}

/// `v` rotated by the unit quaternion `q` (x, y, z, w).
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let (u, w) = ([q[0], q[1], q[2]], q[3]);
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = cross(u, v).map(|c| 2.0 * c);
    let c = cross(u, t);
    [v[0] + w * t[0] + c[0], v[1] + w * t[1] + c[1], v[2] + w * t[2] + c[2]]
}

fn barriers_of(space: [u8; 8]) -> Vec<Barrier> {
    use super::combat::{LEVEL_COMBUSTION, LEVEL_DYSON, LEVEL_FISSION, LEVEL_GAUSS};
    let text = match u64::from_le_bytes(space) {
        LEVEL_GAUSS => include_str!("data/combat_gauss_barriers.txt"),
        LEVEL_DYSON => include_str!("data/combat_dyson_barriers.txt"),
        LEVEL_FISSION => include_str!("data/combat_fission_barriers.txt"),
        LEVEL_COMBUSTION => include_str!("data/combat_combustion_barriers.txt"),
        _ if space == AREA => include_str!("data/lobby_combat_barriers.txt"),
        _ => "",
    };
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let n = |i: usize| f.get(i).and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0);
            Some(Barrier {
                pool: f.first()?.parse().ok()?,
                handle: f.get(1)?.parse().ok()?,
                max: f.get(2)?.parse().ok()?,
                respawn: f.get(3)?.parse().ok()?,
                pos: [n(4), n(5), n(6)],
                rot: if f.len() > 10 { [n(7), n(8), n(9), n(10)] } else { [0.0, 0.0, 0.0, 1.0] },
                half: [n(11), n(12), n(13)],
                off_z: n(14),
            })
        })
        .collect()
}

fn barrier_event(handle: u16) -> [u8; 24] {
    super::msg::component_event(BARRIER_EVENT, BARRIER_MODULE, handle, super::msg::FILTER_ALL_ALT)
}
/// OVERCHARGE (the user's spec, 2026-11-16): the user and linked teammates
/// take 25% less damage; linked teammates cannot die for 8 s from their first
/// link, until the link breaks or they leave.
const OVERCHARGE_DR: f32 = 0.25;
/// DOT FIELD's time on the ground (the user's choice, 2026-11-16; stock 8 s).
const DOT_LIFETIME: Duration = Duration::from_secs(11);
const CHEAT_DEATH: Duration = Duration::from_secs(8);

/// How long an OVERCHARGE boost lasts (`ability_properties.buff.dr_duration`).
fn overcharge_duration() -> Duration {
    let secs = weapons().pointer("/ability_properties/buff/dr_duration").and_then(Value::as_f64).unwrap_or(10.0);
    Duration::from_secs_f64(secs.clamp(1.0, 60.0))
}
/// A player's damageable id is this with its low byte xor the slot.
const PLAYER_ID: [u8; 8] = [0xf2, 0x34, 0xd5, 0x09, 0x99, 0x74, 0x11, 0x3f];
const NO_PLAYER: [u8; 8] = [0xff; 8];
/// The damageable id every health message starts with.
const DAMAGEABLE: [u8; 8] = [0x8e, 0xc2, 0x1d, 0x00, 0xd3, 0x15, 0x99, 0x3f];
/// A dead player's client asks to respawn after 6.5 s; if it never does, the
/// server brings them back itself.
const RESPAWN_AFTER: Duration = Duration::from_secs(10);
/// A match: native respawns the dead 6.65 s after the kill (Gauss recording).
const MATCH_RESPAWN: Duration = Duration::from_millis(6650);
/// Gear stations: eight sets of fifteen bits, one bit per player slot.
const STATIONS: u32 = 8;
const STATION_SLOTS: u32 = 15;
/// The area's training dummies, where native reports them hit, and the hit code
/// native's markers carry for each (`LocallyTrackedDamage` byte 32). They sway.
const DUMMIES: &[([f32; 3], u32)] = &[
    ([f32::from_bits(0x422f_ccec), f32::from_bits(0x3f8f_c60a), f32::from_bits(0xc1cb_6388)], 0x17),
    ([55.8, 1.6, -5.25], 0x0d),
    ([50.26, 4.91, -8.09], 0x16),
];
const DUMMY_RADIUS: f32 = 0.8;
/// A match's war rooms' twelve training dummies (the war room level's
/// damageables, `0bef8d109eafa108`, pool slots 0..11): their bodies, 1.3 m over
/// the actors' places (bullets hit them 1.2-1.4 m up), and their pool slot,
/// which markers name (`[slot u32][1]`, in the war room's space).
const WAR_ROOM_DUMMIES: &[[f32; 3]] = &[
    [205.675, -9.571, -187.111],
    [195.879, -6.424, -182.117],
    [210.663, -9.635, -204.388],
    [203.512, -9.565, -213.805],
    [181.457, -13.832, -200.085],
    [185.552, -7.452, -211.445],
    [-168.535, -13.832, 199.912],
    [-146.802, -9.694, 185.853],
    [-139.36, -9.329, 195.551],
    [-144.683, -9.479, 213.064],
    [-164.103, -7.412, 188.919],
    [-153.959, -6.095, 217.622],
];
const WAR_ROOM_DUMMY_RADIUS: f32 = 0.5;
/// A hit code with this bit is a war room dummy's (its marker goes in the war
/// room's space).
const WAR_ROOM_CODE: u32 = 1 << 31;
const PLAYER_CODE: u32 = 0x17;
/// OVERCHARGE's heal bubble (`HealBubbleParamUpdate` bytes 8..16: heal 3, 10 s, radius 12).
const OVERCHARGE_BUBBLE: [u8; 8] = [0x41, 0xe3, 0x5b, 0x49, 0x67, 0x32, 0x83, 0xb9];
/// Full health, as native broadcasts it.
const FULL_HEALTH: f32 = 112.0;
/// The position native reports is the body's, a little lower.
const BODY_DROP: f32 = 0.35;
const BODY_RADIUS: f32 = 0.3;
/// A player's health and what last hurt them, as `DamageableHealthBroadcast`
/// carries it: `[owner][u32 0][health][7.0][kind u32][u32 0][ffffffff until hurt]
/// [hit position][12 zero][cause]`. Native keeps the last hurt's fields.
#[derive(Clone, Copy)]
struct Life {
    health: f32,
    hurt: Option<(u32, [f32; 3], [u8; 8])>,
    dead: bool,
    died_at: Option<Instant>,
    /// When health last went down, and when regeneration last sent.
    lost_at: Option<Instant>,
    regen_at: Option<Instant>,
}

impl Default for Life {
    fn default() -> Self {
        Self { health: FULL_HEALTH, hurt: None, dead: false, died_at: None, lost_at: None, regen_at: None }
    }
}

/// The health broadcast; `entry` = the area-entry reset (cause `ff..` before any hurt).
/// Bytes 8..12 are the player's slot (the 8-byte id before it is the same
/// for every player: a Gauss match's broadcasts carry one 56-byte record per
/// slot after it).
fn health_body(owner: [u8; 8], slot: u32, life: &Life, entry: bool) -> Vec<u8> {
    let mut b = owner.to_vec();
    b.extend_from_slice(&slot.to_le_bytes());
    b.extend_from_slice(&life.health.to_le_bytes());
    // 7.0 while alive, 0 once dead (native's broadcast at a kill). Always 7.0
    // left a player at 0 health standing: "critical", never dying.
    b.extend_from_slice(&(if life.dead { 0f32 } else { 7f32 }).to_le_bytes());
    let (kind, pos, cause) = life.hurt.unwrap_or((0, [0.0; 3], if entry { [0xff; 8] } else { [0; 8] }));
    b.extend_from_slice(&kind.to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(if life.hurt.is_some() { &[0; 4] } else { &[0xff; 4] });
    for x in pos {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b.extend_from_slice(&[0; 12]);
    b.extend_from_slice(&cause);
    b
}

/// The combat area's id as it appears in messages.
const AREA: [u8; 8] = [0x26, 0x45, 0x2b, 0xfc, 0xf7, 0x77, 0x99, 0xcb];
/// Magazine states (`SetMagazineState` bytes 16..24).
const MAG_THROWN: [u8; 8] = [0xa9, 0x50, 0x9f, 0xd4, 0xdf, 0xc7, 0x2d, 0xe3];
const MAG_USED: [u8; 8] = [0x62, 0x81, 0xfb, 0x26, 0x1b, 0x84, 0xf5, 0x33];
/// Second half of every `CooldownActive` the native server sent.
const COOLDOWN_TAIL: [u8; 8] = [0x58, 0xc5, 0x1a, 0x93, 0x3f, 0xe3, 0x35, 0x56];
const DESTROY_AFTER: Duration = Duration::from_millis(2500);
const BITS: usize = 120;

/// Seconds from thrown to used, by ordnance item (`SpawnMagazine` bytes 24..32),
/// as measured: DETONATOR 0.12 s, STUN FIELD 6 s, DOT FIELD 8 s.
/// An ordnance's cooldown (`grenade_properties.<name>.cooldown`), after which
/// the server refunds it (`CooldownRefund`).
fn cooldown(item: [u8; 8]) -> Duration {
    let name = match item {
        ITEM_DET => "det",
        ITEM_STUN => "stun",
        ITEM_DOT => "dot",
        ITEM_DRONE => "drone",
        [0xe9, 0xed, 0x9c, 0xa8, 0xb1, 0xd0, 0xe8, 0xc8] => "burst",
        _ => "",
    };
    let secs = weapons().pointer(&format!("/grenade_properties/{name}/cooldown")).and_then(Value::as_f64).unwrap_or(30.0);
    Duration::from_secs_f64(secs.clamp(1.0, 600.0))
}

fn lifetime(item: [u8; 8]) -> Duration {
    if item == ITEM_DRONE {
        // Natively the drone flies its lifetime (20 s) before it is used up.
        let secs = weapons().pointer("/grenade_properties/drone/lifetime").and_then(Value::as_f64).unwrap_or(20.0);
        return Duration::from_secs_f64(secs.clamp(1.0, 600.0));
    }
    match item {
        [0xbe, 0x0b, 0x67, 0x29, 0x48, 0x3e, 0xc3, 0xc8] => Duration::from_secs(6),
        // DOT FIELD: grenade_properties.dot.lifetime (11 s, the user's choice; stock 8).
        ITEM_DOT => DOT_LIFETIME,
        _ => Duration::from_millis(120),
    }
}

pub fn handles(ty: u64) -> bool {
    [
        TELEPORT_REQUEST,
        BITFIELD_REQUEST,
        SET_BITS,
        SET_OWNER,
        FIRE_GUN,
        BULLET_COLLISION,
        DAMAGE_DEALT,
        ABILITY_ACTIVATE,
        ABILITY_COOLDOWN,
        HEAL_BUBBLE_PARAMS,
        HEAL_BUBBLE_ACTIVATE,
        SPAWN_MAGAZINE,
        MAGAZINE_STATE,
        ACTIVATE_EXPLOSION,
        PLAYER_RESPAWN,
        PLAYER_RESPAWN_COMPLETE,
        DEPOSIT_MAGAZINE,
        AUTO_TARGET_FIRE,
    ]
    .contains(&ty)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum To {
    Sender,
    Others,
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Out {
    pub to: To,
    pub ty: u64,
    pub body: Vec<u8>,
}

fn out(to: To, ty: u64, body: &[u8]) -> Out {
    Out { to, ty, body: body.to_vec() }
}

struct Mag {
    owner: [u8; 8],
    item: [u8; 8],
    thrower: SocketAddr,
    ending: bool,
}

/// A STUN FIELD or DOT FIELD on the ground.
struct Field {
    kind: [u8; 8],
    mag: [u8; 8],
    at: [f32; 3],
    thrower: SocketAddr,
    until: Instant,
    next: Instant,
    every: Duration,
    radius: f32,
    /// Victims a stun field already stunned.
    hit: Vec<[u8; 8]>,
}

/// (victim id, reported position, distance, player, marker hit code)
type Hit = ([u8; 8], [f32; 3], f32, Option<SocketAddr>, u32);

/// An OVERCHARGE running: its self-damage ticks.
struct Drain {
    who: SocketAddr,
    owner: [u8; 8],
    next: Instant,
    left: u32,
}

/// What the match stats credit: damage that landed and kills.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Credit {
    Damage { by: Option<SocketAddr>, victim: SocketAddr, amount: f32 },
    Kill { by: Option<SocketAddr>, victim: SocketAddr },
}

pub struct Gear {
    /// Damage and kills since the session last took them (match stats).
    pub credits: Vec<Credit>,
    bits: [[u8; 4]; BITS],
    mags: HashMap<[u8; 8], Mag>,
    later: Vec<(Instant, SocketAddr, Out)>,
    /// Each player's slot and body position (metres).
    bodies: HashMap<SocketAddr, (u16, [f32; 3])>,
    /// Each player's team (0 blue, 1 orange, else not playing), kept by the session.
    pub teams: HashMap<SocketAddr, u8>,
    /// Where each player's thrown ordnance is.
    thrown_at: HashMap<SocketAddr, [f32; 3]>,
    fields: Vec<Field>,
    /// Heal bubble parameters by bubble, and each player's bubble owner id.
    bubbles: Vec<([u8; 8], Vec<u8>)>,
    bubble_owner: HashMap<SocketAddr, [u8; 8]>,
    /// Each player's health as the server last broadcast it.
    lives: HashMap<SocketAddr, Life>,
    /// Shots by explosion id: (weapon byte, projectile damage), from `GameFireGun`.
    shots: HashMap<[u8; 8], (u8, f32)>,
    drains: Vec<Drain>,
    /// Players inside the combat area (entered with the `SetOwner` burst).
    inside: Vec<SocketAddr>,
    /// When each player last walked out of it.
    left: HashMap<SocketAddr, Instant>,
    /// Walked out (the client's area-leave event): stations freed at this time
    /// unless the player walks back in first.
    leaving: HashMap<SocketAddr, Instant>,
    /// Clients that sent a combat-area event: their walking out is known from
    /// the leave event, not guessed from a loadout number.
    area_events: HashSet<SocketAddr>,
    /// Health of the area's damageables we have seen hit (barriers), by target id.
    area_health: HashMap<[u8; 8], f32>,
    /// The space's barriers (level data) and when broken ones come back.
    barriers: Vec<Barrier>,
    respawns: Vec<(Instant, [u8; 8], Barrier)>,
    /// OVERCHARGE users and until when their boost lasts (PowerBoost: their gun
    /// hits deal more, the hits they take less).
    boosted: HashMap<SocketAddr, Instant>,
    /// Teammates an OVERCHARGE bubble linked, and when their cheat death ends.
    links: HashMap<SocketAddr, Instant>,
    /// The space combat objects live in: the lobby's combat area, or a combat
    /// map's level (native's Gauss server uses the level's id).
    space: [u8; 8],
    /// A combat match: everyone is in combat, and the server respawns the dead.
    in_match: bool,
}

fn arr8(d: &[u8], o: usize) -> [u8; 8] {
    d[o..o + 8].try_into().expect("8 bytes")
}

/// The native server's bit field for the area (the same in every recording).
fn bitfield_base() -> &'static [u8] {
    static B: OnceLock<Vec<u8>> = OnceLock::new();
    B.get_or_init(|| {
        let t = include_str!("data/bitfield_response_2645.hex").trim();
        (0..t.len()).step_by(2).map(|i| u8::from_str_radix(&t[i..i + 2], 16).expect("hex")).collect()
    })
}

impl Default for Gear {
    fn default() -> Self {
        Self::new()
    }
}

impl Gear {
    pub fn new() -> Self {
        let base = bitfield_base();
        let mut bits = [[0xff; 4]; BITS];
        for (i, b) in bits.iter_mut().enumerate() {
            if let Some(v) = base.get(8 + 8 * i..12 + 8 * i) {
                b.copy_from_slice(v);
            }
        }
        Self {
            credits: Vec::new(),
            bits,
            mags: HashMap::new(),
            later: Vec::new(),
            bodies: HashMap::new(),
            teams: HashMap::new(),
            thrown_at: HashMap::new(),
            fields: Vec::new(),
            bubbles: Vec::new(),
            bubble_owner: HashMap::new(),
            lives: HashMap::new(),
            shots: HashMap::new(),
            drains: Vec::new(),
            inside: Vec::new(),
            left: HashMap::new(),
            leaving: HashMap::new(),
            area_events: HashSet::new(),
            area_health: HashMap::new(),
            barriers: barriers_of(AREA),
            respawns: Vec::new(),
            boosted: HashMap::new(),
            links: HashMap::new(),
            space: AREA,
            in_match: false,
        }
    }

    /// A combat match on `level`: its space, everyone in combat.
    pub fn set_match(&mut self, level: u64) {
        self.space = level.to_le_bytes();
        self.barriers = barriers_of(self.space);
        self.in_match = true;
    }

    /// In combat: the lobby's combat area, or anywhere in a match.
    fn in_combat(&self, who: &SocketAddr) -> bool {
        self.in_match || self.inside.contains(who)
    }

    /// Whether a player is alive (not dead and waiting to respawn).
    pub fn alive(&self, who: SocketAddr) -> bool {
        !self.lives.get(&who).is_some_and(|l| l.dead)
    }

    /// A punch stuns only when the puncher is in the combat area.
    pub fn stun_allowed(&self, from: SocketAddr, _d: &[u8]) -> bool {
        self.inside.contains(&from)
    }

    /// Whether this player may stun and block (arena style): in the lobby only
    /// inside the brawler room, from their body's position. The combat area
    /// (gun range) keeps the cease fire (`VAR_HOLD` 1): its gear works with it
    /// on, and lifting it there let players stun and block outside the room.
    pub fn may_fight(&self, who: SocketAddr) -> bool {
        self.bodies.get(&who).is_some_and(|(_, p)| in_brawler_room(*p))
    }

    /// The player's last body position, if known.
    pub fn body_pos(&self, who: SocketAddr) -> Option<[f32; 3]> {
        self.bodies.get(&who).map(|(_, p)| *p)
    }

    /// Using gear means being in the area: covers an entry burst the server
    /// missed. Not right after walking out (the client's last shots).
    fn using_gear(&mut self, from: SocketAddr, now: Instant) {
        if !self.inside.contains(&from) && self.left.get(&from).is_none_or(|t| now.duration_since(*t) > Duration::from_secs(3)) {
            info!(%from, "gear used: counted as inside the combat area");
            self.inside.push(from);
        }
    }

    fn bitfield(&self) -> Vec<u8> {
        let mut b = self.space.to_vec();
        for (i, v) in self.bits.iter().enumerate() {
            b.extend_from_slice(v);
            b.extend_from_slice(&(i as u32).to_le_bytes());
        }
        b
    }

    /// A client's 64-byte physics update: its body, or its thrown ordnance.
    pub fn observe_physics(&mut self, from: SocketAddr, d: &[u8]) {
        if d.len() != 64 {
            return;
        }
        let pos = transform(d);
        if d[4..8] == BODY_OBJECT {
            self.bodies.insert(from, (u16::from_le_bytes([d[0], d[1]]), pos));
        } else if d[4..8] == self.space[..4] {
            self.thrown_at.insert(from, pos);
        }
    }

    /// A player is gone: what the others must be told (their stations freed).
    pub fn forget(&mut self, who: SocketAddr) -> Vec<Out> {
        let outs = if self.inside.contains(&who) { self.free_stations(self.slot_of(&who)) } else { Vec::new() };
        self.bodies.remove(&who);
        self.thrown_at.remove(&who);
        self.bubble_owner.remove(&who);
        self.lives.remove(&who);
        self.drains.retain(|d| d.who != who);
        self.inside.retain(|a| *a != who);
        self.left.remove(&who);
        self.leaving.remove(&who);
        self.boosted.remove(&who);
        self.links.remove(&who);
        // The leaver itself is gone: these go to the others.
        outs.into_iter().map(|o| Out { to: To::Others, ..o }).collect()
    }

    /// Whether `by` may hurt (or stun) `victim`, as native's explosion query
    /// (`SR15VolumeQueryUtils::RunQuery`, libr15.so 0x13bf318): a victim not on
    /// blue or orange is never a target (team mask `& 3` zero), and a victim
    /// sharing the owner's team is skipped unless it is the owner itself (the
    /// owner's own blast counts, at `owner_damage_modifier`). The owner's own
    /// team is not checked. Unknown teams are not limited.
    fn may_hurt(&self, by: Option<SocketAddr>, victim: SocketAddr) -> bool {
        let Some(&vt) = self.teams.get(&victim) else { return true };
        if vt > 1 {
            return false;
        }
        match by.filter(|a| *a != victim).and_then(|a| self.teams.get(&a)) {
            Some(&bt) => bt != vt,
            None => true,
        }
    }

    /// Native's explosion damage on a player (`ApplyAreaDamage`, libr15.so
    /// 0x22469e4): every body part the blast reaches is scored as
    /// `falloff(distance to that part) x the part's damage table multiplier`
    /// (`CalculateTotalDamage`), and the best part is the hit. Only the head is
    /// tracked here, so the parts stand at fixed heights below it.
    fn part_damage(&self, who: SocketAddr, at: [f32; 3], falloff: &dyn Fn(f32) -> f32, table: &str) -> (f32, &'static str) {
        let Some((_, head)) = self.bodies.get(&who) else { return (falloff(f32::MAX), "torso") };
        let w = if self.in_match { combat_weapons() } else { weapons() };
        let mult = |part: &str| w.pointer(&format!("/damage_table/{table}/{part}")).and_then(Value::as_f64).map_or(1.0, |m| m as f32);
        BODY_PARTS
            .iter()
            .map(|(part, drop)| (falloff(dist(at, [head[0], head[1] - drop, head[2]])) * mult(part), *part))
            .fold((0.0, "torso"), |best, x| if x.0 > best.0 { x } else { best })
    }

    /// Everything an explosion at `at` from `by` reaches: (victim id, reported position, distance, player).
    fn victims(&self, at: [f32; 3], radius: f32, by: Option<SocketAddr>) -> Vec<Hit> {
        let mut v = Vec::new();
        for (a, (slot, p)) in &self.bodies {
            // Only players in the combat area can be hit (no stun or damage in the lobby).
            if !self.in_combat(a) || !self.may_hurt(by, *a) {
                continue;
            }
            // The tracked position is the head: a blast at the feet is measured
            // to the body below it, not to the head 1.5 m up.
            let d = body_dist(at, *p);
            if d <= radius + BODY_RADIUS {
                let mut id = PLAYER_ID;
                id[0] ^= *slot as u8;
                v.push((id, [p[0], p[1] - BODY_DROP, p[2]], d, Some(*a), PLAYER_CODE));
            }
        }
        // The lobby's training dummies (none in a match).
        for (p, code) in DUMMIES.iter().filter(|_| !self.in_match) {
            let d = dist(at, *p);
            if d <= radius + DUMMY_RADIUS {
                v.push((NO_PLAYER, *p, d, None, *code));
            }
        }
        // A match's war room dummies.
        for (i, p) in WAR_ROOM_DUMMIES.iter().enumerate().filter(|_| self.in_match) {
            let d = dist(at, *p);
            if d <= radius + WAR_ROOM_DUMMY_RADIUS {
                v.push((NO_PLAYER, *p, d, None, WAR_ROOM_CODE | i as u32));
            }
        }
        v
    }

    /// Damage to a player: their new health to everyone, and `PlayerKilled` at 0.
    /// A player's slot, from their body physics.
    pub fn slot_of(&self, who: &SocketAddr) -> u32 {
        self.bodies.get(who).map_or(0, |b| u32::from(b.0))
    }

    /// The player a damageable id names (`PLAYER_ID` with the low byte xor
    /// their slot), in the combat area.
    fn player_by_id(&self, id: [u8; 8]) -> Option<SocketAddr> {
        if id[1..] != PLAYER_ID[1..] {
            return None;
        }
        let slot = u16::from(id[0] ^ PLAYER_ID[0]);
        self.bodies.iter().find(|(a, (s, _))| *s == slot && self.in_combat(a)).map(|(a, _)| *a)
    }

    /// OVERCHARGE's bonus on a gun hit by `shooter` (PowerBoost): +body, or +head
    /// on body part 5, while the user's own boost lasts (linked teammates get
    /// no bonus).
    fn overcharge_bonus(&self, shooter: SocketAddr, part: u64, now: Instant) -> f32 {
        if !self.boosted.get(&shooter).is_some_and(|t| *t > now) || self.links.contains_key(&shooter) {
            return 0.0;
        }
        let b = |k: &str, def: f64| weapons().pointer(&format!("/ability_properties/buff/{k}")).and_then(Value::as_f64).unwrap_or(def) as f32;
        if part == 5 { b("damage_bonus_head", 2.0) } else { b("damage_bonus_body", 1.0) }
    }

    /// A client's `DamageableDamageDealt`: `[space 8][target id 8][amount f32]
    /// [body u8 + pad][type 8]`. EchoCombat's OVERCHARGE (PowerBoost) sends its
    /// use and its links as negative amounts: -(1,000,000 + dr permille * 1000 +
    /// seconds), +2,000,000 for a link, +2,000,000 more when the buff carries
    /// cheat death; -(6,000,000 + ...) ends a link.
    fn damage_dealt(&mut self, from: SocketAddr, d: &[u8], now: Instant) {
        let amount = f32::from_le_bytes(d[16..20].try_into().expect("4"));
        let marker = -amount;
        if !(1_000_000.0..7_000_000.0).contains(&marker) {
            debug!(%from, amount, "client damage dealt (not a marker)");
            return;
        }
        let id = arr8(d, 8);
        let Some(target) = self.player_by_id(id).or_else(|| (id[1..] == PLAYER_ID[1..]).then_some(from)) else {
            return;
        };
        let m = marker as u32;
        if (6_000_000..7_000_000).contains(&m) {
            info!(%from, %target, "OVERCHARGE link broken");
            self.links.remove(&target);
            self.boosted.remove(&target);
            return;
        }
        let link = (2_000_000..3_000_000).contains(&m) || (4_000_000..5_000_000).contains(&m);
        if link && target != from {
            // Cheat death runs from the first link of this use.
            let ends = self.links.get(&target).copied().filter(|t| *t > now).unwrap_or(now + CHEAT_DEATH);
            info!(%from, %target, "OVERCHARGE linked: damage cut, cannot die for 8 s");
            self.links.insert(target, ends);
        } else {
            info!(%from, "OVERCHARGE use marker");
        }
        self.boosted.insert(target, now + overcharge_duration());
    }

    /// What a hit on `who` really costs after OVERCHARGE: 25% off while boosted
    /// or linked, and a linked teammate is left at 1 health instead of dying.
    fn after_overcharge(&self, who: SocketAddr, amount: f32, health: f32, now: Instant) -> f32 {
        let mut a = amount;
        if self.boosted.get(&who).is_some_and(|t| *t > now) {
            a *= 1.0 - OVERCHARGE_DR;
        }
        if self.links.get(&who).is_some_and(|t| *t > now) && health - a < 1.0 {
            a = (health - 1.0).max(0.0);
        }
        a
    }

    /// A new round: every barrier back at full health (native Dyson, between
    /// its rounds: the respawn event for all, `DealDamage` max for all, the
    /// event for all again, in one frame).
    pub fn restore_barriers(&mut self) -> Vec<(SocketAddr, Out)> {
        self.respawns.clear();
        let anyone = SocketAddr::from(([0, 0, 0, 0], 0));
        let all = self.barriers.clone();
        let target = |b: &Barrier| {
            let mut t = [0u8; 8];
            t[..2].copy_from_slice(&b.pool.to_le_bytes());
            t[4] = 1;
            t
        };
        let mut v = Vec::new();
        for b in &all {
            v.push((anyone, out(To::All, COMPONENT_EVENT, &barrier_event(b.handle))));
        }
        for b in &all {
            self.area_health.insert(target(b), b.max);
            v.push((anyone, self.deal(target(b), b.max)));
        }
        let echo = Instant::now() + RESPAWN_ECHO_AFTER;
        for b in &all {
            self.later.push((echo, anyone, out(To::All, COMPONENT_EVENT, &barrier_event(b.handle))));
        }
        v
    }

    /// The barriers that are down now (their net handles).
    pub fn barriers_down(&self) -> Vec<u16> {
        let mut v: Vec<u16> = self
            .area_health
            .iter()
            .filter(|(t, h)| **h <= 0.0 && t[4..8] == HANDLE_KIND)
            .filter_map(|(t, _)| self.barrier(*t).map(|b| b.handle))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    fn barrier(&self, target: [u8; 8]) -> Option<Barrier> {
        let pool = u16::from_le_bytes([target[0], target[1]]);
        self.barriers.iter().find(|b| b.pool == pool).copied()
    }

    fn deal(&self, target: [u8; 8], health: f32) -> Out {
        let mut b = self.space.to_vec();
        b.extend_from_slice(&target);
        b.extend_from_slice(&health.to_le_bytes());
        b.extend_from_slice(&0f32.to_le_bytes());
        out(To::All, DEAL_DAMAGE, &b)
    }

    /// A bullet on one of the space's barriers: its health down, then native's
    /// `DealDamage` to everyone. At 0 native follows with the break event a
    /// frame later (`ComponentEvent`, `DealDamage` 0, then another
    /// `ComponentEvent` the frame after: the barrier drops and stops bullets
    /// no more), and after the level's respawn time the same with full health
    /// (exactly 30 s in the Gauss and Dyson recordings). A broken one takes no
    /// more.
    fn hit_barrier(&mut self, from: SocketAddr, target: [u8; 8], amount: f32, now: Instant) -> Vec<Out> {
        let known = self.barrier(target);
        let max = known.map_or(BARRIER_HEALTH, |b| b.max);
        let health = self.area_health.entry(target).or_insert(max);
        if *health <= 0.0 || amount <= 0.0 {
            return Vec::new();
        }
        *health = (*health - amount).max(0.0);
        let left = *health;
        let v = vec![self.deal(target, left)];
        if left > 0.0 {
            info!(%from, barrier = %hex8(target), amount, left, "barrier hit");
            return v;
        }
        info!(%from, barrier = %hex8(target), "barrier destroyed");
        if let Some(b) = known {
            let ev = barrier_event(b.handle);
            let zero = self.deal(target, 0.0);
            self.later.push((now + BREAK_EVENT_AFTER, from, out(To::All, COMPONENT_EVENT, &ev)));
            self.later.push((now + BREAK_EVENT_AFTER, from, zero));
            self.later.push((now + BREAK_ECHO_AFTER, from, out(To::All, COMPONENT_EVENT, &ev)));
            if b.respawn > 0.0 {
                self.respawns.push((now + Duration::from_secs_f32(b.respawn), target, b));
            }
        }
        v
    }

    /// An explosion's reach into the barriers: native (Dyson's dets) sends each
    /// one it reaches its `DealDamage`, then `ExplosionPlayerDamage` with no
    /// victim at the barrier's point nearest the blast, the break a frame later
    /// as for bullets. `table` is the damage table's entry (its `barrier`
    /// multiplier: det 2.15, rocket 2), `damage` the blast's at a distance.
    fn splash_barriers(&mut self, from: SocketAddr, kind: [u8; 8], id: [u8; 8], at: [f32; 3], reach: f32, table: &str, damage: &dyn Fn(f32) -> f32, now: Instant) -> Vec<Out> {
        let w = if self.in_match { combat_weapons() } else { weapons() };
        let scale = w.pointer(&format!("/damage_table/{table}/barrier")).and_then(Value::as_f64).unwrap_or(1.0) as f32;
        if scale <= 0.0 {
            return Vec::new();
        }
        let mut hit: Vec<(Barrier, [f32; 3], f32)> = self
            .barriers
            .iter()
            .filter_map(|b| b.nearest(at).filter(|(_, d)| *d <= reach).map(|(p, d)| (*b, p, d)))
            .collect();
        hit.sort_by(|a, b| a.2.total_cmp(&b.2));
        let mut v = Vec::new();
        for (b, p, d) in hit {
            let mut target = [0u8; 8];
            target[..2].copy_from_slice(&b.pool.to_le_bytes());
            target[4] = 1;
            if self.area_health.get(&target).is_some_and(|h| *h <= 0.0) {
                continue;
            }
            let amount = damage(d) * scale;
            if amount <= 0.0 {
                continue;
            }
            info!(%from, barrier = b.handle, distance = d, amount, "explosion reached a barrier");
            v.extend(self.hit_barrier(from, target, amount, now));
            v.push(out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(&self.space, kind, p, NO_PLAYER, id)));
        }
        v
    }

    fn hurt(&mut self, who: SocketAddr, amount: f32, kind: u32, pos: [f32; 3], cause: [u8; 8], by: Option<SocketAddr>) -> Vec<Out> {
        // The id is the same for every player; a client's own messages only confirm it.
        let owner = self.bubble_owner.get(&who).copied().unwrap_or(DAMAGEABLE);
        let slot = self.slot_of(&who);
        let killer = by.map_or(slot, |a| self.slot_of(&a));
        let health = self.lives.get(&who).map_or(FULL_HEALTH, |l| l.health);
        if !self.may_hurt(by, who) {
            return Vec::new();
        }
        let amount = self.after_overcharge(who, amount, health, Instant::now());
        let life = self.lives.entry(who).or_default();
        if life.dead || amount <= 0.0 {
            return Vec::new();
        }
        let before = life.health;
        life.health = (life.health - amount).max(0.0);
        let landed = before - life.health;
        life.hurt = Some((kind, pos, cause));
        life.lost_at = Some(Instant::now());
        life.regen_at = None;
        let mut v = Vec::new();
        if life.health <= 0.0 {
            life.dead = true;
            life.died_at = Some(Instant::now());
            info!(%who, cause = %hex8(cause), "player killed");
            self.credits.push(Credit::Kill { by: by.filter(|a| *a != who), victim: who });
            // `[id][cause][victim slot][killer slot]` (Gauss match).
            v.push(out(To::All, PLAYER_KILLED, &[owner.as_slice(), &cause, &slot.to_le_bytes(), &killer.to_le_bytes()].concat()));
        }
        let life = *life;
        if landed > 0.0 {
            self.credits.insert(self.credits.len() - usize::from(life.dead), Credit::Damage { by: by.filter(|a| *a != who), victim: who, amount: landed });
        }
        v.push(out(To::All, HEALTH_BROADCAST, &health_body(owner, slot, &life, false)));
        v
    }

    /// One explosion: `ExplosionPlayerDamage` per victim, else `NothingHit`.
    fn explode(&self, kind: [u8; 8], id: [u8; 8], at: [f32; 3], radius: f32, by: SocketAddr) -> (Vec<Out>, Vec<Hit>) {
        let space = self.space;
        let hits = self.victims(at, radius, Some(by));
        info!(kind = %hex8(kind), x = at[0], y = at[1], z = at[2], radius, hits = hits.len(), "explosion");
        if hits.is_empty() {
            let b = [space.as_slice(), &kind, &id].concat();
            return (vec![out(To::All, EXPLOSION_NOTHING_HIT, &b)], hits);
        }
        let outs = hits.iter().map(|(victim, pos, ..)| out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(&space, kind, *pos, *victim, id))).collect();
        (outs, hits)
    }

    /// Re-entering the combat area: what native sends back (tac mod reset).
    fn reset(&mut self, from: SocketAddr) -> Vec<Out> {
        let mut v = Vec::new();
        self.drains.retain(|d| d.who != from);
        let life = self.lives.entry(from).or_default();
        life.health = FULL_HEALTH;
        life.dead = false;
        life.died_at = None;
        let life = *life;
        if !self.inside.contains(&from) {
            self.inside.push(from);
        }
        info!(%from, "combat area entered: tac mod and health reset");
        let slot = self.slot_of(&from);
        if let Some(owner) = self.bubble_owner.get(&from) {
            v.push(out(To::Sender, HEALTH_BROADCAST, &health_body(*owner, slot, &life, true)));
            for (_, p) in self.bubbles.iter().filter(|(_, p)| p[..8] == owner[..]) {
                v.push(out(To::Sender, HEAL_BUBBLE_PARAMS, p));
            }
        }
        v
    }

    /// The client's combat-area trigger events (`SR15NetComponentEvent`, module
    /// `c4b6bab2ee99ea6a`). Native (lobby recordings 2026-11-15/16): entering
    /// sends `c06471ffde095d63`, and native answers it with the full health
    /// broadcast and the player's heal bubble params; leaving sends
    /// `c16471ffde095d63`, and native frees the gear stations 3.3-4.7 s later.
    /// Older clients also sent `LoadoutNumberNotification` + `SetOwner`
    /// around these, newer ones did not: a player who walked out stayed in
    /// combat mode (stun, block and gear outside the brawler room).
    pub fn area_event(&mut self, from: SocketAddr, sym: u64, now: Instant) -> Vec<Out> {
        if matches!(sym, AREA_ENTER_EVENT | AREA_LEAVE_EVENT) {
            self.area_events.insert(from);
        }
        match sym {
            AREA_ENTER_EVENT => {
                self.leaving.remove(&from);
                if self.inside.contains(&from) { Vec::new() } else { self.reset(from) }
            }
            AREA_LEAVE_EVENT => {
                if self.inside.contains(&from) {
                    info!(%from, "combat area leave event: stations freed in {LEAVE_DELAY:?}");
                    self.leaving.insert(from, now + LEAVE_DELAY);
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Whether this client reports the combat-area edge itself.
    pub fn sends_area_events(&self, who: SocketAddr) -> bool {
        self.area_events.contains(&who)
    }

    /// A player walked out of the combat area (the client then sends its loadout
    /// number and entrant state). Native frees the eight gear stations
    /// (`SetBits` `fcffffff` at indices 0, 15, ... 105), which takes the client
    /// out of combat (gear, stun and block are the brawler room's only).
    pub fn left_area(&mut self, from: SocketAddr) -> Vec<Out> {
        if !self.inside.contains(&from) {
            return Vec::new();
        }
        self.inside.retain(|a| *a != from);
        self.leaving.remove(&from);
        self.left.insert(from, Instant::now());
        self.drains.retain(|d| d.who != from);
        info!(%from, still_inside = self.inside.len(), "combat area left: gear stations freed");
        self.free_stations(self.slot_of(&from))
    }

    /// `SetBits fcffffff` at a player's bit of every station set (index
    /// `set * 15 + slot`; a Gauss match shows slot 3 at 3, 18, 33, ...), to
    /// everyone: the others drop that player's guns and tac mod too. Sent for
    /// slot 0's indices only, anyone else's gear stayed floating where they
    /// left and their client stayed in combat (stun and block anywhere).
    fn free_stations(&mut self, slot: u32) -> Vec<Out> {
        (0..STATIONS)
            .map(|k| {
                let i = k * STATION_SLOTS + slot % STATION_SLOTS;
                let v = [0xfc, 0xff, 0xff, 0xff];
                if let Some(b) = self.bits.get_mut(i as usize) {
                    *b = v;
                }
                let mut d = v.to_vec();
                d.extend_from_slice(&i.to_le_bytes());
                out(To::All, SET_BITS, &d)
            })
            .collect()
    }

    /// Replies to one client message.
    pub fn on_message(&mut self, from: SocketAddr, ty: u64, d: &[u8], now: Instant) -> Vec<Out> {
        let space = self.space;
        match ty {
            TELEPORT_REQUEST => vec![out(To::Sender, TELEPORT, d)],
            BITFIELD_REQUEST if d.len() == 8 && d == self.space => vec![out(To::Sender, BITFIELD_RESPONSE, &self.bitfield())],
            SET_BITS if d.len() == 8 => {
                let i = u32::from_le_bytes(d[4..8].try_into().expect("4")) as usize;
                if let Some(b) = self.bits.get_mut(i) {
                    b.copy_from_slice(&d[..4]);
                }
                vec![out(To::Others, ty, d)]
            }
            // The first of the area-entry burst: `[owner slot u32][index u32]`
            // with index 0. (Matching eight zero bytes only ever saw slot 0
            // walk in: nobody else could be shot, and their leaving was missed.)
            SET_OWNER if d.len() == 8 => {
                info!(%from, body = %d.iter().map(|x| format!("{x:02x}")).collect::<String>(), inside = self.inside.contains(&from), "set owner");
                // Only sent walking in (four of them, at this player's bit
                // of four station sets): the first one is the entry.
                if self.inside.contains(&from) { Vec::new() } else { self.reset(from) }
            }
            HEAL_BUBBLE_ACTIVATE if d.len() >= 16 && arr8(d, 8) == OVERCHARGE_BUBBLE => {
                // OVERCHARGE: self-damage every tick for its duration (native:
                // 112, 107, ... one health broadcast a second, 9 of them).
                let b = |k: &str, def: f64| weapons().pointer(&format!("/ability_properties/buff/{k}")).and_then(Value::as_f64).unwrap_or(def) as f32;
                let every = b("self_damage_tick_seconds", 1.0).max(0.1);
                let left = ((b("dr_duration", 10.0) / every).ceil() as u32).saturating_sub(1);
                self.drains.retain(|x| x.who != from);
                self.drains.push(Drain { who: from, owner: arr8(d, 0), next: now + Duration::from_secs_f32(every), left });
                self.boosted.insert(from, now + overcharge_duration());
                info!(%from, ticks = left, "OVERCHARGE on");
                vec![out(To::Others, ty, d)]
            }
            FIRE_GUN => {
                self.using_gear(from, now);
                // Remember which gun fired each explosive shot (by its explosion id).
                if d.len() == 64 {
                    if self.shots.len() > 256 {
                        self.shots.clear();
                    }
                    self.shots.insert(arr8(d, 56), (d[0], f32::from_le_bytes(d[48..52].try_into().expect("4"))));
                }
                vec![out(To::Others, ty, d)]
            }
            HEAL_BUBBLE_ACTIVATE => vec![out(To::Others, ty, d)],
            PLAYER_RESPAWN if d.len() >= 8 => {
                self.bubble_owner.insert(from, arr8(d, 0));
                let life = self.lives.entry(from).or_default();
                life.health = FULL_HEALTH;
                life.dead = false;
                life.died_at = None;
                life.hurt = None;
                let life = *life;
                info!(%from, "player respawned");
                let slot = d.get(8..12).map_or(self.slot_of(&from), |b| u32::from_le_bytes(b.try_into().expect("4")));
                vec![out(To::All, ty, d), out(To::All, HEALTH_BROADCAST, &health_body(arr8(d, 0), slot, &life, false))]
            }
            PLAYER_RESPAWN_COMPLETE => vec![out(To::Others, ty, d)],
            DEPOSIT_MAGAZINE if d.len() == 16 => {
                info!(%from, mag = %hex8(arr8(d, 8)), "ordnance put back");
                self.mags.remove(&arr8(d, 8));
                vec![out(To::Others, ty, d)]
            }
            // Native answers each cooldown with two.
            ABILITY_COOLDOWN => vec![out(To::All, ty, d), out(To::Sender, ty, d)],
            HEAL_BUBBLE_PARAMS if d.len() == 56 => {
                let key = arr8(d, 8);
                match self.bubbles.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, p)) => *p = d.to_vec(),
                    None => self.bubbles.push((key, d.to_vec())),
                }
                self.bubble_owner.insert(from, arr8(d, 0));
                vec![out(To::All, ty, d)]
            }
            ABILITY_ACTIVATE | HEAL_BUBBLE_PARAMS => vec![out(To::All, ty, d)],
            // `[gun 8][direction 12][hit point 12][target 8][target space u32]
            // [damage f32][body part u8 + pad][shooter 8]`: the target is what
            // the shooter's client hit (native `NetBulletCollisionCB` damages
            // exactly it), a player's id in the body space or a pool handle in
            // the area's.
            BULLET_COLLISION if d.len() == 64 => {
                self.using_gear(from, now);
                let at = f3(d, 20);
                let target = arr8(d, 32);
                let part = u64::from(d[48]);
                let table = if self.in_match { combat_weapons() } else { weapons() };
                let mut marker = tracked_damage(table, &space, d);
                let amount = f32::from_le_bytes(marker[16..20].try_into().expect("4")) + self.overcharge_bonus(from, part, now);
                marker[16..20].copy_from_slice(&amount.to_le_bytes());
                if d[40..44] == WAR_ROOM_SPACE[..4] {
                    return vec![out(To::Sender, TRACKED_DAMAGE, &tracked_damage(combat_weapons(), &WAR_ROOM_SPACE, d))];
                }
                // In a match native marks nothing itself: the shooter's client does.
                let mark = !self.in_match;
                if d[40..44] == BODY_OBJECT {
                    let mut v = if mark { vec![out(To::Sender, TRACKED_DAMAGE, &marker)] } else { Vec::new() };
                    match self.player_by_id(target).filter(|a| *a != from) {
                        Some(victim) => {
                            info!(%from, %victim, amount, part, "shot hit a player");
                            v.extend(self.hurt(victim, amount, part as u32, at, arr8(d, 0), Some(from)));
                        }
                        None => debug!(%from, target = %hex8(target), "shot hit a player not in the area"),
                    }
                    return v;
                }
                let index = u32::from(u16::from_le_bytes([target[0], target[1]]));
                let dummy = DUMMIES.iter().any(|(_, c)| *c == index);
                if d[40..44] == space[..4] && target[4..8] == HANDLE_KIND && !dummy && (part == PART_BARRIER || self.barrier(target).is_some()) {
                    return self.hit_barrier(from, target, amount, now);
                }
                if d[40..44] == space[..4] && self.mags.contains_key(&target) {
                    let health = self.area_health.entry(target).or_insert(ORDNANCE_HEALTH);
                    *health = (*health - amount).max(0.0);
                    let left = *health;
                    return vec![self.deal(target, left)];
                }
                if mark {
                    vec![out(To::Sender, TRACKED_DAMAGE, &marker)]
                } else {
                    Vec::new()
                }
            }
            DAMAGE_DEALT if d.len() >= 20 => {
                self.damage_dealt(from, d, now);
                Vec::new()
            }
            AUTO_TARGET_FIRE => vec![out(To::Others, ty, d)],
            ACTIVATE_EXPLOSION if d.len() == 40 => {
                let kind = arr8(d, 8);
                let at = f3(d, 16);
                let id = arr8(d, 32);
                let wp = |w: &str, k: &str, def: f64| weapons().pointer(&format!("/weapon_properties/{w}/{k}")).and_then(Value::as_f64).unwrap_or(def) as f32;
                // METEOR and COMET explode with the same kind; the shot says which.
                let shot = self.shots.remove(&id);
                let comet = shot.is_some_and(|(gun, _)| gun == GUN_SCOUT);
                let own_scale = wp("rocket_aoe", "owner_damage_modifier", 0.5);
                // (radius, damage at distance d, marker)
                let (radius, mark, damage): (f32, [u8; 8], Box<dyn Fn(f32) -> f32>) = if comet {
                    let (lo, hi) = (wp("scout", "projectile_damage_min", 30.0), wp("scout", "projectile_damage_max", 60.0));
                    let charge = shot.map_or(0.0, |(_, p)| ((p - lo) / (hi - lo).max(0.01)).clamp(0.0, 1.0));
                    let r = wp("scout_aoe", "min_charge_explosion_radius", 0.6) * (1.0 + (wp("scout_aoe", "full_charge_explosion_radius_ratio", 1.5) - 1.0) * charge);
                    let top = wp("scout_aoe", "min_charge_explosion_damage", 7.0) + (wp("scout_aoe", "max_charge_explosion_damage", 21.0) - wp("scout_aoe", "min_charge_explosion_damage", 7.0)) * charge;
                    let least = wp("scout_aoe", "min_explosion_damage", 1.0);
                    (r, MARK_SCOUT, Box::new(move |dd: f32| (top * (1.0 - dd / (r + BODY_RADIUS))).max(least)))
                } else if kind == EX_ROCKET {
                    let (far, near, full) = (wp("rocket_aoe", "explosion_radius_far", 1.75), wp("rocket_aoe", "explosion_radius_near", 1.0), wp("rocket_aoe", "explosion_damage", 40.0));
                    // Native: t = clamp((d - near) / (far - near)), damage =
                    // near_dmg (1 - t^e) + far_dmg t^e. The level gives far_dmg 0
                    // and e 1 unless its data says otherwise (not in the binaries).
                    let f = move |dd: f32| full * (1.0 - ((dd - near) / (far - near).max(0.01)).clamp(0.0, 1.0));
                    (far, MARK_ROCKET, Box::new(f))
                } else {
                    (1.0, MARK_ROCKET, Box::new(|_| 0.0))
                };
                let (mut v, hits) = self.explode(kind, id, at, radius, from);
                // Rockets break barriers too (rocket_aoe's barrier 2); COMET's
                // blast does not (scout_aoe's 0).
                let table = if comet { "scout_aoe" } else { "rocket_aoe" };
                let walls = self.splash_barriers(from, kind, id, at, radius + BODY_RADIUS, table, &*damage, now);
                if !walls.is_empty() && hits.is_empty() {
                    v.retain(|o| o.ty != EXPLOSION_NOTHING_HIT);
                }
                v.extend(walls);
                for (_, pos, dd, who, code) in hits {
                    let own = who == Some(from);
                    let base = match who {
                        Some(a) if !comet => self.part_damage(a, at, &*damage, table).0,
                        _ => damage(dd),
                    };
                    let amount = base * if own { own_scale } else { 1.0 };
                    // Native marks hits on others; a self-hit only costs health.
                    if !own {
                        v.push(out(To::Sender, TRACKED_DAMAGE, &marker(&space, mark, amount, pos, code)));
                    }
                    if let Some(a) = who {
                        v.extend(self.hurt(a, amount, HURT_EXPLOSION, pos, mark, Some(from)));
                    }
                }
                v
            }
            SPAWN_MAGAZINE if d.len() == 40 => {
                info!(%from, item = %hex8(arr8(d, 24)), mag = %hex8(arr8(d, 32)), "ordnance spawned");
                self.mags.insert(arr8(d, 32), Mag { owner: arr8(d, 0), item: arr8(d, 24), thrower: from, ending: false });
                vec![out(To::Others, ty, d)]
            }
            MAGAZINE_STATE if d.len() == 24 => {
                let (id, state) = (arr8(d, 8), arr8(d, 16));
                info!(%from, mag = %hex8(id), state = %hex8(state), known = self.mags.contains_key(&id), "ordnance state");
                let mut v = vec![out(To::Others, ty, d)];
                let thrown = self.mags.get(&id).filter(|m| !m.ending && state == MAG_THROWN).map(|m| (m.item, m.thrower));
                if let Some((item, thrower)) = thrown
                    && let Some(at) = self.thrown_at.get(&thrower).copied()
                {
                    if item == ITEM_DET {
                        // Reach: grenade_properties.det.radius (native hit only
                        // what was within about 0.4 m; 3 m caught bystanders).
                        // One blast event, at the nearest victim: one per victim
                        // drew several blasts.
                        let radius = weapons().pointer("/grenade_properties/det/radius").and_then(Value::as_f64).unwrap_or(DET_RADIUS) as f32;
                        let (mut o, mut hits) = self.explode(EX_DET, id, at, radius, thrower);
                        hits.sort_by(|x, y| x.2.total_cmp(&y.2));
                        if !hits.is_empty() {
                            o.truncate(0);
                            o.push(out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(&space, EX_DET, hits[0].1, hits[0].0, id)));
                        }
                        // Barriers: native's dets broke Dyson's 101s outright
                        // (65 x 2.15) and left one 2 m off at 4 (97 taken).
                        let walls = self.splash_barriers(thrower, EX_DET, id, at, radius.max(DET_BARRIER_REACH), "det", &det_barrier_falloff, now);
                        if !walls.is_empty() && hits.is_empty() {
                            o.retain(|x| x.ty != EXPLOSION_NOTHING_HIT);
                        }
                        v.extend(o);
                        v.extend(walls);
                        for (_, pos, _, who, code) in hits {
                            if who != Some(thrower) {
                                self.later.push((now, thrower, out(To::Sender, TRACKED_DAMAGE, &marker(&space, ITEM_DET, DET_DAMAGE, pos, code))));
                            }
                            if let Some(a) = who {
                                v.extend(self.hurt(a, DET_DAMAGE, HURT_ORDNANCE, pos, ITEM_DET, Some(thrower)));
                            }
                        }
                    } else if item == ITEM_STUN || item == ITEM_DOT {
                        let dot = item == ITEM_DOT;
                        let g = |k: &str, def: f64| {
                            weapons().pointer(&format!("/grenade_properties/{}/{k}", if dot { "dot" } else { "stun" })).and_then(Value::as_f64).unwrap_or(def) as f32
                        };
                        let every = if dot { Duration::from_secs_f32(1.0 / g("ticks_per_second", 3.0).max(0.5)) } else { Duration::from_millis(8) };
                        let radius = if dot { g("radius", 3.5) } else { 3.0 };
                        let kind = if dot { EX_DOT } else { EX_STUN };
                        self.fields.push(Field { kind, mag: id, at, thrower, until: now + lifetime(item), next: now, every, radius, hit: Vec::new() });
                    }
                }
                if let Some(m) = self.mags.get_mut(&id).filter(|m| !m.ending) {
                    if state == MAG_THROWN {
                        m.ending = true;
                        let used = now + lifetime(m.item);
                        let mut b = d[..16].to_vec();
                        b.extend_from_slice(&MAG_USED);
                        let cool = [m.owner, COOLDOWN_TAIL].concat();
                        info!(mag = %hex8(id), "ordnance used: cooldown follows");
                        self.later.push((used, from, out(To::All, MAGAZINE_STATE, &b)));
                        self.later.push((used, m.thrower, out(To::Sender, COOLDOWN_ACTIVE, &cool)));
                        self.later.push((used + cooldown(m.item), m.thrower, out(To::Sender, COOLDOWN_REFUND, &cool)));
                        self.later.push((used + DESTROY_AFTER, from, out(To::All, DESTROY_MAGAZINE, &id)));
                    } else if state == MAG_USED {
                        m.ending = true;
                        v.push(out(To::Sender, COOLDOWN_ACTIVE, &[m.owner, COOLDOWN_TAIL].concat()));
                        self.later.push((now + cooldown(m.item), m.thrower, out(To::Sender, COOLDOWN_REFUND, &[m.owner, COOLDOWN_TAIL].concat())));
                        self.later.push((now + DESTROY_AFTER, from, out(To::All, DESTROY_MAGAZINE, &id)));
                    }
                }
                v
            }
            _ => Vec::new(),
        }
    }

    /// Replies whose time has come, with the client each `To` is relative to.
    pub fn due(&mut self, now: Instant) -> Vec<(SocketAddr, Out)> {
        let space = self.space;
        let mut ready = Vec::new();
        let gone: Vec<SocketAddr> = self.leaving.iter().filter(|(_, t)| **t <= now).map(|(a, _)| *a).collect();
        for a in gone {
            for o in self.left_area(a) {
                ready.push((a, o));
            }
        }
        if !self.drains.is_empty() {
            let w = weapons();
            let lethal = w.pointer("/ability_properties/buff/self_damage_lethal").and_then(Value::as_bool).unwrap_or(false);
            let tick = w.pointer("/ability_properties/buff/self_damage_per_tick").and_then(Value::as_f64).unwrap_or(5.0) as f32;
            let every = w.pointer("/ability_properties/buff/self_damage_tick_seconds").and_then(Value::as_f64).unwrap_or(1.0).max(0.1);
            for dr in &mut self.drains {
                while dr.left > 0 && dr.next <= now {
                    dr.left -= 1;
                    dr.next += Duration::from_secs_f64(every);
                    let slot = self.bodies.get(&dr.who).map_or(0, |b| u32::from(b.0));
                    let life = self.lives.entry(dr.who).or_default();
                    if life.dead {
                        continue;
                    }
                    life.health = (life.health - tick).max(if lethal { 0.0 } else { 1.0 });
                    life.lost_at = Some(now);
                    life.regen_at = None;
                    let life = *life;
                    ready.push((dr.who, out(To::All, HEALTH_BROADCAST, &health_body(dr.owner, slot, &life, false))));
                }
            }
            self.drains.retain(|d| d.left > 0);
        }
        // A dead player whose client never asked to respawn. In a match the
        // server respawns everyone (native: `PlayerRespawn [damageable 8]
        // [slot u32][0 u32]` 6.65 s after each kill in the Gauss recording).
        let wait = if self.in_match { MATCH_RESPAWN } else { RESPAWN_AFTER };
        let stuck: Vec<SocketAddr> =
            self.lives.iter().filter(|(_, l)| l.dead && l.died_at.is_some_and(|t| now.duration_since(t) > wait)).map(|(a, _)| *a).collect();
        for who in stuck {
            let slot = self.slot_of(&who);
            let owner = self.bubble_owner.get(&who).copied().unwrap_or(DAMAGEABLE);
            if let Some(life) = self.lives.get_mut(&who) {
                *life = Life::default();
                info!(%who, "dead player brought back by the server");
                if self.in_match {
                    let b = [DAMAGEABLE.as_slice(), &slot.to_le_bytes(), &[0; 4]].concat();
                    ready.push((who, out(To::All, PLAYER_RESPAWN, &b)));
                }
                ready.push((who, out(To::All, HEALTH_BROADCAST, &health_body(owner, slot, life, false))));
            }
        }
        // Health regeneration.
        let draining: Vec<SocketAddr> = self.drains.iter().map(|d| d.who).collect();
        for (who, life) in &mut self.lives {
            let Some(lost) = life.lost_at else { continue };
            let slot = self.bodies.get(who).map_or(0, |b| u32::from(b.0));
            if life.dead || life.health >= FULL_HEALTH || draining.contains(who) || now < lost + REGEN_DELAY {
                continue;
            }
            let since = life.regen_at.unwrap_or(lost + REGEN_DELAY);
            if life.regen_at.is_some() && now < since + REGEN_STEP {
                continue;
            }
            let owner = self.bubble_owner.get(who).copied().unwrap_or(DAMAGEABLE);
            let gain = REGEN_PER_SEC * now.saturating_duration_since(since).as_secs_f32().max(REGEN_STEP.as_secs_f32());
            life.health = (life.health + gain).min(FULL_HEALTH);
            life.regen_at = Some(now);
            let full = life.health >= FULL_HEALTH;
            life.hurt = None;
            let mut shown = *life;
            if !full {
                shown.hurt = Some((HURT_REGEN, [0.0; 3], [0; 8]));
            } else {
                life.lost_at = None;
            }
            // Native's regen broadcasts keep ffffffff at 28 (no hit position).
            let mut b = health_body(owner, slot, &shown, false);
            b[28..32].copy_from_slice(&[0xff; 4]);
            ready.push((*who, out(To::All, HEALTH_BROADCAST, &b)));
        }
        let mut fields = std::mem::take(&mut self.fields);
        for f in &mut fields {
            while f.next <= now && f.next < f.until {
                f.next += f.every;
                let hits = self.victims(f.at, f.radius, Some(f.thrower));
                if f.kind == EX_DOT {
                    let tick = weapons().pointer("/grenade_properties/dot/damage_per_tick").and_then(Value::as_f64).unwrap_or(4.0) as f32;
                    for (victim, pos, _, who, code) in &hits {
                        ready.push((f.thrower, out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(&space, f.kind, *pos, *victim, f.mag))));
                        if *who != Some(f.thrower) {
                            ready.push((f.thrower, out(To::Sender, TRACKED_DAMAGE, &marker(&space, ITEM_DOT, tick, *pos, *code))));
                        }
                        if let Some(a) = who {
                            for o in self.hurt(*a, tick, HURT_ORDNANCE, *pos, ITEM_DOT, Some(f.thrower)) {
                                ready.push((*a, o));
                            }
                        }
                    }
                    continue;
                }
                let fresh: Vec<Hit> = hits.into_iter().filter(|h| !f.hit.contains(&h.0)).collect();
                if fresh.is_empty() {
                    ready.push((f.thrower, out(To::All, EXPLOSION_NOTHING_HIT, &[space.as_slice(), &f.kind, &f.mag].concat())));
                }
                for (victim, pos, _, who, _) in fresh {
                    f.hit.push(victim);
                    ready.push((f.thrower, out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(&space, f.kind, pos, victim, f.mag))));
                    let target: u64 = if who.is_some() { u64::from(victim[0] ^ PLAYER_ID[0]) } else { 0x0c };
                    ready.push((f.thrower, out(To::All, STUN_MSG, &[space.as_slice(), &target.to_le_bytes()].concat())));
                }
            }
        }
        fields.retain(|f| f.until > now);
        self.fields = fields;
        // Broken barriers whose time is up: native's respawn event, full health.
        let back: Vec<([u8; 8], Barrier)> = self.respawns.iter().filter(|(t, _, _)| *t <= now).map(|(_, t, b)| (*t, *b)).collect();
        self.respawns.retain(|(t, _, _)| *t > now);
        // `To::All` goes to everyone whoever it is relative to.
        let anyone = SocketAddr::from(([0, 0, 0, 0], 0));
        for (target, b) in back {
            info!(barrier = %hex8(target), "barrier respawned");
            self.area_health.insert(target, b.max);
            let ev = barrier_event(b.handle);
            ready.push((anyone, out(To::All, COMPONENT_EVENT, &ev)));
            ready.push((anyone, self.deal(target, b.max)));
            self.later.push((now + RESPAWN_ECHO_AFTER, anyone, out(To::All, COMPONENT_EVENT, &ev)));
        }
        self.later.retain(|(at, from, o)| {
            if *at <= now {
                ready.push((*from, o.clone()));
                false
            } else {
                true
            }
        });
        for (_, o) in &ready {
            if o.ty == DESTROY_MAGAZINE && o.body.len() == 8 {
                self.mags.remove(&arr8(&o.body, 0));
                self.area_health.remove(&arr8(&o.body, 0));
            }
        }
        ready
    }
}

/// A combat match's weapon settings (the EchoCombat block every combat level gets).
fn combat_weapons() -> &'static Value {
    static W: OnceLock<Value> = OnceLock::new();
    W.get_or_init(|| serde_json::from_str(super::balance::ECHOCOMBAT_WEAPONS).unwrap_or_default())
}

/// The weapon settings this server sends (its social block, with the host's changes).
fn weapons() -> &'static Value {
    static W: OnceLock<Value> = OnceLock::new();
    W.get_or_init(|| {
        super::msg::captured()
            .balance
            .iter()
            .find(|(l, _)| *l == super::msg::LEVEL_SOCIAL)
            .and_then(|(_, b)| super::balance::document(b, "mp_weapon_settings"))
            .unwrap_or_default()
    })
}

/// Damage-table multiplier for a hit: the weapon is the one whose
/// `projectile_damage` is the shot's base damage (else SCOUT/COMET, whose
/// damage falls off), the part from the collision (2 arm, 4 torso, 5 head).
fn multiplier(w: &Value, base: f32, part: u64) -> f32 {
    let weapon = w
        .get("weapon_properties")
        .and_then(Value::as_object)
        .and_then(|m| m.iter().find(|(_, v)| v.get("projectile_damage").and_then(Value::as_f64).is_some_and(|p| (p as f32 - base).abs() < 0.01)))
        .map_or("scout", |(k, _)| k.as_str());
    // Native's markers (blaster, scout): 0 and 1 legs, 2 and 3 arms, 4 torso, 5 head.
    let part = match part {
        0 | 1 => "leg",
        2 | 3 => "arm",
        5 => "head",
        6 => "barrier",
        _ => "torso",
    };
    w.pointer(&format!("/damage_table/{weapon}/{part}")).and_then(Value::as_f64).map_or(1.0, |m| m as f32)
}

/// `DamageableLocallyTrackedDamage` for a `DamageableBulletCollision`:
/// `[area 8][target 8][damage f32][hit point 12][kind u32][count u32]`.
fn tracked_damage(w: &Value, space: &[u8; 8], d: &[u8]) -> Vec<u8> {
    let base = f32::from_le_bytes(d[44..48].try_into().expect("4"));
    let part = u64::from_le_bytes(d[48..56].try_into().expect("8"));
    let mut b = space.to_vec();
    b.extend_from_slice(&d[0..8]);
    b.extend_from_slice(&(base * multiplier(w, base, part)).to_le_bytes());
    b.extend_from_slice(&d[20..40]);
    b
}

fn hex8(b: [u8; 8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn f3(d: &[u8], o: usize) -> [f32; 3] {
    let f = |i: usize| f32::from_le_bytes(d[o + 4 * i..o + 4 * i + 4].try_into().expect("4"));
    [f(0), f(1), f(2)]
}

/// How far below the tracked head a body reaches (to the feet).
const BODY_HEIGHT: f32 = 1.4;
/// A player's body parts, by how far below the tracked head their centres sit
/// (metres), for native's best-part explosion damage.
const BODY_PARTS: [(&str, f32); 4] = [("head", 0.0), ("arm", 0.45), ("torso", 0.5), ("leg", 1.1)];

/// Distance from `at` to a player's body: the vertical segment from the
/// tracked head `head` down `BODY_HEIGHT`.
fn body_dist(at: [f32; 3], head: [f32; 3]) -> f32 {
    let y = at[1].clamp(head[1] - BODY_HEIGHT, head[1]);
    dist(at, [head[0], y, head[2]])
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The brawler room's dome (`mpl_lobby_b_punch_hex_sphere_a`), measured from
/// the level model: a sphere centred here with a 7.6 m radius (shell 7.2..7.9).
const BRAWLER_DOME: [f32; 3] = [26.5, -2.17, 14.15];
/// The fight zone reaches just past the shell. Native's cease fire flipped
/// 7.7..8.3 m from the centre at the doorway (2026-11-16 recording), and our
/// walk along the inside wall stayed within 7.9 m.
const BRAWLER_RADIUS: f32 = 8.0;

/// Whether a body position is inside the lobby's brawler room dome.
pub fn in_brawler_room(p: [f32; 3]) -> bool {
    let d: f32 = (0..3).map(|i| (p[i] - BRAWLER_DOME[i]).powi(2)).sum();
    d <= BRAWLER_RADIUS * BRAWLER_RADIUS
}

/// Position in a 64-byte physics update: three 19-bit millimetre fields from bit 267.

fn transform(d: &[u8]) -> [f32; 3] {
    let field = |bit: usize| {
        let mut v = 0u32;
        for k in 0..19 {
            let i = bit + k;
            v |= u32::from((d[i >> 3] >> (i & 7)) & 1) << k;
        }
        (v as i32 - 250_000) as f32 / 1000.0
    };
    [field(267), field(286), field(305)]
}

/// `ExplosionPlayerDamage`: `[area][kind][position 12][u32 0][victim][explosion id]`.
fn player_damage(space: &[u8; 8], kind: [u8; 8], pos: [f32; 3], victim: [u8; 8], id: [u8; 8]) -> Vec<u8> {
    let mut b = [space.as_slice(), &kind].concat();
    for x in pos {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&victim);
    b.extend_from_slice(&id);
    b
}

/// The shooter's hit marker: `LocallyTrackedDamage` naming what dealt it.
fn marker(space: &[u8; 8], what: [u8; 8], damage: f32, pos: [f32; 3], code: u32) -> Vec<u8> {
    let (space, code) = if code & WAR_ROOM_CODE != 0 { (&WAR_ROOM_SPACE, code & !WAR_ROOM_CODE) } else { (space, code) };
    let mut b = [space.as_slice(), &what].concat();
    b.extend_from_slice(&damage.to_le_bytes());
    for x in pos {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b.extend_from_slice(&code.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
    }

    fn addr() -> SocketAddr {
        "127.0.0.1:6794".parse().expect("addr")
    }

    #[test]
    fn barrier_event_is_the_recorded_bytes() {
        // Native Dyson (netrec-dyson-20261116-server, t 290.493): barrier 0xbd.
        assert_eq!(barrier_event(0xbd).to_vec(), unhex("ff278203989fcc2bc4b6bab2ee99ea6abd000d00ffffffff"));
    }

    #[test]
    fn entry_health_broadcast_matches_native() {
        // Native lobby recording 2026-11-16, answer to the area-enter event (slot 0).
        let mut g = Gear::new();
        g.on_message(addr(), HEAL_BUBBLE_PARAMS, &unhex("8ec21d00d315993f41e35b49673283b9f234d5099974113f00000000000080bf0000803f0000404000004040000020418401404100000000"), Instant::now());
        let outs = g.area_event(addr(), AREA_ENTER_EVENT, Instant::now());
        let h = outs.iter().find(|o| o.ty == HEALTH_BROADCAST).expect("health broadcast");
        assert_eq!(h.body, unhex("8ec21d00d315993f000000000000e0420000e0400000000000000000ffffffff000000000000000000000000000000000000000000000000ffffffffffffffff"));
    }

    #[test]
    fn dummy_hit_markers_match_native() {
        // Native lobby recording 2026-11-16: BulletCollision -> LocallyTrackedDamage (part 4, part 5).
        for (hit, marker) in [
            ("be4d8ed1d9c72de371b7514227238cc0132ce7c1662d2f4207a6963fff02cbc1170000000100000026452bfc0000e0400400000000000000f234d5099974113f",
             "26452bfcf77799cbbe4d8ed1d9c72de30000e040662d2f4207a6963fff02cbc11700000001000000"),
            ("be4d8ed1d9c72de3e2ff524223c0d8bf454bebc120332f429d41b23f42a6cac1170000000100000026452bfc0000e0400500000000000000f234d5099974113f",
             "26452bfcf77799cbbe4d8ed1d9c72de30000e04020332f429d41b23f42a6cac11700000001000000"),
        ] {
            assert_eq!(tracked_damage(weapons(), &AREA, &unhex(hit)), unhex(marker));
        }
    }

    #[test]
    fn area_events_enter_and_free_the_stations_after_a_delay() {
        // Native lobby recording 2026-11-16: no SetOwner, no LoadoutNumberNotification.
        let mut g = Gear::new();
        let t0 = Instant::now();
        assert_eq!(AREA_ENTER_EVENT.to_le_bytes(), [0xc0, 0x64, 0x71, 0xff, 0xde, 0x09, 0x5d, 0x63]);
        assert!(!g.sends_area_events(addr()));
        g.area_event(addr(), AREA_ENTER_EVENT, t0);
        assert!(g.sends_area_events(addr()), "no loadout-number guess for this client");
        assert!(g.inside.contains(&addr()));
        assert!(g.area_event(addr(), AREA_LEAVE_EVENT, t0).is_empty());
        assert!(g.due(t0 + Duration::from_secs(1)).iter().all(|(_, o)| o.ty != SET_BITS));
        let freed: Vec<_> = g.due(t0 + Duration::from_secs(5)).into_iter().filter(|(_, o)| o.ty == SET_BITS).collect();
        assert_eq!(freed.len(), 8);
        assert!(!g.inside.contains(&addr()));
        // Walking back in before the delay keeps the stations.
        g.area_event(addr(), AREA_ENTER_EVENT, t0);
        g.area_event(addr(), AREA_LEAVE_EVENT, t0);
        g.area_event(addr(), AREA_ENTER_EVENT, t0 + Duration::from_secs(1));
        assert!(g.due(t0 + Duration::from_secs(9)).iter().all(|(_, o)| o.ty != SET_BITS));
        assert!(g.inside.contains(&addr()));
    }

    #[test]
    fn symbols_match_the_recording() {
        assert_eq!(TELEPORT_REQUEST, 0x0fba_c5d9_94dd_e2e8);
    }

    #[test]
    fn brawler_room_is_the_dome() {
        // Native recording 2026-11-16 (deep inside, and the last point before
        // the var rose), then our walk along the dome's inside wall, where the
        // old box wrongly put the cease fire back.
        for p in [
            [31.0, 0.16, 15.2],
            [26.3, 0.95, 11.9],
            [22.3, 0.98, 9.85],
            [19.14, 0.46, 15.42],
            [21.86, -1.14, 9.04],
            [32.76, -1.48, 10.24],
            [27.42, -1.87, 6.92],
            [29.8, -1.33, 20.57],
            [23.18, -1.72, 7.79],
        ] {
            assert!(in_brawler_room(p), "{p:?}");
        }
        // Native's doorway point before the var dropped, the lobby, the
        // combat area, and our way out through the gap.
        for p in [[0.0, 2.0, 0.0], [19.09, 0.54, 10.11], [37.0, 1.9, -22.0], [16.6, 1.3, 6.4], [20.63, -1.36, 7.96], [14.08, -0.29, 6.87]] {
            assert!(!in_brawler_room(p), "{p:?}");
        }
    }

    #[test]
    fn only_opponents_on_blue_or_orange_are_hurt() {
        let mut g = Gear::new();
        let (a, b, c) = (SocketAddr::from(([10, 0, 0, 1], 1)), SocketAddr::from(([10, 0, 0, 2], 2)), SocketAddr::from(([10, 0, 0, 3], 3)));
        g.teams.extend([(a, 0), (b, 0), (c, 3)]);
        assert!(!g.may_hurt(Some(a), b), "teammate");
        assert!(!g.may_hurt(Some(a), c), "social player");
        assert!(!g.may_hurt(Some(c), c), "social player's own blast");
        assert!(g.may_hurt(Some(a), a), "own blast on a team");
        g.teams.insert(b, 1);
        assert!(g.may_hurt(Some(a), b), "opponent");
        assert!(g.may_hurt(Some(c), b), "native does not check the owner's own team");
    }

    #[test]
    fn blasts_are_measured_to_the_whole_body() {
        let head = [0.0, 1.6, 0.0];
        assert!((body_dist([0.0, 0.2, 0.5], head) - 0.5).abs() < 1e-5, "at the feet");
        assert!((body_dist([0.0, 2.0, 0.0], head) - 0.4).abs() < 1e-5, "above the head");
    }

    #[test]
    fn any_slot_enters_the_area_and_only_those_inside_may_stun() {
        // (`body` is defined below.)
        let mut g = Gear::new();
        let who = addr();
        assert!(!g.stun_allowed(who, &[]));
        // Slot 5's entry burst: `[owner 5][index 0]` (slot 0's is eight zeros).
        // Slot 5's entry burst starts at its bit of a station set (65 = 4 * 15 + 5).
        g.observe_physics(who, &body(5, [0, 0, 0]));
        g.on_message(who, SET_OWNER, &[0, 0, 0, 0, 65, 0, 0, 0], Instant::now());
        assert!(g.stun_allowed(who, &[]));
        // Leaving frees slot 5's bit of all eight sets, for everyone.
        let freed = g.left_area(who);
        assert_eq!(freed.len(), 8);
        assert_eq!(freed[1], out(To::All, SET_BITS, &[0xfc, 0xff, 0xff, 0xff, 20, 0, 0, 0]));
        assert!(!g.stun_allowed(who, &[]));
    }

    #[test]
    fn only_the_brawler_room_lifts_the_cease_fire() {
        let mut g = Gear::new();
        let who = addr();
        // In the combat area (gun range): gear, but no arena stun or block.
        g.observe_physics(who, &body(1, [32_515, 1_557, -20_608]));
        g.on_message(who, SET_OWNER, &[1, 0, 0, 0, 76, 0, 0, 0], Instant::now());
        assert!(g.stun_allowed(who, &[]));
        assert!(!g.may_fight(who));
        // In the brawler room.
        g.observe_physics(who, &body(1, [26_500, 0, 14_150]));
        assert!(g.may_fight(who));
    }

    #[test]
    fn teleport_is_echoed() {
        let d = unhex("e0ff4746001400d1d8a1933e9121c93f2ee78f3d000080400000000000000000cd66a6d6");
        assert_eq!(Gear::new().on_message(addr(), TELEPORT_REQUEST, &d, Instant::now()), vec![out(To::Sender, TELEPORT, &d)]);
    }

    #[test]
    fn bitfield_matches_the_native_answer() {
        let g = Gear::new();
        assert_eq!(g.bitfield(), bitfield_base());
    }

    #[test]
    fn bullet_collision_becomes_tracked_damage() {
        // PULSAR head shot: 9 x 1.5 (both from the recording).
        let bc = unhex("9e5b615f8c9cd62f95b2924260e454bd12132ac2fb362f429749c23fcc77cac1170000000100000026452bfc000010410500000000000000f234d5099974113f");
        let want = unhex("26452bfcf77799cb9e5b615f8c9cd62f00005841fb362f429749c23fcc77cac11700000001000000");
        assert_eq!(tracked_damage(weapons(), &AREA, &bc), want);
    }

    #[test]
    fn reply_types_match_the_recording() {
        assert_eq!(EXPLOSION_NOTHING_HIT, 0x7da3_136f_f175_37ed);
        assert_eq!(STUN_MSG, h("SR15StunMsg"));
        assert_eq!(EXPLOSION_PLAYER_DAMAGE, h("SR15NetExplosionPlayerDamage"));
        assert_eq!(HEALTH_BROADCAST, h("SR15NetDamageableHealthBroadcast"));
    }

    fn body(slot: u8, mm: [i32; 3]) -> [u8; 64] {
        let mut d = [0u8; 64];
        d[0] = slot;
        d[4..8].copy_from_slice(&BODY_OBJECT);
        for (n, v) in mm.iter().enumerate() {
            let raw = (v + 250_000) as u32;
            for k in 0..19 {
                let i = 267 + 19 * n + k;
                if raw >> k & 1 == 1 {
                    d[i >> 3] |= 1 << (i & 7);
                }
            }
        }
        d
    }

    fn explosion(at: [f32; 3]) -> Vec<u8> {
        let mut ex = unhex("26452bfcf77799cbdc354e7e5c3d0ab3");
        for x in at {
            ex.extend_from_slice(&x.to_le_bytes());
        }
        ex.extend_from_slice(&[0; 4]);
        ex.extend_from_slice(&unhex("060404dd10903ea0"));
        ex
    }

    #[test]
    fn meteor_splash_hits_players_in_range() {
        let mut g = Gear::new();
        let other: SocketAddr = "127.0.0.1:7000".parse().expect("addr");
        g.observe_physics(other, &body(2, [40_646, 1_012, -23_434]));
        assert_eq!(g.bodies[&other], (2, [40.646, 1.012, -23.434]));
        // In the lobby: not hit.
        let v = g.on_message(addr(), ACTIVATE_EXPLOSION, &explosion([40.9, 1.2, -23.5]), Instant::now());
        assert_eq!(v[0].ty, EXPLOSION_NOTHING_HIT, "outside the combat area");
        g.inside.push(other);
        let v = g.on_message(addr(), ACTIVATE_EXPLOSION, &explosion([40.9, 1.2, -23.5]), Instant::now());
        assert_eq!(v[0].ty, EXPLOSION_PLAYER_DAMAGE);
        assert_eq!(&v[0].body[32..40], &unhex("f034d5099974113f")[..]);
        assert_eq!(v[1].ty, TRACKED_DAMAGE);
        assert_eq!(&v[1].body[16..20], &40f32.to_le_bytes());
        // Out of range: nothing hit.
        let v = g.on_message(addr(), ACTIVATE_EXPLOSION, &explosion([30.0, 1.2, -23.5]), Instant::now());
        assert_eq!(v[0].ty, EXPLOSION_NOTHING_HIT);
    }

    #[test]
    fn barrier_boxes_put_natives_blast_points_on_their_edges() {
        // Native Dyson 251.401: a det's point on pool 107 (handle 0xba) sat on
        // its edge, (-0.035, -1.913, 3.074).
        let b = barriers_of(super::super::combat::LEVEL_DYSON.to_le_bytes()).into_iter().find(|b| b.pool == 107).expect("pool 107");
        let (p, d) = b.nearest([0.2, -2.5, 3.5]).expect("a box");
        // (Within the box's 0.26 m depth: native's blast was on its other face.)
        assert!(dist(p, [-0.035, -1.913, 3.074]) < 0.3, "{p:?}");
        assert!(d > 0.0);
        // Inside the box: the point itself.
        let (_, d) = b.nearest(b.pos).expect("a box");
        assert!(d < 1e-4);
    }

    #[test]
    fn explosions_break_barriers() {
        use super::super::combat::LEVEL_DYSON;
        let mut g = Gear::new();
        g.set_match(LEVEL_DYSON);
        let t0 = Instant::now();
        // A METEOR at Dyson's 101-health barriers 106 and 107 (handles 0xb9,
        // 0xba): 40 x 2 to each it reaches, DealDamage then a victimless
        // ExplosionPlayerDamage at the barrier.
        let v = g.on_message(addr(), ACTIVATE_EXPLOSION, &explosion([-2.4, -1.5, 2.3]), t0);
        assert!(!v.iter().any(|o| o.ty == EXPLOSION_NOTHING_HIT));
        let deals: Vec<&Out> = v.iter().filter(|o| o.ty == DEAL_DAMAGE).collect();
        assert_eq!(deals.len(), 2, "{:?}", v.iter().map(|o| o.ty).collect::<Vec<_>>());
        for d in &deals {
            assert!([106u8, 107].contains(&d.body[8]));
            assert_eq!(&d.body[16..20], &21f32.to_le_bytes(), "101 - 80");
        }
        let blasts: Vec<&Out> = v.iter().filter(|o| o.ty == EXPLOSION_PLAYER_DAMAGE).collect();
        assert_eq!(blasts.len(), 2);
        assert!(blasts.iter().all(|o| o.body[32..40] == NO_PLAYER));
        // A second one breaks them: the break event a frame later.
        let v = g.on_message(addr(), ACTIVATE_EXPLOSION, &explosion([-2.4, -1.5, 2.3]), t0);
        assert!(v.iter().filter(|o| o.ty == DEAL_DAMAGE).all(|o| o.body[16..20] == 0f32.to_le_bytes()));
        let later = g.due(t0 + Duration::from_millis(11));
        let events: Vec<u16> = later.iter().filter(|(_, o)| o.ty == COMPONENT_EVENT).map(|(_, o)| u16::from_le_bytes([o.body[16], o.body[17]])).collect();
        assert!(events.contains(&0xb9) && events.contains(&0xba), "{events:?}");
        assert_eq!(g.barriers_down(), vec![0xb9, 0xba]);
        // Down ones take no more; far away, nothing.
        let v = g.on_message(addr(), ACTIVATE_EXPLOSION, &explosion([-2.4, -1.5, 2.3]), t0);
        assert!(!v.iter().any(|o| o.ty == DEAL_DAMAGE));
        let v = g.on_message(addr(), ACTIVATE_EXPLOSION, &explosion([30.0, 10.0, 30.0]), t0);
        assert_eq!(v.iter().map(|o| o.ty).collect::<Vec<_>>(), [EXPLOSION_NOTHING_HIT]);
    }

    #[test]
    fn war_room_dummies_take_splash() {
        use super::super::combat::LEVEL_GAUSS;
        let mut g = Gear::new();
        g.set_match(LEVEL_GAUSS);
        // A METEOR beside war room dummy 4 (the one native's Gauss bullets hit
        // at about (181.8, -13.7, -200.4)).
        let v = g.on_message(addr(), ACTIVATE_EXPLOSION, &explosion([181.8, -13.7, -199.6]), Instant::now());
        assert_eq!(v[0].ty, EXPLOSION_PLAYER_DAMAGE);
        let m = v.iter().find(|o| o.ty == TRACKED_DAMAGE).expect("a marker");
        assert_eq!(&m.body[..8], &WAR_ROOM_SPACE);
        assert_eq!(&m.body[32..40], &unhex("0400000001000000"));
        assert!(f32::from_le_bytes(m.body[16..20].try_into().expect("4")) > 0.0);
    }

    #[test]
    fn body_parts_match_natives_markers() {
        // Native war room markers: BLASTER (2) on parts 0, 1 (legs) 1.5, on
        // 3 (arm) 1.8, on 4 2, on 5 4; COMET on 2 (arm) x 0.9.
        let w = combat_weapons();
        for (part, want) in [(0, 0.75), (1, 0.75), (2, 0.9), (3, 0.9), (4, 1.0), (5, 2.0)] {
            assert_eq!(multiplier(w, 2.0, part), want, "part {part}");
        }
    }

    #[test]
    fn meteor_on_the_dummy_matches_native() {
        let v = Gear::new().on_message(addr(), ACTIVATE_EXPLOSION, &explosion([43.83, 1.195, -25.31]), Instant::now());
        assert_eq!(v[0].body, unhex("26452bfcf77799cbdc354e7e5c3d0ab3eccc2f420ac68f3f8863cbc100000000ffffffffffffffff060404dd10903ea0"));
        assert_eq!(v[1].body, unhex("26452bfcf77799cb8da7eb67bd590f1100002042eccc2f420ac68f3f8863cbc11700000001000000"));
    }

    #[test]
    fn dot_field_ticks_on_players_inside() {
        let mut g = Gear::new();
        let t0 = Instant::now();
        g.inside.push(addr());
        g.observe_physics(addr(), &body(0, [41_000, 1_000, -24_000]));
        let mut area = [0u8; 64];
        area[4..8].copy_from_slice(&AREA_OBJECT);
        g.observe_physics(addr(), &{
            let mut d = body(0, [41_500, 900, -24_300]);
            d[4..8].copy_from_slice(&AREA_OBJECT);
            d
        });
        g.on_message(addr(), SPAWN_MAGAZINE, &unhex("f234d5099974113f010000009d24204220836f3f5f49bfc1feed94a8b1d0e8c8e803876f456f1ba6"), t0);
        g.on_message(addr(), MAGAZINE_STATE, &unhex("26452bfcf77799cbe803876f456f1ba6a9509fd4dfc72de3"), t0);
        let ticks = g.due(t0 + Duration::from_secs(1));
        let damage = ticks.iter().filter(|(_, o)| o.ty == EXPLOSION_PLAYER_DAMAGE && o.body[32..40] != NO_PLAYER).count();
        assert!((3..=4).contains(&damage), "{damage} ticks in the first second");
        assert!(ticks.iter().any(|(_, o)| o.ty == TRACKED_DAMAGE && o.to == To::Sender));
    }

    /// The native recording's DETONATOR: spawn, armed, released, flying, detonated (twice).
    #[test]
    fn detonator_as_native_recorded_it() {
        let mut g = Gear::new();
        let t0 = Instant::now();
        g.observe_physics(addr(), &unhex("00000000"));
        let mut v = g.on_message(addr(), SPAWN_MAGAZINE, &unhex("f234d5099974113f01000000dcf521424efb6d3f8045bac1fee794a8b1d0e8c886b304072f528195"), t0);
        v.extend(g.on_message(addr(), MAGAZINE_STATE, &unhex("26452bfcf77799cb86b304072f5281951b0b211b29d4d241"), t0));
        g.observe_physics(addr(), &unhex("0000000026452bfc3484311600000000500000000000000000329c253878910aac14752306f522eb7691cdb56a5ee0c55e91029112752306f522eb0600000000"));
        v.extend(g.on_message(addr(), MAGAZINE_STATE, &unhex("26452bfcf77799cb86b304072f5281956388fd280387e323"), t0));
        g.observe_physics(addr(), &unhex("0000000026452bfc7bbd3e1600000000500000000000000000309c253878910aaca4d7a392f582dba6c2c067376814e65a8a2619aa89a332f5d0e70600000000"));
        for _ in 0..2 {
            v.extend(g.on_message(addr(), MAGAZINE_STATE, &unhex("26452bfcf77799cb86b304072f528195a9509fd4dfc72de3"), t0));
        }
        let kinds: Vec<u64> = v.iter().map(|o| o.ty).collect();
        // It landed on the first dummy: native (second recording) hit it for 65.
        assert!(kinds.contains(&EXPLOSION_PLAYER_DAMAGE), "{kinds:x?}");
        let later = g.due(t0 + Duration::from_millis(200));
        assert!(later.iter().any(|(_, o)| o.ty == COOLDOWN_ACTIVE && o.body == unhex("f234d5099974113f58c51a933fe33556")));
        assert!(later.iter().any(|(_, o)| o.ty == MAGAZINE_STATE && o.body[16..] == MAG_USED));
    }

    #[test]
    fn overcharge_drains_health_like_native() {
        let mut g = Gear::new();
        let t0 = Instant::now();
        let v = g.on_message(addr(), HEAL_BUBBLE_ACTIVATE, &unhex("8ec21d00d315993f41e35b49673283b90000000000000000"), t0);
        assert_eq!(v[0].to, To::Others);
        let ticks = g.due(t0 + Duration::from_secs(20));
        assert_eq!(ticks.len(), 9);
        assert_eq!(ticks[0].1.body, unhex("8ec21d00d315993f000000000000d6420000e0400000000000000000ffffffff0000000000000000000000000000000000000000000000000000000000000000"));
        assert_eq!(&ticks[8].1.body[12..16], &67f32.to_le_bytes());
        // The area-entry reset: full health with the trailing ff.
        g.on_message(addr(), HEAL_BUBBLE_PARAMS, &unhex("8ec21d00d315993f41e35b49673283b9f234d5099974113f00000000000080bf0000803f0000404000004040000020418401404100000000"), t0);
        let r = g.on_message(addr(), SET_OWNER, &[0; 8], t0);
        assert_eq!(r[0].body, unhex("8ec21d00d315993f000000000000e0420000e0400000000000000000ffffffff000000000000000000000000000000000000000000000000ffffffffffffffff"));
    }

    #[test]
    fn leaving_the_area_frees_the_gear_stations() {
        let mut g = Gear::new();
        assert!(g.left_area(addr()).is_empty(), "never entered");
        g.on_message(addr(), SET_OWNER, &[0; 8], Instant::now());
        let v = g.left_area(addr());
        assert_eq!(v.len(), 8);
        assert_eq!(v[1].body, unhex("fcffffff0f000000"));
        assert!(g.left_area(addr()).is_empty(), "second notification");
    }

    /// The second native recording: METEOR on yourself (half of 40), then a
    /// DETONATOR kills you, then the respawn.
    #[test]
    fn self_damage_kill_and_respawn_like_native() {
        let mut g = Gear::new();
        let t0 = Instant::now();
        let me = addr();
        g.on_message(me, HEAL_BUBBLE_PARAMS, &unhex("8ec21d00d315993f41e35b49673283b9f234d5099974113f00000000000080bf0000803f0000404000004040000020418401404100000000"), t0);
        g.on_message(me, SET_OWNER, &[0; 8], t0);
        g.observe_physics(me, &body(0, [45_640, 2_000, -68_590]));
        let v = g.on_message(me, ACTIVATE_EXPLOSION, &explosion([45.64, 1.8, -68.59]), t0);
        assert!(v.iter().all(|o| o.ty != TRACKED_DAMAGE), "no marker for a self-hit");
        let hb = v.iter().find(|o| o.ty == HEALTH_BROADCAST).expect("health");
        assert_eq!(&hb.body[12..16], &92f32.to_le_bytes());
        assert_eq!(&hb.body[20..24], &4u32.to_le_bytes());
        assert_eq!(&hb.body[56..64], &MARK_ROCKET);
        // A detonator on yourself: 92 - 65 = 27, then a second one kills.
        let det = |g: &mut Gear, mag: &str| {
            g.on_message(me, SPAWN_MAGAZINE, &unhex(&format!("f234d5099974113f01000000cd5e5f4048d8d3bd692f45c2fee794a8b1d0e8c8{mag}")), t0);
            let mut a = [0u8; 64];
            a[4..8].copy_from_slice(&AREA_OBJECT);
            a.copy_from_slice(&body(0, [45_640, 1_700, -68_590]));
            a[4..8].copy_from_slice(&AREA_OBJECT);
            g.observe_physics(me, &a);
            g.on_message(me, MAGAZINE_STATE, &unhex(&format!("26452bfcf77799cb{mag}a9509fd4dfc72de3")), t0)
        };
        let v = det(&mut g, "ee03a6cdb19a7f51");
        let hb = v.iter().find(|o| o.ty == HEALTH_BROADCAST).expect("health");
        assert_eq!(&hb.body[12..16], &27f32.to_le_bytes());
        let v = det(&mut g, "ee03a6cdb19a7f52");
        let killed = v.iter().find(|o| o.ty == PLAYER_KILLED).expect("killed");
        assert_eq!(killed.body, unhex("8ec21d00d315993ffee794a8b1d0e8c80000000000000000"));
        let hb = v.iter().find(|o| o.ty == HEALTH_BROADCAST).expect("health");
        assert_eq!(&hb.body[12..16], &0f32.to_le_bytes());
        assert_eq!(&hb.body[20..24], &2u32.to_le_bytes());
        // Dead: no more damage until the respawn.
        assert!(det(&mut g, "ee03a6cdb19a7f53").iter().all(|o| o.ty != HEALTH_BROADCAST));
        let v = g.on_message(me, PLAYER_RESPAWN, &unhex("8ec21d00d315993f0000000000000000"), t0);
        assert_eq!(v[0].ty, PLAYER_RESPAWN);
        assert_eq!(&v[1].body[12..16], &112f32.to_le_bytes());
    }

    #[test]
    fn comet_is_told_from_meteor_by_its_shot() {
        let mut g = Gear::new();
        let t0 = Instant::now();
        // COMET fire (byte 0 = 0x1e), projectile damage 30 = no charge, explosion id at 56.
        let mut fire = [0u8; 64];
        fire[0] = 0x1e;
        fire[48..52].copy_from_slice(&30f32.to_le_bytes());
        fire[56..64].copy_from_slice(&unhex("060404dd10903ea0"));
        g.on_message(addr(), FIRE_GUN, &fire, t0);
        let v = g.on_message(addr(), ACTIVATE_EXPLOSION, &explosion([43.83, 1.195, -25.31]), t0);
        let m = v.iter().find(|o| o.ty == TRACKED_DAMAGE).expect("marker");
        assert_eq!(&m.body[8..16], &MARK_SCOUT);
        let dmg = f32::from_le_bytes(m.body[16..20].try_into().expect("4"));
        assert!((1.0..=7.0).contains(&dmg), "{dmg}");
    }

    /// Two players in the area, 0.3 m apart: the collision names slot 1 (the
    /// target id, as the shooter's client hit it), so slot 1 is hurt whoever
    /// stands nearer the hit point; the kill names victim and killer.
    fn two_in_the_area() -> (Gear, SocketAddr, SocketAddr, Instant) {
        let mut g = Gear::new();
        let t0 = Instant::now();
        let (me, them): (SocketAddr, SocketAddr) = ("127.0.0.1:7000".parse().expect("a"), "127.0.0.1:7001".parse().expect("a"));
        let params = unhex("8ec21d00d315993f41e35b49673283b9f234d5099974113f00000000000080bf0000803f0000404000004040000020418401404100000000");
        for (a, slot) in [(me, 0u8), (them, 1)] {
            g.observe_physics(a, &body(slot, [43_800 + 300 * i32::from(slot), 1_870, -25_300]));
            g.on_message(a, HEAL_BUBBLE_PARAMS, &params, t0);
            g.on_message(a, SET_OWNER, &[0; 8], t0);
        }
        (g, me, them, t0)
    }

    /// A PULSAR head shot (9 x 1.5) from slot 0 on slot 1, as the client sends it
    /// (recording 2026-11-16: target `f334d5099974113f` in space `8ec21d00`).
    fn head_shot_on_slot_1() -> Vec<u8> {
        unhex("9e5b615f8c9cd62f2e5d68bf4c5a40c1332c9e420259d54146a4b040a36a38c2f334d5099974113f8ec21d00000010410500000000000000f234d5099974113f")
    }

    #[test]
    fn a_shot_hurts_the_player_it_hit() {
        let (mut g, me, them, t0) = two_in_the_area();
        let mut bc = head_shot_on_slot_1();
        // The hit point is right on slot 0 (the shooter's own body): slot 1 is still the one hit.
        for (k, x) in [43.8f32, 1.52, -25.3].iter().enumerate() {
            bc[20 + 4 * k..24 + 4 * k].copy_from_slice(&x.to_le_bytes());
        }
        let v = g.on_message(me, BULLET_COLLISION, &bc, t0);
        let hb = v.iter().find(|o| o.ty == HEALTH_BROADCAST).expect("victim's health");
        assert_eq!(&hb.body[8..12], &1u32.to_le_bytes());
        assert_eq!(&hb.body[12..16], &(112.0f32 - 13.5).to_le_bytes());
        for _ in 0..7 {
            g.on_message(me, BULLET_COLLISION, &bc, t0);
        }
        let v = g.on_message(me, BULLET_COLLISION, &bc, t0);
        let killed = v.iter().find(|o| o.ty == PLAYER_KILLED).expect("killed");
        assert_eq!(&killed.body[16..24], &[1, 0, 0, 0, 0, 0, 0, 0]);
        // A shot naming its own shooter hurts nobody.
        assert!(g.on_message(them, BULLET_COLLISION, &bc, t0).iter().all(|o| o.ty != HEALTH_BROADCAST));
    }

    /// The recording's barrier hits (area handle 8, body part 6): native's
    /// DealDamage to everyone with the health left; at 0 it breaks and takes no more.
    #[test]
    fn shooting_a_barrier_breaks_it() {
        let mut g = Gear::new();
        let me = addr();
        let t0 = Instant::now();
        g.on_message(me, SET_OWNER, &[0; 8], t0);
        let bc = unhex("9e5b615f8c9cd62f58c27d3f69924bc17af39d42fb362f429749c23fcc77cac1080000000100000026452bfc000010410600000000000000f234d5099974113f");
        let v = g.on_message(me, BULLET_COLLISION, &bc, t0);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].ty, DEAL_DAMAGE);
        assert_eq!(v[0].to, To::All);
        assert_eq!(&v[0].body[..16], &unhex("26452bfcf77799cb0800000001000000")[..]);
        // PULSAR on a barrier: 9 x 0.5.
        assert_eq!(&v[0].body[16..20], &(BARRIER_HEALTH - 4.5).to_le_bytes());
        let mut last = Vec::new();
        for _ in 0..200 {
            last = g.on_message(me, BULLET_COLLISION, &bc, t0);
            if g.area_health.get(&arr8(&bc, 32)) == Some(&0.0) {
                break;
            }
        }
        // Native's break: DealDamage 0 in the hit's frame; the event and
        // DealDamage 0 a frame later; the script's event a frame after that
        // (pool 8 is the IdsResponse's handle 66).
        let ev = barrier_event(66);
        assert_eq!(last.iter().map(|o| o.ty).collect::<Vec<_>>(), [DEAL_DAMAGE]);
        assert_eq!(&last[0].body[16..20], &0f32.to_le_bytes());
        let kinds = |v: Vec<(SocketAddr, Out)>| v.into_iter().map(|(_, o)| o).filter(|o| o.ty == COMPONENT_EVENT || o.ty == DEAL_DAMAGE).collect::<Vec<_>>();
        let next = kinds(g.due(t0 + Duration::from_millis(11)));
        assert_eq!(next.iter().map(|o| o.ty).collect::<Vec<_>>(), [COMPONENT_EVENT, DEAL_DAMAGE]);
        assert_eq!(next[0].body, ev);
        assert_eq!(&next[1].body[16..20], &0f32.to_le_bytes());
        let echo = kinds(g.due(t0 + Duration::from_millis(18)));
        assert_eq!(echo.iter().map(|o| o.ty).collect::<Vec<_>>(), [COMPONENT_EVENT]);
        assert!(g.on_message(me, BULLET_COLLISION, &bc, t0).is_empty(), "a broken barrier takes no more");
        // Back after 30 s at full health.
        assert!(g.due(t0 + Duration::from_secs(29)).iter().all(|(_, o)| o.ty != COMPONENT_EVENT));
        let at = t0 + Duration::from_secs(31);
        let back = kinds(g.due(at));
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].body, ev);
        assert_eq!(&back[1].body[16..20], &230f32.to_le_bytes());
        let echo = kinds(g.due(at + Duration::from_millis(8)));
        assert_eq!(echo.len(), 1);
        assert_eq!(echo[0].body, ev);
        assert_eq!(g.on_message(me, BULLET_COLLISION, &bc, t0).len(), 1, "it takes hits again");
    }

    /// A war room target in a match: native's `LocallyTrackedDamage` in the
    /// war room's space to the shooter only (Gauss recording, 252.905: a head
    /// shot, 6 x 1.4), nothing else.
    #[test]
    fn war_room_targets_get_natives_marker() {
        let mut g = Gear::new();
        g.set_match(super::super::combat::LEVEL_GAUSS);
        let me = addr();
        let t0 = Instant::now();
        let bc = unhex("edef83a8b1d0e8c80a838ec2591639c165aee4bf31e13543536d59c1135e48c304000000010000000bef8d100000c0400500000000000000f234d5099974113f");
        let v = g.on_message(me, BULLET_COLLISION, &bc, t0);
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].ty, v[0].to), (TRACKED_DAMAGE, To::Sender));
        assert_eq!(v[0].body, unhex("0bef8d109eafa108edef83a8b1d0e8c86666064131e13543536d59c1135e48c30400000001000000"));
    }

    /// The level data's barriers: Gauss's 9 (pool 0x69.. = handle 0x164..,
    /// as recorded), the lobby area's 11.
    #[test]
    fn barriers_come_from_the_levels() {
        let gauss = barriers_of(super::super::combat::LEVEL_GAUSS.to_le_bytes());
        assert_eq!(gauss.len(), 9);
        assert!(gauss.iter().any(|b| b.pool == 0x71 && b.handle == 0x16c && b.max == 230.0 && b.respawn == 30.0));
        assert_eq!(barriers_of(AREA).len(), 11);
        assert!(barriers_of(super::super::combat::LEVEL_FISSION.to_le_bytes()).is_empty());
    }

    /// OVERCHARGE: the user's hits deal +1 (+2 on the head) and they take 25%
    /// less; a linked teammate takes 25% less and cannot die for 8 s.
    #[test]
    fn overcharge_boosts_cuts_and_saves() {
        let (mut g, me, them, t0) = two_in_the_area();
        let oc = unhex("f234d5099974113f41e35b49673283b9f234d5099974113f");
        g.on_message(me, HEAL_BUBBLE_ACTIVATE, &oc, t0);
        let v = g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
        let hb = v.iter().find(|o| o.ty == HEALTH_BROADCAST).expect("hurt");
        assert_eq!(&hb.body[12..16], &(112.0f32 - 15.5).to_le_bytes(), "13.5 + 2 on the head");
        // Slot 1 shoots back at the boosted user: 25% off.
        let mut back = head_shot_on_slot_1();
        back[32] = 0xf2;
        back[56] = 0xf3;
        let v = g.on_message(them, BULLET_COLLISION, &back, t0);
        let hb = v.iter().find(|o| o.ty == HEALTH_BROADCAST).expect("hurt");
        assert_eq!(&hb.body[12..16], &(112.0f32 - 13.5 * 0.75).to_le_bytes());
        // The user's client links slot 1 (PowerBoost link marker with cheat death).
        let mut link = unhex("8ec21d00d315993ff334d5099974113f0000000000000000000000000000000000");
        link.truncate(32);
        link[16..20].copy_from_slice(&(-4_090_010f32).to_le_bytes());
        g.on_message(me, DAMAGE_DEALT, &link, t0);
        // Linked: no bonus for them, 25% off, and never below 1 health.
        for _ in 0..30 {
            let v = g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
            assert!(v.iter().all(|o| o.ty != PLAYER_KILLED), "cheat death");
        }
        assert_eq!(g.lives.get(&them).map(|l| l.health), Some(1.0));
        // The link breaks: the next lethal hit kills.
        link[16..20].copy_from_slice(&(-6_090_010f32).to_le_bytes());
        g.on_message(me, DAMAGE_DEALT, &link, t0);
        let v = g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
        assert!(v.iter().any(|o| o.ty == PLAYER_KILLED));
    }

    /// The drone flies its 20 s before it is used up (it was ended after 0.12 s,
    /// so it blew up as it was thrown), and its shots reach everyone else.
    #[test]
    fn the_drone_lives_and_its_shots_are_relayed() {
        assert_eq!(lifetime(ITEM_DRONE), Duration::from_secs(20));
        assert_eq!(lifetime(ITEM_DOT), Duration::from_secs(11));
        let mut g = Gear::new();
        let v = g.on_message(addr(), AUTO_TARGET_FIRE, &[1, 2, 3], Instant::now());
        assert_eq!(v, vec![out(To::Others, AUTO_TARGET_FIRE, &[1, 2, 3])]);
    }

    #[test]
    fn health_regenerates_after_a_pause() {
        let mut g = Gear::new();
        let me = addr();
        let t0 = Instant::now();
        g.on_message(me, HEAL_BUBBLE_PARAMS, &unhex("8ec21d00d315993f41e35b49673283b9f234d5099974113f00000000000080bf0000803f0000404000004040000020418401404100000000"), t0);
        g.hurt(me, 50.0, HURT_EXPLOSION, [0.0; 3], MARK_ROCKET, None);
        let t = Instant::now();
        assert!(g.due(t + Duration::from_secs(5)).iter().all(|(_, o)| o.ty != HEALTH_BROADCAST), "too early");
        let first = g.due(t + Duration::from_millis(12_500));
        let hb = &first.iter().find(|(_, o)| o.ty == HEALTH_BROADCAST).expect("regen").1;
        assert_eq!(&hb.body[20..24], &8u32.to_le_bytes());
        let mut last = Vec::new();
        for k in 1..40 {
            last = g.due(t + Duration::from_millis(12_500 + 50 * k));
            if last.iter().any(|(_, o)| o.ty == HEALTH_BROADCAST && o.body[20..24] == [0; 4]) {
                break;
            }
        }
        let hb = &last.iter().find(|(_, o)| o.ty == HEALTH_BROADCAST).expect("full").1;
        assert_eq!(&hb.body[12..16], &112f32.to_le_bytes());
        assert!(g.due(t + Duration::from_secs(30)).iter().all(|(_, o)| o.ty != HEALTH_BROADCAST), "stops at full");
    }

    #[test]
    fn used_ordnance_is_refunded_after_its_cooldown() {
        let mut g = Gear::new();
        let t0 = Instant::now();
        g.on_message(addr(), SPAWN_MAGAZINE, &unhex("f234d5099974113f01000000dcf521424efb6d3f8045bac1fee794a8b1d0e8c886b304072f528195"), t0);
        g.on_message(addr(), MAGAZINE_STATE, &unhex("26452bfcf77799cb86b304072f528195a9509fd4dfc72de3"), t0);
        assert!(g.due(t0 + Duration::from_secs(20)).iter().all(|(_, o)| o.ty != COOLDOWN_REFUND));
        let later = g.due(t0 + Duration::from_secs(40));
        assert!(later.iter().any(|(_, o)| o.ty == COOLDOWN_REFUND && o.body == unhex("f234d5099974113f58c51a933fe33556")));
    }

    #[test]
    fn thrown_magazine_ends_after_its_lifetime() {
        let mut g = Gear::new();
        let t0 = Instant::now();
        let spawn = unhex("f234d5099974113f01000000dcf521424efb6d3f8045bac1fee794a8b1d0e8c886b304072f528195");
        g.on_message(addr(), SPAWN_MAGAZINE, &spawn, t0);
        let thrown = unhex("26452bfcf77799cb86b304072f528195a9509fd4dfc72de3");
        g.on_message(addr(), MAGAZINE_STATE, &thrown, t0);
        assert!(g.due(t0).is_empty());
        let at = g.due(t0 + Duration::from_millis(200));
        assert_eq!(at[0].1, out(To::All, MAGAZINE_STATE, &unhex("26452bfcf77799cb86b304072f5281956281fb261b84f533")));
        assert_eq!(at[1].1, out(To::Sender, COOLDOWN_ACTIVE, &unhex("f234d5099974113f58c51a933fe33556")));
        let end = g.due(t0 + Duration::from_secs(3));
        assert_eq!(end[0].1, out(To::All, DESTROY_MAGAZINE, &unhex("86b304072f528195")));
    }
}
