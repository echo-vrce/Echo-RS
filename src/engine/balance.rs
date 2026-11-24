//! The balance settings block (`SR15NetBalanceSettingsMassUpdate`) and the
//! host's changes to it.
//!
//! The block carries the game's balance files as JSON (weapons, tac mods,
//! ordnance, damage table, movement, arena rules ...); clients play with the
//! server's values. Body: u32 uncompressed size, then a zstd frame holding a
//! u64 document count N, N-1 u32 end offsets, N NUL-terminated documents: N-1
//! JSON documents and an index of their ids (16 upper-case hex digits each,
//! CSymbol64 of the balance file's name). `tools/balance_block.py` does the same
//! in Python.
//!
//! With EchoCombat on (the default; `--no-veilnet` turns it off) the
//! `mp_weapon_settings` sent is EchoCombat's, as EchoVRCE's combat servers send
//! it (`data/mp_weapon_settings_echocombat.json`, from a Gauss match on
//! 2026-11-16: the mod's gear tuning and extra keys). Its `medigun.enabled` is
//! always `echocombat.ini`'s `[server] medigun`, whatever else says.
//!
//! `--balance <file>` (the launcher's Settings page) gives changes as
//! `{"<file name or 16-hex-digit id>": {partial JSON}}`; each is merged into
//! that document in every captured block that has it.

use std::sync::OnceLock;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value};
use tracing::{info, warn};

static CHANGES: OnceLock<Map<String, Value>> = OnceLock::new();
static BASE: OnceLock<Base> = OnceLock::new();
static SERVER_ARENA: OnceLock<Map<String, Value>> = OnceLock::new();

/// `mp_arena_rules` values only this server has (the launcher's Settings page
/// shows them with the game's): kept out of what clients get.
pub const SERVER_ARENA_KEYS: [&str; 10] = [
    "goalie_ping_advantage_public",
    "goalie_ping_advantage_private",
    "team_only_voice_public",
    "team_only_voice_private",
    // RULES screen rows the game has no private setting for.
    "overtime_private",
    "goal_stops_time_private",
    "carry_points_over_private",
    "standard_chassis_private",
    "disc_curve_private",
    "disc_spawn_private",
];

/// One of `SERVER_ARENA_KEYS` as the host set it.
pub fn server_arena_flag(key: &str) -> Option<bool> {
    SERVER_ARENA.get()?.get(key)?.as_bool()
}

/// Every one of `SERVER_ARENA_KEYS` the host set, as one object.
pub fn server_arena() -> Value {
    Value::Object(SERVER_ARENA.get().cloned().unwrap_or_default())
}

pub(crate) const ECHOCOMBAT_WEAPONS: &str = include_str!("data/mp_weapon_settings_echocombat.json");
/// The gear list a native H server with EchoCombat sends (its social lobby, 2026-11-16):
/// the stock gear plus PROTOSTAR, QUARK, BLAZAR, STARBURST, UMBRAL VEIL, OVERCHARGE, ...
const ECHOCOMBAT_EQUIPMENT: &str = include_str!("data/mp_equipment_echocombat.json");
/// Today's stock `mp_weapon_settings` (a native server without EchoCombat,
/// Fission, 2026-11-21), for games without EchoCombat. EchoCombat's changes
/// UMBRAL VEIL (90 s, 0.85 opacity; stock 240 s, 0), QUARK's limbs and DOT
/// FIELD (no remote trigger, sticks to walls, 11 s, 2.5 m; stock waits for its
/// thrower's trigger, 8 s, 3.5 m), which a stock game's scripts don't handle.
/// The gear list is the same in both (`mp_equipment`).
const STOCK_WEAPONS: &str = include_str!("data/mp_weapon_settings_stock.json");

/// Gear only EchoCombat games can use: kept out of the loadout terminal of
/// games without it (`gear_table` list, item `name`).
pub const ECHOCOMBAT_ONLY_GEAR: [(&str, &str); 4] = [("tacmods", "smoke"), ("tacmods", "buff"), ("tacmods", "point"), ("ordnances", "drone")];

