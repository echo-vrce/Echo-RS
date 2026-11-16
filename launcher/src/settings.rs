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

struct Group {
    game: Game,
    title: &'static str,
    doc: &'static str,
    /// Where in the document this group's values are.
    at: &'static [&'static str],
    /// The object there holds one entry per piece of gear.
    items: bool,
    help: &'static str,
}

const GROUPS: &[Group] = &[
    Group { game: Game::Combat, title: "Weapons", doc: "mp_weapon_settings", at: &["weapon_properties"], items: true, help: "Fire rate, heat, spread and damage per shot." },
    Group { game: Game::Combat, title: "Tac mods", doc: "mp_weapon_settings", at: &["ability_properties"], items: true, help: "" },
    Group { game: Game::Combat, title: "Ordnance", doc: "mp_weapon_settings", at: &["grenade_properties"], items: true, help: "" },
    Group { game: Game::Combat, title: "Damage", doc: "mp_weapon_settings", at: &["damage_table"], items: true, help: "Damage multiplier by the part that is hit." },
    Group { game: Game::Combat, title: "Player", doc: "mp_weapon_settings", at: &["player_properties"], items: false, help: "" },
    Group { game: Game::Combat, title: "Movement", doc: "mp_combat_movement", at: &[], items: false, help: "" },
    Group { game: Game::Arena, title: "Match rules", doc: "mp_arena_rules", at: &[], items: false, help: "Public round time, pre-match wait, launch wait, mercy spread and self goals are also followed by this server's match flow." },
    Group { game: Game::Arena, title: "Disc & grab", doc: DISC_GRAB, at: &[], items: false, help: "" },
    Group { game: Game::Arena, title: "Movement", doc: "mp_arena_movement", at: &[], items: false, help: "Also used in the social lobby." },
    Group { game: Game::Arena, title: "Punch", doc: "mp_arena_punch", at: &[], items: false, help: "Also used in the social lobby." },
];

/// Gear in the order the game shows it, with its in-game name.
const GEAR: &[(&str, &str)] = &[
    ("assault", "PULSAR"),
    ("blaster", "NOVA"),
    ("blaster_cone", "NOVA cone"),
    ("scout", "COMET"),
    ("scout_aoe", "COMET blast"),
    ("rocket", "METEOR"),
    ("rocket_aoe", "METEOR blast"),
    ("heal", "REPAIR MATRIX"),
    ("sensor", "THREAT SCANNER"),
    ("shield", "ENERGY BARRIER"),
    ("wraith", "PHASE SHIFT"),
    ("buff", "OVERCHARGE"),
    ("smoke", "UMBRAL VEIL"),
    ("point", "NEUTRON"),
    ("det", "DETONATOR"),
    ("stun", "STUN FIELD"),
    ("arc", "ARC MINE"),
    ("burst", "INSTANT REPAIR"),
    ("dot", "DOT FIELD"),
    ("drone", "DRONE"),
    ("loc", "Loc"),
    ("magnum_aoe", "Magnum blast"),
    ("smg", "SMG"),
];

fn defaults() -> &'static Map<String, Value> {
    static D: OnceLock<Map<String, Value>> = OnceLock::new();
    D.get_or_init(|| DOCS.iter().map(|(k, t)| (k.to_string(), serde_json::from_str(t).expect("bundled default"))).collect())
}

fn gear_name(k: &str) -> String {
    GEAR.iter().find(|(g, _)| *g == k).map_or_else(|| words(k), |(_, n)| n.to_string())
}

