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

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rad_protocol::symbol::MatSym;
use serde_json::Value;
use tracing::info;

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

/// Physics objects (bytes 4..8 of a 64-byte `PhysicsSimStateEvent`).
const BODY_OBJECT: [u8; 4] = [0x8e, 0xc2, 0x1d, 0x00];
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
/// A player's damageable id is this with its low byte xor the slot.
const PLAYER_ID: [u8; 8] = [0xf2, 0x34, 0xd5, 0x09, 0x99, 0x74, 0x11, 0x3f];
const NO_PLAYER: [u8; 8] = [0xff; 8];
/// The damageable id every health message starts with.
const DAMAGEABLE: [u8; 8] = [0x8e, 0xc2, 0x1d, 0x00, 0xd3, 0x15, 0x99, 0x3f];
/// A dead player's client asks to respawn after 6.5 s; if it never does, the
/// server brings them back itself.
const RESPAWN_AFTER: Duration = Duration::from_secs(10);
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
const PLAYER_CODE: u32 = 0x17;
/// OVERCHARGE's heal bubble (`HealBubbleParamUpdate` bytes 8..16: heal 3, 10 s, radius 12).
const OVERCHARGE_BUBBLE: [u8; 8] = [0x41, 0xe3, 0x5b, 0x49, 0x67, 0x32, 0x83, 0xb9];
/// Full health, as native broadcasts it.
const FULL_HEALTH: f32 = 112.0;
/// The position native reports is the body's, a little lower.
const BODY_DROP: f32 = 0.35;
const BODY_RADIUS: f32 = 0.3;
/// How far from a body's reported point a bullet's hit point may be (head
/// to feet, arms out).
const SHOT_REACH: f32 = 1.2;
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
        [0xe9, 0xed, 0x9c, 0xa8, 0xb1, 0xd0, 0xe8, 0xc8] => "burst",
        _ => "",
    };
    let secs = weapons().pointer(&format!("/grenade_properties/{name}/cooldown")).and_then(Value::as_f64).unwrap_or(30.0);
    Duration::from_secs_f64(secs.clamp(1.0, 600.0))
}