/// The gear list for games without EchoCombat: EchoCombat's without
/// UMBRAL VEIL, OVERCHARGE, NEUTRON and DRONE.
fn stock_equipment() -> String {
    let mut doc: Value = serde_json::from_str(ECHOCOMBAT_EQUIPMENT).expect("mp_equipment_echocombat.json");
    for (list, name) in ECHOCOMBAT_ONLY_GEAR {
        if let Some(Value::Array(items)) = doc.pointer_mut(&format!("/gear_table/{list}")) {
            items.retain(|i| i.get("name").and_then(Value::as_str) != Some(name));
        }
    }
    serde_json::to_string(&doc).expect("json")
}

/// Which weapon settings the blocks start from.
#[derive(Clone, Copy, Debug, Default)]
pub struct Base {
    pub echocombat: bool,
    pub medigun: bool,
    /// Today's stock gear tables (`STOCK_WEAPONS`), when not EchoCombat's.
    pub stock: bool,
}

/// Call once at startup, before any session (tests use the stock base).
pub fn set_base(base: Base) {
    let _ = BASE.set(base);
}

/// Loads the host's changes; call once at startup, before any session.
pub fn load(path: &std::path::Path) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let Value::Object(mut changes) = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))? else {
        bail!("{} is not a JSON object", path.display());
    };
    let mut server = Map::new();
    if let Some(Value::Object(rules)) = changes.get_mut("mp_arena_rules") {
        for k in SERVER_ARENA_KEYS {
            if let Some(v) = rules.remove(k) {
                server.insert(k.to_string(), v);
            }
        }
    }
    let _ = SERVER_ARENA.set(server);
    let n = count_leaves(&Value::Object(changes.clone()));
    let _ = CHANGES.set(changes);
    info!(file = %path.display(), values = n, "balance changes loaded");
    Ok(())
}

fn count_leaves(v: &Value) -> usize {
    match v {
        Value::Object(m) => m.values().map(count_leaves).sum(),
        _ => 1,
    }
}

fn doc_id(key: &str) -> u64 {
    if key.len() == 16
        && let Ok(id) = u64::from_str_radix(key, 16)
    {
        return id;
    }
    crate::sns::symbol(key)
}

pub fn decode(body: &[u8]) -> Result<Vec<(u64, Vec<u8>)>> {
    ensure!(body.len() > 4, "short balance block");
    let size = u32::from_le_bytes(body[..4].try_into().expect("4")) as usize;
    let raw = zstd::bulk::decompress(&body[4..], size).context("balance block: zstd")?;
    ensure!(raw.len() >= 8, "balance block: no header");
    let n = u64::from_le_bytes(raw[..8].try_into().expect("8")) as usize;
    ensure!((1..=64).contains(&n), "balance block: {n} documents");
    let base = 8 + 4 * (n - 1);
    ensure!(raw.len() >= base, "balance block: short offsets");
    let mut bounds = vec![0usize];
    for i in 0..n - 1 {
        bounds.push(u32::from_le_bytes(raw[8 + 4 * i..12 + 4 * i].try_into().expect("4")) as usize);
    }
    bounds.push(raw.len() - base);
    let doc = |i: usize| -> Result<&[u8]> {
        let d = raw.get(base + bounds[i]..base + bounds[i + 1]).context("balance block: bad offset")?;
        Ok(d.strip_suffix(&[0]).unwrap_or(d))
    };
    let index = std::str::from_utf8(doc(n - 1)?).context("balance block: index")?;
    ensure!(index.len() == 16 * (n - 1), "balance block: index has {} ids for {} documents", index.len() / 16, n - 1);
    (0..n - 1)
        .map(|i| Ok((u64::from_str_radix(&index[16 * i..16 * i + 16], 16)?, doc(i)?.to_vec())))
        .collect()
}