fn gear_rank(k: &str) -> usize {
    GEAR.iter().position(|(g, _)| *g == k).unwrap_or(GEAR.len())
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

/// Every editable value, of one game or both, in page order.
fn all_leaves(game: Option<Game>) -> Vec<Leaf> {
    let mut out = Vec::new();
    for g in GROUPS.iter().filter(|g| game.is_none_or(|x| g.game == x)) {
        for (key, v) in group_entries(g) {
            let mut path: Vec<String> = g.at.iter().map(|s| s.to_string()).collect();
            let label = match key {
                Some(k) => {
                    path.push(k.clone());
                    format!("{} › {}", g.title, gear_name(&k))
                }
                None => g.title.to_string(),
            };
            let start = out.len();
            leaves(g.doc, v, &mut path, "", &mut out);
            for l in &mut out[start..] {
                l.label = format!("{label} › {}", l.label);
            }
        }
    }
    out
}

/// The group's object, split into its gear entries (`items`) or as one.
fn group_entries(g: &Group) -> Vec<(Option<String>, &'static Value)> {
    let path: Vec<String> = g.at.iter().map(|s| s.to_string()).collect();
    let Some(v) = defaults().get(g.doc).and_then(|d| at(d, &path)) else { return Vec::new() };
    if !g.items {
        return vec![(None, v)];
    }
    let mut items: Vec<(Option<String>, &Value)> = v.as_object().map(|m| m.iter().map(|(k, x)| (Some(k.clone()), x)).collect()).unwrap_or_default();
    items.sort_by_key(|(k, _)| (gear_rank(k.as_deref().unwrap_or("")), k.clone()));
    items
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

    let groups: Vec<&Group> = GROUPS.iter().filter(|g| g.game == st.game).collect();
    let group_names: Vec<(String, bool)> = groups
        .iter()
        .map(|g| {
            let mut list = Vec::new();
            for (k, v) in group_entries(g) {
                let mut path: Vec<String> = g.at.iter().map(|s| s.to_string()).collect();
                path.extend(k);
                leaves(g.doc, v, &mut path, "", &mut list);
            }
            (g.title.to_string(), dirty(changes, &list))
        })
        .collect();
    let before = st.group;
    tabs(ui, &group_names, &mut st.group);
    if st.group != before {
        st.item = 0;
    }
    st.group = st.group.min(groups.len() - 1);
    let g = groups[st.group];
    ui.add_space(8.0);

    let entries = group_entries(g);
    let entry_leaves: Vec<(Option<String>, Vec<Leaf>)> = entries
        .into_iter()
        .map(|(k, v)| {
            let mut path: Vec<String> = g.at.iter().map(|s| s.to_string()).collect();
            path.extend(k.clone());
            let mut list = Vec::new();
            leaves(g.doc, v, &mut path, "", &mut list);
            (k, list)
        })
        .collect();
    if g.items {
        let names: Vec<(String, bool)> = entry_leaves.iter().map(|(k, l)| (gear_name(k.as_deref().unwrap_or("")), dirty(changes, l))).collect();
        egui::Frame::default().fill(FIELD).corner_radius(10).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            tabs(ui, &names, &mut st.item);
        });
        ui.add_space(8.0);
    }
    let idx = if g.items { st.item.min(entry_leaves.len().saturating_sub(1)) } else { 0 };
    let Some((key, list)) = entry_leaves.get(idx) else { return };
    let title = match key {
        Some(k) => format!("{} · {}", g.title, gear_name(k)),
        None => g.title.to_string(),
    };
    egui::Frame::default().fill(CARD).stroke(egui::Stroke::new(1.0_f32, BORDER)).corner_radius(12).inner_margin(egui::Margin::same(18)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title.to_uppercase()).size(12.0).strong().extra_letter_spacing(1.2).color(MUTED));
        if !g.help.is_empty() {
            hint(ui, g.help);
        }
        ui.add_space(4.0);
        rows(ui, &format!("rows-{}-{idx}", g.title), list, |l| l.label.clone(), changes);
        if list.is_empty() {
            hint(ui, "No values to change here.");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_and_every_group_has_values() {
        for g in GROUPS {
            let n: usize = group_entries(g)
                .into_iter()
                .map(|(_, v)| {
                    let mut out = Vec::new();
                    leaves(g.doc, v, &mut Vec::new(), "", &mut out);
                    out.len()
                })
                .sum();
            assert!(n > 0, "{} has no values", g.title);
        }
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
