//! Private-match rules terminal (echo_arena_private), from a native capture
//! (captures/native-private-20261115/NOTES.md).
//!
//! Each row is a stream-1 setting var the terminal shows, plus the stream-0
//! game var native derives from it. Inc/Dec step ints by one within bounds;
//! either press toggles a bool. Bounds marked "seen" were hit in the capture;
//! the others are guesses wide enough not to get in the way.

use super::msg::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    None,
    /// Round length: minutes (row 0.0) and seconds (row 0.1) → game clock.
    RoundTime,
    /// Starting score for a team (true: blue), plus its own s0 mirror.
    Score(bool, u64),
    /// Disc spawn: setting 2→1, 1→2, 0→0.
    DiscSpawn,
    TotalRounds,
    /// Rounds won by a team (true: blue); the round counter is their sum.
    RoundsWon(bool),
    Lo8(u64),
    Lo32(u64),
    F32(u64),
}

#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub page: u32,
    pub row: u32,
    pub var: u64,
    pub default: i32,
    pub min: i32,
    pub max: i32,
    pub boolean: bool,
    pub target: Target,
}

const fn int(page: u32, row: u32, var: u64, default: i32, min: i32, max: i32, target: Target) -> Rule {
    Rule { page, row, var, default, min, max, boolean: false, target }
}
const fn flag(page: u32, row: u32, var: u64, default: bool, target: Target) -> Rule {
    Rule { page, row, var, default: default as i32, min: 0, max: 1, boolean: true, target }
}

/// s0 var the round-time rows feed (min*60 + sec).
pub const VAR_ROUND_TIME: u64 = 0x5b74_8a2b_4792_dd2d;
/// s0 latch native sets the first time the disc spawn rule is changed.
pub const VAR_SPAWN_CHANGED: u64 = 0xd389_fa29_81d5_124c;
pub const ROW_START_COUNT: usize = 10;
/// "GOALIE PING ADV" (row 1.1, default off).
pub const ROW_GOALIE_PING: usize = 5;
/// "ROUND COUNT" (row 2.0, default 3).
pub const ROW_ROUND_COUNT: usize = 8;
/// `net_catapult_time`: the launch-tube count.
pub const ROW_CATAPULT_TIME: usize = 7;
/// `net_self_goaling`: own goals count.
pub const ROW_SELF_GOALING: usize = 20;
/// Mercy: on/off (row 4.0) and the winning margin (row 4.1, default 20).
pub const ROW_MERCY_ON: usize = 16;
pub const ROW_MERCY_MARGIN: usize = 17;
/// `net_respawn_time` (row 1.2): seconds after a goal before players are
/// sent back to their team rooms.
pub const ROW_RESPAWN_TIME: usize = 6;
/// `overtime` (row 3.2): 0 off; otherwise a tied round plays on to the
/// next goal.
pub const ROW_OVERTIME: usize = 14;
/// Row 4.2 (bool, default on): the game clock stops after a goal.
pub const ROW_GOAL_STOPS_TIME: usize = 18;
pub const ROW_DISC_SPAWN: usize = 4;
/// Row 2.1 "ROUNDS PLAYED": 1 stops once a team has a majority (best of),
/// 0 plays every round.
pub const ROW_ROUNDS_PLAYED: usize = 9;
/// Row 2.3 "CARRY POINTS OVER" (assumed row): scores carry into the next round.
pub const ROW_CARRY_POINTS: usize = 11;

/// The names the client's API uses for each row (rows 1.1, 2.3 and 5.1 are
/// matched to the three remaining names by elimination and panel grouping).
pub const API_NAMES: [&str; 22] = [
    "MINUTES",
    "SECONDS",
    "BLUE SCORE",
    "ORANGE SCORE",
    "DISC LOCATION",
    "GOALIE PING ADV",
    "RESPAWN TIME",
    "CATAPULT TIME",
    "ROUND COUNT",
    "ROUNDS PLAYED",
    "ROUND WAIT TIME",
    "CARRY POINTS OVER",
    "BLUE ROUNDS WON",
    "ORANGE ROUNDS WON",
    "OVERTIME",
    "STANDARD CHASSIS",
    "MERCY ENABLED",
    "MERCY SCORE DIFF",
    "GOAL STOPS TIME",
    "DISC CURVE",
    "SELF-GOALING",
    "TEAM ONLY VOICE",
];

