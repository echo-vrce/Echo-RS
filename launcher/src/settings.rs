//! The Settings page: the game's balance values the server sends every
//! player (weapons, tac mods, ordnance, damage, movement, arena rules ...).
//!
//! Defaults are the stock values the server sends (`defaults/*.json`, decoded
//! from its balance blocks). Only values that differ from them are kept, as
//! `{"<document>": {partial JSON}}`; on Start they are written next to the
//! launcher and passed to the server with `--balance`.

use std::sync::OnceLock;

use eframe::egui::{self, Color32, RichText};
use serde_json::{Map, Value};

use super::{ACCENT, AMBER, BORDER, CARD, FIELD, MUTED, RAISED, TEXT, card, hint, segmented};

/// The disc & grab document's file name is unknown; the server takes its id.
const DISC_GRAB: &str = "e3830436d1d46da9";

const DOCS: &[(&str, &str)] = &[
    ("mp_weapon_settings", include_str!("../defaults/mp_weapon_settings.json")),
    ("mp_combat_movement", include_str!("../defaults/mp_combat_movement.json")),
    ("mp_arena_rules", include_str!("../defaults/mp_arena_rules.json")),
    (DISC_GRAB, include_str!("../defaults/disc_grab_aim_assist.json")),
    ("mp_arena_movement", include_str!("../defaults/mp_arena_movement.json")),
    ("mp_arena_punch", include_str!("../defaults/mp_arena_punch.json")),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Game {
    Combat,
    Arena,
}

/// One page of the settings: a row of gear (`tabs`) or a single list.
struct Section {
    title: &'static str,
    help: &'static str,
    /// One button per item (a piece of gear).
    tabs: bool,
    items: Vec<Item>,
}

/// A piece of gear with everything that belongs to it, wherever the game's
/// files keep it (the DETONATOR: its ordnance values and its damage table).
struct Item {
    name: String,
    parts: Vec<Part>,
}

/// One object in a document, shown under a heading.
struct Part {
    heading: String,
    doc: &'static str,
    path: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Weapon,
    TacMod,
    Ordnance,
}

/// Gear in the game's own order with its in-game name, both from the game's
/// `mp_equipment.json` (the entries it has commented out included). MEDIGUN
/// is EchoCombat's and not in that file.
const GEAR: &[(&str, &str, Kind)] = &[
    ("assault", "PULSAR", Kind::Weapon),
    ("blaster", "NOVA", Kind::Weapon),
    ("scout", "COMET", Kind::Weapon),
    ("rocket", "METEOR", Kind::Weapon),
    ("magnum", "PROTOSTAR", Kind::Weapon),
    ("smg", "QUARK", Kind::Weapon),
    ("chain", "BLAZAR", Kind::Weapon),
    ("rifle", "STARBURST", Kind::Weapon),
    ("det", "DETONATOR", Kind::Ordnance),
    ("stun", "STUN FIELD", Kind::Ordnance),
    ("arc", "ARC MINE", Kind::Ordnance),
    ("burst", "INSTANT REPAIR", Kind::Ordnance),
    ("loc", "TRANSLOCATOR", Kind::Ordnance),
    ("dot", "DOT FIELD", Kind::Ordnance),
    ("drone", "ATTACK DRONE", Kind::Ordnance),
    ("heal", "REPAIR MATRIX", Kind::TacMod),
    ("sensor", "THREAT SCANNER", Kind::TacMod),
    ("shield", "ENERGY BARRIER", Kind::TacMod),
    ("wraith", "PHASE SHIFT", Kind::TacMod),
    ("buff", "POWER BOOST", Kind::TacMod),
    ("smoke", "UMBRAL VEIL", Kind::TacMod),
    ("point", "POINT BLANK", Kind::TacMod),
    ("medigun", "MEDIGUN", Kind::TacMod),
];

/// Entries that are part of another piece of gear: (entry, gear, what of it).
/// The game's file lists them as that gear's aliases.
const PART_OF: &[(&str, &str, &str)] = &[
    ("blaster_cone", "blaster", "cone"),
    ("scout_aoe", "scout", "explosion"),
    ("rocket_aoe", "rocket", "explosion"),
    ("magnum_aoe", "magnum", "explosion"),
];

/// The weapon document's per-gear tables: (key, heading, the kind of gear an
/// entry is when `GEAR` does not know it).
const TABLES: &[(&str, &str, Kind)] = &[
    ("weapon_properties", "Weapon", Kind::Weapon),
    ("ability_properties", "Tac mod", Kind::TacMod),
    ("grenade_properties", "Ordnance", Kind::Ordnance),
    ("damage_table", "Damage by body part", Kind::Weapon),
];

const WEAPONS: &str = "mp_weapon_settings";

fn defaults() -> &'static Map<String, Value> {
    static D: OnceLock<Map<String, Value>> = OnceLock::new();
    D.get_or_init(|| DOCS.iter().map(|(k, t)| (k.to_string(), serde_json::from_str(t).expect("bundled default"))).collect())
}

