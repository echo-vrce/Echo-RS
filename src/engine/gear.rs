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

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::wire::symbol::mat_sym;
use serde_json::Value;
use tracing::{debug, info};

const fn h(name: &str) -> u64 {
    mat_sym(name)
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
/// A match's stun (native Fission with EchoCombat, 2026-11-21): `[victim
/// slot u32][1 u32][seconds f32][0 u32]`, to everyone, with the explosion.
/// `SR15StunMsg` (`STUN_MSG`) is the social lobby's: in a match the games
/// ignored it, and STUN FIELD, ARC MINE and NEUTRON stunned nobody.
pub const STUN_PLAYER_MSG: u64 = 0xccd9_ed2d_3125_ff67;
pub const HEALTH_BROADCAST: u64 = 0x97bb_bd24_5d46_7354;
/// `[damageable 8][slot u32][0 u32]`: native sends it with each INSTANT REPAIR
/// that gives a player's shield back.
pub const SHIELD_REPAIRED: u64 = h("SR15NetDamageableShieldRepaired");
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
/// Fission's end-game pillars (the climax sublevel's damageables, 1000
/// health): no break. Native (2026-11-21) raised one once it had taken 50
/// (health at or under 950): the pillar's own `ComponentEvent`, twice in the
/// frame. 25.03 s later the same event lowered it, and a frame after that
/// (34 ms) `DealDamage` put it back at full. Hits while it is up still count
/// down; health does not come back on its own while it is down.
const PILLAR_EVENT: u64 = 0x8099_0d12_aec2_bcda;
const PILLAR_RAISE_DAMAGE: f32 = 50.0;
const PILLAR_UP: Duration = Duration::from_millis(25_030);
const PILLAR_RESET_AFTER: Duration = Duration::from_millis(34);
#[cfg(test)]
const AREA_OBJECT: [u8; 4] = [0x26, 0x45, 0x2b, 0xfc];
/// Explosion kinds (bytes 8..16 of the explosion messages).
const EX_ROCKET: [u8; 8] = [0xdc, 0x35, 0x4e, 0x7e, 0x5c, 0x3d, 0x0a, 0xb3];
const EX_DET: [u8; 8] = [0x6c, 0x8a, 0xc2, 0x55, 0xa1, 0x24, 0x51, 0x20];
const EX_STUN: [u8; 8] = [0x10, 0x3f, 0x1d, 0xe6, 0x54, 0x5b, 0x90, 0x25];
/// ARC MINE's own blast (native Fission, 2026-11-21; ours used the STUN FIELD's).
const EX_ARC: [u8; 8] = [0xae, 0xda, 0x1e, 0xfa, 0xef, 0x03, 0xeb, 0x17];
const EX_DOT: [u8; 8] = [0x06, 0x2a, 0xbc, 0x9b, 0x8f, 0xed, 0x2c, 0xea];
/// What a shooter's hit marker names (`LocallyTrackedDamage` bytes 8..16).
const MARK_ROCKET: [u8; 8] = [0x8d, 0xa7, 0xeb, 0x67, 0xbd, 0x59, 0x0f, 0x11];
const MARK_SCOUT: [u8; 8] = [0xe7, 0xda, 0xbe, 0xe8, 0x48, 0x77, 0x82, 0x03];
/// `GameFireGun` byte 0: the weapon that fired (COMET 0x1e, METEOR 0x2d).
const GUN_SCOUT: u8 = 0x1e;
/// `DamageableHealthBroadcast` damage kinds (u32 at 20): explosions 4, ordnance 2.
const HURT_EXPLOSION: u32 = 4;
/// A shot's body part for the head (`multiplier`'s markers).
const PART_HEAD: u32 = 5;
const HURT_ORDNANCE: u32 = 2;
/// DETONATOR damage (native: 65 on a dummy, killed a player at 27).
const DET_DAMAGE: f32 = 65.0;
const DET_RADIUS: f64 = 1.5;
/// How far a det reaches barriers, and its damage to one at a distance:
/// full to 1 m, then down to 40% at the edge. Native's hits fit a 3 m edge:
/// Surge (2026-11-21) took 101.79 off a barrier 1.93 m from the blast, Dyson
/// 97 at about 2 m (our 2.5 m edge gave 88 and 84).
const DET_BARRIER_REACH: f32 = 3.0;
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
/// PHASE SHIFT's tac mod in `SR15NetPlayerEquipmentUpdate` bytes 16..24
/// (the symbol of its `mp_equipment` name, `wraith`).
const TAC_WRAITH: u64 = 0x41d2_d02f_1b2a_1316;
/// ARC MINE (`grenade_properties.arc`).
const ITEM_ARC: [u8; 8] = [0xe9, 0xf0, 0x91, 0xa8, 0xb1, 0xd0, 0xe8, 0xc8];
/// NEUTRON's tac mod (EchoCombat's `point`; `CSymbol64("point")`).
const TAC_POINT: u64 = 0xe32d_c7ca_d686_4aa4;
/// UMBRAL VEIL's tac mod (EchoCombat's `smoke`).
const TAC_SMOKE: u64 = crate::wire::symbol::symbol("smoke");
/// INSTANT REPAIR (`grenade_properties.burst`).
const ITEM_BURST: [u8; 8] = [0xe9, 0xed, 0x9c, 0xa8, 0xb1, 0xd0, 0xe8, 0xc8];
/// A bullet collision's target (bytes 32..40) is a pool handle when its kind
/// (bytes 36..40) is 1: `[index u16][generation u16]`.
const HANDLE_KIND: [u8; 4] = [1, 0, 0, 0];
/// Hit body part 6 is "barrier" in the damage table.
const PART_BARRIER: u64 = 6;
/// A barrier the level data does not list (the stock maximum).
const BARRIER_HEALTH: f32 = 230.0;
/// ENERGY BARRIER (and NEUTRON) walls: the pooled actor template of the
/// 96-byte `PooledActorCreate`, their health (native: a DETONATOR's 140 left
/// one at 275; drone bullets took Gauss's from 415 down by ones), and when
/// native's server removes them (`PooledActorDestroy` to everyone, the placer
/// too, 18.04 s after the create in all ten recorded).
const WALL_TEMPLATE: [u8; 8] = [0x26, 0x29, 0x29, 0xd5, 0xa4, 0x8c, 0x8b, 0x3d];
const WALL_HEALTH: f32 = 415.0;
const WALL_LIFETIME: Duration = Duration::from_millis(18_040);
/// A wall's box in its own frame (the level barriers' 230-health box).
const WALL_BOX: [f32; 3] = [1.5, 1.0, 0.15];
/// Native's barrier break / respawn event (`ComponentEvent` to the barrier's
/// net handle, all clients, around a `DealDamage` with its new health).
/// Wire bytes `ff278203989fcc2b` / `c4b6bab2ee99ea6a` (u64s little-endian;
/// these were written byte-reversed before 2026-11-16, so clients never saw
/// the break and kept broken barriers up).
const BARRIER_EVENT: u64 = 0x2bcc_9f98_0382_27ff;
pub const BARRIER_MODULE: u64 = 0x6aea_99ee_b2ba_b6c4;
use super::msg::{ACTOR_DESTROY, COMPONENT_EVENT};

/// A level's damageable net components (`tools/gen_combat_maps.py`): pool
/// slot, net handle, max health, respawn seconds (0: stays down).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Barrier {
    /// The space its damageable is in: the level's, or a sublevel's (Fission's
    /// climax).
    space: [u8; 8],
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

    /// Whether the segment `a`..`b` passes through the barrier's box, past
    /// its first `skip` metres.
    fn crosses(&self, a: [f32; 3], b: [f32; 3], skip: f32) -> bool {
        if self.half.iter().all(|h| *h <= 0.0) {
            return false;
        }
        let [x, y, z, w] = self.rot;
        let local = |p: [f32; 3]| {
            let mut l = rotate([-x, -y, -z, w], [p[0] - self.pos[0], p[1] - self.pos[1], p[2] - self.pos[2]]);
            l[2] -= self.off_z;
            l
        };
        let (la, lb) = (local(a), local(b));
        let len = dist(a, b);
        if len <= skip + skip {
            return false;
        }
        let (mut t0, mut t1) = (skip / len, 1.0 - skip / len);
        for i in 0..3 {
            let d = lb[i] - la[i];
            if d.abs() < 1e-9 {
                if la[i].abs() > self.half[i] {
                    return false;
                }
                continue;
            }
            let (mut e, mut f) = ((-self.half[i] - la[i]) / d, (self.half[i] - la[i]) / d);
            if e > f {
                std::mem::swap(&mut e, &mut f);
            }
            t0 = t0.max(e);
            t1 = t1.min(f);
            if t0 > t1 {
                return false;
            }
        }
        true
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
    let mut v = parse_barriers(space, text);
    // Fission's climax sublevel has its own damageables (1000 health; native
    // dealt them damage in their own space, pool slot 2 = handle 258).
    if u64::from_le_bytes(space) == LEVEL_FISSION {
        v.extend(parse_barriers(super::msg::LEVEL_FISSION_CLIMAX.to_le_bytes(), include_str!("data/combat_fission_climax_barriers.txt")));
    }
    v
}

fn parse_barriers(space: [u8; 8], text: &str) -> Vec<Barrier> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let n = |i: usize| f.get(i).and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0);
            Some(Barrier {
                space,
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

fn pillar_event(handle: u16) -> [u8; 24] {
    super::msg::component_event(PILLAR_EVENT, BARRIER_MODULE, handle, super::msg::FILTER_ALL_ALT)
}

fn barrier_event(handle: u16) -> [u8; 24] {
    super::msg::component_event(BARRIER_EVENT, BARRIER_MODULE, handle, super::msg::FILTER_ALL_ALT)
}
/// Each player's gun heat meter, a replicated var per player slot (native,
/// every combat recording and the lobby): 0..1 up a step a shot, down after
/// the gun's cooldown delay. Keys for slots 0..9: slots 1..9 recorded, slot 0
/// from them (the slot digit sits 13 characters from the end of the var's
/// name, so keys differ by the CRC of the digits; slot 0's is the one the
/// Gauss server recording carries at tag 0). Slots 10 and up: unknown.
const HEAT_KEYS: [u64; 10] = [
    0x1ca6_8bb9_941d_b80a,
    0xf6b7_893b_b825_5e34,
    0x9bc3_8757_08d1_40a8,
    0x71d2_85d5_24e9_a696,
    0xa5b0_8065_11d7_72d4,
    0x4fa1_82e7_3def_94ea,
    0x22d5_8c8b_8d1b_8a76,
    0xc8c4_8e09_a123_6c48,
    0x379d_9636_3206_10b6,
    0xdd8c_94b4_1e3e_f688,
];
/// How long an overheat shows full before the meter drops to 0.
const HEAT_OVERHEAT_SHOWN: Duration = Duration::from_millis(400);

#[derive(Clone, Copy)]
struct GunHeat {
    level: f32,
    last_shot: Option<Instant>,
    last_tick: Instant,
    delay: Duration,
    /// Per second, as a fraction of full.
    cooling: f32,
    overheated: Option<Instant>,
}

impl Default for GunHeat {
    fn default() -> Self {
        Self { level: 0.0, last_shot: None, last_tick: Instant::now(), delay: Duration::from_secs(1), cooling: 0.5, overheated: None }
    }
}

/// OVERCHARGE (the user's spec, 2026-11-16): the user and linked teammates
/// take 25% less damage; linked teammates cannot die for `cheat_death()` from their first
/// link, until the link breaks or they leave.
const OVERCHARGE_DR: f32 = 0.25;
/// DOT FIELD's time on the ground (the user's choice, 2026-11-16; stock 8 s).
const DOT_LIFETIME: Duration = Duration::from_secs(11);
/// OVERCHARGE's cheat death: `ability_properties.buff.cheat_death_seconds`
/// (EchoCombat: 6.5) from a teammate's first link in a use, or none when
/// `do_rez_buff` is 0.
fn cheat_death() -> Duration {
    let b = |k: &str| weapons().pointer(&format!("/ability_properties/buff/{k}")).and_then(Value::as_f64);
    if b("do_rez_buff").is_some_and(|x| x == 0.0) {
        return Duration::ZERO;
    }
    Duration::from_secs_f64(b("cheat_death_seconds").unwrap_or(5.0).clamp(0.0, 600.0))
}

/// How long an OVERCHARGE boost lasts (`ability_properties.buff.dr_duration`).
fn overcharge_duration() -> Duration {
    let secs = weapons().pointer("/ability_properties/buff/dr_duration").and_then(Value::as_f64).unwrap_or(10.0);
    Duration::from_secs_f64(secs.clamp(1.0, 60.0))
}
/// A player's damageable id is this with its low byte xor the slot.
const PLAYER_ID: [u8; 8] = [0xf2, 0x34, 0xd5, 0x09, 0x99, 0x74, 0x11, 0x3f];
const NO_PLAYER: [u8; 8] = [0xff; 8];

/// A player's damageable id: `PLAYER_ID` with bytes 0..2 xor their slot and
/// bytes 4..6 xor their generation (a player who rejoined, generation 1, is
/// `f7 34 d5 09 98 74 11 3f` at slot 5: hits on them carry that).
fn player_id(slot: u16, generation: u16) -> [u8; 8] {
    let mut id = PLAYER_ID;
    let (s, g) = (slot.to_le_bytes(), generation.to_le_bytes());
    id[0] ^= s[0];
    id[1] ^= s[1];
    id[4] ^= g[0];
    id[5] ^= g[1];
    id
}

/// The (slot, generation) a player's damageable id names, if it is one.
fn id_handle(id: [u8; 8]) -> Option<(u16, u16)> {
    if id[2..4] != PLAYER_ID[2..4] || id[6..8] != PLAYER_ID[6..8] {
        return None;
    }
    let x = |i: usize| id[i] ^ PLAYER_ID[i];
    let (slot, generation) = (u16::from_le_bytes([x(0), x(1)]), u16::from_le_bytes([x(4), x(5)]));
    // Real slots and generations are small: anything else is another object.
    (slot < 64 && generation < 256).then_some((slot, generation))
}
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
/// REPAIR MATRIX's heal bubble (`HealBubbleParamUpdate`: heal 114, 12 s, radius 8,
/// `ability_properties.heal`).
const REPAIR_BUBBLE: [u8; 8] = [0xd1, 0xeb, 0x3d, 0x3f, 0xa8, 0xa2, 0xfd, 0xb1];
/// A full shield, as native broadcasts it (the first of the two values).
/// Every hit, healing and regeneration seen in native moves this one.
const FULL_HEALTH: f32 = 112.0;
/// The health under the shield (the second value: 7.0 alive, 0 dead). The
/// game's `ProcessDamageServer` (EchoCombat's powerboost.cpp): the shield takes
/// a hit first; health only loses what the shield did not take, and only once
/// the shield is down or the hit is at least twice the shield. So no single hit
/// kills from a full shield (a charged COMET head hit, 120, leaves 0 shield and
/// 7 health: native Fission, 2026-11-16, then regenerated); the next one does.
const FULL_CORE: f32 = 7.0;
/// The position native reports is the body's, a little lower.
const BODY_DROP: f32 = 0.35;
const BODY_RADIUS: f32 = 0.3;
/// A player's health and what last hurt them, as `DamageableHealthBroadcast`
/// carries it: `[owner][u32 0][health][7.0][kind u32][u32 0][ffffffff until hurt]
/// [hit position][12 zero][cause]`. Native keeps the last hurt's fields.
#[derive(Clone, Copy)]
struct Life {
    /// The shield (see `FULL_HEALTH`).
    health: f32,
    /// The health under it (see `FULL_CORE`).
    core: f32,
    /// The last hit: kind, place, cause, and the shot's direction (native:
    /// the bullet's direction from its `BulletCollision`; 0 for blasts).
    hurt: Option<(u32, [f32; 3], [u8; 8], [f32; 3])>,
    dead: bool,
    died_at: Option<Instant>,
    /// When health last went down, and when regeneration last sent.
    lost_at: Option<Instant>,
    regen_at: Option<Instant>,
}

impl Life {
    fn full(&self) -> bool {
        self.health >= FULL_HEALTH && self.core >= FULL_CORE
    }
}

impl Default for Life {
    fn default() -> Self {
        Self { health: FULL_HEALTH, core: FULL_CORE, hurt: None, dead: false, died_at: None, lost_at: None, regen_at: None }
    }
}

/// The health broadcast; `entry` = the area-entry reset (cause `ff..` before any hurt).
/// Bytes 8..12 are the player's slot (the 8-byte id before it is the same
/// for every player: a Gauss match's broadcasts carry one 56-byte record per
/// slot after it).
/// A player's handle as health broadcasts and kills carry it: their slot
/// (from their body physics, else the session's) with their generation
/// above it (`gen << 16 | slot`; a player who rejoined is generation 1).
fn player_handle(bodies: &HashMap<SocketAddr, (u16, [f32; 3])>, slots: &HashMap<SocketAddr, u32>, gens: &HashMap<SocketAddr, u16>, who: &SocketAddr) -> u32 {
    let slot = bodies.get(who).map(|b| u32::from(b.0)).or_else(|| slots.get(who).copied()).unwrap_or(0);
    (u32::from(gens.get(who).copied().unwrap_or(0)) << 16) | (slot & 0xffff)
}

fn health_body(owner: [u8; 8], slot: u32, life: &Life, entry: bool) -> Vec<u8> {
    let mut b = owner.to_vec();
    b.extend_from_slice(&slot.to_le_bytes());
    b.extend_from_slice(&life.health.to_le_bytes());
    // The health under the shield: 7.0 at full, 0 once dead (native's
    // broadcast at a kill).
    b.extend_from_slice(&(if life.dead { 0f32 } else { life.core }).to_le_bytes());
    let (kind, pos, cause, dir) = life.hurt.unwrap_or((0, [0.0; 3], if entry { [0xff; 8] } else { [0; 8] }, [0.0; 3]));
    b.extend_from_slice(&kind.to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(if life.hurt.is_some() { &[0; 4] } else { &[0xff; 4] });
    for x in pos.into_iter().chain(dir) {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b.extend_from_slice(&cause);
    b
}

/// A health broadcast for health coming back (regen, heals), as native's
/// regen sends it: the regen kind until full, and ffffffff at 28 (no hit).
fn regen_body(owner: [u8; 8], slot: u32, life: &Life) -> Vec<u8> {
    let mut shown = *life;
    shown.hurt = if life.full() { None } else { Some((HURT_REGEN, [0.0; 3], [0; 8], [0.0; 3])) };
    let mut b = health_body(owner, slot, &shown, false);
    b[28..32].copy_from_slice(&[0xff; 4]);
    b
}

/// A hit of `amount` on `shield` over `core` (`ProcessDamageServer`): what
/// each loses.
fn split_hit(shield: f32, core: f32, amount: f32) -> (f32, f32) {
    if shield > 0.0 {
        let to_core = if amount >= shield + shield { amount - shield } else { 0.0 };
        (amount.min(shield), to_core.min(core.max(0.0)))
    } else {
        (0.0, amount.min(core.max(0.0)))
    }
}

/// The combat area's id as it appears in messages.
const AREA: [u8; 8] = [0x26, 0x45, 0x2b, 0xfc, 0xf7, 0x77, 0x99, 0xcb];
/// Magazine states (`SetMagazineState` bytes 16..24).
const MAG_THROWN: [u8; 8] = [0xa9, 0x50, 0x9f, 0xd4, 0xdf, 0xc7, 0x2d, 0xe3];
const MAG_USED: [u8; 8] = [0x62, 0x81, 0xfb, 0x26, 0x1b, 0x84, 0xf5, 0x33];
/// Ordnance in hand (spawned, or picked back up) and let go of.
const MAG_HELD: [u8; 8] = [0x1b, 0x0b, 0x21, 0x1b, 0x29, 0xd4, 0xd2, 0x41];
const MAG_RELEASED: [u8; 8] = [0x63, 0x88, 0xfd, 0x28, 0x03, 0x87, 0xe3, 0x23];
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
        ITEM_ARC => "arc",
        ITEM_BURST => "burst",
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

#[derive(Clone, Copy)]
struct Mag {
    owner: [u8; 8],
    item: [u8; 8],
    thrower: SocketAddr,
    ending: bool,
    /// The thrower's last thrown-ordnance position when this one spawned:
    /// still the same when it is used means this one never sent its own
    /// (an INSTANT REPAIR let go of in the hand), so that position is an
    /// older grenade's.
    at_spawn: Option<[f32; 3]>,
}

/// An ARC MINE let go of: where it is (its last thrown position until it
/// arms), when it arms, and when it goes off once something set it off.
struct Mine {
    mag: [u8; 8],
    owner: [u8; 8],
    /// `SetMagazineState` bytes 0..8, for the used state the server sends.
    head: [u8; 8],
    thrower: SocketAddr,
    at: Option<[f32; 3]>,
    armed_at: Instant,
    fire_at: Option<Instant>,
    /// When a shot first hit it: its owner's client then says it is used
    /// (native); the server takes it down itself if that never comes.
    shot_at: Option<Instant>,
}

/// How long the server waits for a shot ARC MINE's owner to say it is used
/// (native: 16 ms after the hits) before taking it down itself.
const MINE_SHOT_GRACE: Duration = Duration::from_millis(500);

/// How far apart a NEUTRON's wall and its VeilNet `P` token may come.
const POINT_TOKEN_WINDOW: Duration = Duration::from_millis(1500);

/// A NEUTRON wall: where it stands, whose it is, its actor (the blast's id)
/// and when it goes off.
struct Neutron {
    at: [f32; 3],
    thrower: SocketAddr,
    actor: [u8; 8],
    fire_at: Instant,
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
    /// When it went off (a STUN FIELD grows from then).
    start: Instant,
    /// Victims a stun field already stunned.
    hit: Vec<[u8; 8]>,
    /// A STUN FIELD rolls with its grenade (`rolling_effect_lifetime`):
    /// when `at` last followed the grenade's position.
    moved: Instant,
    /// Whether the grenade has sent a position since the field opened: until
    /// it does, `at` may still be the thrower's hand.
    tracked: bool,
}

/// How long a STUN FIELD waits for its grenade's own position before it
/// stuns where it opened.
const STUN_SETTLE: Duration = Duration::from_millis(150);
/// The fastest a STUN FIELD's grenade is followed (m/s), plus a margin:
/// a bigger jump is another of its thrower's objects, not this grenade.
const STUN_FOLLOW_SPEED: f32 = 40.0;
/// The longest gap the follow allowance grows over (s).
const STUN_FOLLOW_MAX_DT: f32 = 0.1;
/// Once on its grenade, each update is the grenade's next step: a field
/// never moves further than its speed allows plus this (m).
const STUN_FOLLOW_TRACKED_SLACK: f32 = 0.5;
const STUN_FOLLOW_SLACK: f32 = 2.0;

/// (victim id, reported position, distance, player, marker hit code)
type Hit = ([u8; 8], [f32; 3], f32, Option<SocketAddr>, u32);

/// An OVERCHARGE running: its self-damage ticks.
/// OVERCHARGE's small heal at work: its user, until when, when it last
/// healed, and how much each player has had from it (capped).
struct OcHeal {
    who: SocketAddr,
    until: Instant,
    last: Instant,
    given: HashMap<SocketAddr, f32>,
}

/// A REPAIR MATRIX at work: its user, until when, and when it last healed.
struct Matrix {
    who: SocketAddr,
    until: Instant,
    last: Instant,
}

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
    /// `head`: the killing blow was a shot to the head (body part 5).
    Kill { by: Option<SocketAddr>, victim: SocketAddr, head: bool },
    /// A stun field stunned a player.
    Stun { by: SocketAddr },
    /// A REPAIR MATRIX or INSTANT REPAIR gave back health (to its user or a teammate).
    Heal { by: SocketAddr, amount: f32 },
}

pub struct Gear {
    /// Damage and kills since the session last took them (match stats).
    pub credits: Vec<Credit>,
    bits: [[u8; 4]; BITS],
    mags: HashMap<[u8; 8], Mag>,
    later: Vec<(Instant, SocketAddr, Out)>,
    /// Each player's slot and body position (metres).
    bodies: HashMap<SocketAddr, (u16, [f32; 3])>,
    /// Lag compensation: each body's recent positions (`HISTORY` of them).
    history: HashMap<SocketAddr, VecDeque<(Instant, [f32; 3])>>,
    /// Each player's round trip, from the session (`set_ping`).
    pings: HashMap<SocketAddr, Duration>,
    /// Each player's team (0 blue, 1 orange, else not playing), kept by the session.
    pub teams: HashMap<SocketAddr, u8>,
    /// Each player's lobby slot, kept by the session: whose gear stations to
    /// free when a player leaves before any body physics came from them.
    pub slots: HashMap<SocketAddr, u32>,
    /// Each player's generation (1 once they rejoined into a used slot), kept
    /// by the session: part of their damageable id and handle.
    pub gens: HashMap<SocketAddr, u16>,
    /// What clients claim, checked (see `checks`).
    checks: super::checks::Checks,
    /// Where each player's thrown ordnance is.
    thrown_at: HashMap<SocketAddr, [f32; 3]>,
    /// When each player last took a new ordnance: from then on their thrown
    /// position is that one's, not an older STUN FIELD's grenade.
    spawned_at: HashMap<SocketAddr, Instant>,
    /// Safety net: each player's ordnance thrown or placed and not yet given
    /// back (when the refund is overdue, its `CooldownRefund` body).
    owed_refund: HashMap<SocketAddr, (Instant, Vec<u8>)>,
    /// Safety net: each player's tac mod in use (when its `AbilityCooldown`
    /// is overdue, its `[slot u32]`).
    tac_running: HashMap<SocketAddr, (Instant, [u8; 4])>,
    /// Each player's gun heat, as native's server sends it (see `HEAT_KEYS`).
    heat: HashMap<SocketAddr, GunHeat>,
    /// Where it was before its last move, and whether it has stopped since
    /// (stuck to a surface): a DETONATOR's blast does not go through that surface.
    thrown_from: HashMap<SocketAddr, ([f32; 3], bool)>,
    fields: Vec<Field>,
    /// Heal bubble parameters by bubble, and each player's bubble owner id.
    bubbles: Vec<([u8; 8], Vec<u8>)>,
    bubble_owner: HashMap<SocketAddr, [u8; 8]>,
    /// The bullet direction for the next `hurt` (a shot's).
    shot_dir: Option<[f32; 3]>,
    /// Each player's health as the server last broadcast it.
    lives: HashMap<SocketAddr, Life>,
    /// Shots by explosion id: (weapon byte, projectile damage), from `GameFireGun`.
    shots: HashMap<[u8; 8], (u8, f32)>,
    /// DRONE hits the server found from a shooter's shots (drone, shooter):
    /// how many their client's own hit reports still have to match.
    drone_rays: HashMap<([u8; 8], SocketAddr), u32>,
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
    /// Fission's raised pillars and when they come down.
    pillars_up: Vec<(Instant, [u8; 8], Barrier)>,
    /// Standing ENERGY BARRIER walls by actor id: their box (pool and handle
    /// unused) and placer.
    walls: HashMap<[u8; 8], (Barrier, SocketAddr)>,
    /// OVERCHARGE users and until when their boost lasts (PowerBoost: their gun
    /// hits deal more, the hits they take less).
    boosted: HashMap<SocketAddr, Instant>,
    /// Each player's tac mod (equipment update bytes 16..24).
    tactical: HashMap<SocketAddr, u64>,
    /// PHASE SHIFT users and until when it lasts at most.
    phased: HashMap<SocketAddr, Instant>,
    /// NEUTRON walls waiting to go off.
    neutrons: Vec<Neutron>,
    /// Each player's last wall that wasn't taken as a NEUTRON (when, what):
    /// a VeilNet `P` token that comes just after it makes it one.
    last_wall: HashMap<SocketAddr, (Instant, Vec<u8>)>,
    /// Each player's last VeilNet `P` token not yet matched to a wall.
    point_tokens: HashMap<SocketAddr, Instant>,
    /// For games without EchoCombat only (`due_stock`): the stock DETONATOR
    /// smoke standing in for UMBRAL VEIL, which they can't show.
    stock_only: Vec<(Instant, Out)>,
    /// The next stand-in magazine's number.
    veil_mags: u32,
    /// ARC MINEs out.
    mines: Vec<Mine>,
    /// Running OVERCHARGE heals, and each linked teammate's OVERCHARGE user.
    oc_heals: Vec<OcHeal>,
    oc_linked: HashMap<SocketAddr, SocketAddr>,
    /// Running REPAIR MATRIXes.
    matrices: Vec<Matrix>,
    /// Teammates an OVERCHARGE bubble linked, and when their cheat death ends.
    links: HashMap<SocketAddr, Instant>,
    /// When each OVERCHARGE user's current use began, and which use each
    /// linked teammate's cheat death came from: one cheat death per use.
    oc_use: HashMap<SocketAddr, Instant>,
    link_use: HashMap<SocketAddr, (SocketAddr, Instant)>,
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
            history: HashMap::new(),
            pings: HashMap::new(),
            teams: HashMap::new(),
            slots: HashMap::new(),
            gens: HashMap::new(),
            checks: super::checks::Checks::default(),
            thrown_at: HashMap::new(),
            spawned_at: HashMap::new(),
            owed_refund: HashMap::new(),
            tac_running: HashMap::new(),
            heat: HashMap::new(),
            thrown_from: HashMap::new(),
            fields: Vec::new(),
            bubbles: Vec::new(),
            bubble_owner: HashMap::new(),
            shot_dir: None,
            lives: HashMap::new(),
            shots: HashMap::new(),
            drone_rays: HashMap::new(),
            drains: Vec::new(),
            matrices: Vec::new(),
            oc_heals: Vec::new(),
            oc_linked: HashMap::new(),
            mines: Vec::new(),
            neutrons: Vec::new(),
            last_wall: HashMap::new(),
            point_tokens: HashMap::new(),
            stock_only: Vec::new(),
            veil_mags: 0,
            tactical: HashMap::new(),
            phased: HashMap::new(),
            inside: Vec::new(),
            left: HashMap::new(),
            leaving: HashMap::new(),
            area_events: HashSet::new(),
            area_health: HashMap::new(),
            barriers: barriers_of(AREA),
            walls: HashMap::new(),
            respawns: Vec::new(),
            pillars_up: Vec::new(),
            boosted: HashMap::new(),
            links: HashMap::new(),
            oc_use: HashMap::new(),
            link_use: HashMap::new(),
            space: AREA,
            in_match: false,
        }
    }

    /// A combat match on `level`: its space, everyone in combat.
    pub fn set_match(&mut self, level: u64) {
        self.space = level.to_le_bytes();
        self.barriers = barriers_of(self.space);
        self.walls.clear();
        self.in_match = true;
        // Load the map's walls now, not at the first DETONATOR.
        let _ = super::walls::of(level);
    }

    /// In combat: the lobby's combat area, or anywhere in a match.
    fn in_combat(&self, who: &SocketAddr) -> bool {
        self.in_match || self.inside.contains(who)
    }

    /// A player's equipment update: remembers their tac mod (PHASE SHIFT
    /// or not); a change of tac mod ends a running phase.
    pub fn set_equipment(&mut self, who: SocketAddr, d: &[u8]) {
        let Some(b) = d.get(16..24) else { return };
        let tac = u64::from_le_bytes(b.try_into().expect("8"));
        if tac != TAC_WRAITH {
            self.phased.remove(&who);
        }
        self.tactical.insert(who, tac);
    }

    /// A placed wall (`PooledActorCreate`, 96 bytes) from this player is
    /// EchoCombat's UMBRAL VEIL or NEUTRON: only EchoCombat's scripts turn it
    /// into that, so a game without them would show an ENERGY BARRIER.
    pub fn echocombat_wall(&self, from: SocketAddr, d: &[u8]) -> bool {
        d.len() == 96
            && d[32..40] == WALL_TEMPLATE
            && (matches!(self.tactical.get(&from), Some(&TAC_SMOKE | &TAC_POINT))
                || self.point_tokens.get(&from).is_some_and(|t| t.elapsed() <= POINT_TOKEN_WINDOW))
    }

    /// UMBRAL VEIL for games without EchoCombat: a stock game has no veil
    /// (EchoCombat's script spawns DETONATOR smoke, scaled), so it gets real
    /// DETONATOR smoke where the veil is, a puff every `VEIL_PUFF_EVERY` for
    /// `veil_cloud_lifetime`. Each puff is a magazine as another player's
    /// DETONATOR reaches a game (client recording 2026-11-16: spawned with
    /// its position at bytes 12..24, detonated, used, destroyed 2.5 s later),
    /// with nothing hit: it hurts nobody.
    fn veil_stand_in(&mut self, from: SocketAddr, at: [f32; 3], now: Instant) {
        let space = self.space;
        let slot = self.slot_of(&from) as u16;
        let owner = self.id_of(&from, slot);
        let life = weapons().pointer("/ability_properties/smoke/veil_cloud_lifetime").and_then(Value::as_f64).unwrap_or(8.0).clamp(0.5, 60.0);
        let puffs = (life / VEIL_PUFF_EVERY.as_secs_f64()).ceil().max(1.0) as u32;
        for k in 0..puffs {
            self.veil_mags = self.veil_mags.wrapping_add(1);
            let mut mag = VEIL_MAG;
            mag[4..8].copy_from_slice(&self.veil_mags.to_le_bytes());
            let t = now + VEIL_PUFF_EVERY * k;
            let mut spawn = owner.to_vec();
            spawn.extend_from_slice(&1u32.to_le_bytes());
            for c in at {
                spawn.extend_from_slice(&c.to_le_bytes());
            }
            spawn.extend_from_slice(&ITEM_DET);
            spawn.extend_from_slice(&mag);
            self.stock_only.push((t, out(To::All, SPAWN_MAGAZINE, &spawn)));
            let t = t + Duration::from_millis(50);
            self.stock_only.push((t, out(To::All, MAGAZINE_STATE, &[space.as_slice(), &mag, &MAG_THROWN].concat())));
            self.stock_only.push((t, out(To::All, EXPLOSION_NOTHING_HIT, &[space.as_slice(), &EX_DET, &mag].concat())));
            self.stock_only.push((t + Duration::from_millis(100), out(To::All, MAGAZINE_STATE, &[space.as_slice(), &mag, &MAG_USED].concat())));
            self.stock_only.push((t + DESTROY_AFTER, out(To::All, DESTROY_MAGAZINE, &mag)));
        }
    }

    /// What only games without EchoCombat get, now.
    pub fn due_stock(&mut self, now: Instant) -> Vec<Out> {
        let mut v = Vec::new();
        self.stock_only.retain(|(t, o)| {
            if *t <= now {
                v.push(o.clone());
                false
            } else {
                true
            }
        });
        v
    }

    /// The safety net's view of a player's gear: what was used and not yet
    /// recharged. Cleared by the recharge itself (or proof of it: a new
    /// ordnance in hand), set by a throw or a tac mod switched on.
    fn watch_recharge(&mut self, from: SocketAddr, ty: u64, d: &[u8], now: Instant) {
        match ty {
            ABILITY_ACTIVATE if d.len() == 8 && d[4..8] != [0; 4] => {
                let tac = self.tactical.get(&from).copied().unwrap_or(0);
                let mut slot = [0; 4];
                slot.copy_from_slice(&d[..4]);
                self.tac_running.insert(from, (now + tac_overdue(tac), slot));
            }
            // A respawn gives the tac mod back on the client's own.
            ABILITY_COOLDOWN | PLAYER_RESPAWN => {
                self.tac_running.remove(&from);
            }
            SPAWN_MAGAZINE | DEPOSIT_MAGAZINE => {
                self.owed_refund.remove(&from);
            }
            MAGAZINE_STATE if d.len() == 24 => {
                let state = arr8(d, 16);
                let Some(m) = self.mags.get(&arr8(d, 8)).copied() else { return };
                if state == MAG_THROWN || state == MAG_RELEASED {
                    let due = now + lifetime(m.item) + cooldown(m.item) + REFUND_OVERDUE_SLACK;
                    // A later state of the same throw keeps the first deadline.
                    self.owed_refund.entry(m.thrower).or_insert((due, [m.owner, COOLDOWN_TAIL].concat()));
                } else if state == MAG_HELD {
                    self.owed_refund.remove(&m.thrower);
                }
            }
            _ => {}
        }
    }

    /// Recharges the server owes and never sent: an ordnance whose
    /// `CooldownRefund` is overdue gets it, a tac mod whose `AbilityCooldown`
    /// never came gets one (as native answers it: two to the user, one to
    /// everyone else, then the tac mod off).
    fn overdue_recharges(&mut self, now: Instant, sent: &[(SocketAddr, Out)], ready: &mut Vec<(SocketAddr, Out)>) {
        for (who, o) in sent {
            if o.ty == COOLDOWN_REFUND {
                self.owed_refund.remove(who);
            }
        }
        // An ARC MINE waits where it was placed for as long as it takes.
        let placed: HashSet<SocketAddr> = self.mines.iter().map(|m| m.thrower).collect();
        let late: Vec<SocketAddr> = self.owed_refund.iter().filter(|(a, (t, _))| *t <= now && !placed.contains(a)).map(|(a, _)| *a).collect();
        for who in late {
            if let Some((_, cool)) = self.owed_refund.remove(&who) {
                tracing::warn!(%who, "ordnance never recharged: refund overdue, sent by the safety net");
                ready.push((who, out(To::Sender, COOLDOWN_REFUND, &cool)));
            }
        }
        let late: Vec<SocketAddr> = self.tac_running.iter().filter(|(_, (t, _))| *t <= now).map(|(a, _)| *a).collect();
        for who in late {
            let Some((_, slot)) = self.tac_running.remove(&who) else { continue };
            tracing::warn!(%who, "tac mod never recharged: cooldown overdue, sent by the safety net");
            self.phased.remove(&who);
            let b = [slot.as_slice(), &[0; 4]].concat();
            ready.push((who, out(To::All, ABILITY_COOLDOWN, &b)));
            ready.push((who, out(To::Sender, ABILITY_COOLDOWN, &b)));
            ready.push((who, out(To::All, ABILITY_ACTIVATE, &b)));
        }
    }

    /// A shot (`GameFireGun`: origin at 20, velocity at 36, damage at 48) at
    /// the DRONEs out: the nearest one the shot's line passes within
    /// `DRONE_HIT_RADIUS` of, with no wall between, takes the shot's damage,
    /// if the shooter may hurt it (its thrower's opponents, or the thrower).
    /// Clients often report no hit on a drone, so the server decides.
    fn shot_at_drones(&mut self, from: SocketAddr, d: &[u8], now: Instant) -> Vec<Out> {
        let (origin, vel) = (f3(d, 20), f3(d, 36));
        let damage = f32::from_le_bytes(d[48..52].try_into().expect("4"));
        let speed = (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt();
        if speed < 1e-3 || damage <= 0.0 || !damage.is_finite() {
            return Vec::new();
        }
        let dir = vel.map(|c| c / speed);
        let mut best: Option<([u8; 8], f32)> = None;
        for (mag, m) in &self.mags {
            if m.item != ITEM_DRONE || !m.ending || self.area_health.get(mag).is_some_and(|h| *h <= 0.0) || !self.may_hurt(Some(from), m.thrower) {
                continue;
            }
            let Some(&p) = self.thrown_at.get(&m.thrower) else { continue };
            let rel = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
            let t = rel[0] * dir[0] + rel[1] * dir[1] + rel[2] * dir[2];
            if !(0.0..=DRONE_SHOT_RANGE).contains(&t) {
                continue;
            }
            let near = [origin[0] + dir[0] * t, origin[1] + dir[1] * t, origin[2] + dir[2] * t];
            if dist(near, p) > DRONE_HIT_RADIUS || best.is_some_and(|(_, bt)| bt <= t) || self.line_blocked(origin, p, BLAST_SKIP) {
                continue;
            }
            best = Some((*mag, t));
        }
        let Some((mag, _)) = best else { return Vec::new() };
        let health = self.area_health.entry(mag).or_insert(ordnance_health(ITEM_DRONE));
        *health = (*health - damage).max(0.0);
        let left = *health;
        *self.drone_rays.entry((mag, from)).or_default() += 1;
        info!(%from, mag = %hex8(mag), damage, left, "DRONE shot");
        let mut v = vec![self.deal(mag, left)];
        if left <= 0.0 {
            v.extend(self.drone_down(mag, now));
        }
        v
    }

    /// Whether a blast at `at` reaches the player whose head is at `head`: a
    /// clear line to the head or to the body below it.
    fn reaches(&self, at: [f32; 3], head: [f32; 3]) -> bool {
        let body = [head[0], head[1] - BLAST_BODY_BELOW, head[2]];
        !self.line_blocked(at, head, BLAST_SKIP) || !self.line_blocked(at, body, BLAST_SKIP)
    }

    /// `reaches`; with `through_walls` (the DOT FIELD) only a standing ENERGY
    /// BARRIER stops it: it goes through the level's walls and barriers.
    fn reaches_in(&self, at: [f32; 3], head: [f32; 3], through_walls: bool) -> bool {
        if !through_walls {
            return self.reaches(at, head);
        }
        let body = [head[0], head[1] - BLAST_BODY_BELOW, head[2]];
        !self.tac_wall_blocked(at, head, BLAST_SKIP) || !self.tac_wall_blocked(at, body, BLAST_SKIP)
    }

    /// `line_blocked` with `skip` `BLAST_SKIP`, or with `through_walls` only
    /// a standing ENERGY BARRIER.
    fn blocked_in(&self, a: [f32; 3], b: [f32; 3], through_walls: bool) -> bool {
        if through_walls { self.tac_wall_blocked(a, b, BLAST_SKIP) } else { self.line_blocked(a, b, BLAST_SKIP) }
    }

    /// Whether a standing ENERGY BARRIER wall stands between `a` and `b`.
    fn tac_wall_blocked(&self, a: [f32; 3], b: [f32; 3], skip: f32) -> bool {
        self.walls.values().any(|(w, _)| w.crosses(a, b, skip))
    }

    /// Whether the level's own walls stand between `a` and `b`.
    fn level_blocked(&self, a: [f32; 3], b: [f32; 3], skip: f32) -> bool {
        super::walls::of(u64::from_le_bytes(self.space)).is_some_and(|w| w.blocked(a, b, skip))
    }

    /// Whether level geometry or a standing barrier stands between `a` and `b`
    /// (the first `skip` metres left out: the surface a blast sits on).
    fn line_blocked(&self, a: [f32; 3], b: [f32; 3], skip: f32) -> bool {
        if self.level_blocked(a, b, skip) {
            return true;
        }
        self.barriers.iter().any(|bar| {
            let mut target = [0u8; 8];
            target[..2].copy_from_slice(&bar.pool.to_le_bytes());
            target[4] = 1;
            !self.area_health.get(&target).is_some_and(|h| *h <= 0.0) && bar.crosses(a, b, skip)
        }) || self.tac_wall_blocked(a, b, skip)
    }

    /// Whether PHASE SHIFT is on for this player now.
    fn phased(&self, who: &SocketAddr) -> bool {
        self.phased.get(who).is_some_and(|t| Instant::now() < *t)
    }

    /// PHASE SHIFT's immunities (`ability_properties.wraith`): `dmg_imm`
    /// (shots), `exp_imm` (explosions, ordnance), `stun_imm`; and
    /// `disable_objectives` keeps its user off the objective.
    pub fn immune(&self, who: &SocketAddr, what: &str) -> bool {
        self.phased(who) && weapons().pointer(&format!("/ability_properties/wraith/{what}")).and_then(Value::as_bool).unwrap_or(true)
    }

    /// The gear tables in use: a match's (EchoCombat's) or the lobby's.
    fn weapons_table(&self) -> &'static Value {
        if self.in_match { combat_weapons() } else { weapons() }
    }

    /// A stun for `victim` (a player id, else the lobby's training dummy).
    /// A match: native's `SR15StunPlayerMsg` with the gear's stun length.
    /// The lobby: `SR15StunMsg` (`[space][slot | generation << 16]`, the
    /// dummy 0x0c), as native's lobby sent it.
    fn stun_out(&self, space: &[u8; 8], victim: [u8; 8], player: bool, secs: f32) -> Out {
        if self.in_match
            && player
            && let Some((slot, _)) = id_handle(victim)
        {
            let mut b = u32::from(slot).to_le_bytes().to_vec();
            b.extend_from_slice(&1u32.to_le_bytes());
            b.extend_from_slice(&secs.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes());
            return out(To::All, STUN_PLAYER_MSG, &b);
        }
        let target: u64 = if player { u64::from(id_handle(victim).map_or(0, |(s, g)| (u32::from(g) << 16) | u32::from(s))) } else { 0x0c };
        out(To::All, STUN_MSG, &[space.as_slice(), &target.to_le_bytes()].concat())
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
        self.observe_physics_at(from, d, Instant::now());
    }

    fn observe_physics_at(&mut self, from: SocketAddr, d: &[u8], now: Instant) {
        if d.len() != 64 {
            return;
        }
        let pos = transform(d);
        if d[4..8] == BODY_OBJECT {
            if self.in_combat(&from) {
                self.checks.moved(from, pos, now);
                let h = self.history.entry(from).or_default();
                h.push_back((now, pos));
                while h.front().is_some_and(|(t, _)| now.saturating_duration_since(*t) > HISTORY) {
                    h.pop_front();
                }
            }
            self.bodies.insert(from, (u16::from_le_bytes([d[0], d[1]]), pos));
        } else if d[4..8] == self.space[..4] {
            if let Some(last) = self.thrown_at.insert(from, pos) {
                if dist(last, pos) > STILL {
                    self.thrown_from.insert(from, (last, false));
                } else if let Some(f) = self.thrown_from.get_mut(&from) {
                    f.1 = true;
                }
            }
        }
    }

    /// A player is gone: what the others must be told (their stations freed).
    pub fn forget(&mut self, who: SocketAddr) -> Vec<Out> {
        self.checks.forget(who);
        self.heat.remove(&who);
        let outs = match self.station_slot(&who) {
            // Native (Dyson server recording): a player leaving a match gets
            // the same, from the server, at once.
            Some(slot) if self.in_match || self.inside.contains(&who) => self.free_stations(slot),
            _ => Vec::new(),
        };
        self.slots.remove(&who);
        self.bodies.remove(&who);
        self.history.remove(&who);
        self.pings.remove(&who);
        self.thrown_at.remove(&who);
        self.spawned_at.remove(&who);
        self.owed_refund.remove(&who);
        self.tac_running.remove(&who);
        self.thrown_from.remove(&who);
        self.bubble_owner.remove(&who);
        self.lives.remove(&who);
        self.drains.retain(|d| d.who != who);
        self.matrices.retain(|m| m.who != who);
        self.oc_heals.retain(|h| h.who != who);
        self.oc_linked.retain(|t, u| *t != who && *u != who);
        self.mines.retain(|m| m.thrower != who);
        self.tactical.remove(&who);
        self.last_wall.remove(&who);
        self.point_tokens.remove(&who);
        self.phased.remove(&who);
        self.inside.retain(|a| *a != who);
        self.left.remove(&who);
        self.leaving.remove(&who);
        self.boosted.remove(&who);
        self.links.remove(&who);
        self.oc_use.remove(&who);
        self.link_use.retain(|t, (u, _)| *t != who && *u != who);
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
    /// Lag compensated: each player where they are now or where the thrower's
    /// game showed them, whichever the blast reaches nearer.
    fn victims(&self, at: [f32; 3], radius: f32, by: Option<SocketAddr>) -> Vec<Hit> {
        self.victims_lagged(at, radius, by, false, true)
    }

    /// A player's round trip as the session measured it.
    pub fn set_ping(&mut self, who: SocketAddr, rtt: Duration) {
        self.pings.insert(who, rtt);
    }

    /// Where `victim` was on `by`'s screen: their position one round trip of
    /// `by`'s (at most `REWIND_MAX`) before their latest. A blast reported by
    /// a player was set off against what that player saw, which the victim's
    /// game sent here a round trip earlier.
    fn seen_by(&self, victim: SocketAddr, by: SocketAddr) -> Option<[f32; 3]> {
        let back = self.pings.get(&by)?.min(&REWIND_MAX);
        let h = self.history.get(&victim)?;
        let (last, _) = h.back()?;
        let when = last.checked_sub(*back)?;
        h.iter().rev().find(|(t, _)| *t <= when).or(h.front()).map(|(_, p)| *p)
    }

    /// `victims`; with `through_walls` (the DOT FIELD, the one effect that
    /// reaches through walls) only a standing ENERGY BARRIER shields.
    fn victims_in(&self, at: [f32; 3], radius: f32, by: Option<SocketAddr>, through_walls: bool) -> Vec<Hit> {
        self.victims_lagged(at, radius, by, through_walls, false)
    }

    /// `victims_in`; with `rewind`, each player is also tried where `by` saw them.
    fn victims_lagged(&self, at: [f32; 3], radius: f32, by: Option<SocketAddr>, through_walls: bool, rewind: bool) -> Vec<Hit> {
        let mut v = Vec::new();
        for (a, (slot, p)) in &self.bodies {
            // Only players in the combat area can be hit (no stun or damage in the lobby).
            if !self.in_combat(a) || !self.may_hurt(by, *a) || self.immune(a, "exp_imm") {
                continue;
            }
            // The tracked position is the head: a blast at the feet is measured
            // to the body below it, not to the head 1.5 m up.
            let then = if rewind { by.and_then(|b| self.seen_by(*a, b)) } else { None };
            let mut best: Option<([f32; 3], f32)> = None;
            for q in std::iter::once(*p).chain(then) {
                let d = body_dist(at, q);
                if d <= radius + BODY_RADIUS && best.is_none_or(|(_, bd)| d < bd) && self.reaches_in(at, q, through_walls) {
                    best = Some((q, d));
                }
            }
            if let Some((q, d)) = best {
                if q != *p && body_dist(at, *p) > radius + BODY_RADIUS {
                    info!(victim = %a, by = ?by, now_m = body_dist(at, *p), then_m = d, "blast hit where the thrower saw the victim (lag compensation)");
                }
                let id = self.id_of(a, *slot);
                v.push((id, [q[0], q[1] - BODY_DROP, q[2]], d, Some(*a), PLAYER_CODE));
            }
        }
        // The lobby's training dummies (none in a match).
        for (p, code) in DUMMIES.iter().filter(|_| !self.in_match) {
            let d = dist(at, *p);
            if d <= radius + DUMMY_RADIUS && !self.blocked_in(at, *p, through_walls) {
                v.push((NO_PLAYER, *p, d, None, *code));
            }
        }
        // A match's war room dummies.
        for (i, p) in WAR_ROOM_DUMMIES.iter().enumerate().filter(|_| self.in_match) {
            let d = dist(at, *p);
            if d <= radius + WAR_ROOM_DUMMY_RADIUS && !self.blocked_in(at, *p, through_walls) {
                v.push((NO_PLAYER, *p, d, None, WAR_ROOM_CODE | i as u32));
            }
        }
        v
    }

    /// Damage to a player: their new health to everyone, and `PlayerKilled` at 0.
    /// A player's slot, from their body physics.
    /// With their generation above it (`gen << 16 | slot`, the player handle
    /// health broadcasts and kills carry: a player who rejoined is generation 1).
    /// A shot's heat on its shooter's gun (the weapon told by its damage, as
    /// `multiplier` does; a COMET's charged damage is SCOUT's).
    fn heat_shot(&mut self, from: SocketAddr, damage: f32, now: Instant) {
        let w = weapons();
        let props = w.get("weapon_properties").and_then(Value::as_object);
        let weapon = props
            .and_then(|m| m.iter().find(|(_, v)| v.get("projectile_damage").and_then(Value::as_f64).is_some_and(|p| (p as f32 - damage).abs() < 0.01)).map(|(k, _)| k.clone()))
            .unwrap_or_else(|| "scout".to_owned());
        let wp = |k: &str, def: f64| w.pointer(&format!("/weapon_properties/{weapon}/{k}")).and_then(Value::as_f64).unwrap_or(def) as f32;
        let h = self.heat.entry(from).or_default();
        h.cooling = wp("heat_venting_rate", 50.0) / 100.0;
        h.delay = Duration::from_secs_f32(wp("heat_cooldown_delay", 1.0).clamp(0.0, 10.0));
        h.last_shot = Some(now);
        h.last_tick = now;
        if h.overheated.is_none() {
            h.level = (h.level + wp("heat_per_shot", 10.0) / 100.0).min(1.0);
            if h.level >= 1.0 {
                h.overheated = Some(now + HEAT_OVERHEAT_SHOWN);
            }
        }
    }

    /// Heat drains after each gun's cooldown delay; an overheat shows full
    /// for a moment, then 0 (native: 0.998, 0.947, then 0 within 0.45 s).
    fn cool_guns(&mut self, now: Instant) {
        for h in self.heat.values_mut() {
            let dt = now.saturating_duration_since(h.last_tick).as_secs_f32();
            h.last_tick = now;
            if let Some(t) = h.overheated {
                if now >= t {
                    h.overheated = None;
                    h.level = 0.0;
                }
            } else if h.last_shot.is_none_or(|s| now >= s + h.delay) {
                h.level = (h.level - h.cooling * dt).max(0.0);
            }
        }
    }

    /// The gun heat vars: `(key, value)` for every player with a slot that
    /// has a known key, value `[heat f32][0 u16][slot u16]` (native).
    pub fn heat_vars(&self) -> Vec<(u64, u64)> {
        self.heat
            .iter()
            .filter_map(|(who, h)| {
                let slot = self.station_slot(who)?;
                let key = *HEAT_KEYS.get(slot as usize)?;
                Some((key, u64::from(h.level.to_bits()) | (u64::from(slot as u16) << 48)))
            })
            .collect()
    }

    pub fn slot_of(&self, who: &SocketAddr) -> u32 {
        player_handle(&self.bodies, &self.slots, &self.gens, who)
    }

    /// The slot whose gear stations are a player's: the session's, else their
    /// body's; none if neither is known. (Falling back to 0 freed slot 0's
    /// stations when a player who never sent body physics left: whoever was
    /// slot 0 lost their gun.)
    fn station_slot(&self, who: &SocketAddr) -> Option<u32> {
        self.slots.get(who).copied().or_else(|| self.bodies.get(who).map(|b| u32::from(b.0)))
    }

    /// An arena throw faster than the game allows (logged only).
    pub fn flag_throw(&mut self, who: SocketAddr, speed: f32) {
        self.checks.flag(who, super::checks::Flag::ThrowSpeed, &format!("a disc thrown at {speed:.2} m/s (the game's most is 18.95)"));
    }

    /// A shot (`GameFireGun`, damage at 48, the gun's instance at 0) checked:
    /// its damage one some gun does, its gun under its fire rate. None: drop
    /// it (enforce mode). The damage capped at the most any gun does.
    fn check_shot(&mut self, from: SocketAddr, d: &[u8], now: Instant) -> Option<Vec<u8>> {
        use super::checks::{Flag, Mode, mode};
        let w = if self.in_match { combat_weapons() } else { weapons() };
        let damage = f32_at(d, 48);
        let in_rate = self.checks.fired(w, from, d[0], damage, now);
        let most = super::checks::most_damage(w, 0);
        let mut d = d.to_vec();
        if !damage.is_finite() || damage > most + 0.01 {
            self.checks.flag(from, Flag::Damage, &format!("a shot claiming {damage} damage (most {most})"));
            if mode() == Mode::Enforce {
                d[48..52].copy_from_slice(&most.to_le_bytes());
            }
        }
        (in_rate || mode() != Mode::Enforce).then_some(d)
    }

    /// A hit (`DamageableBulletCollision`: gun at 0, damage at 44) checked:
    /// damage over its gun's most is flagged, and capped in enforce mode.
    fn check_hit_damage(&mut self, from: SocketAddr, d: &[u8]) -> Vec<u8> {
        use super::checks::{Flag, Mode, mode};
        let mut d = d.to_vec();
        if mode() == Mode::Off {
            return d;
        }
        let w = if self.in_match { combat_weapons() } else { weapons() };
        let gun = u64::from_le_bytes(arr8(&d, 0));
        let most = super::checks::most_damage(w, gun);
        let base = f32_at(&d, 44);
        if !base.is_finite() || base > most + 0.01 {
            self.checks.flag(from, Flag::Damage, &format!("a hit claiming {base} damage (its gun's most {most})"));
            if mode() == Mode::Enforce {
                d[44..48].copy_from_slice(&most.to_le_bytes());
            }
        }
        d
    }

    /// A hit on `victim` at `at`: near their body (else flagged, and dropped
    /// in enforce mode), and no level wall between the shooter and it
    /// (flagged only). Whether to apply it.
    fn check_hit_position(&mut self, from: SocketAddr, victim: SocketAddr, at: [f32; 3], gun: u64) -> bool {
        use super::checks::{FAR_FROM_VICTIM, Flag, Mode, mode};
        if mode() == Mode::Off {
            return true;
        }
        let w = if self.in_match { combat_weapons() } else { weapons() };
        // A drone's shots come from the drone's own aim, not the thrower's:
        // only hand guns are checked.
        if !super::checks::hand_gun(w, gun) {
            return true;
        }
        let Some(&(_, head)) = self.bodies.get(&victim) else { return true };
        // Nearest to where the victim was over the last second.
        let off = self.checks.trail(victim).map(|p| body_dist(at, p)).fold(body_dist(at, head), f32::min);
        if off > FAR_FROM_VICTIM {
            self.checks.flag(from, Flag::FarFromVictim, &format!("a hit {off:.1} m from {victim}'s body"));
            return mode() != Mode::Enforce;
        }
        // From the head and the chest both: a gun held out past a corner
        // sees round it.
        if let Some(&(_, eye)) = self.bodies.get(&from)
            && self.level_blocked(eye, at, HIT_WALL_SKIP)
            && self.level_blocked([eye[0], eye[1] - 0.45, eye[2]], at, HIT_WALL_SKIP)
        {
            self.checks.flag(from, Flag::ThroughWall, &format!("a hit on {victim} with a level wall in between"));
        }
        true
    }

    /// The player a damageable id names (see `player_id`), in the combat area:
    /// the one with that slot and generation, else (generation unknown) the slot.
    fn player_by_id(&self, id: [u8; 8]) -> Option<SocketAddr> {
        let (slot, generation) = id_handle(id)?;
        let here = |a: &&SocketAddr| self.in_combat(a);
        let at_slot = || self.bodies.iter().filter(|(_, (s, _))| *s == slot).map(|(a, _)| a).filter(here);
        at_slot().find(|a| self.gens.get(*a).copied().unwrap_or(0) == generation).or_else(|| at_slot().next()).copied()
    }

    /// A player's damageable id (see `player_id`).
    fn id_of(&self, who: &SocketAddr, slot: u16) -> [u8; 8] {
        player_id(slot, self.gens.get(who).copied().unwrap_or(0))
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
        let Some(target) = self.player_by_id(id).or_else(|| id_handle(id).is_some().then_some(from)) else {
            return;
        };
        let m = marker as u32;
        if (6_000_000..7_000_000).contains(&m) {
            info!(%from, %target, "OVERCHARGE link broken");
            self.links.remove(&target);
            self.oc_linked.remove(&target);
            self.boosted.remove(&target);
            return;
        }
        let link = (2_000_000..3_000_000).contains(&m) || (4_000_000..5_000_000).contains(&m);
        // This use of the user's: its start (a use marker, else the first
        // link once the last use is over).
        let fresh = |t: &Instant| now.saturating_duration_since(*t) > overcharge_duration() + Duration::from_secs(1);
        if self.oc_use.get(&from).is_none_or(fresh) {
            self.oc_use.insert(from, now);
        }
        let use_start = self.oc_use.get(&from).copied().unwrap_or(now);
        if link && target != from {
            // Cheat death runs from the first link of this use, once: later
            // links of the same use don't start it again (they did, so a
            // teammate kept linked could never die).
            let ends = if self.link_use.get(&target) == Some(&(from, use_start)) {
                self.links.get(&target).copied().unwrap_or(now)
            } else {
                self.link_use.insert(target, (from, use_start));
                now + cheat_death()
            };
            info!(%from, %target, cheat_death = ?ends.saturating_duration_since(now), "OVERCHARGE linked: damage cut, cannot die");
            self.links.insert(target, ends);
            self.oc_linked.insert(target, from);
        } else {
            info!(%from, "OVERCHARGE use marker");
        }
        self.boosted.insert(target, now + overcharge_duration());
    }

    /// What a hit on `who` really costs after OVERCHARGE: 25% off while boosted
    /// or linked.
    fn after_overcharge(&self, who: SocketAddr, amount: f32, now: Instant) -> f32 {
        if self.boosted.get(&who).is_some_and(|t| *t > now) { amount * (1.0 - OVERCHARGE_DR) } else { amount }
    }

    /// OVERCHARGE's cheat death: a linked teammate keeps at least 1 health.
    fn cheats_death(&self, who: SocketAddr, now: Instant) -> bool {
        self.links.get(&who).is_some_and(|t| *t > now)
    }

    /// A new round: every barrier back at full health (native Dyson, between
    /// its rounds: the respawn event for all, `DealDamage` max for all, the
    /// event for all again, in one frame).
    pub fn restore_barriers(&mut self) -> Vec<(SocketAddr, Out)> {
        self.respawns.clear();
        let anyone = SocketAddr::from(([0, 0, 0, 0], 0));
        let mut v = Vec::new();
        // Raised pillars come down (they have no break event of their own).
        for (_, _, b) in std::mem::take(&mut self.pillars_up) {
            let ev = pillar_event(b.handle);
            v.push((anyone, out(To::All, COMPONENT_EVENT, &ev)));
            v.push((anyone, out(To::All, COMPONENT_EVENT, &ev)));
        }
        let pillar = |b: &Barrier| u64::from_le_bytes(b.space) == super::msg::LEVEL_FISSION_CLIMAX;
        let (pillars, all): (Vec<Barrier>, Vec<Barrier>) = self.barriers.iter().partition(|b| pillar(b));
        let target = |b: &Barrier| {
            let mut t = [0u8; 8];
            t[..2].copy_from_slice(&b.pool.to_le_bytes());
            t[4] = 1;
            t
        };
        for b in &all {
            v.push((anyone, out(To::All, COMPONENT_EVENT, &barrier_event(b.handle))));
        }
        for b in all.iter().chain(&pillars) {
            self.area_health.insert(target(b), b.max);
            v.push((anyone, self.deal_in(b.space, target(b), b.max)));
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
            .filter_map(|(t, _)| self.barrier(*t).filter(|b| b.space == self.space).map(|b| b.handle))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    fn barrier(&self, target: [u8; 8]) -> Option<Barrier> {
        self.barrier_in(&self.space, target)
    }

    /// The barrier at `target`'s pool slot in the space starting `space`.
    fn barrier_in(&self, space: &[u8], target: [u8; 8]) -> Option<Barrier> {
        let pool = u16::from_le_bytes([target[0], target[1]]);
        self.barriers.iter().find(|b| b.pool == pool && b.space[..space.len().min(8)] == space[..space.len().min(8)]).copied()
    }

    fn deal(&self, target: [u8; 8], health: f32) -> Out {
        self.deal_in(self.space, target, health)
    }

    /// `deal` in another space (a sublevel's barriers, a wall's).
    fn deal_in(&self, space: [u8; 8], target: [u8; 8], health: f32) -> Out {
        let mut b = space.to_vec();
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
    fn hit_barrier(&mut self, from: SocketAddr, known: Option<Barrier>, target: [u8; 8], amount: f32, now: Instant) -> Vec<Out> {
        if let Some(b) = known.filter(|b| u64::from_le_bytes(b.space) == super::msg::LEVEL_FISSION_CLIMAX) {
            return self.hit_pillar(from, b, target, amount, now);
        }
        let space = known.map_or(self.space, |b| b.space);
        let max = known.map_or(BARRIER_HEALTH, |b| b.max);
        let health = self.area_health.entry(target).or_insert(max);
        if *health <= 0.0 || amount <= 0.0 {
            return Vec::new();
        }
        *health = (*health - amount).max(0.0);
        let left = *health;
        let v = vec![self.deal_in(space, target, left)];
        if left > 0.0 {
            info!(%from, barrier = %hex8(target), amount, left, "barrier hit");
            return v;
        }
        info!(%from, barrier = %hex8(target), "barrier destroyed");
        if let Some(b) = known {
            let ev = barrier_event(b.handle);
            let zero = self.deal_in(space, target, 0.0);
            self.later.push((now + BREAK_EVENT_AFTER, from, out(To::All, COMPONENT_EVENT, &ev)));
            self.later.push((now + BREAK_EVENT_AFTER, from, zero));
            self.later.push((now + BREAK_ECHO_AFTER, from, out(To::All, COMPONENT_EVENT, &ev)));
            if b.respawn > 0.0 {
                self.respawns.push((now + Duration::from_secs_f32(b.respawn), target, b));
            }
        }
        v
    }

    /// A hit on one of Fission's end-game pillars (`PILLAR_EVENT`).
    fn hit_pillar(&mut self, from: SocketAddr, b: Barrier, target: [u8; 8], amount: f32, now: Instant) -> Vec<Out> {
        if amount <= 0.0 {
            return Vec::new();
        }
        let health = self.area_health.entry(target).or_insert(b.max);
        *health = (*health - amount).max(0.0);
        let left = *health;
        let mut v = vec![self.deal_in(b.space, target, left)];
        if left <= b.max - PILLAR_RAISE_DAMAGE && !self.pillars_up.iter().any(|(_, t, _)| *t == target) {
            info!(%from, pillar = b.handle, left, "pillar raised");
            let ev = pillar_event(b.handle);
            v.push(out(To::All, COMPONENT_EVENT, &ev));
            v.push(out(To::All, COMPONENT_EVENT, &ev));
            self.pillars_up.push((now + PILLAR_UP, target, b));
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
            .filter_map(|b| b.nearest(at).filter(|(p, d)| *d <= reach && !self.level_blocked(at, *p, BLAST_SKIP)).map(|(p, d)| (*b, p, d)))
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
            // Fission's climax barriers (their own space) take a blast as
            // anything else does: native, a METEOR's blast took 40 off one,
            // not the barrier multiplier's 80.
            let amount = damage(d) * if b.space == self.space { scale } else { 1.0 };
            if amount <= 0.0 {
                continue;
            }
            info!(%from, barrier = b.handle, distance = d, amount, "explosion reached a barrier");
            v.extend(self.hit_barrier(from, Some(b), target, amount, now));
            v.push(out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(&b.space, kind, p, NO_PLAYER, id)));
        }
        // ENERGY BARRIER walls (native Fission: the wall's DealDamage, then
        // `ExplosionPlayerDamage` at the wall's place with the wall as victim).
        let mut walls: Vec<([u8; 8], Barrier, f32)> = self
            .walls
            .iter()
            .filter_map(|(a, (w, _))| w.nearest(at).filter(|(p, d)| *d <= reach && !self.level_blocked(at, *p, BLAST_SKIP)).map(|(_, d)| (*a, *w, d)))
            .collect();
        walls.sort_by(|a, b| a.2.total_cmp(&b.2));
        for (actor, w, d) in walls {
            let amount = damage(d) * scale;
            if amount <= 0.0 {
                continue;
            }
            info!(%from, wall = %hex8(actor), distance = d, amount, "explosion reached an ENERGY BARRIER");
            v.extend(self.hit_wall(from, actor, amount, now));
            v.push(out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(&w.space, kind, w.pos, actor, id)));
        }
        v
    }

    /// A wall placed (a 96-byte `PooledActorCreate` of the wall template:
    /// `[rotation 16][place 12][scale 4][template 8][space 8][actor 8]..`):
    /// native's health, and native's removal 18.04 s later.
    fn add_wall(&mut self, from: SocketAddr, d: &[u8], now: Instant) {
        if d.len() != 96 || d[32..40] != WALL_TEMPLATE {
            return;
        }
        let (space, actor) = (arr8(d, 40), arr8(d, 48));
        let q = [f32_at(d, 0), f32_at(d, 4), f32_at(d, 8), f32_at(d, 12)];
        let norm = q.iter().map(|x| x * x).sum::<f32>().sqrt();
        let rot = if norm > 0.5 && norm.is_finite() { q.map(|x| x / norm) } else { [0.0, 0.0, 0.0, 1.0] };
        let bar = Barrier { space, pool: 0, handle: 0, max: WALL_HEALTH, respawn: 0.0, pos: f3(d, 16), rot, half: WALL_BOX, off_z: 0.0 };
        info!(%from, wall = %hex8(actor), "ENERGY BARRIER wall up");
        self.walls.insert(actor, (bar, from));
        self.area_health.insert(actor, WALL_HEALTH);
        self.later.push((now + WALL_LIFETIME, from, out(To::All, ACTOR_DESTROY, &d[40..56])));
    }

    /// A hit on a standing wall: its health down for everyone, and at 0 it
    /// is gone (removed now rather than at the end of its lifetime).
    fn hit_wall(&mut self, from: SocketAddr, actor: [u8; 8], amount: f32, now: Instant) -> Vec<Out> {
        let Some((w, _)) = self.walls.get(&actor).copied() else { return Vec::new() };
        let health = self.area_health.entry(actor).or_insert(WALL_HEALTH);
        if *health <= 0.0 || amount <= 0.0 {
            return Vec::new();
        }
        *health = (*health - amount).max(0.0);
        let left = *health;
        let v = vec![self.deal_in(w.space, actor, left)];
        if left > 0.0 {
            info!(%from, wall = %hex8(actor), amount, left, "ENERGY BARRIER hit");
            return v;
        }
        info!(%from, wall = %hex8(actor), "ENERGY BARRIER broken");
        let body = [w.space, actor].concat();
        self.later.retain(|(_, _, o)| !(o.ty == ACTOR_DESTROY && o.body == body));
        self.walls.remove(&actor);
        self.later.push((now + BREAK_EVENT_AFTER, from, out(To::All, ACTOR_DESTROY, &body)));
        v
    }

    fn hurt(&mut self, who: SocketAddr, amount: f32, kind: u32, pos: [f32; 3], cause: [u8; 8], by: Option<SocketAddr>) -> Vec<Out> {
        let dir = self.shot_dir.take().unwrap_or([0.0; 3]);
        // The id is the same for every player; a client's own messages only confirm it.
        let owner = self.bubble_owner.get(&who).copied().unwrap_or(DAMAGEABLE);
        let slot = self.slot_of(&who);
        let killer = by.map_or(slot, |a| self.slot_of(&a));
        if !self.may_hurt(by, who) {
            return Vec::new();
        }
        let blast = matches!(kind, HURT_EXPLOSION | HURT_ORDNANCE);
        if self.immune(&who, if blast { "exp_imm" } else { "dmg_imm" }) {
            return Vec::new();
        }
        let now = Instant::now();
        let amount = self.after_overcharge(who, amount, now);
        let cheat = self.cheats_death(who, now);
        let life = self.lives.entry(who).or_default();
        if life.dead || amount <= 0.0 {
            return Vec::new();
        }
        let (off_shield, mut off_core) = split_hit(life.health, life.core, amount);
        if cheat {
            off_core = off_core.min((life.core - 1.0).max(0.0));
        }
        life.health = (life.health - off_shield).max(0.0);
        life.core = (life.core - off_core).max(0.0);
        let landed = off_shield + off_core;
        life.hurt = Some((kind, pos, cause, dir));
        life.lost_at = Some(now);
        life.regen_at = None;
        let mut v = Vec::new();
        if life.core <= 0.0 {
            life.health = 0.0;
            life.dead = true;
            life.died_at = Some(Instant::now());
            info!(victim = %who, victim_slot = format_args!("{slot:#x}"), killer = ?by, killer_slot = format_args!("{killer:#x}"), cause = %hex8(cause), "player killed");
            // Shots pass their body part as the kind (explosions 4, ordnance 2).
            self.credits.push(Credit::Kill { by: by.filter(|a| *a != who), victim: who, head: kind == PART_HEAD && by.is_some() });
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
        life.core = FULL_CORE;
        life.dead = false;
        life.died_at = None;
        let life = *life;
        if !self.inside.contains(&from) {
            self.inside.push(from);
        }
        info!(%from, "combat area entered: tac mod and health reset");
        self.tac_running.remove(&from);
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
        let Some(slot) = self.station_slot(&from) else {
            info!(%from, "combat area left: slot unknown, no gear stations freed");
            return Vec::new();
        };
        info!(%from, slot, still_inside = self.inside.len(), "combat area left: gear stations freed");
        self.free_stations(slot)
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
                // In a match native clears only the low two bits of what is
                // there: fe goes to fc, ee to ec (Dyson, a leaver and the match
                // end). The lobby's recordings always say fc.
                let mut v = [0xfc, 0xff, 0xff, 0xff];
                if let Some(b) = self.bits.get_mut(i as usize) {
                    if self.in_match {
                        v[0] = b[0] & 0xfc;
                    }
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
        self.watch_recharge(from, ty, d, now);
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
                self.oc_heals.retain(|h| h.who != from);
                self.oc_linked.retain(|_, u| *u != from);
                self.oc_heals.push(OcHeal { who: from, until: now + overcharge_duration(), last: now, given: HashMap::new() });
                info!(%from, ticks = left, "OVERCHARGE on");
                vec![out(To::Others, ty, d)]
            }
            FIRE_GUN => {
                self.using_gear(from, now);
                if d.len() == 64 && self.in_combat(&from) {
                    self.heat_shot(from, f32::from_le_bytes(d[48..52].try_into().expect("4")), now);
                }
                let checked;
                let d = if d.len() == 64 && self.in_combat(&from) {
                    match self.check_shot(from, d, now) {
                        Some(c) => {
                            checked = c;
                            checked.as_slice()
                        }
                        None => return Vec::new(),
                    }
                } else {
                    d
                };
                // Remember which gun fired each explosive shot (by its explosion id).
                if d.len() == 64 {
                    if self.shots.len() > 256 {
                        self.shots.clear();
                    }
                    self.shots.insert(arr8(d, 56), (d[0], f32::from_le_bytes(d[48..52].try_into().expect("4"))));
                }
                let mut v = vec![out(To::Others, ty, d)];
                if d.len() == 64 {
                    v.extend(self.shot_at_drones(from, d, now));
                }
                v
            }
            HEAL_BUBBLE_ACTIVATE if d.len() >= 16 && arr8(d, 8) == REPAIR_BUBBLE => {
                // Native heals everyone in it a little each frame (114 over
                // 12 s: about 9.5 a second) and broadcasts their health.
                let secs = heal_setting("heal_duration", 12.0).max(0.1);
                self.matrices.retain(|m| m.who != from);
                self.matrices.push(Matrix { who: from, until: now + Duration::from_secs_f32(secs), last: now });
                info!(%from, secs, "REPAIR MATRIX on");
                vec![out(To::Others, ty, d)]
            }
            HEAL_BUBBLE_ACTIVATE => vec![out(To::Others, ty, d)],
            PLAYER_RESPAWN if d.len() >= 8 && self.in_match && !self.lives.get(&from).is_some_and(|l| l.dead) => {
                // In a match the server respawns everyone (`MATCH_RESPAWN`
                // after the kill). A lagging client's own ask came in after
                // that and went out as a second respawn: the others' games
                // then left its body where it respawned while it played on.
                info!(%from, "respawn ask from a player who is not dead: dropped (the server respawned them)");
                Vec::new()
            }
            PLAYER_RESPAWN if d.len() >= 8 => {
                self.bubble_owner.insert(from, arr8(d, 0));
                let life = self.lives.entry(from).or_default();
                life.health = FULL_HEALTH;
                life.core = FULL_CORE;
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
                self.mines.retain(|m| m.mag != arr8(d, 8));
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
            // `[slot u32][on u32]`, for every tac mod: PHASE SHIFT's user is
            // phased from on to off (or its duration, if off never comes).
            ABILITY_ACTIVATE if d.len() == 8 && self.tactical.get(&from) == Some(&TAC_WRAITH) => {
                if d[4..8] != [0; 4] {
                    let secs = wraith_setting("duration", 10.0).max(0.1) + WRAITH_SLACK;
                    self.phased.insert(from, now + Duration::from_secs_f32(secs));
                    info!(%from, "PHASE SHIFT on");
                } else if self.phased.remove(&from).is_some() {
                    info!(%from, "PHASE SHIFT off");
                }
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
                let checked = self.check_hit_damage(from, d);
                let d = checked.as_slice();
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
                    let shot = self.player_by_id(target).filter(|a| *a != from);
                    // No marker on a PHASE SHIFT user: the shot does nothing.
                    let through = shot.is_some_and(|a| self.immune(&a, "dmg_imm"));
                    let mut v = if mark && !through { vec![out(To::Sender, TRACKED_DAMAGE, &marker)] } else { Vec::new() };
                    if let Some(victim) = shot
                        && !self.check_hit_position(from, victim, at, u64::from_le_bytes(arr8(d, 0)))
                    {
                        return v;
                    }
                    match shot {
                        Some(victim) => {
                            info!(%from, %victim, amount, part, "shot hit a player");
                            self.shot_dir = Some(f3(d, 8));
                            v.extend(self.hurt(victim, amount, part as u32, at, arr8(d, 0), Some(from)));
                        }
                        None => debug!(%from, target = %hex8(target), "shot hit a player not in the area"),
                    }
                    return v;
                }
                let index = u32::from(u16::from_le_bytes([target[0], target[1]]));
                let dummy = DUMMIES.iter().any(|(_, c)| *c == index);
                if target[4..8] == HANDLE_KIND && !dummy {
                    // The level's barriers, or a sublevel's (Fission's climax).
                    let known = self.barrier_in(&d[40..44], target);
                    if known.is_some() || (d[40..44] == space[..4] && part == PART_BARRIER) {
                        return self.hit_barrier(from, known, target, amount, now);
                    }
                }
                if self.walls.contains_key(&target) {
                    return self.hit_wall(from, target, amount, now);
                }
                if d[40..44] == space[..4] && self.mags.contains_key(&target) {
                    // An ARC MINE or DRONE: only its thrower's opponents (or
                    // the thrower) shoot it down.
                    let (item, thrower) = self.mags.get(&target).map(|m| (m.item, m.thrower)).expect("known");
                    if matches!(item, ITEM_ARC | ITEM_DRONE) && !self.may_hurt(Some(from), thrower) {
                        return Vec::new();
                    }
                    if item == ITEM_DRONE
                        && let Some(n) = self.drone_rays.get_mut(&(target, from)).filter(|n| **n > 0)
                    {
                        // Already dealt from the shot itself.
                        *n -= 1;
                        return Vec::new();
                    }
                    if let Some(i) = self.mines.iter().position(|m| m.mag == target) {
                        info!(%from, mag = %hex8(target), "ARC MINE shot");
                        self.mines[i].shot_at.get_or_insert(now);
                    }
                    let health = self.area_health.entry(target).or_insert(ordnance_health(item));
                    let was = *health;
                    *health = (*health - amount).max(0.0);
                    let left = *health;
                    let mut v = vec![self.deal(target, left)];
                    if item == ITEM_DRONE && left <= 0.0 && was > 0.0 {
                        v.extend(self.drone_down(target, now));
                    }
                    return v;
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
                // Splash that deals damage (not an unknown blast's 0) takes mines down.
                if damage(0.0) > 0.0 {
                    v.extend(self.blast_mines(from, at, radius, &*damage, now));
                }
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
                let at_spawn = self.thrown_at.get(&from).copied();
                self.spawned_at.insert(from, now);
                self.mags.insert(arr8(d, 32), Mag { owner: arr8(d, 0), item: arr8(d, 24), thrower: from, ending: false, at_spawn });
                vec![out(To::Others, ty, d)]
            }
            MAGAZINE_STATE if d.len() == 24 => {
                let (id, state) = (arr8(d, 8), arr8(d, 16));
                info!(%from, mag = %hex8(id), state = %hex8(state), known = self.mags.contains_key(&id), "ordnance state");
                let mut v = vec![out(To::Others, ty, d)];
                // ARC MINE: let go of, it sticks and arms; picked back up or
                // used up (shot, or the client's own say), it is gone.
                if state == MAG_RELEASED
                    && let Some(m) = self.mags.get(&id).filter(|m| !m.ending && m.item == ITEM_ARC)
                {
                    let armed_at = now + Duration::from_secs_f32(arc_setting("arming_length", 3.5).max(0.0));
                    let mine = Mine { mag: id, owner: m.owner, head: arr8(d, 0), thrower: m.thrower, at: None, armed_at, fire_at: None, shot_at: None };
                    self.mines.retain(|x| x.mag != id);
                    self.mines.push(mine);
                    info!(%from, mag = %hex8(id), "ARC MINE placed");
                } else if state == MAG_HELD || state == MAG_USED {
                    self.mines.retain(|x| x.mag != id);
                }
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
                        // Stuck to a wall, it does not hurt the far side.
                        if let Some(&(from_pos, true)) = self.thrown_from.get(&thrower) {
                            let before = hits.len();
                            hits.retain(|h| !behind_surface(at, from_pos, h.3.and_then(|a| self.bodies.get(&a)).map_or(h.1, |b| b.1)));
                            if hits.len() != before {
                                info!(%thrower, behind = before - hits.len(), "DETONATOR: victims behind the surface it stuck to, not hit");
                                o = if hits.is_empty() {
                                    vec![out(To::All, EXPLOSION_NOTHING_HIT, &[space.as_slice(), &EX_DET, &id].concat())]
                                } else {
                                    Vec::new()
                                };
                            }
                        }
                        hits.sort_by(|x, y| x.2.total_cmp(&y.2));
                        if !hits.is_empty() {
                            o.truncate(0);
                            o.push(out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(&space, EX_DET, hits[0].1, hits[0].0, id)));
                        }
                        // Barriers: native's dets broke Dyson's 101s outright
                        // (65 x 2.15) and left one 2 m off at 4 (97 taken).
                        let walls = self.splash_barriers(thrower, EX_DET, id, at, radius.max(DET_BARRIER_REACH), "det", &det_barrier_falloff, now);
                        v.extend(self.blast_mines(thrower, at, radius.max(DET_BARRIER_REACH), &det_barrier_falloff, now));
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
                        let radius = if dot { g("radius", 2.5) } else { stun_reach(Duration::MAX) };
                        let kind = if dot { EX_DOT } else { EX_STUN };
                        self.fields.push(Field { kind, mag: id, at, thrower, until: now + lifetime(item), next: now, every, radius, start: now, hit: Vec::new(), moved: now, tracked: false });
                    }
                }
                if let Some(m) = self.mags.get_mut(&id).filter(|m| !m.ending) {
                    // INSTANT REPAIR thrown (not just let go of) still pops on
                    // its client's own used state ~1 s later. Ended here, that
                    // used state was ignored and nobody was ever healed.
                    if state == MAG_THROWN && m.item != ITEM_BURST {
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
                        let (item, thrower) = (m.item, m.thrower);
                        if item == ITEM_BURST {
                            // INSTANT REPAIR: its thrower's client says when it
                            // pops (native: held, released, then used ~1 s later).
                            // Where: this one's own thrown position if it sent
                            // any, else its thrower (an older grenade's spot
                            // healed nobody near the player).
                            let own = self.thrown_at.get(&thrower).copied().filter(|p| self.mags.get(&id).is_none_or(|m| m.at_spawn != Some(*p)));
                            let at = own.or_else(|| self.bodies.get(&thrower).map(|b| b.1));
                            if let Some(at) = at {
                                v.extend(self.burst_heal(thrower, at));
                            }
                        }
                        let Some(m) = self.mags.get(&id) else { return v };
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

    /// INSTANT REPAIR popping at `at`: `grenade_properties.burst.heal_amount`
    /// (50) at once to its thrower and their teammates within `radius` (3.5 m).
    fn burst_heal(&mut self, thrower: SocketAddr, at: [f32; 3]) -> Vec<Out> {
        let b = |k: &str, def: f64| weapons().pointer(&format!("/grenade_properties/burst/{k}")).and_then(Value::as_f64).unwrap_or(def) as f32;
        let (amount, radius) = (b("heal_amount", 50.0), b("radius", 3.5));
        let team = self.teams.get(&thrower).copied().filter(|t| *t < 2);
        let near: Vec<SocketAddr> = self
            .bodies
            .iter()
            .filter(|(a, (_, p))| (**a == thrower || (team.is_some() && self.teams.get(*a).copied() == team)) && self.in_combat(a) && body_dist(at, *p) <= radius + BODY_RADIUS && self.reaches(at, *p))
            .map(|(a, _)| *a)
            .collect();
        let mut v = Vec::new();
        for a in near {
            if let Some(o) = self.give_health(a, amount, thrower) {
                let slot = self.slot_of(&a);
                let owner = self.bubble_owner.get(&a).copied().unwrap_or(DAMAGEABLE);
                v.push(out(To::All, SHIELD_REPAIRED, &[owner.as_slice(), &slot.to_le_bytes(), &[0; 4]].concat()));
                v.push(o);
            }
        }
        let own = self.bodies.get(&thrower).map(|(_, p)| (body_dist(at, *p), self.reaches(at, *p)));
        let life = self.lives.get(&thrower).map(|l| (l.health, l.core, l.dead));
        info!(%thrower, healed = v.len() / 2, ?at, own_distance_and_clear = ?own, own_life = ?life, "INSTANT REPAIR");
        v
    }

    /// Health back for `who` (never past full), credited to `by`: the
    /// broadcast, or None when there was nothing to give.
    fn give_health(&mut self, who: SocketAddr, amount: f32, by: SocketAddr) -> Option<Out> {
        let slot = self.slot_of(&who);
        let owner = self.bubble_owner.get(&who).copied().unwrap_or(DAMAGEABLE);
        // No life yet: never hurt, so at full health.
        let life = self.lives.get_mut(&who)?;
        if life.dead || life.full() {
            return None;
        }
        // The health under the shield comes back first, then the shield.
        let to_core = amount.min(FULL_CORE - life.core).max(0.0);
        let to_shield = (amount - to_core).min(FULL_HEALTH - life.health).max(0.0);
        let got = to_core + to_shield;
        if got <= 0.0 {
            return None;
        }
        life.core += to_core;
        life.health += to_shield;
        if life.full() {
            life.lost_at = None;
        }
        // As native's regen broadcasts: no hit in it. It carried the last
        // hit (kind, place, cause), so every heal tick (REPAIR MATRIX: each
        // 50 ms) played as a fresh hit on the healed player's game.
        life.hurt = None;
        let b = regen_body(owner, slot, life);
        self.credits.push(Credit::Heal { by, amount: got });
        Some(out(To::All, HEALTH_BROADCAST, &b))
    }

    /// A tac mod's wall placed (`SR15NetPooledActorCreate`, 96 bytes:
    /// rotation 0..16, position 16..28 in metres, then template, space and
    /// the actor at 48..56). From a player with NEUTRON equipped it is a
    /// NEUTRON: EchoCombat's wall script (tools/fix_point.py) hides it, waits
    /// `charge_up_time`, then sets off the STUN FIELD's own explosion
    /// (`25905b54e61d3f10`) with NEUTRON's `radius`. That blast is the
    /// server's alone (the clients' copy is switched off), so it is done here.
    pub fn wall_placed(&mut self, from: SocketAddr, d: &[u8], now: Instant) {
        if d.len() != 96 {
            return;
        }
        // UMBRAL VEIL's wall is the veil: no collision, nothing to break.
        if self.tactical.get(&from) == Some(&TAC_SMOKE) {
            info!(%from, "UMBRAL VEIL placed");
            self.veil_stand_in(from, f3(d, 16), now);
            self.later.push((now + WALL_LIFETIME, from, out(To::All, ACTOR_DESTROY, &d[40..56])));
            return;
        }
        self.add_wall(from, d, now);
        // EchoCombat's NEUTRON script sends a VeilNet `P` token with each use,
        // so a token close to the wall marks it whatever the equipment said.
        let token = self.point_tokens.get(&from).is_some_and(|t| now.saturating_duration_since(*t) <= POINT_TOKEN_WINDOW);
        if !token && self.tactical.get(&from) != Some(&TAC_POINT) {
            self.last_wall.insert(from, (now, d.to_vec()));
            return;
        }
        self.point_tokens.remove(&from);
        self.arm_neutron(from, d, now);
    }

    /// A VeilNet `P` (NEUTRON used) from this player: their wall just placed,
    /// or the next one, is a NEUTRON.
    pub fn point_token(&mut self, from: SocketAddr, now: Instant) {
        match self.last_wall.remove(&from) {
            Some((at, d)) if now.saturating_duration_since(at) <= POINT_TOKEN_WINDOW => {
                info!(%from, "NEUTRON token for a wall placed just before");
                self.arm_neutron(from, &d, at);
            }
            _ => {
                self.point_tokens.insert(from, now);
            }
        }
    }

    fn arm_neutron(&mut self, from: SocketAddr, d: &[u8], placed: Instant) {
        let at = f3(d, 16);
        let delay = Duration::from_secs_f32(point_setting("charge_up_time", 1.25).max(0.0));
        info!(%from, ?at, "NEUTRON placed");
        // Its wall is the field itself, not an ENERGY BARRIER: it neither
        // blocks nor takes damage (native still removes it at 18 s).
        let actor = arr8(d, 48);
        if self.walls.remove(&actor).is_some() {
            self.area_health.remove(&actor);
        }
        self.neutrons.push(Neutron { at, thrower: from, actor: arr8(d, 48), fire_at: placed + delay });
    }

    /// NEUTRONs whose charge is up: every enemy within `radius` (to the
    /// body's line, as the field players are shown) is stunned, as a STUN
    /// FIELD stuns, credited to its owner.
    fn neutron_blasts(&mut self, now: Instant, ready: &mut Vec<(SocketAddr, Out)>) {
        if self.neutrons.is_empty() {
            return;
        }
        let space = self.space;
        let radius = point_setting("radius", 2.5);
        let due: Vec<Neutron> = {
            let (go, wait): (Vec<Neutron>, Vec<Neutron>) = std::mem::take(&mut self.neutrons).into_iter().partition(|n| n.fire_at <= now);
            self.neutrons = wait;
            go
        };
        for n in due {
            // A bubble going off around its user (where they are now), 2.5 m
            // to the body; walls and barriers in the way shield (`victims`).
            let at = self.bodies.get(&n.thrower).map_or(n.at, |b| b.1);
            let hits: Vec<Hit> = self
                .victims(at, (radius - BODY_RADIUS).max(0.0), Some(n.thrower))
                .into_iter()
                .filter(|h| h.3.is_some_and(|a| a != n.thrower && self.alive(a) && !self.immune(&a, "stun_imm")))
                .collect();
            info!(thrower = %n.thrower, ?at, stunned = hits.len(), "NEUTRON went off around its user");
            if hits.is_empty() {
                ready.push((n.thrower, out(To::All, EXPLOSION_NOTHING_HIT, &[space.as_slice(), &EX_STUN, &n.actor].concat())));
            }
            for (victim, pos, _, _, _) in hits {
                self.credits.push(Credit::Stun { by: n.thrower });
                ready.push((n.thrower, out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(&space, EX_STUN, pos, victim, n.actor))));
                ready.push((n.thrower, self.stun_out(&space, victim, true, point_setting("stun_duration", 1.25))));
            }
        }
    }

    /// ARC MINEs (`grenade_properties.arc`): `arming_length` after it is let
    /// go of, an enemy within `detect_radius` sets it off `detect_delay_length`
    /// later; it then stuns every enemy within `radius` (as a STUN FIELD's
    /// stun does) and sets off the same team's other armed mines within
    /// `chain_radius` `chain_delay` later. Then it is used up: native's used
    /// state, cooldown and destroy, as for the other ordnance.
    fn arc_mines(&mut self, now: Instant, ready: &mut Vec<(SocketAddr, Out)>) {
        if self.mines.is_empty() {
            return;
        }
        let space = self.space;
        let (detect, radius, chain) = (arc_setting("detect_radius", 3.5), arc_setting("radius", 4.5), arc_setting("chain_radius", 4.0));
        let detect_delay = Duration::from_secs_f32(arc_setting("detect_delay_length", 1.0).max(0.0));
        let chain_delay = Duration::from_secs_f32(arc_setting("chain_delay", 1.25).max(0.0));
        // Until it arms it may still be flying: follow the thrown position.
        for m in &mut self.mines {
            if (m.at.is_none() || now < m.armed_at)
                && let Some(p) = self.thrown_at.get(&m.thrower)
            {
                m.at = Some(*p);
            }
        }
        let enemy_near = |g: &Gear, m: &Mine, r: f32| -> Vec<Hit> {
            let Some(at) = m.at else { return Vec::new() };
            g.victims(at, r, Some(m.thrower)).into_iter().filter(|h| h.3.is_some_and(|a| a != m.thrower && g.alive(a) && !g.immune(&a, "stun_imm"))).collect()
        };
        for i in 0..self.mines.len() {
            let m = &self.mines[i];
            if m.fire_at.is_none() && now >= m.armed_at && !enemy_near(self, m, detect).is_empty() {
                info!(mag = %hex8(m.mag), "ARC MINE set off");
                self.mines[i].fire_at = Some(now + detect_delay);
            }
        }
        // Shot down and its owner never said so: gone, without going off.
        let shot: Vec<usize> = (0..self.mines.len()).filter(|&i| self.mines[i].shot_at.is_some_and(|t| t + MINE_SHOT_GRACE <= now)).collect();
        let mut gone = Vec::new();
        for i in shot {
            let m = &self.mines[i];
            info!(mag = %hex8(m.mag), "ARC MINE shot down (server)");
            let (mag, owner, head, thrower) = (m.mag, m.owner, m.head, m.thrower);
            self.use_up(mag, owner, head, thrower, now, ready);
            gone.push(mag);
        }
        let firing: Vec<usize> = (0..self.mines.len()).filter(|&i| !gone.contains(&self.mines[i].mag) && self.mines[i].fire_at.is_some_and(|t| t <= now)).collect();
        for i in firing {
            let m = &self.mines[i];
            let hits = enemy_near(self, m, radius);
            let (mag, owner, head, thrower, at) = (m.mag, m.owner, m.head, m.thrower, m.at);
            info!(mag = %hex8(mag), stunned = hits.len(), "ARC MINE went off");
            if hits.is_empty() {
                ready.push((thrower, out(To::All, EXPLOSION_NOTHING_HIT, &[space.as_slice(), &EX_ARC, &mag].concat())));
            }
            for (victim, pos, _, _, _) in hits {
                self.credits.push(Credit::Stun { by: thrower });
                ready.push((thrower, out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(&space, EX_ARC, pos, victim, mag))));
                ready.push((thrower, self.stun_out(&space, victim, true, arc_setting("stun_length", 2.0))));
            }
            // The chain: the same team's armed mines nearby.
            let team = self.teams.get(&thrower).copied().filter(|t| *t < 2);
            let teams = &self.teams;
            for (j, o) in self.mines.iter_mut().enumerate() {
                let same = o.thrower == thrower || (team.is_some() && teams.get(&o.thrower).copied() == team);
                if j != i && same && o.fire_at.is_none() && now >= o.armed_at && at.zip(o.at).is_some_and(|(a, b)| dist(a, b) <= chain) {
                    o.fire_at = Some(now + chain_delay);
                }
            }
            self.use_up(mag, owner, head, thrower, now, ready);
            gone.push(mag);
        }
        self.mines.retain(|m| !gone.contains(&m.mag));
    }

    /// A blast (or a DOT FIELD tick) at `at` reaching `reach`: the thrower's
    /// opponents' (or the blast owner's own) placed ARC MINEs and flying DRONEs in it take its damage at their
    /// distance, as a shot's, and are taken down the same way (their owner's
    /// client says used, or the server after `MINE_SHOT_GRACE`).
    fn blast_mines(&mut self, by: SocketAddr, at: [f32; 3], reach: f32, damage: &dyn Fn(f32) -> f32, now: Instant) -> Vec<Out> {
        self.blast_mines_in(by, at, reach, damage, now, false)
    }

    /// `blast_mines`; with `through_walls` (the DOT FIELD) only a standing
    /// ENERGY BARRIER shields them.
    fn blast_mines_in(&mut self, by: SocketAddr, at: [f32; 3], reach: f32, damage: &dyn Fn(f32) -> f32, now: Instant, through_walls: bool) -> Vec<Out> {
        let mut v = Vec::new();
        for i in 0..self.mines.len() {
            let (mag, thrower, pos) = (self.mines[i].mag, self.mines[i].thrower, self.mines[i].at);
            let Some(p) = pos else { continue };
            let d = dist(at, p);
            if d > reach || !self.may_hurt(Some(by), thrower) || self.blocked_in(at, p, through_walls) {
                continue;
            }
            info!(%by, mag = %hex8(mag), "ARC MINE caught in a blast");
            self.mines[i].shot_at.get_or_insert(now);
            let health = self.area_health.entry(mag).or_insert(ORDNANCE_HEALTH);
            *health = (*health - damage(d).max(1.0)).max(0.0);
            let left = *health;
            v.push(self.deal(mag, left));
        }
        // DRONEs in flight (thrown, not yet down), where their thrower's
        // thrown object is: native dealt a blast's damage to one too.
        let drones: Vec<([u8; 8], SocketAddr)> = self
            .mags
            .iter()
            .filter(|(k, m)| m.item == ITEM_DRONE && m.ending && self.area_health.get(*k).is_none_or(|h| *h > 0.0))
            .map(|(k, m)| (*k, m.thrower))
            .collect();
        for (mag, thrower) in drones {
            let Some(p) = self.thrown_at.get(&thrower).copied() else { continue };
            let d = dist(at, p);
            if d > reach || !self.may_hurt(Some(by), thrower) || self.blocked_in(at, p, through_walls) {
                continue;
            }
            let health = self.area_health.entry(mag).or_insert(ordnance_health(ITEM_DRONE));
            *health = (*health - damage(d).max(1.0)).max(0.0);
            let left = *health;
            info!(%by, mag = %hex8(mag), left, "DRONE caught in a blast");
            v.push(self.deal(mag, left));
            if left <= 0.0 {
                v.extend(self.drone_down(mag, now));
            }
        }
        v
    }

    /// A DRONE shot down (its health gone): it ends now, not at the end of its
    /// lifetime. What its lifetime's end would send (the used state, the
    /// thrower's cooldown and refund, the destroy) goes out from now instead.
    fn drone_down(&mut self, mag: [u8; 8], now: Instant) -> Vec<Out> {
        let Some(m) = self.mags.get(&mag).copied() else { return Vec::new() };
        let cool = [m.owner, COOLDOWN_TAIL].concat();
        self.later.retain(|(_, from, o)| {
            let this_mag = (o.ty == MAGAZINE_STATE && o.body.get(8..16) == Some(mag.as_slice())) || (o.ty == DESTROY_MAGAZINE && o.body == mag);
            let its_cooldown = *from == m.thrower && matches!(o.ty, COOLDOWN_ACTIVE | COOLDOWN_REFUND) && o.body == cool;
            !this_mag && !its_cooldown
        });
        self.drone_rays.retain(|(d, _), _| *d != mag);
        info!(mag = %hex8(mag), thrower = %m.thrower, "DRONE shot down: destroyed");
        let used = [self.space.as_slice(), &mag, &MAG_USED].concat();
        self.later.push((now + cooldown(m.item), m.thrower, out(To::Sender, COOLDOWN_REFUND, &cool)));
        self.later.push((now + DRONE_DOWN_DESTROY, m.thrower, out(To::All, DESTROY_MAGAZINE, &mag)));
        self.later.push((now, m.thrower, out(To::Sender, COOLDOWN_ACTIVE, &cool)));
        vec![out(To::All, MAGAZINE_STATE, &used)]
    }

    /// An ARC MINE the server used up, as the other ordnance: the used state
    /// to everyone, the thrower's cooldown (and refund), then destroyed.
    fn use_up(&mut self, mag: [u8; 8], owner: [u8; 8], head: [u8; 8], thrower: SocketAddr, now: Instant, ready: &mut Vec<(SocketAddr, Out)>) {
        let used = [head.as_slice(), &mag, &MAG_USED].concat();
        let cool = [owner, COOLDOWN_TAIL].concat();
        ready.push((thrower, out(To::All, MAGAZINE_STATE, &used)));
        ready.push((thrower, out(To::Sender, COOLDOWN_ACTIVE, &cool)));
        self.later.push((now + cooldown(ITEM_ARC), thrower, out(To::Sender, COOLDOWN_REFUND, &cool)));
        self.later.push((now + DESTROY_AFTER, thrower, out(To::All, DESTROY_MAGAZINE, &mag)));
        if let Some(x) = self.mags.get_mut(&mag) {
            x.ending = true;
        }
    }

    /// OVERCHARGE's heal (EchoCombat, tools/fix_buff.py): its bubble heals
    /// each teammate it links `heal_amt` over `dr_duration`
    /// (EchoCombat: `heal_amt_on_success` x `heal_fraction`, 60 x 0.05 = 3),
    /// at most `max_healing` each per use, so REPAIR MATRIX stays the healer.
    fn overcharge_heal(&mut self, now: Instant, ready: &mut Vec<(SocketAddr, Out)>) {
        if self.oc_heals.is_empty() {
            return;
        }
        let b = |k: &str| weapons().pointer(&format!("/ability_properties/buff/{k}")).and_then(Value::as_f64).map(|x| x as f32);
        let amount = b("heal_amt").unwrap_or_else(|| b("heal_amt_on_success").unwrap_or(60.0) * b("heal_fraction").unwrap_or(0.05));
        let capped = weapons().pointer("/ability_properties/buff/cap_healing").and_then(Value::as_bool).unwrap_or(true);
        let cap = if capped { b("max_healing").unwrap_or(amount) } else { f32::INFINITY };
        let secs = b("heal_duration").or_else(|| b("dr_duration")).unwrap_or(10.0).max(0.1);
        let rate = amount / secs;
        let mut hs = std::mem::take(&mut self.oc_heals);
        for h in &mut hs {
            let end = now.min(h.until);
            if now < h.until && end < h.last + REGEN_STEP {
                continue;
            }
            let dt = end.saturating_duration_since(h.last).as_secs_f32();
            h.last = end;
            if !self.alive(h.who) {
                h.until = now;
                continue;
            }
            // Linked teammates only: native's recorded drain (112, 107, ... in
            // whole 5s) shows the user never healed, `self_heal_mult` aside.
            let targets: Vec<SocketAddr> = self.oc_linked.iter().filter(|(t, u)| **u == h.who && **t != h.who).map(|(t, _)| *t).collect();
            for a in targets {
                let had = h.given.get(&a).copied().unwrap_or(0.0);
                let want = (rate * dt).min(cap - had);
                if want <= 0.0 {
                    continue;
                }
                // Not through a wall: a clear line from its user to them.
                if let (Some((_, from)), Some((_, to))) = (self.bodies.get(&h.who), self.bodies.get(&a))
                    && !self.reaches(*from, *to)
                {
                    continue;
                }
                let before = self.lives.get(&a).map_or(FULL_HEALTH + FULL_CORE, |l| l.health + l.core);
                if let Some(o) = self.give_health(a, want, h.who) {
                    let after = self.lives.get(&a).map_or(before, |l| l.health + l.core);
                    *h.given.entry(a).or_default() += after - before;
                    ready.push((a, o));
                }
            }
        }
        hs.retain(|h| h.until > now);
        self.oc_heals = hs;
    }

    /// REPAIR MATRIX healing: every `REGEN_STEP`, its user and the teammates
    /// within `heal_radius` of them get `heal_amt / heal_duration` a second
    /// (the user times `self_heal_mult`), one matrix per player at a time
    /// (`unique_healing_only`). It stops early when its user dies or leaves.
    /// Native (Surge, 2026-11-21) measured the 8 m from its user's body as
    /// they moved (a teammate's healing stopped at 8.02 m and came back at
    /// 7.94) and nothing in its settings asks for occlusion (INSTANT
    /// REPAIR's do): no line of sight needed, which ours wrongly asked.
    fn repair(&mut self, now: Instant, ready: &mut Vec<(SocketAddr, Out)>) {
        if self.matrices.is_empty() {
            return;
        }
        let rate = heal_setting("heal_amt", 114.0) / heal_setting("heal_duration", 12.0).max(0.1);
        let radius = heal_setting("heal_radius", 8.0);
        let self_mult = heal_setting("self_heal_mult", 1.0);
        let mut healed: HashSet<SocketAddr> = HashSet::new();
        let mut ms = std::mem::take(&mut self.matrices);
        for m in &mut ms {
            let end = now.min(m.until);
            if now < m.until && end < m.last + REGEN_STEP {
                continue;
            }
            let dt = end.saturating_duration_since(m.last).as_secs_f32();
            m.last = end;
            let Some(&(_, at)) = self.bodies.get(&m.who) else {
                m.until = now;
                continue;
            };
            if !self.alive(m.who) {
                m.until = now;
                continue;
            }
            let team = self.teams.get(&m.who).copied().filter(|t| *t < 2);
            let near: Vec<SocketAddr> = self
                .bodies
                .iter()
                .filter(|(a, (_, p))| (**a == m.who || (team.is_some() && self.teams.get(*a).copied() == team)) && self.in_combat(a) && dist(at, *p) <= radius)
                .map(|(a, _)| *a)
                .collect();
            for a in near {
                if !healed.insert(a) {
                    continue;
                }
                if let Some(o) = self.give_health(a, rate * dt * if a == m.who { self_mult } else { 1.0 }, m.who) {
                    ready.push((a, o));
                }
            }
        }
        ms.retain(|m| m.until > now);
        self.matrices = ms;
    }

    /// Replies whose time has come, with the client each `To` is relative to.
    pub fn due(&mut self, now: Instant) -> Vec<(SocketAddr, Out)> {
        self.cool_guns(now);
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
                    let slot = player_handle(&self.bodies, &self.slots, &self.gens, &dr.who);
                    let life = self.lives.entry(dr.who).or_default();
                    if life.dead {
                        continue;
                    }
                    // The shield first, then the health under it.
                    let off_shield = tick.min(life.health);
                    life.health -= off_shield;
                    life.core = (life.core - (tick - off_shield)).max(if lethal { 0.0 } else { 1.0 });
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
            // With the generation: a rejoined player's game (generation 1)
            // took no regen broadcast for generation 0, so its health never
            // came back on screen until the next hit's broadcast showed it.
            let slot = player_handle(&self.bodies, &self.slots, &self.gens, who);
            if life.dead || life.full() || draining.contains(who) || now < lost + REGEN_DELAY {
                continue;
            }
            let since = life.regen_at.unwrap_or(lost + REGEN_DELAY);
            if life.regen_at.is_some() && now < since + REGEN_STEP {
                continue;
            }
            let owner = self.bubble_owner.get(who).copied().unwrap_or(DAMAGEABLE);
            let gain = REGEN_PER_SEC * now.saturating_duration_since(since).as_secs_f32().max(REGEN_STEP.as_secs_f32());
            // Native: the health under the shield fills alongside the shield.
            life.core = (life.core + gain).min(FULL_CORE);
            life.health = (life.health + gain).min(FULL_HEALTH);
            life.regen_at = Some(now);
            let full = life.full();
            life.hurt = None;
            if full {
                life.lost_at = None;
            }
            let b = regen_body(owner, slot, life);
            ready.push((*who, out(To::All, HEALTH_BROADCAST, &b)));
        }
        self.repair(now, &mut ready);
        self.overcharge_heal(now, &mut ready);
        self.arc_mines(now, &mut ready);
        self.neutron_blasts(now, &mut ready);
        let mut fields = std::mem::take(&mut self.fields);
        for f in &mut fields {
            // A STUN FIELD rolls with its grenade for its whole life. Kept
            // where it opened, it stunned players at that spot (often the
            // thrower's hand) seconds after the field had rolled away.
            // Only while the thrower has taken no newer ordnance (their thrown
            // position is then that one's), and never more than a grenade can
            // roll in a tick or two: a field left still for seconds used to
            // jump to wherever the thrower's next item was and stun there.
            if f.kind == EX_STUN
                && self.spawned_at.get(&f.thrower).is_none_or(|t| *t <= f.start)
                && let Some(&p) = self.thrown_at.get(&f.thrower)
                && p != f.at
            {
                let dt = now.saturating_duration_since(f.moved).as_secs_f32().min(STUN_FOLLOW_MAX_DT);
                let slack = if f.tracked { STUN_FOLLOW_TRACKED_SLACK } else { STUN_FOLLOW_SLACK };
                if dist(p, f.at) <= slack + STUN_FOLLOW_SPEED * dt {
                    f.at = p;
                    f.moved = now;
                    f.tracked = true;
                }
            }
            while f.next <= now && f.next < f.until {
                f.next += f.every;
                // A DOT FIELD reaches exactly its radius (to the body's line, no
                // body width added: the field players are shown is that size).
                // A STUN FIELD grows to its full size (`stun_reach`), measured the same way.
                let reach = if f.kind == EX_DOT { (f.radius - BODY_RADIUS).max(0.0) } else { (stun_reach(f.next.saturating_duration_since(f.start)) - BODY_RADIUS).max(0.0) };
                // The DOT FIELD reaches through walls; a STUN FIELD does not.
                let hits = self.victims_in(f.at, reach, Some(f.thrower), f.kind == EX_DOT);
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
                    // Mines and drones in the field take its ticks too.
                    for o in self.blast_mines_in(f.thrower, f.at, f.radius, &move |_| tick, now, true) {
                        ready.push((f.thrower, o));
                    }
                    continue;
                }
                // Nobody until the grenade's own position is in (or it had
                // time to send one): before that `at` can be the hand.
                if !f.tracked && f.next.saturating_duration_since(f.start) < STUN_SETTLE {
                    continue;
                }
                // Never its own thrower: the field opens where the grenade was last
                // seen, which on a short throw is still the thrower's hand.
                let fresh: Vec<Hit> = hits.into_iter().filter(|h| h.3 != Some(f.thrower) && !f.hit.contains(&h.0) && !h.3.is_some_and(|a| self.immune(&a, "stun_imm"))).collect();
                if fresh.is_empty() {
                    ready.push((f.thrower, out(To::All, EXPLOSION_NOTHING_HIT, &[space.as_slice(), &f.kind, &f.mag].concat())));
                }
                for (victim, pos, _, who, _) in fresh {
                    f.hit.push(victim);
                    info!(thrower = %f.thrower, victim = ?who, dist = format!("{:.1}", dist(pos, f.at)), age_ms = f.next.saturating_duration_since(f.start).as_millis() as u64, tracked = f.tracked, "stun field hit");
                    if who.is_some_and(|a| a != f.thrower) {
                        self.credits.push(Credit::Stun { by: f.thrower });
                    }
                    ready.push((f.thrower, out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(&space, f.kind, pos, victim, f.mag))));
                    let secs = self.weapons_table().pointer("/grenade_properties/stun/stun_length").and_then(Value::as_f64).unwrap_or(1.75) as f32;
                    ready.push((f.thrower, self.stun_out(&space, victim, who.is_some(), secs)));
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
            ready.push((anyone, self.deal_in(b.space, target, b.max)));
            self.later.push((now + RESPAWN_ECHO_AFTER, anyone, out(To::All, COMPONENT_EVENT, &ev)));
        }
        // Raised pillars whose time is up: down, then full health.
        let down: Vec<([u8; 8], Barrier)> = self.pillars_up.iter().filter(|(t, _, _)| *t <= now).map(|(_, t, b)| (*t, *b)).collect();
        self.pillars_up.retain(|(t, _, _)| *t > now);
        for (target, b) in down {
            info!(pillar = b.handle, "pillar lowered");
            self.area_health.insert(target, b.max);
            let ev = pillar_event(b.handle);
            ready.push((anyone, out(To::All, COMPONENT_EVENT, &ev)));
            ready.push((anyone, out(To::All, COMPONENT_EVENT, &ev)));
            self.later.push((now + PILLAR_RESET_AFTER, anyone, self.deal_in(b.space, target, b.max)));
        }
        self.later.retain(|(at, from, o)| {
            if *at <= now {
                ready.push((*from, o.clone()));
                false
            } else {
                true
            }
        });
        let sent = std::mem::take(&mut ready);
        self.overdue_recharges(now, &sent, &mut ready);
        ready.splice(0..0, sent);
        for (_, o) in &ready {
            if o.ty == ACTOR_DESTROY && o.body.len() == 16 {
                let actor = arr8(&o.body, 8);
                if self.walls.remove(&actor).is_some() {
                    info!(wall = %hex8(actor), "ENERGY BARRIER wall down (its time)");
                }
                self.area_health.remove(&actor);
            }
            if o.ty == DESTROY_MAGAZINE && o.body.len() == 8 {
                let gone = arr8(&o.body, 0);
                self.drone_rays.retain(|(d, _), _| *d != gone);
                self.mags.remove(&arr8(&o.body, 0));
                self.area_health.remove(&arr8(&o.body, 0));
            }
        }
        ready
    }
}

/// A combat match's weapon settings (the EchoCombat block every combat level gets).
#[cfg(test)]
pub fn combat_weapons_for_tests() -> &'static Value {
    combat_weapons()
}

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
            // As clients get it: this server's base with the host's changes,
            // so the gear's reach and damage match what players are shown.
            .and_then(|(_, b)| super::balance::document(&super::balance::apply(b.to_vec()), "mp_weapon_settings"))
            .unwrap_or_default()
    })
}