/// Body of an API rule set: the rule's row and its new value.
pub fn parse_api(d: &[u8]) -> Option<(usize, i32)> {
    let json = d.get(16..)?;
    let json = json.split(|&b| b == 0).next()?;
    let v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let (k, v) = v.as_object()?.iter().next()?;
    let i = API_NAMES.iter().position(|n| n == k)?;
    let v = match v {
        serde_json::Value::Bool(b) => i32::from(*b),
        serde_json::Value::Number(n) => n.as_f64()?.round() as i32,
        _ => return None,
    };
    Some((i, v))
}

pub const RULES: [Rule; 22] = [
    int(0, 0, 0x2fda_8691_4b60_5299, 10, 0, 60, Target::RoundTime),
    int(0, 1, 0x2fc4_8a9c_517a_5399, 0, 0, 59, Target::RoundTime), // min seen
    int(0, 2, 0x1c97_7a2a_21e0_9f11, 0, 0, 99, Target::Score(true, 0x4d41_736c_db39_fe99)),
    int(0, 3, 0x8935_d445_0225_d7f9, 0, 0, 99, Target::Score(false, 0x7a4f_1202_6413_4e4d)),
    int(1, 0, 0xd186_64b7_c9b8_8f78, 1, 0, 2, Target::DiscSpawn), // min seen
    flag(1, 1, 0x090a_5a50_56c5_cd37, false, Target::Lo8(0x0494_f023_8b8a_c3e6)),
    int(1, 2, 0xd8c1_4bc0_4dba_a43b, 3, 0, 99, Target::F32(0xbce3_7790_f0cc_d5f9)),
    int(1, 3, 0xbb37_0bfc_c18f_cf33, 12, 0, 99, Target::F32(0x8902_e3ce_e2b6_f75b)),
    int(2, 0, 0xdad7_1ae3_6e5b_7e62, 3, 1, 99, Target::TotalRounds),
    int(2, 1, 0x62f0_fea8_5dac_ba6c, 1, 0, 1, Target::Lo32(0x50ba_16e5_7e95_8204)), // both seen
    int(2, 2, 0x354d_f873_aa1b_2d91, 59, 0, 59, Target::F32(0x5746_a009_381c_12a9)), // max seen
    flag(2, 3, 0x107c_0c00_8fdb_0de4, false, Target::Lo8(0x7011_ca8f_367d_b1a2)),
    int(3, 0, 0x2a1b_a2d5_176a_95fe, 0, 0, 99, Target::RoundsWon(true)),
    int(3, 1, 0x6348_ae13_c1da_2b2c, 0, 0, 99, Target::RoundsWon(false)),
    int(3, 2, 0x39fb_9105_3df6_8963, 0, 0, 2, Target::Lo32(0x5ab0_f851_26ea_3b2b)), // max seen
    flag(3, 3, 0xa9c9_dc63_6923_c387, true, Target::Lo8(0x29e2_61a1_df77_4dd3)),
    flag(4, 0, 0x8865_777f_808b_b35e, true, Target::Lo8(0x1b93_265a_02d6_ca8e)),
    int(4, 1, 0xc84b_c1ce_05c1_19e2, 20, 0, 99, Target::Lo32(0x6a53_c5cc_3c3f_d9c8)),
    flag(4, 2, 0xd46e_79d8_053e_d4a5, true, Target::Lo8(0xe67c_d351_7ff5_f0e9)),
    flag(4, 3, 0xc937_64bc_a4fd_235d, false, Target::Lo8(0x90a0_1ed5_2450_59cf)),
    flag(5, 0, 0xfc3e_0ff3_01b1_c55d, true, Target::Lo8(0xea1c_33dc_bcc7_b49f)),
    flag(5, 1, 0x8880_1e52_8000_5aac, false, Target::None),
];

#[derive(Clone, Debug)]
pub struct Rules {
    pub v: [i32; RULES.len()],
}

impl Default for Rules {
    fn default() -> Self {
        Self { v: RULES.map(|r| r.default) }
    }
}