fn gear_name(k: &str) -> String {
    GEAR.iter().find(|(g, ..)| *g == k).map_or_else(|| words(k), |(_, n, _)| n.to_string())
}

fn gear_rank(k: &str) -> usize {
    GEAR.iter().position(|(g, ..)| *g == k).unwrap_or(GEAR.len())
}

/// Every piece of gear of one kind, each with all its tables' entries.
fn gear_items(kind: Kind) -> Vec<Item> {
    let Some(doc) = defaults().get(WEAPONS) else { return Vec::new() };
    let mut found: Vec<(String, Kind, Vec<Part>)> = Vec::new();
    for (table, heading, table_kind) in TABLES {
        let Some(entries) = doc.get(*table).and_then(Value::as_object) else { continue };
        for key in entries.keys() {
            let (gear, what) = PART_OF.iter().find(|(k, ..)| k == key).map_or((key.as_str(), None), |(_, g, w)| (*g, Some(*w)));
            let heading = match what {
                Some(w) => format!("{heading} · {w}"),
                None => heading.to_string(),
            };
            let part = Part { heading, doc: WEAPONS, path: vec![table.to_string(), key.clone()] };
            match found.iter_mut().find(|(g, ..)| g == gear) {
                Some((.., parts)) => parts.push(part),
                None => {
                    let kind = GEAR.iter().find(|(g, ..)| *g == gear).map_or(*table_kind, |(.., k)| *k);
                    found.push((gear.to_string(), kind, vec![part]));
                }
            }
        }
    }
    found.sort_by_key(|(g, ..)| (gear_rank(g), g.clone()));
    // A gear's own entry before its cone / explosion, table by table.
    for (.., parts) in &mut found {
        parts.sort_by_key(|p| (TABLES.iter().position(|(t, ..)| *t == p.path[0]), p.heading.contains(" · ")));
    }
    found.into_iter().filter(|(_, k, _)| *k == kind).map(|(g, _, parts)| Item { name: gear_name(&g), parts }).collect()
}

fn single(title: &'static str, doc: &'static str, at: &[&str], help: &'static str) -> Section {
    Section { title, help, tabs: false, items: vec![Item { name: title.to_string(), parts: vec![Part { heading: String::new(), doc, path: at.iter().map(|s| s.to_string()).collect() }] }] }
}

fn sections(game: Game) -> Vec<Section> {
    match game {
        Game::Combat => vec![
            Section { title: "Weapons", help: "Each gun with everything of its own: fire rate, heat, spread, its explosion and the damage each body part takes.", tabs: true, items: gear_items(Kind::Weapon) },
            Section { title: "Tac mods", help: "", tabs: true, items: gear_items(Kind::TacMod) },
            Section { title: "Ordnance", help: "Each ordnance with its own values and its damage.", tabs: true, items: gear_items(Kind::Ordnance) },
            single("Player", WEAPONS, &["player_properties"], ""),
            single("Movement", "mp_combat_movement", &[], ""),
        ],
        Game::Arena => vec![
            single("Match rules", "mp_arena_rules", &[], "Public round time, pre-match wait, launch wait, mercy spread and self goals are also followed by this server's match flow."),
            single("Disc & grab", DISC_GRAB, &[], ""),
            single("Movement", "mp_arena_movement", &[], "Also used in the social lobby."),
            single("Punch", "mp_arena_punch", &[], "Also used in the social lobby."),
        ],
    }
}

fn part_leaves(p: &Part) -> Vec<Leaf> {
    let mut out = Vec::new();
    if let Some(v) = defaults().get(p.doc).and_then(|d| at(d, &p.path)) {
        leaves(p.doc, v, &mut p.path.clone(), "", &mut out);
    }
    out
}

fn item_leaves(it: &Item) -> Vec<Leaf> {
    it.parts.iter().flat_map(part_leaves).collect()
}

/// `heat_per_shot` -> `Heat per shot`.
fn words(k: &str) -> String {
    let s = k.replace('_', " ");
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

fn at<'a>(v: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(v, |v, k| v.get(k))
}