/// Damage-table multiplier for a hit: the weapon is the one whose
/// `projectile_damage` is the shot's base damage (else SCOUT/COMET, whose
/// damage falls off), the part from the collision (2 arm, 4 torso, 5 head).
fn multiplier(w: &Value, gun: [u8; 8], base: f32, part: u64) -> f32 {
    // The hit's first 8 bytes are its gun: the symbol of the weapon's name
    // (native: `assault`, `rifle`, `magnum`; drone shots `blaster`). Telling
    // the gun by damage alone took an uncharged COMET (30) for a METEOR:
    // barriers took x2 instead of x0.5.
    let gun = u64::from_le_bytes(gun);
    let props = w.get("weapon_properties").and_then(Value::as_object);
    let weapon = props
        .and_then(|m| m.keys().find(|k| crate::sns::symbol(k) == gun))
        .or_else(|| props.and_then(|m| m.iter().find(|(_, v)| v.get("projectile_damage").and_then(Value::as_f64).is_some_and(|p| (p as f32 - base).abs() < 0.01)).map(|(k, _)| k)))
        .map_or("scout", String::as_str);
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
    b.extend_from_slice(&(base * multiplier(w, arr8(d, 0), base, part)).to_le_bytes());
    b.extend_from_slice(&d[20..40]);
    b
}