pub fn encode(docs: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let index: String = docs.iter().map(|(id, _)| format!("{id:016X}")).collect();
    let mut parts: Vec<Vec<u8>> = docs.iter().map(|(_, d)| [d.as_slice(), &[0]].concat()).collect();
    parts.push([index.as_bytes(), &[0]].concat());
    let mut raw = (parts.len() as u64).to_le_bytes().to_vec();
    let mut end = 0u32;
    for p in &parts[..parts.len() - 1] {
        end += p.len() as u32;
        raw.extend_from_slice(&end.to_le_bytes());
    }
    for p in &parts {
        raw.extend_from_slice(p);
    }
    let mut out = (raw.len() as u32).to_le_bytes().to_vec();
    out.extend(zstd::bulk::compress(&raw, 19).expect("zstd compress"));
    out
}

/// Sets every value `from` gives in `into`; only keys `into` already has,
/// so a stale or misspelt setting cannot add something the game never had.
fn merge(into: &mut Value, from: &Value, path: &str, skipped: &mut Vec<String>) {
    let (Value::Object(dst), Value::Object(src)) = (into, from) else { return };
    for (k, v) in src {
        let here = format!("{path}.{k}");
        match dst.get_mut(k) {
            Some(d @ Value::Object(_)) if v.is_object() => merge(d, v, &here, skipped),
            Some(d) if !d.is_object() && !v.is_object() => *d = v.clone(),
            _ => skipped.push(here),
        }
    }
}

/// `body` with `changes` merged in (documents without changes keep their bytes).
pub fn apply_to(body: &[u8], changes: &Map<String, Value>) -> Result<Vec<u8>> {
    build(body, Base::default(), changes)
}

/// `body` on `base`, with `changes` merged in; untouched documents keep their bytes.
pub fn build(body: &[u8], base: Base, changes: &Map<String, Value>) -> Result<Vec<u8>> {
    let mut docs = decode(body)?;
    // Native's blocks list their ids sorted (as signed numbers). Our Dyson,
    // Combustion and Fission blocks had the map's own document (capture
    // point and round timers) out of place, where a sorted lookup misses it.
    let mut changed = !docs.is_sorted_by_key(|(id, _)| *id as i64);
    docs.sort_by_key(|(id, _)| *id as i64);
    let weapons = doc_id("mp_weapon_settings");
    let swap = if base.echocombat {
        Some(ECHOCOMBAT_WEAPONS)
    } else if base.stock {
        Some(STOCK_WEAPONS)
    } else {
        None
    };
    if let Some(weapons_doc) = swap {
        // Games without EchoCombat: its gear left out of the loadout terminal.
        let equipment = if base.echocombat { ECHOCOMBAT_EQUIPMENT.trim_end().to_owned() } else { stock_equipment() };
        for (id, doc) in [(weapons, weapons_doc.trim_end()), (doc_id("mp_equipment"), equipment.as_str())] {
            if let Some((_, bytes)) = docs.iter_mut().find(|(d, _)| *d == id) {
                *bytes = doc.as_bytes().to_vec();
                changed = true;
            }
        }
    }
    for (key, partial) in changes {
        let id = doc_id(key);
        let Some((_, bytes)) = docs.iter_mut().find(|(d, _)| *d == id) else { continue };
        let mut doc: Value = serde_json::from_slice(bytes).with_context(|| format!("balance document {key}"))?;
        let mut skipped = Vec::new();
        merge(&mut doc, partial, key, &mut skipped);
        if !skipped.is_empty() {
            warn!(settings = ?skipped, "balance settings the game does not have; ignored");
        }
        *bytes = serde_json::to_vec(&doc)?;
        changed = true;
    }
    // MEDIGUN only ever comes from echocombat.ini.
    if let Some((_, bytes)) = docs.iter_mut().find(|(d, _)| *d == weapons) {
        let mut doc: Value = serde_json::from_slice(bytes).context("mp_weapon_settings")?;
        let mut forced = false;
        for at in ["/medigun/enabled", "/ability_properties/medigun/enabled"] {
            if let Some(on) = doc.pointer_mut(at)
                && *on != Value::Bool(base.medigun)
            {
                *on = Value::Bool(base.medigun);
                forced = true;
            }
        }
        if forced {
            *bytes = serde_json::to_vec(&doc)?;
            changed = true;
        }
    }
    Ok(if changed { encode(&docs) } else { body.to_vec() })
}