/// One editable value: its path in the document and its stock value.
struct Leaf {
    doc: &'static str,
    path: Vec<String>,
    label: String,
    default: Value,
}

/// Every number and switch under `v` (lists and text are left alone).
fn leaves(doc: &'static str, v: &Value, path: &mut Vec<String>, label: &str, out: &mut Vec<Leaf>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                path.push(k.clone());
                let l = if label.is_empty() { words(k) } else { format!("{label} · {}", k.replace('_', " ")) };
                leaves(doc, x, path, &l, out);
                path.pop();
            }
        }
        Value::Number(_) | Value::Bool(_) => out.push(Leaf { doc, path: path.clone(), label: label.to_string(), default: v.clone() }),
        _ => {}
    }
}

/// The current value: the host's change, else the default.
fn current<'a>(changes: &'a Map<String, Value>, l: &'a Leaf) -> &'a Value {
    changes.get(l.doc).and_then(|d| at(d, &l.path)).filter(|v| !v.is_object()).unwrap_or(&l.default)
}

fn set(changes: &mut Map<String, Value>, l: &Leaf, v: Value) {
    if v == l.default {
        remove(changes, l);
        return;
    }
    let mut node = changes.entry(l.doc.to_string()).or_insert_with(|| Value::Object(Map::new()));
    for k in &l.path[..l.path.len() - 1] {
        if !node.is_object() {
            *node = Value::Object(Map::new());
        }
        node = node.as_object_mut().expect("object").entry(k.clone()).or_insert_with(|| Value::Object(Map::new()));
    }
    if !node.is_object() {
        *node = Value::Object(Map::new());
    }
    node.as_object_mut().expect("object").insert(l.path[l.path.len() - 1].clone(), v);
}

/// Drops a change and any objects it leaves empty.
fn remove(changes: &mut Map<String, Value>, l: &Leaf) {
    fn go(m: &mut Map<String, Value>, path: &[String]) {
        let Some((k, rest)) = path.split_first() else { return };
        if rest.is_empty() {
            m.remove(k);
        } else if let Some(Value::Object(sub)) = m.get_mut(k) {
            go(sub, rest);
            if sub.is_empty() {
                m.remove(k);
            }
        }
    }
    if let Some(Value::Object(doc)) = changes.get_mut(l.doc) {
        go(doc, &l.path);
        if doc.is_empty() {
            changes.remove(l.doc);
        }
    }
}

/// How many values differ from the defaults (stale or unknown keys aside).
pub fn changed_count(changes: &Map<String, Value>) -> usize {
    all_leaves(None).iter().filter(|l| *current(changes, l) != l.default).count()
}

/// Every editable value, of one game or both, in page order, labelled with
/// where it is (for the search).
fn all_leaves(game: Option<Game>) -> Vec<Leaf> {
    let mut out = Vec::new();
    for g in [Game::Combat, Game::Arena].into_iter().filter(|g| game.is_none_or(|x| *g == x)) {
        for s in sections(g) {
            for it in &s.items {
                for p in &it.parts {
                    let mut place = s.title.to_string();
                    if s.tabs {
                        place = format!("{place} › {}", it.name);
                    }
                    if !p.heading.is_empty() {
                        place = format!("{place} › {}", p.heading);
                    }
                    for mut l in part_leaves(p) {
                        l.label = format!("{place} › {}", l.label);
                        out.push(l);
                    }
                }
            }
        }
    }
    out
}

pub struct State {
    pub game: Game,
    group: usize,
    item: usize,
    search: String,
}

impl Default for State {
    fn default() -> Self {
        Self { game: Game::Combat, group: 0, item: 0, search: String::new() }
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Bool(b) => if *b { "on" } else { "off" }.into(),
        _ => v.to_string(),
    }
}

