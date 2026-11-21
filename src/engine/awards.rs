//! End-of-match awards (`SR15NetTopAwardsMsg`), as native picks them.
//!
//! Source: the game's own `json/r14/multiplayer/player_rewards.json`
//! (`gamerewards`: per game mode, `maxawardsdisplayed` 3 plus the MVP, and
//! `awards` as name, stat, multiplier) and echovr.exe's
//! `ProcessPostMatchStats` (0x1405e8040), which reads it:
//!
//! 1. Each player scores every award: `round(stat x multiplier)` (+0.5,
//!    truncated). A positive one is a candidate; their sum is the player's
//!    `score`.
//! 2. The MVP is the player with the highest score (the first on a tie).
//! 3. Then, `maxawardsdisplayed` times: of the players without an award
//!    yet (the MVP has one), the one whose best candidate scores highest gets
//!    that award (their best; a tie keeps the earlier one in the table).
//!
//! The value shown is the stat itself, rounded; the MVP's is the score.
//! Awards commented out in the file (`speedster`, `mathematician`) are left
//! out here too.

/// One `awards` entry: award name, stat name, multiplier.
pub type AwardDef = (&'static str, &'static str, f32);

/// `echo_arena` and `echo_arena_private` (the same list).
pub const ARENA: [AwardDef; 16] = [
    ("bruiser", "stuns", 5.0),
    ("long_shot", "threepointgoals", 50.0),
    ("bouncer", "bouncegoals", 30.0),
    ("thief", "steals", 20.0),
    ("team_player", "assists", 30.0),
    ("guardian", "saves", 30.0),
    ("jouster", "joustswon", 40.0),
    ("unrelenting", "shotsongoal", 10.0),
    ("unstoppable", "goals", 20.0),
    ("greedy", "longestpossession", 4.0),
    ("eagle_eye", "passes", 20.0),
    ("quick_reflexes", "catches", 30.0),
    ("long_arm", "clears", 20.0),
    ("interceptor", "interceptions", 20.0),
    // Native counts 1-point goals only; ours adds slam dunks (2 points).
    ("dunker", "onepointgoals", 20.0),
    ("hat_tricker", "hattricks", 100.0),
];

/// `echo_combat` and `echo_combat_private` (the same list).
pub const COMBAT: [AwardDef; 15] = [
    ("super_star", "CombatEliminations", 30.0),
    ("defender", "CombatObjectiveEliminations", 120.0),
    ("guard", "CombatObjectiveTime", 10.0),
    ("controller", "CombatObjectiveDamage", 0.5),
    ("hot_shot", "CombatBestEliminationStreak", 120.0),
    ("assassin", "CombatKills", 80.0),
    ("specialist", "CombatAssists", 50.0),
    ("cannoneer", "CombatDamage", 0.4),
    ("sniper", "CombatHeadshotKills", 150.0),
    ("tank", "CombatDamageAbsorbed", 1.5),
    ("medic", "CombatHealing", 1.5),
    ("shock_trooper", "CombatStuns", 150.0),
    ("renegade", "CombatSoloKills", 200.0),
    ("infiltrator", "CombatHillCaptures", 150.0),
    ("immovable_object", "CombatHillDefends", 150.0),
];

/// `maxawardsdisplayed` for every arena and combat mode ("do not include MVP").
pub const MAX_AWARDS: usize = 3;

/// native's rounding: +0.5, truncated (toward zero).
fn round(x: f32) -> i64 {
    (x + 0.5) as i64
}