/// A captured block as clients get it: on this server's base, with the host's changes.
pub fn apply(body: Vec<u8>) -> Vec<u8> {
    let base = BASE.get().copied().unwrap_or_default();
    let empty = Map::new();
    let changes = CHANGES.get().unwrap_or(&empty);
    if !base.echocombat && !base.stock && changes.is_empty() && decode(&body).is_ok_and(|d| d.is_sorted_by_key(|(id, _)| *id as i64)) {
        return body;
    }
    match build(&body, base, changes) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "balance changes not applied; sending the stock settings");
            body
        }
    }
}

/// A captured block for a game without EchoCombat: the stock gear tables,
/// with the host's changes. A stock game given EchoCombat's tables (its own
/// tac mods retuned, gear it has no scripts for) broke its tac mod for good.
pub fn apply_stock(body: Vec<u8>) -> Vec<u8> {
    let empty = Map::new();
    let changes = CHANGES.get().unwrap_or(&empty);
    match build(&body, Base { stock: true, ..Base::default() }, changes) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "balance changes not applied; sending the stock settings");
            body
        }
    }
}

/// Whether the blocks clients get are EchoCombat's (`set_base`).
pub fn echocombat_base() -> bool {
    BASE.get().is_some_and(|b| b.echocombat)
}

/// One JSON document of a block, by file name.
pub fn document(body: &[u8], name: &str) -> Option<Value> {
    let id = doc_id(name);
    let docs = decode(body).ok()?;
    let (_, d) = docs.into_iter().find(|(i, _)| *i == id)?;
    serde_json::from_slice(&d).ok()
}

/// The public arena values this server's match flow follows, from the
/// `mp_arena_rules` it sends (so the host's changes apply to both).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArenaPublic {
    pub round_secs: f32,
    pub pre_match_secs: f32,
    pub launch_secs: f32,
    pub mercy_margin: u32,
    pub self_goals: bool,
    /// Sudden death (overtime) when time runs out tied (`enable_sudden_death`).
    pub sudden_death: bool,
    /// Each overtime round's clock (`sudden_death_round_time`, 120 s).
    pub sd_round_secs: f32,
    /// From regulation's end to the first overtime round (`sudden_death_regulation_over_time`, 0 s).
    pub sd_regulation_over_secs: f32,
    /// From a scoreless overtime round's end to the next (`sudden_death_round_over_time`, 5 s).
    pub sd_round_over_secs: f32,
    /// Goalie ping advantage in public matches (this server's
    /// `goalie_ping_advantage_public`, off unless the host turns it on).
    pub goalie_advantage: bool,
    /// Its window (`goalie_ping_window`, 150 ms), for public and private matches.
    pub goalie_window_ms: f32,
    /// Team-only voice in public matches (`team_only_voice_public`, off
    /// unless the host turns it on).
    pub team_voice: bool,
}

impl Default for ArenaPublic {
    fn default() -> Self {
        Self { round_secs: 300.0, pre_match_secs: 45.0, launch_secs: 15.0, mercy_margin: 12, self_goals: false, sudden_death: true, sd_round_secs: 120.0, sd_regulation_over_secs: 0.0, sd_round_over_secs: 5.0, goalie_advantage: false, goalie_window_ms: 150.0, team_voice: false }
    }
}