/// One row per value: name, editor, and a reset arrow when it was changed.
fn rows(ui: &mut egui::Ui, id: &str, list: &[Leaf], label_of: impl Fn(&Leaf) -> String, changes: &mut Map<String, Value>) {
    egui::Grid::new(id).num_columns(3).spacing([18.0, 6.0]).min_col_width(40.0).striped(true).show(ui, |ui| {
        for l in list {
            let now = current(changes, l).clone();
            let changed = now != l.default;
            let name = RichText::new(label_of(l)).color(if changed { ACCENT } else { TEXT });
            ui.label(if changed { name.strong() } else { name }).on_hover_text(format!("Default: {}", value_text(&l.default)));
            let mut new = None;
            match &now {
                Value::Bool(b) => {
                    let mut b = *b;
                    if ui.checkbox(&mut b, "").changed() {
                        new = Some(Value::Bool(b));
                    }
                }
                Value::Number(n) if l.default.is_i64() || l.default.is_u64() => {
                    let mut x = n.as_i64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0).round() as i64);
                    if ui.add(egui::DragValue::new(&mut x).speed(0.2)).changed() {
                        new = Some(Value::from(x));
                    }
                }
                Value::Number(n) => {
                    let mut x = n.as_f64().unwrap_or(0.0);
                    let scale = l.default.as_f64().unwrap_or(1.0).abs().max(0.01);
                    if ui.add(egui::DragValue::new(&mut x).speed(scale * 0.01).max_decimals(5)).changed() {
                        new = serde_json::Number::from_f64(x).map(Value::Number);
                    }
                }
                _ => {
                    ui.label("");
                }
            }
            if changed {
                if ui.small_button("↺").on_hover_text(format!("Back to {}", value_text(&l.default))).clicked() {
                    new = Some(l.default.clone());
                }
            } else {
                ui.label("");
            }
            if let Some(v) = new {
                set(changes, l, v);
            }
            ui.end_row();
        }
    });
}

/// Item / group buttons: a changed one shows a dot.
fn tabs(ui: &mut egui::Ui, names: &[(String, bool)], selected: &mut usize) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for (i, (n, dirty)) in names.iter().enumerate() {
            let on = *selected == i;
            let text = if *dirty { format!("{n}  •") } else { n.clone() };
            let b = egui::Button::new(RichText::new(text).color(if on { Color32::WHITE } else { TEXT }))
                .fill(if on { ACCENT } else { RAISED })
                .stroke(egui::Stroke::new(1.0_f32, if on { ACCENT } else { BORDER }))
                .corner_radius(20);
            if ui.add(b).clicked() {
                *selected = i;
            }
        }
    });
}

fn dirty(changes: &Map<String, Value>, list: &[Leaf]) -> bool {
    list.iter().any(|l| *current(changes, l) != l.default)
}