/// The awards: (player index, award index into `defs` or None for the MVP,
/// value shown). `stats[i][k]` is player i's value of `defs[k]`'s stat.
pub fn choose(defs: &[AwardDef], stats: &[Vec<f32>]) -> Vec<(usize, Option<usize>, u64)> {
    let mut out = Vec::new();
    if stats.is_empty() {
        return out;
    }
    // Each player's candidates, best first (a stable sort: ties keep the
    // table's order), and their score.
    let cands: Vec<Vec<(usize, i64)>> = stats
        .iter()
        .map(|v| {
            let mut c: Vec<(usize, i64)> = defs.iter().enumerate().map(|(k, d)| (k, round(v.get(k).copied().unwrap_or(0.0) * d.2))).filter(|(_, s)| *s > 0).collect();
            c.sort_by_key(|x| std::cmp::Reverse(x.1));
            c
        })
        .collect();
    let score = |i: usize| cands[i].iter().map(|(_, s)| s).sum::<i64>();
    let mut mvp = 0;
    for i in 1..stats.len() {
        if score(i) > score(mvp) {
            mvp = i;
        }
    }
    out.push((mvp, None, score(mvp).max(0) as u64));
    let mut taken = vec![mvp];
    // Each award goes out once: two players both shown QUICK REFLEXES was a
    // bug report. A player whose best one is gone gets their next best.
    let mut given: Vec<usize> = Vec::new();
    for _ in 0..MAX_AWARDS {
        let best = (0..stats.len()).filter(|i| !taken.contains(i)).filter_map(|i| cands[i].iter().find(|(k, _)| !given.contains(k)).map(|&(k, s)| (i, k, s))).fold(None, |b: Option<(usize, usize, i64)>, c| match b {
            Some(b) if b.2 >= c.2 => Some(b),
            _ => Some(c),
        });
        let Some((i, k, _)) = best else { break };
        taken.push(i);
        given.push(k);
        out.push((i, Some(k), round(stats[i][k]).max(0) as u64));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_award_goes_to_two_players() {
        let defs = ARENA;
        let n = defs.len();
        let mut players = vec![vec![0.0; n]; 4];
        players[0] = vec![50.0; n];
        players[1][0] = 9.0;
        players[2][0] = 8.0;
        players[2][1] = 0.5;
        players[3][0] = 7.0;
        players[3][2] = 0.5;
        let out = choose(&defs, &players);
        let kinds: Vec<usize> = out.iter().filter_map(|o| o.1).collect();
        assert_eq!(kinds.len(), 3);
        let mut d = kinds.clone();
        d.sort();
        d.dedup();
        assert_eq!(d.len(), kinds.len(), "{kinds:?}");
    }

    /// Native combat (client-combat-20261116-b): MVP, then guard 162,
    /// defender 10, cannoneer 2021: each the best award of the next player.
    #[test]
    fn each_award_goes_to_the_player_it_suits_best() {
        let defs = COMBAT;
        let at = |name: &str| defs.iter().position(|d| d.1 == name).expect("stat");
        let mut players = vec![vec![0.0; defs.len()]; 4];
        players[0][at("CombatEliminations")] = 20.0;
        players[0][at("CombatObjectiveTime")] = 200.0;
        players[1][at("CombatObjectiveTime")] = 162.0;
        players[1][at("CombatEliminations")] = 3.0;
        players[2][at("CombatObjectiveEliminations")] = 10.0;
        players[3][at("CombatDamage")] = 2021.0;
        let got: Vec<(usize, Option<&str>, u64)> = choose(&defs, &players).into_iter().map(|(i, k, v)| (i, k.map(|k| defs[k].0), v)).collect();
        assert_eq!(got, vec![(0, None, 2600), (1, Some("guard"), 162), (2, Some("defender"), 10), (3, Some("cannoneer"), 2021)]);
    }

    #[test]
    fn nobody_gets_two_and_empty_players_get_none() {
        let defs = ARENA;
        let mut players = vec![vec![0.0; defs.len()]; 3];
        players[0][0] = 4.0; // stuns
        players[1][10] = 2.0; // passes
        let got = choose(&defs, &players);
        // 2 passes x 20 = 40 beats 4 stuns x 5 = 20: the passer is the MVP,
        // the stunner gets Bruiser, the third player (nothing) none.
        assert_eq!(got, vec![(1, None, 40), (0, Some(0), 4)]);
    }

    #[test]
    fn award_names_are_the_games_symbols() {
        for (name, id) in [("bruiser", 0x2fd5_9d8a_5767_5298u64), ("eagle_eye", 0x4fce_de87_b2b2_913b), ("quick_reflexes", 0xdafa_9a46_0fe1_0167), ("unstoppable", 0x4303_0e90_92f3_c421)] {
            assert_eq!(crate::sns::symbol(name), id);
            assert!(ARENA.iter().any(|d| d.0 == name));
        }
        assert_eq!(crate::sns::symbol("score"), super::super::msg::AWARD_MVP_STAT);
    }
}