fn hex8(b: [u8; 8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn f32_at(d: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(d[o..o + 4].try_into().expect("4"))
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
/// A thrown ordnance's starting health: a DRONE's `grenade_properties.drone.
/// health` (EchoVRCE: 70; native's DealDamage went 70, 40, 13, 11, 0 and it
/// stopped firing), the rest native's 9999.
fn ordnance_health(item: [u8; 8]) -> f32 {
    if item == ITEM_DRONE {
        return weapons().pointer("/grenade_properties/drone/health").and_then(Value::as_f64).unwrap_or(70.0) as f32;
    }
    ORDNANCE_HEALTH
}

/// A STUN FIELD's radius `t` after it went off (`grenade_properties.stun`):
/// `target_aoe_radius` (1 m), growing by `aoe_radius_growth_rate` (6 m/s)
/// for `growth_time` (0.5 s) to 4 m, eased out by `growth_exponent` (3:
/// fast first, then slowing). No recording measured it; the settings'
/// own numbers, read this way, make the field players see.
fn stun_reach(t: Duration) -> f32 {
    let g = |k: &str, def: f64| weapons().pointer(&format!("/grenade_properties/stun/{k}")).and_then(Value::as_f64).unwrap_or(def) as f32;
    let (start, rate, grow, exp) = (g("target_aoe_radius", 1.0), g("aoe_radius_growth_rate", 6.0), g("growth_time", 0.5).max(0.0), g("growth_exponent", 3.0).max(0.01));
    let p = if grow <= 0.0 { 1.0 } else { (t.as_secs_f32() / grow).clamp(0.0, 1.0) };
    start + rate * grow * (1.0 - (1.0 - p).powf(exp))
}

/// Lag compensation: a blast is measured against the victim as the thrower
/// saw them, at most this far back (beyond it, the thrower's view is too old
/// to trust).
const REWIND_MAX: Duration = Duration::from_millis(250);
/// How much of each body's movement is kept for it.
const HISTORY: Duration = Duration::from_millis(400);

/// UMBRAL VEIL's stand-in for games without EchoCombat: a DETONATOR puff this often.
const VEIL_PUFF_EVERY: Duration = Duration::from_secs(2);
/// Its magazines' ids: this, with a counter in the last four bytes.
const VEIL_MAG: [u8; 8] = [0x5e, 0x11, 0x7e, 0x11, 0, 0, 0, 0];

/// The safety net's slack: an ordnance's refund is overdue this long after
/// its lifetime and cooldown have run out.
const REFUND_OVERDUE_SLACK: Duration = Duration::from_secs(5);
/// A tac mod's `AbilityCooldown` is overdue this long after its duration
/// (a held ENERGY BARRIER hologram can wait a while before it is placed).
const TAC_OVERDUE_SLACK: Duration = Duration::from_secs(30);
/// The tac mods (`mp_equipment` names) whose `ability_properties` say how long they run.
const TAC_NAMES: [&str; 8] = ["heal", "sensor", "shield", "wraith", "smoke", "buff", "point", "medigun"];

/// When a tac mod switched on is overdue for its `AbilityCooldown`: its
/// `duration` (or `effect_time`, NEUTRON's), 15 s when it has none, plus the slack.
fn tac_overdue(tac: u64) -> Duration {
    let name = TAC_NAMES.iter().find(|n| crate::wire::symbol::symbol(n) == tac);
    let secs = name
        .and_then(|n| {
            let w = weapons();
            ["duration", "effect_time"].iter().find_map(|k| w.pointer(&format!("/ability_properties/{n}/{k}")).and_then(Value::as_f64))
        })
        .unwrap_or(15.0);
    Duration::from_secs_f64(secs.clamp(1.0, 600.0)) + TAC_OVERDUE_SLACK
}

/// `ability_properties.point.<k>` (NEUTRON).
fn point_setting(k: &str, def: f64) -> f32 {
    weapons().pointer(&format!("/ability_properties/point/{k}")).and_then(Value::as_f64).unwrap_or(def) as f32
}

/// `grenade_properties.arc.<k>` (ARC MINE).
fn arc_setting(k: &str, def: f64) -> f32 {
    weapons().pointer(&format!("/grenade_properties/arc/{k}")).and_then(Value::as_f64).unwrap_or(def) as f32
}

/// `ability_properties.wraith.<k>` (PHASE SHIFT).
fn wraith_setting(k: &str, def: f64) -> f32 {
    weapons().pointer(&format!("/ability_properties/wraith/{k}")).and_then(Value::as_f64).unwrap_or(def) as f32
}

/// How long past its duration a PHASE SHIFT may run before the server ends
/// it itself (its user's off normally comes first).
const WRAITH_SLACK: f32 = 0.5;

/// `ability_properties.heal.<k>` (REPAIR MATRIX).
fn heal_setting(k: &str, def: f64) -> f32 {
    weapons().pointer(&format!("/ability_properties/heal/{k}")).and_then(Value::as_f64).unwrap_or(def) as f32
}

/// A thrown object moving less than this between updates has stopped.
const STILL: f32 = 0.01;
/// Within this of a stuck DETONATOR a player is hit whichever side they are
/// on (native hit what was within about 0.4 m, a det stuck on a player too).
const DET_CLOSE: f32 = 0.5;
/// A blast's line of sight leaves out 15 cm at each end (the surface the blast
/// sits on, or the mine on one), and is tested to a player's head and to a
/// point this far below it.
const BLAST_SKIP: f32 = 0.15;
/// A hit's wall check leaves this much out at each end: the gun is in the
/// shooter's hand, not at their head, and the hit is on the victim's surface.
const HIT_WALL_SKIP: f32 = 0.8;
/// A shot whose line passes this close to a DRONE's centre hits it (its body
/// is about 0.3 m across; the extra covers its movement between updates).
const DRONE_HIT_RADIUS: f32 = 0.6;
const DRONE_SHOT_RANGE: f32 = 150.0;
/// A shot-down DRONE's destroy follows its used state this much later.
const DRONE_DOWN_DESTROY: Duration = Duration::from_millis(200);
const BLAST_BODY_BELOW: f32 = 0.7;
/// How far past the surface a stuck DETONATOR's blast still reaches.
const DET_SURFACE_SLACK: f32 = 0.1;

/// Whether a player (head at `head`) is on the far side of the surface a thrown
/// object stuck to at `at`, having come from `from`: the surface faces back
/// along its flight, so the near point of their body lies ahead of it.
fn behind_surface(at: [f32; 3], from: [f32; 3], head: [f32; 3]) -> bool {
    let dir = [at[0] - from[0], at[1] - from[1], at[2] - from[2]];
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < STILL {
        return false;
    }
    let y = at[1].clamp(head[1] - BODY_HEIGHT, head[1]);
    let near = [head[0], y, head[2]];
    if dist(at, near) <= DET_CLOSE {
        return false;
    }
    let ahead = ((near[0] - at[0]) * dir[0] + (near[1] - at[1]) * dir[1] + (near[2] - at[2]) * dir[2]) / len;
    ahead > DET_SURFACE_SLACK
}

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
        g.slots.insert(addr(), 0);
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
            assert_eq!(multiplier(w, [0; 8], 2.0, part), want, "part {part}");
        }
    }

    /// The gun comes from the hit's gun symbol, as native's barrier damage
    /// shows (Dyson/Gauss server recordings): PULSAR 9 -> 4.5, STARBURST
    /// 11 -> 5.5, PROTOSTAR 45 -> 78.75, drone shots (`blaster`) 2 -> 1; an
    /// uncharged COMET (30, same as METEOR's damage) x0.5, not METEOR's x2.
    /// Native Surge (Gauss), 2026-11-21, server recording: every gun's hit
    /// on a barrier (230) or wall (415) took half its damage: drone shots 2
    /// -> 1, 6 -> 3, STARBURST 11 -> 5.5, in EchoCombat's tables and stock's.
    #[test]
    fn surge_barrier_hits_take_natives_damage() {
        let stock: Value = serde_json::from_str(include_str!("data/mp_weapon_settings_stock.json")).expect("stock json");
        for w in [combat_weapons(), &stock] {
            for (sym, base, want) in [("9852604d9e83d52f", 2.0, 1.0), ("b54889d0c8c72de3", 11.0, 5.5), ("edef83a8b1d0e8c8", 6.0, 3.0)] {
                let gun: [u8; 8] = unhex(sym).try_into().expect("8");
                assert_eq!(base * multiplier(w, gun, base, PART_BARRIER), want, "{sym}");
            }
        }
    }

    #[test]
    fn barrier_damage_follows_the_gun() {
        let w = combat_weapons();
        let gun = |n: &str| crate::sns::symbol(n).to_le_bytes();
        for (name, base, want) in [("assault", 9.0, 4.5), ("rifle", 11.0, 5.5), ("magnum", 45.0, 78.75), ("blaster", 2.0, 1.0), ("scout", 30.0, 15.0), ("rocket", 30.0, 60.0)] {
            assert_eq!(base * multiplier(w, gun(name), base, PART_BARRIER), want, "{name}");
        }
        assert_eq!(unhex("9e5b615f8c9cd62f"), gun("assault").to_vec(), "native's PULSAR symbol");
        // The COMET hits in the 2026-11-20 Dyson match (30 damage, 60 off a barrier then).
        assert_eq!(unhex("a45180dac9c72de3"), gun("scout").to_vec(), "COMET symbol");
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
        // It goes through the level's walls, but not an ENERGY BARRIER.
        let mut wall = vec![0u8; 96];
        wall[12..16].copy_from_slice(&1f32.to_le_bytes());
        for (k, x) in [41.25f32, 0.95, -24.15].iter().enumerate() {
            wall[16 + 4 * k..20 + 4 * k].copy_from_slice(&x.to_le_bytes());
        }
        wall[28..32].copy_from_slice(&1f32.to_le_bytes());
        wall[32..40].copy_from_slice(&WALL_TEMPLATE);
        wall[40..48].copy_from_slice(&g.space);
        wall[48..56].copy_from_slice(&[7; 8]);
        g.wall_placed(addr(), &wall, t0);
        let ticks = g.due(t0 + Duration::from_secs(2));
        let damage = ticks.iter().filter(|(_, o)| o.ty == EXPLOSION_PLAYER_DAMAGE && o.body[32..40] != NO_PLAYER).count();
        assert_eq!(damage, 0, "behind an ENERGY BARRIER");
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
        g.slots.insert(addr(), 0);
        assert!(g.left_area(addr()).is_empty(), "never entered");
        g.on_message(addr(), SET_OWNER, &[0; 8], Instant::now());
        let v = g.left_area(addr());
        assert_eq!(v.len(), 8);
        assert_eq!(v[1].body, unhex("fcffffff0f000000"));
        assert!(g.left_area(addr()).is_empty(), "second notification");
    }

    /// Our Combustion match (2026-11-19 20:06 recording): a player who used
    /// gear but never sent body physics left, and slot 0's stations were
    /// freed, taking slot 0's gun. A leaver frees their own slot's stations,
    /// none if their slot is unknown.
    #[test]
    fn a_leaver_frees_only_their_own_stations() {
        let mut g = Gear::new();
        let (me, them) = (addr(), "10.0.0.9:7".parse().expect("addr"));
        for a in [me, them] {
            g.on_message(a, SET_OWNER, &[0; 8], Instant::now());
        }
        g.slots.insert(me, 0);
        g.slots.insert(them, 3);
        let v = g.forget(them);
        assert_eq!(v.len(), 8);
        assert_eq!(v[0].body, unhex("fcffffff03000000"), "slot 3's, not slot 0's");
        g.slots.clear();
        assert!(g.forget(me).is_empty(), "slot unknown: nothing freed");
    }

    /// Gun heat (native's per-slot meter): a step a shot, `[heat][0][slot]`
    /// under the slot's key, full shows briefly then 0, and it drains after
    /// the gun's cooldown delay.
    #[test]
    fn gun_heat_is_sent_per_slot() {
        let mut g = Gear::new();
        let t0 = Instant::now();
        let me = addr();
        g.slots.insert(me, 3);
        g.heat_shot(me, 60.0, t0);
        let v = g.heat_vars();
        assert_eq!(v.len(), 1);
        let (k, raw) = v[0];
        assert_eq!(k, 0x71d2_85d5_24e9_a696, "slot 3's key");
        assert_eq!(raw >> 48, 3, "slot in the top u16");
        let one = f32::from_bits(raw as u32);
        assert!(one > 0.0 && one < 1.0, "{one}");
        // Many shots: overheat, shown full, then 0.
        for i in 1..40 {
            g.heat_shot(me, 60.0, t0 + Duration::from_millis(i));
        }
        assert_eq!(f32::from_bits(g.heat_vars()[0].1 as u32), 1.0);
        g.cool_guns(t0 + Duration::from_secs(1));
        assert_eq!(f32::from_bits(g.heat_vars()[0].1 as u32), 0.0);
        // One shot drains back to 0 after its delay.
        let t1 = t0 + Duration::from_secs(2);
        g.heat_shot(me, 60.0, t1);
        g.cool_guns(t1 + Duration::from_millis(50));
        assert!(f32::from_bits(g.heat_vars()[0].1 as u32) > 0.0, "held during the delay");
        for i in 1..=100 {
            g.cool_guns(t1 + Duration::from_millis(100 * i));
        }
        assert_eq!(f32::from_bits(g.heat_vars()[0].1 as u32), 0.0);
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

    /// Lag compensation: a DETONATOR thrown by a player with a 200 ms round
    /// trip hits a victim who was in its reach 200 ms ago and has moved out;
    /// without that ping it misses.
    #[test]
    fn blasts_hit_where_the_thrower_saw_the_victim() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.set_match(super::super::combat::LEVEL_GAUSS);
        let at = |x: i32| body(1, [43_800 + x, 1_870, -25_300]);
        for k in 0..=20 {
            // Walking 7 m/s away along x, one update every 10 ms.
            g.observe_physics_at(them, &at(300 + 70 * k), t0 + Duration::from_millis(10 * k as u64));
        }
        let blast = [43.8 + 0.3, 1.87, -25.3];
        let near = |g: &Gear| g.victims(blast, 0.6, Some(me)).iter().any(|h| h.3 == Some(them));
        assert!(!near(&g), "1.4 m away now");
        g.set_ping(me, Duration::from_millis(200));
        assert!(near(&g), "there 200 ms ago, on the thrower's screen");
        g.set_ping(me, Duration::from_secs(2));
        assert!(near(&g), "capped, still a hit within 250 ms");
    }

    #[test]
    fn tac_mod_symbols_are_their_names() {
        assert_eq!(crate::wire::symbol::symbol("point"), TAC_POINT);
        assert_eq!(crate::wire::symbol::symbol("wraith"), TAC_WRAITH);
    }

    /// The safety net: a tac mod switched on with no `AbilityCooldown` after
    /// its duration and slack gets one from the server (two to its user, one
    /// to everyone, then off); one that cooled normally gets nothing.
    #[test]
    fn overdue_tac_mod_is_recharged() {
        let (mut g, me, _, t0) = two_in_the_area();
        g.on_message(me, ABILITY_ACTIVATE, &unhex("0300000001000000"), t0);
        let late = tac_overdue(0);
        assert!(g.due(t0 + late - Duration::from_secs(1)).iter().all(|(_, o)| o.ty != ABILITY_COOLDOWN));
        let v = g.due(t0 + late);
        let cools: Vec<_> = v.iter().filter(|(a, o)| *a == me && o.ty == ABILITY_COOLDOWN).collect();
        assert_eq!(cools.len(), 2);
        assert!(cools.iter().all(|(_, o)| o.body == unhex("0300000000000000")));
        assert!(v.iter().any(|(_, o)| o.ty == ABILITY_ACTIVATE && o.to == To::All && o.body == unhex("0300000000000000")));
        assert!(g.due(t0 + late * 3).iter().all(|(_, o)| o.ty != ABILITY_COOLDOWN), "once");
        // Cooled by the client itself, or respawned: nothing.
        g.on_message(me, ABILITY_ACTIVATE, &unhex("0300000001000000"), t0);
        g.on_message(me, ABILITY_COOLDOWN, &unhex("0300000000000000"), t0 + Duration::from_secs(12));
        assert!(g.due(t0 + late * 2).iter().all(|(_, o)| o.ty != ABILITY_COOLDOWN));
    }

    /// The safety net: a thrown ordnance whose refund never came gets one
    /// (here its magazine was put away by nothing and never used); a new
    /// ordnance in hand clears the debt.
    #[test]
    fn overdue_ordnance_is_refunded() {
        let (mut g, me, _, t0) = two_in_the_area();
        let mag = [7u8; 8];
        let owner = [9u8; 8];
        g.mags.insert(mag, Mag { owner, item: ITEM_DET, thrower: me, ending: true, at_spawn: None });
        let state = [g.space.as_slice(), &mag, &MAG_THROWN].concat();
        g.on_message(me, MAGAZINE_STATE, &state, t0);
        let late = lifetime(ITEM_DET) + cooldown(ITEM_DET) + REFUND_OVERDUE_SLACK;
        assert!(g.due(t0 + late - Duration::from_secs(1)).iter().all(|(_, o)| o.ty != COOLDOWN_REFUND));
        let v = g.due(t0 + late);
        assert!(v.iter().any(|(a, o)| *a == me && o.ty == COOLDOWN_REFUND && o.to == To::Sender && o.body == [owner, COOLDOWN_TAIL].concat()));
        assert!(g.due(t0 + late * 2).iter().all(|(_, o)| o.ty != COOLDOWN_REFUND), "once");
        // A refund on time clears it.
        g.on_message(me, MAGAZINE_STATE, &state, t0);
        g.later.push((t0 + Duration::from_secs(1), me, out(To::Sender, COOLDOWN_REFUND, &[owner, COOLDOWN_TAIL].concat())));
        assert_eq!(g.due(t0 + Duration::from_secs(1)).iter().filter(|(_, o)| o.ty == COOLDOWN_REFUND).count(), 1);
        assert!(g.due(t0 + late * 3).iter().all(|(_, o)| o.ty != COOLDOWN_REFUND));
    }

    /// UMBRAL VEIL and NEUTRON walls are EchoCombat's; a veil is no barrier.
    #[test]
    fn veil_and_neutron_walls_are_echocombat_only() {
        let (mut g, me, them, t0) = two_in_the_area();
        let wall = unhex("0e9d8bbe68f0ce3e8b710b3e81c65c3f0c98004252b41841f6fdde400000803f262929d5a48c8b3d193a620c7adae243d263e3b2e176fbcdf034d5099874113f8652624a9681de2f3c881a3f0a8cb73d37a60d3f6f0000000100000000000000");
        assert!(!g.echocombat_wall(them, &wall), "ENERGY BARRIER");
        g.tactical.insert(them, TAC_SMOKE);
        assert!(g.echocombat_wall(them, &wall));
        g.wall_placed(them, &wall, t0);
        assert!(g.walls.is_empty() && !g.area_health.contains_key(&arr8(&wall, 48)), "the veil blocks nothing");
        g.tactical.insert(me, TAC_POINT);
        assert!(g.echocombat_wall(me, &wall));
        // Games without EchoCombat get DETONATOR smoke there instead, hurting nobody.
        let first = g.due_stock(t0);
        let spawn = first.iter().find(|o| o.ty == SPAWN_MAGAZINE).expect("a puff");
        assert_eq!(spawn.body.len(), 40);
        assert_eq!(&spawn.body[12..24], &wall[16..28], "at the veil");
        assert_eq!(&spawn.body[24..32], &ITEM_DET);
        let rest = g.due_stock(t0 + Duration::from_secs(60));
        let all: Vec<_> = first.iter().chain(&rest).collect();
        let puffs = all.iter().filter(|o| o.ty == SPAWN_MAGAZINE).count();
        assert_eq!(puffs, 4, "8 s of veil, one every 2 s");
        assert_eq!(all.iter().filter(|o| o.ty == DESTROY_MAGAZINE).count(), puffs);
        assert!(all.iter().all(|o| o.ty != EXPLOSION_PLAYER_DAMAGE && o.ty != HEALTH_BROADCAST));
        assert!(g.due(t0 + Duration::from_secs(60)).iter().all(|(_, o)| o.ty != SPAWN_MAGAZINE), "not for EchoCombat games");
    }

    /// ENERGY BARRIER walls (native Gauss, t 546.58: this create): 415
    /// health, bullets take it down for everyone, native's removal 18.04 s
    /// after placing, or right away when broken; a standing one stops blasts.
    #[test]
    fn energy_barrier_walls_have_health_and_go_after_18s() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.set_match(super::super::combat::LEVEL_GAUSS);
        let wall = unhex("0e9d8bbe68f0ce3e8b710b3e81c65c3f0c98004252b41841f6fdde400000803f262929d5a48c8b3d193a620c7adae243d263e3b2e176fbcdf034d5099874113f8652624a9681de2f3c881a3f0a8cb73d37a60d3f6f0000000100000000000000");
        let actor = arr8(&wall, 48);
        g.wall_placed(them, &wall, t0);
        assert_eq!(g.area_health.get(&actor), Some(&WALL_HEALTH));
        // A shot on it, in the level's space.
        let mut bc = head_shot_on_slot_1();
        bc[32..40].copy_from_slice(&actor);
        bc[40..44].copy_from_slice(&wall[40..44]);
        let v = g.on_message(me, BULLET_COLLISION, &bc, t0);
        let dd = v.iter().find(|o| o.ty == DEAL_DAMAGE).expect("wall's health");
        assert_eq!(&dd.body[..16], &wall[40..56]);
        let left = f32::from_le_bytes(dd.body[16..20].try_into().expect("4"));
        assert!(left < WALL_HEALTH && left > 0.0, "{left}");
        // It stands between: a blast behind it does not reach through.
        let p = f3(&wall, 16);
        assert!(g.line_blocked([p[0], p[1], p[2] - 2.0], [p[0], p[1], p[2] + 2.0], 0.0) || g.line_blocked([p[0] - 2.0, p[1], p[2]], [p[0] + 2.0, p[1], p[2]], 0.0));
        // Native's removal, to everyone.
        assert!(g.due(t0 + Duration::from_secs(18)).iter().all(|(_, o)| o.ty != ACTOR_DESTROY));
        let v = g.due(t0 + Duration::from_millis(18_100));
        let gone = v.iter().find(|(_, o)| o.ty == ACTOR_DESTROY).expect("destroy");
        assert_eq!(gone.1.body, wall[40..56].to_vec());
        assert!(matches!(gone.1.to, To::All));
        assert!(g.walls.is_empty());
        // Broken: gone now, not at 18 s.
        g.wall_placed(them, &wall, t0);
        let v = g.hit_wall(me, actor, 500.0, t0);
        assert!(v.iter().any(|o| o.ty == DEAL_DAMAGE && o.body[16..20] == 0f32.to_le_bytes()));
        assert!(g.walls.is_empty());
        let v = g.due(t0 + Duration::from_millis(20));
        assert_eq!(v.iter().filter(|(_, o)| o.ty == ACTOR_DESTROY).count(), 1);
        assert!(g.due(t0 + Duration::from_secs(19)).iter().all(|(_, o)| o.ty != ACTOR_DESTROY), "no second destroy");
    }

    /// Fission's climax barriers (1000 health) live in the climax sublevel's
    /// space: native's DealDamage to pool slot 2 there (t 338.31: 999 left).
    #[test]
    fn fission_climax_barriers_are_known_in_their_space() {
        let (mut g, me, _, t0) = two_in_the_area();
        g.set_match(super::super::combat::LEVEL_FISSION);
        let climax = super::super::msg::LEVEL_FISSION_CLIMAX.to_le_bytes();
        let target = unhex("0200000001000000");
        let b = g.barrier_in(&climax, arr8(&target, 0)).expect("climax barrier");
        assert_eq!((b.handle, b.max), (258, 1000.0));
        assert!(g.barrier(arr8(&target, 0)).is_none(), "not in the level's own space");
        let mut bc = head_shot_on_slot_1();
        bc[32..40].copy_from_slice(&target);
        bc[40..44].copy_from_slice(&climax[..4]);
        let v = g.on_message(me, BULLET_COLLISION, &bc, t0);
        let dd = v.iter().find(|o| o.ty == DEAL_DAMAGE).expect("barrier's health");
        assert_eq!(&dd.body[..16], &[climax.as_slice(), &target].concat()[..]);
        let left = f32::from_le_bytes(dd.body[16..20].try_into().expect("4"));
        assert!(left < 1000.0 && left > 900.0, "{left}");
        assert!(g.barriers_down().is_empty());
    }

    /// Fission's end-game pillars as native's (2026-11-21): up once 50 is
    /// taken, the event twice; down 25.03 s later, the same event twice and
    /// full health a frame after; never broken.
    #[test]
    fn fission_pillars_go_up_when_shot_and_come_down() {
        let (mut g, me, _, t0) = two_in_the_area();
        g.set_match(super::super::combat::LEVEL_FISSION);
        let climax = super::super::msg::LEVEL_FISSION_CLIMAX.to_le_bytes();
        let target = arr8(&unhex("0500000001000000"), 0);
        let b = g.barrier_in(&climax, target).expect("pillar 5");
        let events = |v: &[Out]| v.iter().filter(|o| o.ty == COMPONENT_EVENT && o.body == pillar_event(261)).count();
        assert_eq!(events(&g.hit_barrier(me, Some(b), target, 30.0, t0)), 0, "970: still down");
        let v = g.hit_barrier(me, Some(b), target, 30.0, t0);
        assert_eq!(events(&v), 2, "940: up");
        assert_eq!(&pillar_event(261)[..], &unhex("dabcc2ae120d9980c4b6bab2ee99ea6a05010d00ffffffff")[..]);
        assert_eq!(events(&g.hit_barrier(me, Some(b), target, 30.0, t0)), 0, "already up");
        // Even all its health gone: no break.
        let v = g.hit_barrier(me, Some(b), target, 5000.0, t0);
        assert!(v.iter().all(|o| o.ty != COMPONENT_EVENT));
        assert!(g.later.is_empty() && g.respawns.is_empty());
        let early: Vec<Out> = g.due(t0 + Duration::from_secs(25)).into_iter().map(|(_, o)| o).collect();
        assert_eq!(events(&early), 0);
        let down: Vec<Out> = g.due(t0 + PILLAR_UP).into_iter().map(|(_, o)| o).collect();
        assert_eq!(events(&down), 2, "down");
        let full: Vec<Out> = g.due(t0 + PILLAR_UP + PILLAR_RESET_AFTER).into_iter().map(|(_, o)| o).collect();
        let dd = full.iter().find(|o| o.ty == DEAL_DAMAGE).expect("full health");
        assert_eq!(&dd.body[16..20], &1000f32.to_le_bytes());
        assert_eq!(events(&g.hit_barrier(me, Some(b), target, 30.0, t0 + PILLAR_UP)), 0, "a fresh 1000");
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

    /// Replays a decoded recording (`msgs.csv`) through the cheat checks:
    /// `REPLAY_CSV=<path> REPLAY_LEVEL=dyson|combustion|fission|gauss
    /// [REPLAY_FROM=s REPLAY_TO=s] cargo test --release replay_checks -- --ignored --nocapture`.
    /// Server-role recordings give every player's own messages (by peer);
    /// client-role ones the recorder's own and the others' relayed bodies.
    #[test]
    #[ignore]
    fn replay_checks() {
        use std::io::BufRead;
        let path = std::env::var("REPLAY_CSV").expect("REPLAY_CSV");
        let level = match std::env::var("REPLAY_LEVEL").as_deref() {
            Ok("combustion") => super::super::combat::LEVEL_COMBUSTION,
            Ok("fission") => super::super::combat::LEVEL_FISSION,
            Ok("gauss") => super::super::combat::LEVEL_GAUSS,
            _ => super::super::combat::LEVEL_DYSON,
        };
        let span = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
        let (from_t, to_t) = (span("REPLAY_FROM", 0.0), span("REPLAY_TO", 1e9));
        let client_role = std::env::var("REPLAY_ROLE").as_deref() == Ok("client");
        let mut g = Gear::new();
        g.set_match(level);
        let t0 = Instant::now();
        let peer = |n: u32| -> SocketAddr { format!("10.0.0.1:{}", 20000 + n).parse().expect("addr") };
        let relayed = |slot: u32| -> SocketAddr { format!("10.0.0.2:{}", 30000 + slot).parse().expect("addr") };
        let (mut hits, mut shots) = (0, 0);
        let mut offs: Vec<f32> = Vec::new();
        let mut peers = std::collections::BTreeSet::new();
        let f = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(&path).expect("csv"));
        for line in f.lines().skip(1) {
            let line = line.expect("line");
            let c: Vec<&str> = line.split(',').collect();
            if c.len() < 9 {
                continue;
            }
            let t: f64 = c[1].parse().unwrap_or(0.0);
            if t < from_t || t > to_t {
                continue;
            }
            let name = c[4];
            let at = t0 + Duration::from_secs_f64(t);
            let wanted = matches!(name, "SR15NetPhysicsSimStateEvent" | "SR15NetGameFireGun" | "SR15NetDamageableBulletCollision");
            if !wanted {
                continue;
            }
            let d = unhex(c[7]);
            let p: u32 = c[8].trim().parse().unwrap_or(0);
            match (c[0], name) {
                ("c2s", "SR15NetPhysicsSimStateEvent") => {
                    peers.insert(p);
                    g.observe_physics_at(peer(p), &d, at)
                }
                ("s2c", "SR15NetPhysicsSimStateEvent") if d.len() == 64 && d[4..8] == BODY_OBJECT => {
                    let slot = u32::from(u16::from_le_bytes([d[0], d[1]]));
                    // A client's recording: the others' bodies, relayed.
                    if client_role {
                        g.observe_physics_at(relayed(slot), &d, at);
                    }
                }
                ("c2s", "SR15NetGameFireGun") => {
                    shots += 1;
                    g.on_message(peer(p), FIRE_GUN, &d, at);
                }
                ("c2s", "SR15NetDamageableBulletCollision") if d.len() == 64 => {
                    if d[40..44] == BODY_OBJECT
                        && let Some(v) = g.player_by_id(arr8(&d, 32))
                        && let Some(&(_, head)) = g.bodies.get(&v)
                    {
                        hits += 1;
                        let off = body_dist(f3(&d, 20), head);
                        offs.push(off);
                        let before = g.checks.count(peer(p), super::super::checks::Flag::FarFromVictim) + g.checks.count(peer(p), super::super::checks::Flag::ThroughWall);
                        g.on_message(peer(p), BULLET_COLLISION, &d, at);
                        let flagged = g.checks.count(peer(p), super::super::checks::Flag::FarFromVictim) + g.checks.count(peer(p), super::super::checks::Flag::ThroughWall) > before;
                        if flagged {
                            let eye = g.bodies.get(&peer(p)).map(|b| b.1);
                            println!("REPLAY case t {t:.2} peer {p} victim {v} off {off:.2} hit {:?} victim head {head:?} shooter {eye:?} gun {} dmg {}", f3(&d, 20), hex8(arr8(&d, 0)), f32_at(&d, 44));
                        }
                    }
                    else {
                        g.on_message(peer(p), BULLET_COLLISION, &d, at);
                    }
                }
                _ => {}
            }
        }
        offs.sort_by(f32::total_cmp);
        let pct = |q: f32| offs.get(((offs.len() as f32 - 1.0) * q) as usize).copied().unwrap_or(0.0);
        println!("REPLAY shots {shots} player hits {hits}: hit to body p50 {:.2} p95 {:.2} p99 {:.2} max {:.2}", pct(0.5), pct(0.95), pct(0.99), pct(1.0));
        let all: Vec<SocketAddr> = g.bodies.keys().copied().collect();
        for a in all {
            let n: Vec<String> = super::super::checks::Flag::ALL_FOR_TESTS.iter().map(|f| format!("{f:?} {}", g.checks.count(a, *f))).collect();
            println!("REPLAY {a}: {}", n.join(", "));
        }
    }

    /// A player who rejoined (generation 1) is hit by shots naming them with
    /// the generation in the id (dyson, 2026-11-20: they took no damage), and
    /// their health and kill carry the full handle.
    #[test]
    fn a_rejoined_player_takes_shots() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.gens.insert(them, 1);
        let mut bc = head_shot_on_slot_1();
        bc[32..40].copy_from_slice(&player_id(1, 1));
        assert_eq!(player_id(5, 1), [0xf7, 0x34, 0xd5, 0x09, 0x98, 0x74, 0x11, 0x3f]);
        let v = g.on_message(me, BULLET_COLLISION, &bc, t0);
        let hb = v.iter().find(|o| o.ty == HEALTH_BROADCAST).expect("rejoined victim's health");
        assert_eq!(&hb.body[8..12], &0x1_0001u32.to_le_bytes());
        // The old id (generation 0) still finds the slot's only player.
        bc[32..40].copy_from_slice(&player_id(1, 0));
        assert!(g.on_message(me, BULLET_COLLISION, &bc, t0).iter().any(|o| o.ty == HEALTH_BROADCAST));
        assert_eq!(id_handle(DAMAGEABLE), None);
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
        // Fission's are its climax sublevel's, in that space.
        let fission = barriers_of(super::super::combat::LEVEL_FISSION.to_le_bytes());
        assert_eq!(fission.len(), 8);
        assert!(fission.iter().all(|b| b.max == 1000.0 && b.space == super::super::msg::LEVEL_FISSION_CLIMAX.to_le_bytes()));
    }

    /// REPAIR MATRIX: its user's teammates nearby get 114 / 12 s = 9.5 a
    /// second back, credited to the user, up to full health and for 12 s.
    #[test]
    fn repair_matrix_heals_teammates_and_credits_the_user() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
        assert_eq!(g.lives.get(&them).map(|l| l.health), Some(112.0 - 13.5));
        g.credits.clear();
        g.teams.insert(me, 0);
        g.teams.insert(them, 0);
        g.on_message(me, HEAL_BUBBLE_ACTIVATE, &unhex("8ec21d00d315993fd1eb3d3fa8a2fdb10000000000000000"), t0);
        let v = g.due(t0 + Duration::from_secs(1));
        assert!(v.iter().any(|(a, o)| *a == them && o.ty == HEALTH_BROADCAST));
        let health = g.lives.get(&them).map_or(0.0, |l| l.health);
        assert!((health - (112.0 - 13.5 + 9.5)).abs() < 0.01, "{health}");
        let healed = |g: &Gear| g.credits.iter().map(|c| if let Credit::Heal { by, amount } = c { assert_eq!(*by, me); *amount } else { 0.0 }).sum::<f32>();
        assert!((healed(&g) - 9.5).abs() < 0.01);
        // Never past full; done after 12 s.
        g.due(t0 + Duration::from_secs(3));
        assert_eq!(g.lives.get(&them).map(|l| l.health), Some(112.0));
        assert!((healed(&g) - 13.5).abs() < 0.01);
        g.due(t0 + Duration::from_secs(13));
        assert!(g.matrices.is_empty());
    }

    /// INSTANT REPAIR: when its thrower's client says it popped, 50 back at
    /// once to teammates within 3.5 m, credited to the thrower.
    #[test]
    fn instant_repair_heals_teammates_at_once() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
        for _ in 0..3 {
            g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
        }
        assert_eq!(g.lives.get(&them).map(|l| l.health), Some(112.0 - 4.0 * 13.5));
        g.credits.clear();
        g.teams.insert(me, 0);
        g.teams.insert(them, 0);
        let mag = "84fec19782c6fd39";
        g.on_message(me, SPAWN_MAGAZINE, &unhex(&format!("f234d5099974113f010000009d24204220836f3f5f49bfc1e9ed9ca8b1d0e8c8{mag}")), t0);
        g.on_message(me, MAGAZINE_STATE, &unhex(&format!("193a620c7adae243{mag}6388fd280387e323")), t0);
        let v = g.on_message(me, MAGAZINE_STATE, &unhex(&format!("193a620c7adae243{mag}6281fb261b84f533")), t0);
        assert!(v.iter().any(|o| o.ty == HEALTH_BROADCAST));
        assert!(v.iter().any(|o| o.ty == COOLDOWN_ACTIVE));
        assert_eq!(g.lives.get(&them).map(|l| l.health), Some(112.0 - 4.0 * 13.5 + 50.0));
        assert_eq!(g.credits, vec![Credit::Heal { by: me, amount: 50.0 }]);
    }

    /// INSTANT REPAIR thrown (not just let go of): its later used state still
    /// heals (the throw used to end it, and the used state was ignored).
    #[test]
    fn instant_repair_thrown_still_heals() {
        let (mut g, me, them, t0) = two_in_the_area();
        for _ in 0..4 {
            g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
        }
        g.teams.insert(me, 0);
        g.teams.insert(them, 0);
        let mag = "84fec19782c6fd39";
        g.on_message(me, SPAWN_MAGAZINE, &unhex(&format!("f234d5099974113f010000009d24204220836f3f5f49bfc1e9ed9ca8b1d0e8c8{mag}")), t0);
        g.on_message(me, MAGAZINE_STATE, &unhex(&format!("193a620c7adae243{mag}6388fd280387e323")), t0);
        g.on_message(me, MAGAZINE_STATE, &unhex(&format!("193a620c7adae243{mag}a9509fd4dfc72de3")), t0);
        let v = g.on_message(me, MAGAZINE_STATE, &unhex(&format!("193a620c7adae243{mag}6281fb261b84f533")), t0);
        assert!(v.iter().any(|o| o.ty == HEALTH_BROADCAST));
        assert_eq!(g.lives.get(&them).map(|l| l.health), Some(112.0 - 4.0 * 13.5 + 50.0));
    }

    /// A STUN FIELD rolls with its grenade: once the grenade has moved on,
    /// a player at the spot where it opened is not stunned.
    #[test]
    fn stun_field_follows_its_grenade() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.teams.insert(me, 0);
        g.teams.insert(them, 1);
        let p = g.bodies[&them].0;
        let spot = g.bodies[&them].1;
        let opened = [spot[0], spot[1] - 1.5, spot[2]];
        g.thrown_at.insert(me, opened);
        g.fields.push(Field { kind: EX_STUN, mag: [0; 8], at: opened, thrower: me, until: t0 + Duration::from_secs(6), next: t0, every: Duration::from_millis(8), radius: 4.0, start: t0, hit: Vec::new(), moved: t0, tracked: false });
        // The grenade rolls 10 m away over 0.5 s, before the field stuns anyone there.
        // Native (Gauss recording): updates every ~22 ms, the field on the grenade.
        for i in 1..=10 {
            g.thrown_at.insert(me, [opened[0] + 2.0 + i as f32, opened[1], opened[2]]);
            g.due(t0 + Duration::from_millis(25 * i));
        }
        assert_eq!(g.fields[0].at[0], opened[0] + 12.0, "followed");
        assert!(!g.fields[0].hit.contains(&g.id_of(&them, p)), "not stunned where it opened");
    }

    /// A player leaving a match: the server clears their gear bits (native
    /// Dyson: fe to fc, ee to ec), on every station set.
    #[test]
    fn a_match_leaver_gets_their_bits_cleared() {
        let mut g = Gear::new();
        g.set_match(super::super::combat::LEVEL_DYSON);
        let me = addr();
        g.slots.insert(me, 0);
        g.on_message(me, SET_BITS, &[0xee, 0xff, 0xff, 0xff, 15, 0, 0, 0], Instant::now());
        let v = g.forget(me);
        assert_eq!(v.len(), STATIONS as usize);
        assert_eq!(v[0], out(To::Others, SET_BITS, &[0xe0, 0xff, 0xff, 0xff, 0, 0, 0, 0]));
        assert_eq!(v[1], out(To::Others, SET_BITS, &[0xec, 0xff, 0xff, 0xff, 15, 0, 0, 0]));
    }

    /// A STUN FIELD that has lain still does not jump to the thrower's next
    /// ordnance far away and stun there.
    #[test]
    fn stun_field_does_not_jump_to_the_next_ordnance() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.teams.insert(me, 0);
        g.teams.insert(them, 1);
        let p = g.bodies[&them].0;
        let spot = g.bodies[&them].1;
        let opened = [spot[0] + 30.0, spot[1], spot[2]];
        g.thrown_at.insert(me, opened);
        g.fields.push(Field { kind: EX_STUN, mag: [0; 8], at: opened, thrower: me, until: t0 + Duration::from_secs(6), next: t0, every: Duration::from_millis(8), radius: 4.0, start: t0, hit: Vec::new(), moved: t0, tracked: true });
        g.due(t0 + Duration::from_secs(2));
        // 3 s in, the thrower's next item shows up 30 m away, next to `them`.
        g.thrown_at.insert(me, [spot[0], spot[1] - 1.0, spot[2]]);
        g.due(t0 + Duration::from_secs(3));
        assert!(!g.fields[0].hit.contains(&g.id_of(&them, p)), "not stunned 30 m from the field");
        assert_eq!(g.fields[0].at, opened);
        // Nor once a newer ordnance spawned, even close by.
        let near = [opened[0] + 1.0, opened[1], opened[2]];
        g.spawned_at.insert(me, t0 + Duration::from_secs(3));
        g.thrown_at.insert(me, near);
        g.due(t0 + Duration::from_millis(3010));
        assert_eq!(g.fields[0].at, opened);
    }

    /// PHASE SHIFT: while it is on (its user's AbilityActivate on, with the
    /// `wraith` tac mod equipped), shots and blasts do nothing; off ends it.
    #[test]
    fn phase_shift_makes_its_user_immune() {
        let (mut g, me, them, t0) = two_in_the_area();
        let mut equip = unhex("ffffffff000000009e5b615f8c9cd62f16132a1b2fd0d241fee794a8b1d0e8c80100000000000000ffffffffffffffffffffffffffff0000");
        g.set_equipment(them, &equip);
        g.on_message(them, ABILITY_ACTIVATE, &unhex("0100000001000000"), t0);
        assert!(g.immune(&them, "dmg_imm") && g.immune(&them, "exp_imm") && g.immune(&them, "stun_imm"));
        let v = g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
        assert!(v.iter().all(|o| o.ty != HEALTH_BROADCAST && o.ty != TRACKED_DAMAGE));
        assert!(g.victims([43.8 + 0.3, 1.87, -25.3], 3.0, Some(me)).iter().all(|h| h.3 != Some(them)));
        // Off: hurt again.
        g.on_message(them, ABILITY_ACTIVATE, &unhex("0100000000000000"), t0);
        let v = g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
        assert!(v.iter().any(|o| o.ty == HEALTH_BROADCAST));
        // Another tac mod's activation is not a phase.
        equip[16..24].copy_from_slice(&unhex("bc1f7632483ec3c8"));
        g.set_equipment(them, &equip);
        g.on_message(them, ABILITY_ACTIVATE, &unhex("0100000001000000"), t0);
        assert!(!g.immune(&them, "dmg_imm"));
    }

    #[test]
    fn phase_shift_symbol_is_wraith() {
        assert_eq!(crate::sns::symbol("wraith"), TAC_WRAITH);
    }

    /// ARC MINE: armed 3.5 s after it is let go of, an enemy within 3.5 m sets
    /// it off, 1 s later it stuns them (credited to the thrower) and is used up.
    #[test]
    fn arc_mine_arms_detects_and_stuns() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.teams.insert(me, 0);
        g.teams.insert(them, 1);
        g.credits.clear();
        let mag = "658d84f5712baa05";
        g.on_message(me, SPAWN_MAGAZINE, &unhex(&format!("f134d5099974113f01000000211e8241da8ec03faea66dc1e9f091a8b1d0e8c8{mag}")), t0);
        g.on_message(me, MAGAZINE_STATE, &unhex(&format!("3c7045ed2b0f6742{mag}1b0b211b29d4d241")), t0);
        g.on_message(me, MAGAZINE_STATE, &unhex(&format!("3c7045ed2b0f6742{mag}6388fd280387e323")), t0);
        // It stuck a metre from slot 1.
        g.thrown_at.insert(me, [45.1, 1.0, -25.3]);
        let stuns = |v: &[(SocketAddr, Out)]| v.iter().filter(|(_, o)| o.ty == STUN_MSG).count();
        assert_eq!(stuns(&g.due(t0 + Duration::from_secs(1))), 0, "not armed yet");
        assert_eq!(stuns(&g.due(t0 + Duration::from_millis(3600))), 0, "set off: goes off 1 s later");
        let v = g.due(t0 + Duration::from_millis(4700));
        assert_eq!(stuns(&v), 1);
        assert!(v.iter().any(|(_, o)| o.ty == MAGAZINE_STATE && o.body[16..] == MAG_USED));
        assert!(v.iter().any(|(_, o)| o.ty == COOLDOWN_ACTIVE));
        assert_eq!(g.credits, vec![Credit::Stun { by: me }]);
        assert!(g.mines.is_empty());
    }

    /// Shooting an ARC MINE: health down from 9999 to everyone (native), and
    /// it is gone, without stunning: when its owner says it is used, or by the
    /// server shortly after if that never comes. A teammate's shot does nothing.
    #[test]
    fn arc_mines_can_be_shot_down() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.teams.insert(me, 0);
        g.teams.insert(them, 1);
        let mag = "658d84f5712baa05";
        g.on_message(me, SPAWN_MAGAZINE, &unhex(&format!("f134d5099974113f01000000211e8241da8ec03faea66dc1e9f091a8b1d0e8c8{mag}")), t0);
        g.on_message(me, MAGAZINE_STATE, &unhex(&format!("3c7045ed2b0f6742{mag}6388fd280387e323")), t0);
        g.thrown_at.insert(me, [45.1, 1.0, -25.3]);
        // A PULSAR shot on the mine (target = the mine, in the area's space).
        let mut bc = head_shot_on_slot_1();
        bc[32..40].copy_from_slice(&unhex(mag));
        bc[40..44].copy_from_slice(&AREA[..4]);
        bc[48] = 0;
        let mut own_team = bc.clone();
        own_team[56] = 0xf2;
        g.teams.insert(them, 0);
        assert!(g.on_message(them, BULLET_COLLISION, &own_team, t0).is_empty(), "a teammate's shot");
        g.teams.insert(them, 1);
        let v = g.on_message(them, BULLET_COLLISION, &bc, t0);
        assert!(v.iter().any(|o| o.ty == DEAL_DAMAGE), "{v:?}");
        // Its owner never says so: the server takes it down, with no stun.
        let v = g.due(t0 + Duration::from_secs(5));
        assert!(v.iter().all(|(_, o)| o.ty != STUN_MSG));
        assert!(v.iter().any(|(_, o)| o.ty == MAGAZINE_STATE && o.body[16..] == MAG_USED));
        assert!(g.mines.is_empty());
    }

    /// A Detonator going off by an enemy ARC MINE takes it down (no stun);
    /// one far away leaves it be.
    #[test]
    fn blasts_take_arc_mines_down() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.teams.insert(me, 0);
        g.teams.insert(them, 1);
        let mag = "658d84f5712baa05";
        g.on_message(me, SPAWN_MAGAZINE, &unhex(&format!("f134d5099974113f01000000211e8241da8ec03faea66dc1e9f091a8b1d0e8c8{mag}")), t0);
        g.on_message(me, MAGAZINE_STATE, &unhex(&format!("3c7045ed2b0f6742{mag}6388fd280387e323")), t0);
        g.thrown_at.insert(me, [50.0, 1.0, -25.3]);
        g.due(t0);
        let none = |_: f32| 10.0;
        assert!(g.blast_mines(them, [60.0, 1.0, -25.3], 2.5, &none, t0).is_empty(), "too far");
        assert!(g.blast_mines(me, [50.5, 1.0, -25.3], 2.5, &none, t0).iter().any(|o| o.ty == DEAL_DAMAGE), "its own thrower's blast");
        let v = g.blast_mines(them, [50.5, 1.0, -25.3], 2.5, &none, t0);
        assert!(v.iter().any(|o| o.ty == DEAL_DAMAGE));
        let v = g.due(t0 + Duration::from_secs(1));
        assert!(v.iter().all(|(_, o)| o.ty != STUN_MSG));
        assert!(g.mines.is_empty());
    }

    /// A DRONE starts at 70 health: an enemy's shots take it to 0 (native),
    /// a teammate's do nothing, and a blast near it hurts it too.
    #[test]
    fn drones_can_be_shot_down() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.teams.insert(me, 0);
        g.teams.insert(them, 1);
        let mag = "84b30407aabbccdd";
        g.on_message(me, SPAWN_MAGAZINE, &unhex(&format!("f234d5099974113f01000000211e8241da8ec03faea66dc1b54a80cbdec72de3{mag}")), t0);
        g.on_message(me, MAGAZINE_STATE, &unhex(&format!("3c7045ed2b0f6742{mag}a9509fd4dfc72de3")), t0);
        g.thrown_at.insert(me, [50.0, 3.0, -25.3]);
        let mut bc = head_shot_on_slot_1();
        bc[32..40].copy_from_slice(&unhex(mag));
        bc[40..44].copy_from_slice(&AREA[..4]);
        bc[48] = 0;
        let left = |v: &[Out]| v.iter().find(|o| o.ty == DEAL_DAMAGE).map(|o| f32::from_le_bytes(o.body[16..20].try_into().expect("4")));
        let v = g.on_message(them, BULLET_COLLISION, &bc, t0);
        let first = left(&v).expect("dealt");
        assert!(first > 50.0 && first < 70.0, "from 70: {first}");
        g.teams.insert(them, 0);
        assert!(g.on_message(them, BULLET_COLLISION, &bc, t0).is_empty(), "a teammate's shot");
        g.teams.insert(them, 1);
        let v = g.blast_mines(them, [50.5, 3.0, -25.3], 1.75, &|_| 40.0, t0);
        assert_eq!(left(&v), Some(first - 40.0));
        for _ in 0..4 {
            g.on_message(them, BULLET_COLLISION, &bc, t0);
        }
        assert_eq!(g.area_health.get(&arr8(&unhex(mag), 0)), Some(&0.0));
        assert!(g.blast_mines(them, [50.5, 3.0, -25.3], 1.75, &|_| 40.0, t0).is_empty(), "down: no more");
    }

    /// A shot whose line passes through an enemy's DRONE hits it, whether or
    /// not the shooter's client reports the hit; the client's own report of
    /// that hit then counts nothing more, and at 0 the drone is ended now.
    #[test]
    fn shots_through_a_drone_take_it_down() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.teams.insert(me, 0);
        g.teams.insert(them, 1);
        let mag = "84b30407aabbccdd";
        g.on_message(me, SPAWN_MAGAZINE, &unhex(&format!("f234d5099974113f01000000211e8241da8ec03faea66dc1b54a80cbdec72de3{mag}")), t0);
        g.on_message(me, MAGAZINE_STATE, &unhex(&format!("3c7045ed2b0f6742{mag}a9509fd4dfc72de3")), t0);
        g.thrown_at.insert(me, [50.0, 3.0, -25.3]);
        // From 6 m off along z, aimed 0.3 m wide of its centre.
        let mut shot = vec![0u8; 64];
        for (k, x) in [50.3f32, 3.0, -19.3].iter().enumerate() {
            shot[20 + 4 * k..24 + 4 * k].copy_from_slice(&x.to_le_bytes());
        }
        for (k, x) in [0.0f32, 0.0, -80.0].iter().enumerate() {
            shot[36 + 4 * k..40 + 4 * k].copy_from_slice(&x.to_le_bytes());
        }
        shot[48..52].copy_from_slice(&20f32.to_le_bytes());
        let left = |v: &[Out]| v.iter().find(|o| o.ty == DEAL_DAMAGE).map(|o| f32::from_le_bytes(o.body[16..20].try_into().expect("4")));
        assert_eq!(left(&g.on_message(them, FIRE_GUN, &shot, t0)), Some(50.0));
        // Its client's own hit report on it: already counted.
        let mut bc = head_shot_on_slot_1();
        bc[32..40].copy_from_slice(&unhex(mag));
        bc[40..44].copy_from_slice(&AREA[..4]);
        assert!(g.on_message(them, BULLET_COLLISION, &bc, t0).is_empty());
        // A teammate's shot through it does nothing; a wide one misses.
        g.teams.insert(them, 0);
        assert_eq!(left(&g.on_message(them, FIRE_GUN, &shot, t0)), None);
        g.teams.insert(them, 1);
        let mut wide = shot.clone();
        wide[20..24].copy_from_slice(&51.0f32.to_le_bytes());
        assert_eq!(left(&g.on_message(them, FIRE_GUN, &wide, t0)), None);
        // Down: its used state now, the destroy just after.
        g.on_message(them, FIRE_GUN, &shot, t0);
        let v = g.on_message(them, FIRE_GUN, &shot, t0);
        assert_eq!(left(&v), Some(10.0));
        let v = g.on_message(them, FIRE_GUN, &shot, t0);
        assert_eq!(left(&v), Some(0.0));
        assert!(v.iter().any(|o| o.ty == MAGAZINE_STATE && o.body[16..24] == MAG_USED));
        let later = g.due(t0 + Duration::from_millis(300));
        assert!(later.iter().any(|(_, o)| o.ty == DESTROY_MAGAZINE && o.body == unhex(mag)));
        assert!(!g.mags.contains_key(&arr8(&unhex(mag), 0)));
    }

    /// NEUTRON: a wall from a player with `point` equipped stuns the enemy
    /// within 2.5 m 1.25 s later, credited to its owner; another tac mod's
    /// wall (an ENERGY BARRIER) does nothing.
    #[test]
    fn neutron_stuns_enemies_after_its_charge() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.teams.insert(me, 0);
        g.teams.insert(them, 1);
        g.credits.clear();
        let mut equip = unhex("ffffffff000000009e5b615f8c9cd62fa44a86d6cac72de3fee794a8b1d0e8c80100000000000000ffffffffffffffffffffffffffff0000");
        g.set_equipment(me, &equip);
        // The recording's wall, moved a metre from slot 1.
        let mut wall = unhex("6c602cbed96a183fcf2a563ec5db413f99a981c1bac2344089c5ecc00000803f262929d5a48c8b3d193a620c7adae243b6bd71bb33b275acf434d5099874113f8652624a9681de2f6b59d5be124a87bed1a2b3bd740000000100000000000000");
        for (k, x) in [45.1f32, 1.0, -25.3].iter().enumerate() {
            wall[16 + 4 * k..20 + 4 * k].copy_from_slice(&x.to_le_bytes());
        }
        g.wall_placed(me, &wall, t0);
        let stuns = |v: &[(SocketAddr, Out)]| v.iter().filter(|(_, o)| o.ty == STUN_MSG).count();
        assert_eq!(stuns(&g.due(t0 + Duration::from_secs(1))), 0, "still charging");
        let v = g.due(t0 + Duration::from_millis(1300));
        assert_eq!(stuns(&v), 1);
        assert!(v.iter().any(|(_, o)| o.ty == EXPLOSION_PLAYER_DAMAGE && o.body[8..16] == EX_STUN));
        assert_eq!(g.credits, vec![Credit::Stun { by: me }]);
        assert!(g.neutrons.is_empty());
        // It goes off around its user, wherever the wall is: 2 m from them,
        // with the wall 20 m away, they are stunned; 3 m off they are not.
        let far = |x: f32| {
            let mut w = wall.clone();
            w[16..20].copy_from_slice(&x.to_le_bytes());
            w
        };
        g.observe_physics(them, &body(1, [43_800, 1_870, -23_300]));
        g.wall_placed(me, &far(65.0), t0);
        assert_eq!(stuns(&g.due(t0 + Duration::from_millis(1300))), 1, "2 m from its user");
        g.observe_physics(them, &body(1, [43_800, 1_870, -22_300]));
        g.wall_placed(me, &far(65.0), t0);
        assert_eq!(stuns(&g.due(t0 + Duration::from_millis(1300))), 0, "3 m: out of range");
        // Not through a wall: an ENERGY BARRIER between them shields.
        g.observe_physics(them, &body(1, [43_800, 1_870, -23_300]));
        let mut shield = wall.clone();
        shield[0..16].copy_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x3f]);
        for (k, x) in [43.8f32, 1.6, -24.3].iter().enumerate() {
            shield[16 + 4 * k..20 + 4 * k].copy_from_slice(&x.to_le_bytes());
        }
        shield[48] ^= 0x55;
        let mut barrier_kit = equip.clone();
        barrier_kit[16..24].copy_from_slice(&unhex("1a0b261335d4d241"));
        g.set_equipment(them, &barrier_kit);
        g.wall_placed(them, &shield, t0);
        g.wall_placed(me, &far(65.0), t0);
        assert_eq!(stuns(&g.due(t0 + Duration::from_millis(1300))), 0, "behind a wall");
        g.walls.clear();
        g.credits.clear();
        // ENERGY BARRIER (`shield`): a wall, no blast.
        equip[16..24].copy_from_slice(&unhex("1a0b261335d4d241"));
        g.set_equipment(me, &equip);
        g.wall_placed(me, &wall, t0);
        assert!(g.neutrons.is_empty());
        // ...unless EchoCombat's `P` token says it was a NEUTRON, before or after.
        g.point_token(me, t0 + Duration::from_millis(200));
        assert_eq!(g.neutrons.len(), 1, "token after the wall");
        g.neutrons.clear();
        g.point_token(me, t0);
        g.wall_placed(me, &wall, t0 + Duration::from_millis(100));
        assert_eq!(g.neutrons.len(), 1, "token before the wall");
        g.neutrons.clear();
        g.wall_placed(me, &wall, t0 + Duration::from_secs(5));
        g.point_token(me, t0 + Duration::from_secs(9));
        assert!(g.neutrons.is_empty(), "too far apart");
    }

    /// STUN FIELD: 1 m when it goes off, 4 m from 0.5 s on (the settings).
    #[test]
    fn stun_field_grows_to_its_settings_size() {
        assert!((stun_reach(Duration::ZERO) - 1.0).abs() < 0.01);
        let mid = stun_reach(Duration::from_millis(250));
        assert!(mid > 2.5 && mid < 4.0, "{mid}");
        assert!((stun_reach(Duration::from_secs(1)) - 4.0).abs() < 0.01);
        assert!((stun_reach(Duration::MAX) - 4.0).abs() < 0.01);
    }

    /// A STUN FIELD never stuns its own thrower, even one standing in it
    /// (it opens where the grenade was last seen: on a short throw, the hand).
    #[test]
    fn stun_field_spares_its_thrower() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.teams.insert(me, 0);
        g.teams.insert(them, 1);
        let at = g.bodies[&me].1;
        let p = g.bodies[&them].0;
        g.bodies.insert(them, (p, [at[0] + 1.0, at[1], at[2]]));
        g.fields.push(Field { kind: EX_STUN, mag: [0; 8], at: [at[0], at[1] - 1.5, at[2]], thrower: me, until: t0 + Duration::from_secs(6), next: t0, every: Duration::from_millis(8), radius: 4.0, start: t0, hit: Vec::new(), moved: t0, tracked: false });
        g.due(t0 + Duration::from_secs(1));
        let mine = g.id_of(&me, g.bodies[&me].0);
        let hit = &g.fields[0].hit;
        assert!(hit.contains(&g.id_of(&them, p)), "the enemy");
        assert!(!hit.contains(&mine), "not the thrower");
    }

    /// Cheat death follows the settings: EchoCombat's 5 s by default.
    #[test]
    fn cheat_death_is_the_settings_seconds() {
        assert_eq!(cheat_death(), Duration::from_secs(5));
    }

    /// A DETONATOR stuck to a wall (it flew in, then stopped) hurts the side
    /// it came from, not a player behind the wall; one in the open, or a
    /// player right on it, is hit.
    #[test]
    fn detonator_stuck_to_a_wall_does_not_hit_through_it() {
        let head = |x: f32| [x, 1.7, 0.0];
        // Flew along +x into a wall at x = 0.
        let (at, from) = ([0.0, 1.0, 0.0], [-0.3, 1.05, 0.0]);
        assert!(behind_surface(at, from, head(1.0)), "the far side");
        assert!(!behind_surface(at, from, head(-1.0)), "the near side");
        assert!(!behind_surface(at, from, head(0.3)), "right on it");
        // Thrown onto the floor: players around and above it are hit.
        assert!(!behind_surface([0.0, 0.0, 0.0], [0.0, 0.3, 0.2], head(1.2)));
    }

    /// The shield (native Fission, 2026-11-16): a full-charge COMET head hit
    /// (60 x 2) on a full shield leaves 0 shield and 7 health, alive; the next
    /// hit kills. A hit of twice the shield or more reaches the health at once.
    #[test]
    fn a_full_shield_survives_any_single_hit() {
        assert_eq!(split_hit(112.0, 7.0, 120.0), (112.0, 0.0));
        assert_eq!(split_hit(112.0, 7.0, 223.0), (112.0, 0.0));
        assert_eq!(split_hit(20.89, 7.0, 84.0), (20.89, 7.0), "native Dyson 113.69: shield 20.89, head hit");
        assert_eq!(split_hit(0.0, 7.0, 4.0), (0.0, 4.0));
        let (mut g, me, them, _) = two_in_the_area();
        let v = g.hurt(them, 120.0, PART_HEAD, [0.0; 3], [1; 8], Some(me));
        assert!(v.iter().all(|o| o.ty != PLAYER_KILLED), "a charged COMET head hit does not kill");
        let hb = v.iter().find(|o| o.ty == HEALTH_BROADCAST).expect("hurt");
        assert_eq!(&hb.body[12..20], [0f32.to_le_bytes(), 7f32.to_le_bytes()].concat().as_slice());
        let v = g.hurt(them, 30.0, PART_HEAD, [0.0; 3], [1; 8], Some(me));
        assert!(v.iter().any(|o| o.ty == PLAYER_KILLED), "the next one does");
    }

    /// A DETONATOR's line of sight: a wall of the map between it and a player
    /// blocks it, as does a standing barrier's box; open air does not.
    #[test]
    fn walls_and_barriers_block_the_line() {
        let mut g = Gear::default();
        g.set_match(super::super::combat::LEVEL_COMBUSTION);
        let w = super::super::walls::of(super::super::combat::LEVEL_COMBUSTION).expect("walls");
        let (c, n) = w.face(1000);
        let (a, b) = ([c[0] - n[0], c[1] - n[1], c[2] - n[2]], [c[0] + n[0], c[1] + n[1], c[2] + n[2]]);
        assert!(g.line_blocked(a, b, BLAST_SKIP), "through a wall");
        assert!(!g.line_blocked(b, [b[0] + n[0] * 0.3, b[1] + n[1] * 0.3, b[2] + n[2] * 0.3], BLAST_SKIP), "away from it");
        let bar = Barrier { space: [0; 8], pool: 1, handle: 1, max: 100.0, respawn: 0.0, pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0], half: [1.0, 1.0, 0.1], off_z: 0.0 };
        assert!(bar.crosses([0.0, 0.0, -2.0], [0.0, 0.0, 2.0], 0.15));
        assert!(!bar.crosses([3.0, 0.0, -2.0], [3.0, 0.0, 2.0], 0.15));
    }

    #[test]
    fn neutron_symbol_is_point() {
        assert_eq!(crate::sns::symbol("point"), TAC_POINT);
    }

    /// A teammate walking by does not set it off; picking it back up removes it.
    #[test]
    fn arc_mine_ignores_teammates_and_can_be_picked_up() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.teams.insert(me, 0);
        g.teams.insert(them, 0);
        let mag = "658d84f5712baa05";
        g.on_message(me, SPAWN_MAGAZINE, &unhex(&format!("f134d5099974113f01000000211e8241da8ec03faea66dc1e9f091a8b1d0e8c8{mag}")), t0);
        g.on_message(me, MAGAZINE_STATE, &unhex(&format!("3c7045ed2b0f6742{mag}6388fd280387e323")), t0);
        g.thrown_at.insert(me, [45.1, 1.0, -25.3]);
        g.due(t0 + Duration::from_secs(4));
        let v = g.due(t0 + Duration::from_secs(6));
        assert!(v.iter().all(|(_, o)| o.ty != STUN_MSG));
        g.on_message(me, MAGAZINE_STATE, &unhex(&format!("3c7045ed2b0f6742{mag}1b0b211b29d4d241")), t0);
        assert!(g.mines.is_empty());
    }

    /// An opponent in range is not healed.
    #[test]
    fn repair_matrix_skips_the_other_team() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
        g.teams.insert(me, 0);
        g.teams.insert(them, 1);
        g.credits.clear();
        g.on_message(me, HEAL_BUBBLE_ACTIVATE, &unhex("8ec21d00d315993fd1eb3d3fa8a2fdb10000000000000000"), t0);
        g.due(t0 + Duration::from_secs(2));
        assert_eq!(g.lives.get(&them).map(|l| l.health), Some(112.0 - 13.5));
        assert!(g.credits.is_empty());
    }

    /// OVERCHARGE's heal: 3 health over 10 s for a linked teammate, no more
    /// than 3, credited to the user; the user is not healed.
    #[test]
    fn overcharge_heals_its_user_and_linked_teammates_a_little() {
        let (mut g, me, them, t0) = two_in_the_area();
        g.teams.insert(me, 0);
        g.teams.insert(them, 0);
        // Both hurt (no teams yet for the shots), then on one team.
        g.teams.clear();
        g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
        let mut back = head_shot_on_slot_1();
        back[32] = 0xf2;
        back[56] = 0xf3;
        g.on_message(them, BULLET_COLLISION, &back, t0);
        g.teams.insert(me, 0);
        g.teams.insert(them, 0);
        let before = |g: &Gear, a: SocketAddr| g.lives.get(&a).map_or(0.0, |l| l.health);
        let (me0, them0) = (before(&g, me), before(&g, them));
        g.on_message(me, HEAL_BUBBLE_ACTIVATE, &unhex("f234d5099974113f41e35b49673283b9f234d5099974113f"), t0);
        // The user's client links slot 1.
        let mut link = unhex("8ec21d00d315993ff334d5099974113f0000000000000000000000000000000000");
        link.truncate(32);
        link[16..20].copy_from_slice(&(-4_090_010f32).to_le_bytes());
        g.on_message(me, DAMAGE_DEALT, &link, t0);
        g.credits.clear();
        g.due(t0 + Duration::from_secs(5));
        let healed = |g: &Gear| g.credits.iter().map(|c| if let Credit::Heal { by, amount } = c { assert_eq!(*by, me); *amount } else { 0.0 }).sum::<f32>();
        assert!((before(&g, them) - them0 - 1.5).abs() < 0.05, "half of 3 in 5 s: {}", before(&g, them) - them0);
        assert!(before(&g, me) <= me0, "the user is not healed");
        // Past its 10 s: 3 each at most, then nothing more.
        g.due(t0 + Duration::from_secs(10));
        g.due(t0 + Duration::from_secs(12));
        assert!((before(&g, them) - them0 - 3.0).abs() < 0.05);
        assert!((healed(&g) - 3.0).abs() < 0.05);
        assert!(g.oc_heals.is_empty());
    }

    /// OVERCHARGE: the user's hits deal +1.5 (+2.5 on the head) and they take 25%
    /// less; a linked teammate takes 25% less and cannot die for 6.5 s (`cheat_death_seconds`).
    #[test]
    fn overcharge_boosts_cuts_and_saves() {
        let (mut g, me, them, t0) = two_in_the_area();
        let oc = unhex("f234d5099974113f41e35b49673283b9f234d5099974113f");
        g.on_message(me, HEAL_BUBBLE_ACTIVATE, &oc, t0);
        let v = g.on_message(me, BULLET_COLLISION, &head_shot_on_slot_1(), t0);
        let hb = v.iter().find(|o| o.ty == HEALTH_BROADCAST).expect("hurt");
        let head = weapons().pointer("/ability_properties/buff/damage_bonus_head").and_then(Value::as_f64).unwrap_or(2.0) as f32;
        assert_eq!(&hb.body[12..16], &(112.0f32 - (13.5 + head)).to_le_bytes(), "13.5 + the head bonus");
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
        assert_eq!(g.lives.get(&them).map(|l| (l.health, l.core)), Some((0.0, 1.0)), "no shield, 1 health");
        // The same use linking them again and again: one cheat death only.
        let five = t0 + cheat_death();
        assert!(g.cheats_death(them, five - Duration::from_millis(500)));
        for k in 5..10 {
            g.on_message(me, DAMAGE_DEALT, &link, t0 + Duration::from_secs(k));
        }
        assert!(!g.cheats_death(them, five + Duration::from_millis(100)), "not renewed by later links");
        // A new use later: a new cheat death.
        let next = t0 + Duration::from_secs(20);
        g.on_message(me, DAMAGE_DEALT, &link, next);
        assert!(g.cheats_death(them, next + Duration::from_secs(1)));
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

    /// A rejoined player (generation 1): the regen broadcasts carry the
    /// generation, or the game takes none of them (2026-11-20: health stuck
    /// low until the next hit's broadcast showed it nearly full).
    #[test]
    fn regen_names_a_rejoined_player_with_the_generation() {
        let (mut g, _, them, _) = two_in_the_area();
        g.gens.insert(them, 1);
        g.hurt(them, 50.0, HURT_EXPLOSION, [0.0; 3], MARK_ROCKET, None);
        let t = Instant::now();
        let v = g.due(t + Duration::from_millis(12_500));
        let hb = &v.iter().find(|(w, o)| *w == them && o.ty == HEALTH_BROADCAST).expect("regen").1;
        assert_eq!(&hb.body[8..12], &0x1_0001u32.to_le_bytes());
    }

    /// In a match the server respawns the dead; a lagging client's own late
    /// ask, once it is alive again, is not sent out as a second respawn.
    #[test]
    fn a_late_respawn_ask_from_a_live_player_is_dropped() {
        let (mut g, _, them, t0) = two_in_the_area();
        g.set_match(super::super::combat::LEVEL_GAUSS);
        let ask = unhex("8ec21d00d315993f0100000000000000");
        assert!(g.on_message(them, PLAYER_RESPAWN, &ask, t0).is_empty());
        g.lives.entry(them).or_default().dead = true;
        let v = g.on_message(them, PLAYER_RESPAWN, &ask, t0);
        assert_eq!(v[0].ty, PLAYER_RESPAWN, "dead: taken");
        assert!(g.on_message(them, PLAYER_RESPAWN, &ask, t0).is_empty(), "then a repeat is not");
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