fn lifetime(item: [u8; 8]) -> Duration {
    match item {
        [0xbe, 0x0b, 0x67, 0x29, 0x48, 0x3e, 0xc3, 0xc8] => Duration::from_secs(6),
        [0xfe, 0xed, 0x94, 0xa8, 0xb1, 0xd0, 0xe8, 0xc8] => Duration::from_secs(8),
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

pub struct Gear {
    bits: [[u8; 4]; BITS],
    mags: HashMap<[u8; 8], Mag>,
    later: Vec<(Instant, SocketAddr, Out)>,
    /// Each player's slot and body position (metres).
    bodies: HashMap<SocketAddr, (u16, [f32; 3])>,
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
            bits,
            mags: HashMap::new(),
            later: Vec::new(),
            bodies: HashMap::new(),
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
        }
    }

    /// A punch stuns only when the puncher is in the combat area.
    pub fn stun_allowed(&self, from: SocketAddr, _d: &[u8]) -> bool {
        self.inside.contains(&from)
    }

    /// Whether this player may fight: in the combat area, or (from their
    /// body's position) in the brawler room. Native lifts the lobby's cease
    /// fire (`VAR_HOLD` 0) for them there; without it they cannot punch.
    pub fn may_fight(&self, who: SocketAddr) -> bool {
        self.inside.contains(&who) || self.bodies.get(&who).is_some_and(|(_, p)| in_brawler_room(*p))
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
        let mut b = AREA.to_vec();
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
        } else if d[4..8] == AREA_OBJECT {
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
        // The leaver itself is gone: these go to the others.
        outs.into_iter().map(|o| Out { to: To::Others, ..o }).collect()
    }

    /// Everything an explosion at `at` reaches: (victim id, reported position, distance, player).
    fn victims(&self, at: [f32; 3], radius: f32) -> Vec<Hit> {
        let mut v = Vec::new();
        for (a, (slot, p)) in &self.bodies {
            // Only players in the combat area can be hit (no stun or damage in the lobby).
            if !self.inside.contains(a) {
                continue;
            }
            let d = dist(at, *p);
            if d <= radius + BODY_RADIUS {
                let mut id = PLAYER_ID;
                id[0] ^= *slot as u8;
                v.push((id, [p[0], p[1] - BODY_DROP, p[2]], d, Some(*a), PLAYER_CODE));
            }
        }
        for (p, code) in DUMMIES {
            let d = dist(at, *p);
            if d <= radius + DUMMY_RADIUS {
                v.push((NO_PLAYER, *p, d, None, *code));
            }
        }
        v
    }

    /// Damage to a player: their new health to everyone, and `PlayerKilled` at 0.
    /// A player's slot, from their body physics.
    fn slot_of(&self, who: &SocketAddr) -> u32 {
        self.bodies.get(who).map_or(0, |b| u32::from(b.0))
    }

    /// The player whose body is nearest a bullet's hit point (not the shooter).
    fn shot_player(&self, shooter: SocketAddr, at: [f32; 3]) -> Option<SocketAddr> {
        self.bodies
            .iter()
            .filter(|(a, _)| **a != shooter && self.inside.contains(a))
            .map(|(a, (_, p))| (*a, dist(at, [p[0], p[1] - BODY_DROP, p[2]])))
            .filter(|(_, d)| *d <= SHOT_REACH)
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .map(|(a, _)| a)
    }

    fn hurt(&mut self, who: SocketAddr, amount: f32, kind: u32, pos: [f32; 3], cause: [u8; 8], by: Option<SocketAddr>) -> Vec<Out> {
        // The id is the same for every player; a client's own messages only confirm it.
        let owner = self.bubble_owner.get(&who).copied().unwrap_or(DAMAGEABLE);
        let slot = self.slot_of(&who);
        let killer = by.map_or(slot, |a| self.slot_of(&a));
        let life = self.lives.entry(who).or_default();
        if life.dead || amount <= 0.0 {
            return Vec::new();
        }
        life.health = (life.health - amount).max(0.0);
        life.hurt = Some((kind, pos, cause));
        life.lost_at = Some(Instant::now());
        life.regen_at = None;
        let mut v = Vec::new();
        if life.health <= 0.0 {
            life.dead = true;
            life.died_at = Some(Instant::now());
            info!(%who, cause = %hex8(cause), "player killed");
            // `[id][cause][victim slot][killer slot]` (Gauss match).
            v.push(out(To::All, PLAYER_KILLED, &[owner.as_slice(), &cause, &slot.to_le_bytes(), &killer.to_le_bytes()].concat()));
        }
        let life = *life;
        v.push(out(To::All, HEALTH_BROADCAST, &health_body(owner, slot, &life, false)));
        v
    }

    /// One explosion: `ExplosionPlayerDamage` per victim, else `NothingHit`.
    fn explode(&self, kind: [u8; 8], id: [u8; 8], at: [f32; 3], radius: f32) -> (Vec<Out>, Vec<Hit>) {
        let hits = self.victims(at, radius);
        info!(kind = %hex8(kind), x = at[0], y = at[1], z = at[2], radius, hits = hits.len(), "explosion");
        if hits.is_empty() {
            let b = [AREA.as_slice(), &kind, &id].concat();
            return (vec![out(To::All, EXPLOSION_NOTHING_HIT, &b)], hits);
        }
        let outs = hits.iter().map(|(victim, pos, ..)| out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(kind, *pos, *victim, id))).collect();
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
        match ty {
            TELEPORT_REQUEST => vec![out(To::Sender, TELEPORT, d)],
            BITFIELD_REQUEST if d.len() == 8 && d == AREA => vec![out(To::Sender, BITFIELD_RESPONSE, &self.bitfield())],
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
            BULLET_COLLISION if d.len() == 64 => {
                self.using_gear(from, now);
                let marker = tracked_damage(d);
                let mut v = vec![out(To::Sender, TRACKED_DAMAGE, &marker)];
                // The collision names the gun and the shooter, not the victim:
                // whoever's body is at the hit point takes the damage.
                let at = f3(d, 20);
                if let Some(victim) = self.shot_player(from, at) {
                    let amount = f32::from_le_bytes(marker[16..20].try_into().expect("4"));
                    let part = u32::from_le_bytes(d[48..52].try_into().expect("4"));
                    info!(%from, %victim, amount, part, "shot hit a player");
                    v.extend(self.hurt(victim, amount, part, at, arr8(d, 0), Some(from)));
                }
                v
            }
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
                    let f = move |dd: f32| full * if dd <= near { 1.0 } else { (1.0 - (dd - near) / (far + BODY_RADIUS - near).max(0.01)).clamp(0.0, 1.0) };
                    (far, MARK_ROCKET, Box::new(f))
                } else {
                    (1.0, MARK_ROCKET, Box::new(|_| 0.0))
                };
                let (mut v, hits) = self.explode(kind, id, at, radius);
                for (_, pos, dd, who, code) in hits {
                    let own = who == Some(from);
                    let amount = damage(dd) * if own { own_scale } else { 1.0 };
                    // Native marks hits on others; a self-hit only costs health.
                    if !own {
                        v.push(out(To::Sender, TRACKED_DAMAGE, &marker(mark, amount, pos, code)));
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
                        let (mut o, mut hits) = self.explode(EX_DET, id, at, radius);
                        hits.sort_by(|x, y| x.2.total_cmp(&y.2));
                        if !hits.is_empty() {
                            o.truncate(0);
                            o.push(out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(EX_DET, hits[0].1, hits[0].0, id)));
                        }
                        v.extend(o);
                        for (_, pos, _, who, code) in hits {
                            if who != Some(thrower) {
                                self.later.push((now, thrower, out(To::Sender, TRACKED_DAMAGE, &marker(ITEM_DET, DET_DAMAGE, pos, code))));
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
        // A dead player whose client never asked to respawn.
        let stuck: Vec<SocketAddr> =
            self.lives.iter().filter(|(_, l)| l.dead && l.died_at.is_some_and(|t| now.duration_since(t) > RESPAWN_AFTER)).map(|(a, _)| *a).collect();
        for who in stuck {
            let slot = self.slot_of(&who);
            let owner = self.bubble_owner.get(&who).copied().unwrap_or(DAMAGEABLE);
            if let Some(life) = self.lives.get_mut(&who) {
                *life = Life::default();
                info!(%who, "dead player brought back by the server");
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
                let hits = self.victims(f.at, f.radius);
                if f.kind == EX_DOT {
                    let tick = weapons().pointer("/grenade_properties/dot/damage_per_tick").and_then(Value::as_f64).unwrap_or(4.0) as f32;
                    for (victim, pos, _, who, code) in &hits {
                        ready.push((f.thrower, out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(f.kind, *pos, *victim, f.mag))));
                        if *who != Some(f.thrower) {
                            ready.push((f.thrower, out(To::Sender, TRACKED_DAMAGE, &marker(ITEM_DOT, tick, *pos, *code))));
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
                    ready.push((f.thrower, out(To::All, EXPLOSION_NOTHING_HIT, &[AREA.as_slice(), &f.kind, &f.mag].concat())));
                }
                for (victim, pos, _, who, _) in fresh {
                    f.hit.push(victim);
                    ready.push((f.thrower, out(To::All, EXPLOSION_PLAYER_DAMAGE, &player_damage(f.kind, pos, victim, f.mag))));
                    let target: u64 = if who.is_some() { u64::from(victim[0] ^ PLAYER_ID[0]) } else { 0x0c };
                    ready.push((f.thrower, out(To::All, STUN_MSG, &[AREA.as_slice(), &target.to_le_bytes()].concat())));
                }
            }
        }
        fields.retain(|f| f.until > now);
        self.fields = fields;
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
            }
        }
        ready
    }
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
fn multiplier(base: f32, part: u64) -> f32 {
    let w = weapons();
    let weapon = w
        .get("weapon_properties")
        .and_then(Value::as_object)
        .and_then(|m| m.iter().find(|(_, v)| v.get("projectile_damage").and_then(Value::as_f64).is_some_and(|p| (p as f32 - base).abs() < 0.01)))
        .map_or("scout", |(k, _)| k.as_str());
    let part = match part {
        2 => "arm",
        5 => "head",
        _ => "torso",
    };
    w.pointer(&format!("/damage_table/{weapon}/{part}")).and_then(Value::as_f64).map_or(1.0, |m| m as f32)
}

/// `DamageableLocallyTrackedDamage` for a `DamageableBulletCollision`:
/// `[area 8][target 8][damage f32][hit point 12][kind u32][count u32]`.
fn tracked_damage(d: &[u8]) -> Vec<u8> {
    let base = f32::from_le_bytes(d[44..48].try_into().expect("4"));
    let part = u64::from_le_bytes(d[48..56].try_into().expect("8"));
    let mut b = AREA.to_vec();
    b.extend_from_slice(&d[0..8]);
    b.extend_from_slice(&(base * multiplier(base, part)).to_le_bytes());
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

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Position in a 64-byte physics update: three 19-bit millimetre fields from bit 267.
/// The lobby's brawler room (`mpl_lobby_b2`), from a native recording
/// (2026-11-16): native's cease-fire var dropped to 0 as the player passed
/// (19.7, 0.5, 10.4) going in and rose to 1 at (22.0, 1.0, 9.7) coming out;
/// the deepest point was (31.0, 0.2, 15.2). The combat area is at negative z.
pub fn in_brawler_room(p: [f32; 3]) -> bool {
    (19.5..=42.0).contains(&p[0]) && (-3.0..=5.0).contains(&p[1]) && (9.7..=26.0).contains(&p[2])
}

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
fn player_damage(kind: [u8; 8], pos: [f32; 3], victim: [u8; 8], id: [u8; 8]) -> Vec<u8> {
    let mut b = [AREA.as_slice(), &kind].concat();
    for x in pos {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&victim);
    b.extend_from_slice(&id);
    b
}

/// The shooter's hit marker: `LocallyTrackedDamage` naming what dealt it.
fn marker(what: [u8; 8], damage: f32, pos: [f32; 3], code: u32) -> Vec<u8> {
    let mut b = [AREA.as_slice(), &what].concat();
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
            assert_eq!(tracked_damage(&unhex(hit)), unhex(marker));
        }
    }

    #[test]
    fn area_events_enter_and_free_the_stations_after_a_delay() {
        // Native lobby recording 2026-11-16: no SetOwner, no LoadoutNumberNotification.
        let mut g = Gear::new();
        let t0 = Instant::now();
        assert_eq!(AREA_ENTER_EVENT.to_le_bytes(), [0xc0, 0x64, 0x71, 0xff, 0xde, 0x09, 0x5d, 0x63]);
        g.area_event(addr(), AREA_ENTER_EVENT, t0);
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
    fn brawler_room_matches_where_native_lifted_the_cease_fire() {
        // Native lobby recording 2026-11-16: positions around the var's flips.
        for p in [[19.65, 0.49, 10.36], [31.0, 0.16, 15.2], [26.3, 0.95, 11.9], [22.3, 0.98, 9.85]] {
            assert!(in_brawler_room(p), "{p:?}");
        }
        for p in [[0.0, 2.0, 0.0], [19.09, 0.54, 10.11], [21.31, 0.98, 9.35], [37.0, 1.9, -22.0], [16.6, 1.3, 6.4]] {
            assert!(!in_brawler_room(p), "{p:?}");
        }
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
        assert_eq!(tracked_damage(&bc), want);
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

    /// Two players in the area: a bullet at slot 1's body hurts slot 1 (its
    /// slot in the broadcast, not 0), and the kill names victim and killer.
    #[test]
    fn a_shot_hurts_the_player_it_hit() {
        let mut g = Gear::new();
        let t0 = Instant::now();
        let (me, them): (SocketAddr, SocketAddr) = ("127.0.0.1:7000".parse().expect("a"), "127.0.0.1:7001".parse().expect("a"));
        let params = unhex("8ec21d00d315993f41e35b49673283b9f234d5099974113f00000000000080bf0000803f0000404000004040000020418401404100000000");
        for (a, slot) in [(me, 0u8), (them, 1)] {
            g.observe_physics(a, &body(slot, [43_800 + 2_000 * i32::from(slot), 1_870, -25_300]));
            g.on_message(a, HEAL_BUBBLE_PARAMS, &params, t0);
            g.on_message(a, SET_OWNER, &[0; 8], t0);
        }
        // PULSAR head shot (9 x 1.5) landing on slot 1's head.
        let mut bc = unhex("9e5b615f8c9cd62f95b2924260e454bd12132ac2fb362f429749c23fcc77cac1170000000100000026452bfc000010410500000000000000f234d5099974113f");
        for (k, x) in [45.8f32, 1.52, -25.3].iter().enumerate() {
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
        // A miss (nobody near) hurts nobody.
        bc[20..24].copy_from_slice(&80f32.to_le_bytes());
        assert!(g.on_message(them, BULLET_COLLISION, &bc, t0).iter().all(|o| o.ty != HEALTH_BROADCAST));
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
