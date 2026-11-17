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

pub(crate) const ECHOCOMBAT_WEAPONS: &str = include_str!("data/mp_weapon_settings_echocombat.json");
/// The gear list a native H server with EchoCombat sends (its social lobby, 2026-11-16):
/// the stock gear plus PROTOSTAR, QUARK, BLAZAR, STARBURST, UMBRAL VEIL, OVERCHARGE, ...
const ECHOCOMBAT_EQUIPMENT: &str = include_str!("data/mp_equipment_echocombat.json");

/// Which weapon settings the blocks start from.
#[derive(Clone, Copy, Debug, Default)]
pub struct Base {
    pub echocombat: bool,
    pub medigun: bool,
}

/// Call once at startup, before any session (tests use the stock base).
pub fn set_base(base: Base) {
    let _ = BASE.set(base);
}

/// Loads the host's changes; call once at startup, before any session.
pub fn load(path: &std::path::Path) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let Value::Object(changes) = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))? else {
        bail!("{} is not a JSON object", path.display());
    };
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
    let mut changed = false;
    let weapons = doc_id("mp_weapon_settings");
    if base.echocombat {
        for (id, doc) in [(weapons, ECHOCOMBAT_WEAPONS), (doc_id("mp_equipment"), ECHOCOMBAT_EQUIPMENT)] {
            if let Some((_, bytes)) = docs.iter_mut().find(|(d, _)| *d == id) {
                *bytes = doc.trim_end().as_bytes().to_vec();
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
    if !base.echocombat && changes.is_empty() {
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
}

impl Default for ArenaPublic {
    fn default() -> Self {
        Self { round_secs: 300.0, pre_match_secs: 45.0, launch_secs: 15.0, mercy_margin: 12, self_goals: false }
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
        let ec = build(&body, Base { echocombat: true, medigun: false }, &ask_medigun).expect("build");
        let w = document(&ec, "mp_weapon_settings").expect("weapons");
        assert_eq!(w.pointer("/grenade_properties/dot/radius"), Some(&Value::from(2.25)));
        assert_eq!(w.pointer("/medigun/enabled"), Some(&Value::Bool(false)));
        let gear = document(&ec, "mp_equipment").expect("equipment").to_string();
        assert!(gear.contains("PROTOSTAR") && gear.contains("OVERCHARGE"), "EchoCombat gear list");
        let on = build(&body, Base { echocombat: true, medigun: true }, &Map::new()).expect("build");
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