/// Body of `SR15NetUIIncSettingMsg` / `SR15NetUIDecSettingMsg`:
/// `[u32 0][u32 1][level u64][page u32][row u32]`.
pub fn parse_press(d: &[u8]) -> Option<(u32, u32)> {
    if d.len() < 24 {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
    Some((u(16), u(20)))
}

impl Rules {
    /// A private match's starting rules: the stock RULES screen with the
    /// host's `mp_arena_rules` private values (`round_time_private` and the
    /// rest; the stock ones are the screen's own defaults) and this server's
    /// `goalie_ping_advantage_private`.
    pub fn from_private(r: &serde_json::Value, goalie: Option<bool>) -> Self {
        let mut rules = Self::default();
        let rr = r.get("round_rules");
        let num = |v: Option<&serde_json::Value>| v.and_then(serde_json::Value::as_f64);
        let round = |k: &str| num(rr.and_then(|x| x.get(k)));
        if let Some(t) = round("round_time_private") {
            let t = t.max(0.0).round() as i32;
            rules.set(0, t / 60);
            rules.set(1, t % 60);
        }
        for (k, row) in [("round_count_private", ROW_ROUND_COUNT), ("rounds_played_private", ROW_ROUNDS_PLAYED), ("team_room_wait_time_private", ROW_START_COUNT), ("wait_for_launch_time_private", ROW_CATAPULT_TIME)] {
            if let Some(v) = round(k) {
                rules.set(row, v.round() as i32);
            }
        }
        if let Some(v) = num(r.get("mercy_win_point_spread_private")) {
            rules.set(ROW_MERCY_MARGIN, v.round() as i32);
        }
        for (v, row) in [(r.get("mercy_rule_private_enabled").and_then(serde_json::Value::as_bool), ROW_MERCY_ON), (r.get("self_goaling_private").and_then(serde_json::Value::as_bool), ROW_SELF_GOALING), (goalie, ROW_GOALIE_PING)] {
            if let Some(on) = v {
                rules.set(row, i32::from(on));
            }
        }
        rules
    }

    pub fn goalie_ping_advantage(&self) -> bool {
        self.v[ROW_GOALIE_PING] != 0
    }

    pub fn index(page: u32, row: u32) -> Option<usize> {
        RULES.iter().position(|r| r.page == page && r.row == row)
    }

    /// Set a rule outright (clamped); returns true when the value changed.
    pub fn set(&mut self, i: usize, v: i32) -> bool {
        let r = &RULES[i];
        let old = self.v[i];
        self.v[i] = v.clamp(r.min, r.max);
        self.v[i] != old
    }

    pub fn best_of(&self) -> bool {
        self.v[ROW_ROUNDS_PLAYED] != 0
    }

    pub fn carry_points(&self) -> bool {
        self.v[ROW_CARRY_POINTS] != 0
    }

    /// Apply one press; returns true when the value changed.
    pub fn press(&mut self, i: usize, inc: bool) -> bool {
        let r = &RULES[i];
        let old = self.v[i];
        self.v[i] = if r.boolean { 1 - old } else { (old + if inc { 1 } else { -1 }).clamp(r.min, r.max) };
        self.v[i] != old
    }

    pub fn round_secs(&self) -> f32 {
        (self.v[0] * 60 + self.v[1]) as f32
    }

    pub fn start_count(&self) -> f32 {
        self.v[ROW_START_COUNT] as f32
    }

    pub fn catapult_secs(&self) -> f32 {
        self.v[ROW_CATAPULT_TIME] as f32
    }

    pub fn self_goaling(&self) -> bool {
        self.v[ROW_SELF_GOALING] != 0
    }

    pub fn respawn_secs(&self) -> f32 {
        self.v[ROW_RESPAWN_TIME] as f32
    }

    pub fn overtime(&self) -> bool {
        self.v[ROW_OVERTIME] != 0
    }

    pub fn goal_stops_time(&self) -> bool {
        self.v[ROW_GOAL_STOPS_TIME] != 0
    }

    /// Starting scores (rows 0.2 / 0.3): [blue, orange].
    pub fn start_scores(&self) -> [u32; 2] {
        [self.v[2] as u32, self.v[3] as u32]
    }

    /// The margin that ends the round early, if the mercy rule is on.
    pub fn mercy_margin(&self) -> Option<u32> {
        (self.v[ROW_MERCY_ON] != 0 && self.v[ROW_MERCY_MARGIN] > 0).then_some(self.v[ROW_MERCY_MARGIN] as u32)
    }

    /// Stream-1 value of a rule.
    pub fn s1_value(&self, i: usize) -> Val {
        if RULES[i].boolean { Val::Lo8(self.v[i] as u8) } else { Val::Lo32(self.v[i] as u32) }
    }

    /// Stream-0 overrides a rule derives (besides the arena's own fields).
    pub fn s0_values(&self, i: usize) -> Vec<(u64, Val)> {
        let v = self.v[i];
        match RULES[i].target {
            Target::RoundTime => vec![(VAR_ROUND_TIME, Val::Lo32((self.v[0] * 60 + self.v[1]) as u32))],
            Target::Score(_, mirror) => vec![(mirror, Val::Lo32(v as u32))],
            Target::DiscSpawn => vec![(VAR_SPAWN_CHANGED, Val::Lo8(1))],
            Target::Lo8(h) => vec![(h, Val::Lo8(v as u8))],
            Target::Lo32(h) => vec![(h, Val::Lo32(v as u32))],
            Target::F32(h) => vec![(h, Val::F32(v as f32))],
            Target::None | Target::TotalRounds | Target::RoundsWon(_) => Vec::new(),
        }
    }
}

pub fn disc_spawn(setting: i32) -> u8 {
    match setting {
        2 => 1,
        1 => 2,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private match starts from the host's private values; the stock
    /// ones are the RULES screen's own defaults.
    #[test]
    fn private_rules_follow_the_private_settings() {
        let stock: serde_json::Value = serde_json::from_str(
            r#"{"mercy_win_point_spread_private":20,"mercy_rule_private_enabled":true,"self_goaling_private":true,
            "round_rules":{"round_count_private":3,"round_time_private":600.0,"rounds_played_private":1,
            "team_room_wait_time_private":59.0,"wait_for_launch_time_private":12.0}}"#,
        )
        .expect("json");
        assert_eq!(Rules::from_private(&stock, None).v, Rules::default().v);
        let host: serde_json::Value = serde_json::from_str(
            r#"{"mercy_win_point_spread_private":8,"mercy_rule_private_enabled":false,"self_goaling_private":false,
            "round_rules":{"round_count_private":5,"round_time_private":330.0,"wait_for_launch_time_private":5.0}}"#,
        )
        .expect("json");
        let r = Rules::from_private(&host, Some(true));
        assert_eq!(r.round_secs(), 330.0);
        assert_eq!(r.v[ROW_ROUND_COUNT], 5);
        assert_eq!(r.catapult_secs(), 5.0);
        assert_eq!(r.mercy_margin(), None);
        assert_eq!(r.v[ROW_MERCY_MARGIN], 8);
        assert!(!r.self_goaling());
        assert!(r.goalie_ping_advantage());
    }

    #[test]
    fn ints_clamp_and_bools_toggle() {
        let mut r = Rules::default();
        let start = Rules::index(2, 2).unwrap();
        assert!(!r.press(start, true), "59 is the max");
        assert!(r.press(start, false));
        assert_eq!(r.start_count(), 58.0);
        let b = Rules::index(4, 0).unwrap();
        assert!(r.press(b, false));
        assert_eq!(r.v[b], 0);
        assert!(r.press(b, false));
        assert_eq!(r.v[b], 1);
    }

    #[test]
    fn round_time_from_minutes_and_seconds() {
        let mut r = Rules::default();
        r.press(0, false);
        r.press(1, true);
        assert_eq!(r.round_secs(), 541.0);
        assert_eq!(r.s0_values(1), vec![(VAR_ROUND_TIME, Val::Lo32(541))]);
    }

    #[test]
    fn press_body_parses() {
        let mut d = vec![0u8; 24];
        d[4] = 1;
        d[16] = 3;
        d[20] = 2;
        assert_eq!(parse_press(&d), Some((3, 2)));
        assert_eq!(Rules::index(3, 2), Some(14));
    }

    #[test]
    fn api_body_parses() {
        let mut d = vec![0u8, 0, 0, 0, 1, 0, 0, 0, 0x4b, 0xbc, 0x8e, 0x42, 0xf8, 0xd3, 0x6e, 0x57];
        d.extend_from_slice(br#"{"GOAL STOPS TIME":false}"#);
        assert_eq!(parse_api(&d), Some((ROW_GOAL_STOPS_TIME, 0)));
        d.truncate(16);
        d.extend_from_slice(b"{\"MERCY SCORE DIFF\":20}\0");
        assert_eq!(parse_api(&d), Some((ROW_MERCY_MARGIN, 20)));
        d.truncate(16);
        d.extend_from_slice(br#"{"err_code":0}"#);
        assert_eq!(parse_api(&d), None);
    }

    #[test]
    fn baseline_matches_defaults() {
        let base = &captured().private_baseline;
        for (i, r) in RULES.iter().enumerate() {
            let raw = var_raw(&base[1], r.var).unwrap_or_else(|| panic!("rule {i} var missing from s1"));
            let lo = if r.boolean { raw & 0xff } else { raw & 0xffff_ffff };
            assert_eq!(lo as i32, r.default, "rule {}.{}", r.page, r.row);
        }
    }
}

#[cfg(test)]
mod wire_ids {
    use super::super::msg::*;
    #[test]
    fn stat_message_ids_match_native() {
        assert_eq!(STAT_UPDATE, 0x5d84_b0c2_d4bb_6cfc);
        assert_eq!(TEAM_STAT_UPDATE, 0x7ba4_44f7_d419_b1a2);
    }
}