impl ArenaPublic {
    pub fn from_rules(r: &Value) -> Self {
        let d = Self::default();
        let f = |v: &Value, k: &str, def: f32, lo: f32| v.get(k).and_then(Value::as_f64).map_or(def, |x| (x as f32).max(lo));
        let rr = r.get("round_rules").cloned().unwrap_or_default();
        Self {
            round_secs: f(&rr, "round_time", d.round_secs, 10.0),
            pre_match_secs: f(&rr, "pre_match_wait_time", d.pre_match_secs, 1.0),
            launch_secs: f(&rr, "wait_for_launch_time", d.launch_secs, 1.0),
            mercy_margin: r.get("mercy_win_point_spread").and_then(Value::as_u64).map_or(d.mercy_margin, |m| m.clamp(1, 999) as u32),
            self_goals: r.get("self_goaling_public").and_then(Value::as_bool).unwrap_or(d.self_goals),
            sudden_death: rr.get("enable_sudden_death").and_then(Value::as_bool).unwrap_or(d.sudden_death),
            sd_round_secs: f(&rr, "sudden_death_round_time", d.sd_round_secs, 10.0),
            sd_regulation_over_secs: f(&rr, "sudden_death_regulation_over_time", d.sd_regulation_over_secs, 0.0),
            sd_round_over_secs: f(&rr, "sudden_death_round_over_time", d.sd_round_over_secs, 0.0),
            goalie_advantage: server_arena_flag("goalie_ping_advantage_public").unwrap_or(d.goalie_advantage),
            goalie_window_ms: f(r, "goalie_ping_window", d.goalie_window_ms, 0.0),
            team_voice: server_arena_flag("team_only_voice_public").unwrap_or(d.team_voice),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena_block() -> Vec<u8> {
        crate::engine::msg::captured().balance.iter().find(|(l, _)| *l == crate::engine::msg::LEVEL_ARENA).expect("arena").1.clone()
    }

    #[test]
    fn every_block_lists_its_ids_in_order() {
        let c = super::super::msg::captured();
        for (level, body) in c.balance.iter().chain(&c.balance_stock) {
            // `apply` leaves a block alone without a base or changes: build it as sent.
            let sent = build(body, Base { echocombat: true, medigun: false, stock: false }, &Map::new()).expect("block");
            let ids: Vec<i64> = decode(&sent).expect("decode").iter().map(|(id, _)| *id as i64).collect();
            assert!(ids.is_sorted(), "level {level:#x}: {ids:x?}");
        }
    }

    #[test]
    fn games_without_echocombat_get_todays_stock_tables() {
        let c = super::super::msg::captured();
        for (level, body) in &c.balance_stock {
            let Some(w) = document(body, "mp_weapon_settings") else { continue };
            assert_eq!(w.pointer("/grenade_properties/dot/use_remote_trigger"), Some(&Value::Bool(true)), "level {level:#x}");
            assert_eq!(w.pointer("/ability_properties/smoke/duration").and_then(Value::as_f64), Some(240.0), "level {level:#x}");
            // EchoCombat's gear list without its own four.
            let e = document(body, "mp_equipment").expect("gear list").to_string();
            for name in ["UMBRAL VEIL", "OVERCHARGE", "NEUTRON", "DRONE"] {
                assert!(!e.contains(name), "level {level:#x}: {name} in a stock gear list");
            }
            for name in ["THREAT SCANNER", "PHASE SHIFT", "TRANSLOCATOR", "PROTOSTAR"] {
                assert!(e.contains(name), "level {level:#x}: {name} missing");
            }
        }
    }

    /// OVERCHARGE (the user's tuning, 2026-11-24): +1.5 body, +2.5 head,
    /// linked teammates can't die for 6.5 s, in both tables.
    #[test]
    fn overcharge_tuning() {
        for doc in [ECHOCOMBAT_WEAPONS, STOCK_WEAPONS] {
            let w: Value = serde_json::from_str(doc).expect("json");
            let b = |k: &str| w.pointer(&format!("/ability_properties/buff/{k}")).and_then(Value::as_f64);
            assert_eq!((b("damage_bonus_body"), b("damage_bonus_head"), b("cheat_death_seconds")), (Some(1.5), Some(2.5), Some(6.5)));
        }
    }

    #[test]
    fn ids_are_the_file_names_symbols() {
        assert_eq!(doc_id("mp_equipment"), 0xf8d4_0525_c888_85d8);
        assert_eq!(doc_id("mp_arena_rules"), 0x1222_0d7a_60d8_f9c3);
        assert_eq!(doc_id("e3830436d1d46da9"), 0xe383_0436_d1d4_6da9);
    }

    #[test]
    fn decode_encode_round_trips() {
        for (_, body) in &crate::engine::msg::captured().balance {
            let docs = decode(body).expect("decode");
            assert!(docs.len() >= 8);
            assert_eq!(decode(&encode(&docs)).expect("re-decode"), docs);
        }
    }

    #[test]
    fn changes_merge_into_existing_keys_only() {
        let body = arena_block();
        let changes: Map<String, Value> = serde_json::from_str(
            r#"{"mp_arena_rules": {"mercy_win_point_spread": 8, "round_rules": {"round_time": 120.0}, "made_up": 1},
                "mp_weapon_settings": {"x": 1}}"#,
        )
        .expect("json");
        let out = apply_to(&body, &changes).expect("apply");
        let rules = document(&out, "mp_arena_rules").expect("rules");
        let a = ArenaPublic::from_rules(&rules);
        assert_eq!(a.mercy_margin, 8);
        assert_eq!(a.round_secs, 120.0);
        assert!(rules.get("made_up").is_none());
        // Untouched documents keep their bytes.
        let before = decode(&body).expect("before");
        let after = decode(&out).expect("after");
        let punch = doc_id("mp_arena_punch");
        assert_eq!(before.iter().find(|d| d.0 == punch), after.iter().find(|d| d.0 == punch));
    }

    /// The launcher's Settings page writes this shape (`balance-settings.json`).
    #[test]
    fn launcher_settings_file_applies() {
        let changes: Map<String, Value> = serde_json::from_str(
            r#"{"mp_weapon_settings": {"grenade_properties": {"dot": {"radius": 3.5}}},
                "e3830436d1d46da9": {"aim_assist": {"enable": false}},
                "mp_arena_rules": {"round_rules": {"round_time": 120.0}}}"#,
        )
        .expect("json");
        let mut hits = 0;
        for (_, body) in &crate::engine::msg::captured().balance {
            let out = apply_to(body, &changes).expect("apply");
            if let Some(w) = document(&out, "mp_weapon_settings") {
                assert_eq!(w["grenade_properties"]["dot"]["radius"], 3.5);
                hits += 1;
            }
            if let Some(g) = document(&out, "e3830436d1d46da9") {
                assert_eq!(g["aim_assist"]["enable"], false);
                hits += 1;
            }
            if let Some(r) = document(&out, "mp_arena_rules") {
                assert_eq!(ArenaPublic::from_rules(&r).round_secs, 120.0);
                hits += 1;
            }
        }
        assert!(hits >= 3, "every document was found in some block ({hits})");
    }