pub fn page(ui: &mut egui::Ui, st: &mut State, changes: &mut Map<String, Value>) {
    card(ui, "Balance settings", |ui| {
        hint(ui, "What every player's game uses on this server. Changes apply from the next Start; changed values are blue, ↺ puts one back.");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let before = st.game;
            segmented(ui, &mut st.game, &[(Game::Combat, "Combat"), (Game::Arena, "Arena")]);
            if st.game != before {
                st.group = 0;
                st.item = 0;
            }
            ui.add_space(12.0);
            ui.add(egui::TextEdit::singleline(&mut st.search).hint_text("🔍 Search").desired_width(200.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let n = changed_count(changes);
                if ui.add_enabled(n > 0, egui::Button::new("Reset all")).on_hover_text("Every value back to the stock game's").clicked() {
                    changes.clear();
                }
                if n > 0 {
                    ui.label(RichText::new(format!("{n} changed")).color(AMBER));
                }
            });
        });
        if st.game == Game::Combat {
            hint(ui, "Defaults are EchoCombat's tuning, as EchoVRCE's combat servers send it. With EchoCombat off the server sends stock gear values. Combat maps are not hosted yet: these reach players in the social lobby.");
        }
    });
    ui.add_space(12.0);

    if !st.search.trim().is_empty() {
        let q = st.search.trim().to_lowercase();
        let found: Vec<Leaf> = all_leaves(Some(st.game)).into_iter().filter(|l| l.label.to_lowercase().contains(&q) || l.path.join(" ").to_lowercase().contains(&q)).collect();
        card(ui, &format!("{} found", found.len()), |ui| {
            if found.is_empty() {
                hint(ui, "Nothing matches.");
            }
            rows(ui, "search", &found, |l| l.label.clone(), changes);
        });
        return;
    }

    let all = sections(st.game);
    let names: Vec<(String, bool)> = all.iter().map(|s| (s.title.to_string(), s.items.iter().any(|it| dirty(changes, &item_leaves(it))))).collect();
    let before = st.group;
    tabs(ui, &names, &mut st.group);
    if st.group != before {
        st.item = 0;
    }
    st.group = st.group.min(all.len() - 1);
    let s = &all[st.group];
    ui.add_space(8.0);

    if s.tabs {
        let names: Vec<(String, bool)> = s.items.iter().map(|it| (it.name.clone(), dirty(changes, &item_leaves(it)))).collect();
        egui::Frame::default().fill(FIELD).corner_radius(10).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            tabs(ui, &names, &mut st.item);
        });
        ui.add_space(8.0);
    }
    let idx = if s.tabs { st.item.min(s.items.len().saturating_sub(1)) } else { 0 };
    let Some(it) = s.items.get(idx) else { return };
    let title = if s.tabs { format!("{} · {}", s.title, it.name) } else { s.title.to_string() };
    egui::Frame::default().fill(CARD).stroke(egui::Stroke::new(1.0_f32, BORDER)).corner_radius(12).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title.to_uppercase()).size(12.0).strong().extra_letter_spacing(1.2).color(MUTED));
        if !s.help.is_empty() {
            hint(ui, s.help);
        }
        let mut any = false;
        for (n, p) in it.parts.iter().enumerate() {
            let list = part_leaves(p);
            if list.is_empty() {
                continue;
            }
            any = true;
            ui.add_space(if p.heading.is_empty() { 4.0 } else { 12.0 });
            if !p.heading.is_empty() {
                ui.label(RichText::new(&p.heading).strong().color(TEXT));
                ui.add_space(2.0);
            }
            rows(ui, &format!("rows-{}-{idx}-{n}", s.title), &list, |l| l.label.clone(), changes);
        }
        if !any {
            hint(ui, "No values to change here.");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_and_every_section_has_values() {
        for g in [Game::Combat, Game::Arena] {
            for s in sections(g) {
                assert!(!s.items.is_empty(), "{} has no items", s.title);
                for it in &s.items {
                    assert!(!it.parts.is_empty(), "{} › {}", s.title, it.name);
                }
                assert!(s.items.iter().any(|it| !item_leaves(it).is_empty()), "{} has no values", s.title);
            }
        }
    }

    /// Everything of a piece of gear is on its own page, under the game's name.
    #[test]
    fn gear_pages_hold_all_of_a_gears_values() {
        let combat = sections(Game::Combat);
        let ordnance = combat.iter().find(|s| s.title == "Ordnance").expect("ordnance");
        let det = ordnance.items.iter().find(|it| it.name == "DETONATOR").expect("detonator");
        let paths: Vec<Vec<String>> = det.parts.iter().map(|p| p.path.clone()).collect();
        assert_eq!(paths, [["grenade_properties", "det"], ["damage_table", "det"]]);
        let weapons = combat.iter().find(|s| s.title == "Weapons").expect("weapons");
        let names: Vec<&str> = weapons.items.iter().map(|it| it.name.as_str()).collect();
        assert_eq!(&names[..8], ["PULSAR", "NOVA", "COMET", "METEOR", "PROTOSTAR", "QUARK", "BLAZAR", "STARBURST"]);
        let comet = weapons.items.iter().find(|it| it.name == "COMET").expect("comet");
        let headings: Vec<&str> = comet.parts.iter().map(|p| p.heading.as_str()).collect();
        assert_eq!(headings, ["Weapon", "Weapon · explosion", "Damage by body part", "Damage by body part · explosion"]);
        let tac: Vec<&str> = combat.iter().find(|s| s.title == "Tac mods").expect("tac").items.iter().map(|it| it.name.as_str()).collect();
        assert!(tac.contains(&"POWER BOOST") && tac.contains(&"POINT BLANK"), "{tac:?}");

        // Nothing lost or shown twice: every value of the per-gear tables once.
        let mut want = Vec::new();
        for (table, ..) in TABLES {
            let v = &defaults()[WEAPONS][*table];
            leaves(WEAPONS, v, &mut vec![table.to_string()], "", &mut want);
        }
        let mut want: Vec<Vec<String>> = want.into_iter().map(|l| l.path).collect();
        let mut got: Vec<Vec<String>> = combat.iter().filter(|s| s.tabs).flat_map(|s| s.items.iter().flat_map(item_leaves)).map(|l| l.path).collect();
        want.sort();
        got.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn set_keeps_only_differences() {
        let all = all_leaves(None);
        let dot = all.iter().find(|l| l.path == ["grenade_properties", "dot", "radius"]).expect("dot radius");
        let round = all.iter().find(|l| l.path == ["round_rules", "round_time"]).expect("round time");
        let mut c = Map::new();
        set(&mut c, dot, Value::from(4.0));
        set(&mut c, round, Value::from(120.0));
        assert_eq!(serde_json::to_string(&c).unwrap(), r#"{"mp_arena_rules":{"round_rules":{"round_time":120.0}},"mp_weapon_settings":{"grenade_properties":{"dot":{"radius":4.0}}}}"#);
        assert_eq!(changed_count(&c), 2);
        set(&mut c, dot, dot.default.clone());
        set(&mut c, round, round.default.clone());
        assert!(c.is_empty());
    }
}