    fn social_block() -> Vec<u8> {
        crate::engine::msg::captured().balance.iter().find(|(l, _)| *l == crate::engine::msg::LEVEL_SOCIAL).expect("social").1.clone()
    }

    #[test]
    fn echocombat_base_and_medigun_only_from_the_file() {
        let body = social_block();
        let ask_medigun: Map<String, Value> = serde_json::from_str(r#"{"mp_weapon_settings": {"medigun": {"enabled": true}}}"#).expect("json");
        let ec = build(&body, Base { echocombat: true, medigun: false, stock: false }, &ask_medigun).expect("build");
        let w = document(&ec, "mp_weapon_settings").expect("weapons");
        assert_eq!(w.pointer("/grenade_properties/dot/radius"), Some(&Value::from(2.5)));
        assert_eq!(w.pointer("/medigun/enabled"), Some(&Value::Bool(false)));
        let gear = document(&ec, "mp_equipment").expect("equipment").to_string();
        assert!(gear.contains("PROTOSTAR") && gear.contains("OVERCHARGE"), "EchoCombat gear list");
        let on = build(&body, Base { echocombat: true, medigun: true, stock: false }, &Map::new()).expect("build");
        let w = document(&on, "mp_weapon_settings").expect("w");
        assert_eq!(w.pointer("/medigun/enabled"), Some(&Value::Bool(true)));
        assert_eq!(w.pointer("/ability_properties/medigun/enabled"), Some(&Value::Bool(true)));
        // EchoCombat off: stock weapons, untouched block.
        assert_eq!(build(&body, Base::default(), &Map::new()).expect("build"), body);
    }

    #[test]
    fn stock_arena_rules_match_the_flow_constants() {
        let rules = document(&arena_block(), "mp_arena_rules").expect("rules");
        assert_eq!(ArenaPublic::from_rules(&rules), ArenaPublic::default());
    }
}
