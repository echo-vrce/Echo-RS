//! Native arena announcer picks, from the stock server script
//! `mp_announcer_server_state` (e28785a6428603f9).
//!
//! Each of its `R15NetSendEventNode`s names its cue with an expression that
//! picks one entry of a fixed table at random (uniform). The tables below are
//! those expressions' tables, entry for entry, read by running each
//! expression's code from the script DLL (2026-11-19). The engine events
//! that start them, and the delays before the send:
//!
//! | engine event | table | delay |
//! |---|---|---|
//! | `nevt_player_save` | `SAVE` | none |
//! | `nevt_blue_intercepted` | `INTERCEPTION_BLUE` | 0.75 s |
//! | `nevt_orange_intercepted` | `INTERCEPTION_ORANGE` | 0.75 s |
//! | `nevt_disc_stolen` | `steal_sound` at once, `STEAL` after | 0.75 s |
//! | `nevt_game_score` (arena) | `GOAL_1/2/3` by `lastscorepoints` | none |
//!
//! Native's public overtime recording (netrec-pub-ot-20261119) ties them to
//! the stats: every save, interception and steal cue falls on that player's
//! Saves / Interceptions / Steals going up, and "blue/orange intercepted" is
//! the interceptor's team (both `INTERCEPTION_RED` lines there were orange's).
//!
//! Some entries are hashes no client announcer script listens for (no name
//! known, no line plays): native sends them too, and a pick of one is silent.

/// `steal_sound`, sent at once on a steal (a second `nevt_disc_stolen` handler).
pub const STEAL_SOUND: u64 = 0x8cad_dcde_3f3c_e5c4;

/// Seconds between the interception or steal and its call.
pub const CALL_DELAY_S: f32 = 0.75;

pub const SAVE: [u64; 25] = [
    0xb2f0_9025_107e_3053, // MP_ANNOUNCER_0110E_SAVE
    0xb2f0_9125_107e_3053, // MP_ANNOUNCER_0110D_SAVE
    0xb2f0_9225_107e_3053, // MP_ANNOUNCER_0110G_SAVE
    0xb2f0_9325_107e_3053, // MP_ANNOUNCER_0110F_SAVE
    0xb2f0_9425_107e_3053, // MP_ANNOUNCER_0110A_SAVE
    0xb2f0_9625_107e_3053, // MP_ANNOUNCER_0110C_SAVE
    0xb2f0_9725_107e_3053, // MP_ANNOUNCER_0110B_SAVE
    0xb2f0_9c25_107e_3053, // MP_ANNOUNCER_0110I_SAVE
    0xb2f0_9d25_107e_3053, // MP_ANNOUNCER_0110H_SAVE
    0xb2f0_9e25_107e_3053, // MP_ANNOUNCER_0110K_SAVE
    0xb2f0_9f25_107e_3053, // MP_ANNOUNCER_0110J_SAVE
    0xb904_ceb6_3cc2_815b, // MPL_ARENA_SAVE_I1
    0xb904_ceb6_3cc2_825b, // MPL_ARENA_SAVE_J1
    0xb904_ceb6_3cc2_835b, // MPL_ARENA_SAVE_K1
    0xb904_ceb6_3cc2_845b, // MPL_ARENA_SAVE_L1
    0xb904_ceb6_3cc2_855b, // MPL_ARENA_SAVE_M1
    0xb904_ceb6_3cc2_865b, // MPL_ARENA_SAVE_N1
    0xb904_ceb6_3cc2_875b, // MPL_ARENA_SAVE_O1
    0xb904_ceb6_3cc2_895b, // MPL_ARENA_SAVE_A1
    0xb904_ceb6_3cc2_8a5b, // MPL_ARENA_SAVE_B1
    0xb904_ceb6_3cc2_8b5b, // MPL_ARENA_SAVE_C1
    0xb904_ceb6_3cc2_8c5b, // MPL_ARENA_SAVE_D1
    0xb904_ceb6_3cc2_8d5b, // MPL_ARENA_SAVE_E1
    0xb904_ceb6_3cc2_8f5b, // MPL_ARENA_SAVE_G1
    0x077b_2407_8ee7_bdd0, // (no listener)
];

pub const INTERCEPTION_BLUE: [u64; 5] = [
    0x0e96_864c_e904_c3a1, // MP_ANNOUNCER_0010A_INTERCEPTION_GENERIC
    0xb46d_f126_d117_57c1, // MP_ANNOUNCER_0020A_INTERCEPTION_BLUE
    0xc100_0bbd_b3c5_04fb, // MPL_ARENA_INTERCEPTION_B1
    0xc100_0bbd_b3c5_07fb, // MPL_ARENA_INTERCEPTION_A1
    0x2e6c_848c_4375_edf3, // MP_ANNOUNCER_0010B_INTERCEPTION_GENERIC
];

pub const INTERCEPTION_ORANGE: [u64; 5] = [
    0x0e96_864c_e904_c3a1, // MP_ANNOUNCER_0010A_INTERCEPTION_GENERIC
    0xc100_0bbd_b3c5_04fb, // MPL_ARENA_INTERCEPTION_B1
    0xc100_0bbd_b3c5_07fb, // MPL_ARENA_INTERCEPTION_A1
    0x2e6c_848c_4375_edf3, // MP_ANNOUNCER_0010B_INTERCEPTION_GENERIC
    0x4cf8_c8ad_f6bb_7b58, // MP_ANNOUNCER_0010A_INTERCEPTION_RED
];

pub const STEAL: [u64; 10] = [
    0x9382_7d2d_dfc8_33c8, // MP_ANNOUNCER_0030E_DISC_STOLEN
    0xa5ca_6959_0630_3de6, // MP_ANNOUNCER_0030F_DISC_STOLEN
    0xd606_6b03_a7e2_2018, // MP_ANNOUNCER_0030B_DISC_STOLEN
    0x15eb_4bd6_aa49_8d24, // MP_ANNOUNCER_0030C_DISC_STOLEN
    0xe04e_7f77_7e1a_2e36, // MP_ANNOUNCER_0030A_DISC_STOLEN
    0x076e_3703_bdde_e8eb, // MPL_ARENA_STEAL_D1
    0x076e_3703_bdde_e9eb, // MPL_ARENA_STEAL_E1
    0x076e_3703_bdde_eeeb, // MPL_ARENA_STEAL_B1
    0x076e_3703_bdde_efeb, // MPL_ARENA_STEAL_C1
    0x506f_5df8_d263_9ef4, // MP_ANNOUNCER_0030D_DISC_STOLEN
];

pub const GOAL_1: [u64; 10] = [
    0x91b4_c7d7_f101_cb7a, // MP_ANNOUNCER_0180F_GOAL_1_POINT
    0xc2b3_ce89_58f3_e79a, // MP_ANNOUNCER_0180C_GOAL_1_POINT
    0x16d1_cb39_6dcd_33d8, // MP_ANNOUNCER_0180D_GOAL_1_POINT
    0xfcc0_c9bb_41f5_d5e6, // MP_ANNOUNCER_0180E_GOAL_1_POINT
    0x0399_d184_d2d0_a918, // MP_ANNOUNCER_0180J_GOAL_1_POINT
    0x28a2_cc0b_74cb_01a4, // MP_ANNOUNCER_0180B_GOAL_1_POINT
    0x45d6_c267_c43f_1f38, // MP_ANNOUNCER_0180A_GOAL_1_POINT
    0x6eed_dfe8_6224_b784, // MP_ANNOUNCER_0180I_GOAL_1_POINT
    0x84fc_dd6a_4e1c_51ba, // MP_ANNOUNCER_0180H_GOAL_1_POINT
    0x893e_0a5d_5c06_4cf8, // (no listener)
];

pub const GOAL_2: [u64; 14] = [
    0x8ce5_635d_4d96_471e, // MP_ANNOUNCER_0190E_GOAL_2_POINT
    0xb296_646f_5490_7562, // MP_ANNOUNCER_0190C_GOAL_2_POINT
    0xe191_6d31_fd62_5982, // MP_ANNOUNCER_0190F_GOAL_2_POINT
    0xf4d9_778c_427f_c342, // MP_ANNOUNCER_0190H_GOAL_2_POINT
    0x1ec8_750e_6e47_257c, // MP_ANNOUNCER_0190I_GOAL_2_POINT
    0x35f3_6881_c85c_8dc0, // MP_ANNOUNCER_0190A_GOAL_2_POINT
    0x47a2_4a82_0346_08cd, // MPL_ARENA_2POINT_C1
    0x47a2_4a82_0346_09cd, // MPL_ARENA_2POINT_B1
    0x47a2_4a82_0346_0acd, // MPL_ARENA_2POINT_A1
    0x47a2_4a82_0346_0fcd, // MPL_ARENA_2POINT_D1
    0x5887_66ed_78a8_935c, // MP_ANNOUNCER_0190B_GOAL_2_POINT
    0x66f4_61df_61ae_a120, // MP_ANNOUNCER_0190D_GOAL_2_POINT
    0x0b80_6fb3_d15a_bfbc, // MP_ANNOUNCER_0190G_GOAL_2_POINT
    0x73bc_7b62_deb3_3be0, // MP_ANNOUNCER_0190J_GOAL_2_POINT
];

pub const GOAL_3: [u64; 19] = [
    0x914d_ca5b_fa62_213a, // MP_ANNOUNCER_0200B_GOAL_3_POINT
    0xaf3e_cd69_e364_1346, // MP_ANNOUNCER_0200D_GOAL_3_POINT
    0xc24a_c305_5390_0dda, // MP_ANNOUNCER_0200G_GOAL_3_POINT
    0xfc39_c437_4a96_3fa6, // MP_ANNOUNCER_0200A_GOAL_3_POINT
    0x285b_c187_7fa8_ebe4, // MP_ANNOUNCER_0200F_GOAL_3_POINT
    0x452f_cfeb_cf5c_f578, // MP_ANNOUNCER_0200E_GOAL_3_POINT
    0x6cfb_6cd1_5bd1_90a7, // MPL_ARENA_3POINT_H1
    0x6cfb_6cd1_5bd1_91a7, // MPL_ARENA_3POINT_I1
    0x6cfb_6cd1_5bd1_92a7, // MPL_ARENA_3POINT_J1
    0x6cfb_6cd1_5bd1_93a7, // MPL_ARENA_3POINT_K1
    0x6cfb_6cd1_5bd1_94a7, // MPL_ARENA_3POINT_L1
    0x6cfb_6cd1_5bd1_99a7, // MPL_ARENA_3POINT_A1
    0x6cfb_6cd1_5bd1_9aa7, // MPL_ARENA_3POINT_B1
    0x6cfb_6cd1_5bd1_9ba7, // MPL_ARENA_3POINT_C1
    0x6cfb_6cd1_5bd1_9ca7, // MPL_ARENA_3POINT_D1
    0x6cfb_6cd1_5bd1_9da7, // MPL_ARENA_3POINT_E1
    0x6cfb_6cd1_5bd1_9ea7, // MPL_ARENA_3POINT_F1
    0x6cfb_6cd1_5bd1_9fa7, // MPL_ARENA_3POINT_G1
    0x7b5c_c8d9_d65a_c704, // MP_ANNOUNCER_0200C_GOAL_3_POINT
];

/// One uniform pick, as the stock expressions do.
pub fn pick(table: &[u64]) -> u64 {
    table[rand::random::<usize>() % table.len()]
}

/// The goal call's table for a goal of `points`.
pub fn goal_table(points: u32) -> &'static [u64] {
    match points {
        0 | 1 => &GOAL_1,
        2 => &GOAL_2,
        _ => &GOAL_3,
    }
}

// ── Our own calls ─────────────────────────────────────────────────────────
//
// Lines the client announcer scripts play but the stock server script never
// sends (the `MPL_ARENA_*` scenes), and the stock groups whose engine events
// are not decoded (clears, possession, the post, punches, rampage). When each
// plays is this server's choice (docs/announcer_lines.md); the cues are the
// lines' own names, as the clients listen for them.

/// A line by its cue name, or an unnamed cue's own hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Name(&'static str),
    Hash(u64),
}

impl Cue {
    pub fn symbol(self) -> u64 {
        match self {
            Cue::Name(n) => crate::sns::symbol(n),
            Cue::Hash(h) => h,
        }
    }
}

use Cue::{Hash, Name};

/// Slam dunk goals. `DUNK_FAVOURITE` takes `DUNK_FAVOURITE_SHARE` of the
/// calls (the host's pick, 2026-11-20); the rest share what is left.
pub const DUNK: [Cue; 8] = [
    Name("MPL_ARENA_DUNK_A1"), // What a dunk!
    Name("MPL_ARENA_DUNK_B1"), // That's a dunk!
    Name("MPL_ARENA_DUNK_C1"), // 2 points for the dunk!
    Name("MPL_ARENA_DUNK_D1"), // BOOM.
    Name("MPL_ARENA_DUNK_E1"), // Special delivery!
    Name("MPL_ARENA_DUNK_F1"), // Straight to the goal for two!
    Name("MPL_ARENA_DUNK_H1"), // Boom goes the dynamite!
    Name("MPL_ARENA_DUNK_I1"), // BAAAAANG! That one goes in.
];
pub const DUNK_FAVOURITE: Cue = Name("MPL_ARENA_DUNK_G1"); // Dipsy-doo Dunk-a-roo.
pub const DUNK_FAVOURITE_SHARE: f32 = 0.5;

/// Goals off a wall (bounce shots).
pub const BOUNCE: [Cue; 9] = [
    Name("MPL_ARENA_BOUNCE_A1"),
    Name("MPL_ARENA_BOUNCE_B1"),
    Name("MPL_ARENA_BOUNCE_C1"),
    Name("MPL_ARENA_BOUNCE_D1"),
    Name("MPL_ARENA_BOUNCE_E1"),
    Name("MPL_ARENA_BOUNCE_F1"),
    Name("MPL_ARENA_BOUNCE_G1"),
    Name("MPL_ARENA_BOUNCE_H1"),
    Name("MPL_ARENA_BOUNCE_I1"),
];

/// A player's third (sixth, ...) goal of the match.
pub const TRICK: [Cue; 5] =
    [Name("MPL_ARENA_TRICK_A1"), Name("MPL_ARENA_TRICK_B1"), Name("MPL_ARENA_TRICK_C1"), Name("MPL_ARENA_TRICK_D1"), Name("MPL_ARENA_TRICK_E1")];

/// An assisted goal (`PASS_SHARE` of them; the others keep the points' call).
pub const PASS: [Cue; 8] = [
    Name("MPL_ARENA_PASS_A1"),
    Name("MPL_ARENA_PASS_B1"),
    Name("MPL_ARENA_PASS_C1"),
    Name("MPL_ARENA_PASS_D1"),
    Hash(0x047b_2111_8ee4_bdd0), // MPL_ARENA_PASS_E1: Passes down range.
    Name("MPL_ARENA_PASS_F1"),
    Name("MPL_ARENA_PASS_G1"),
    Name("MPL_ARENA_PASS_H1"),
];
pub const PASS_SHARE: f32 = 0.5;

/// A shot on goal that neither scores, is saved nor comes off the goal.
pub const SHOT_ON_GOAL: [Cue; 6] = [
    Name("MPL_ARENA_SHOTONGOAL_A1"),
    Name("MPL_ARENA_SHOTONGOAL_B1"),
    Name("MPL_ARENA_SHOTONGOAL_C1"),
    Name("MPL_ARENA_SHOTONGOAL_D1"),
    Name("MPL_ARENA_SHOTONGOAL_E1"),
    Name("MPL_ARENA_SHOTONGOAL_F2"),
];

/// A shot on goal that comes off the goal's ring or the wall around it.
pub const HIT_POST: [Cue; 12] = [
    Name("MP_ANNOUNCER_0170A_HIT_POST"),
    Name("MP_ANNOUNCER_0170B_HIT_POST"),
    Name("MP_ANNOUNCER_0170C_HIT_POST"),
    Name("MP_ANNOUNCER_0170D_HIT_POST"),
    Name("MP_ANNOUNCER_0170E_HIT_POST"),
    Name("MP_ANNOUNCER_0170F_HIT_POST"),
    Name("MP_ANNOUNCER_0170G_HIT_POST"),
    Name("MP_ANNOUNCER_0170H_HIT_POST"),
    Name("MP_ANNOUNCER_0170I_HIT_POST"),
    Name("MP_ANNOUNCER_0170J_HIT_POST"),
    Name("MP_ANNOUNCER_0170K_HIT_POST"),
    Name("MP_ANNOUNCER_0170L_HIT_POST"),
];

/// The first grab after a launch: the winning team's own lines and the
/// general ones.
pub const JOUST_BLUE: [Cue; 2] = [Name("MPL_ARENA_JOUST_A1"), Name("MPL_ARENA_JOUST_C1")];
pub const JOUST_ORANGE: [Cue; 2] = [Name("MPL_ARENA_JOUST_B1"), Name("MPL_ARENA_JOUST_D1")];
pub const JOUST_ANY: [Cue; 4] = [
    Hash(0x61aa_93e1_405d_2d86), // MPL_ARENA_JOUST_E1: High speed getaway!
    Name("MPL_ARENA_JOUST_F1"),
    Name("MPL_ARENA_JOUST_G1"),
    Name("MPL_ARENA_JOUST_H1"),
];

/// A clear (a throw from the thrower's own end down the arena): the team's
/// stock lines and the general ones.
pub const CLEAR_BLUE: [Cue; 5] = [
    Name("MP_ANNOUNCER_0160A_CLEAR_DISC_BLUE"),
    Name("MP_ANNOUNCER_0160B_CLEAR_DISC_BLUE"),
    Name("MP_ANNOUNCER_0160C_CLEAR_DISC_BLUE"),
    Name("MP_ANNOUNCER_0150D_CLEAR_DISC_BLUE"),
    Name("MP_ANNOUNCER_0160E_CLEAR_DISC_BLUE"),
];
pub const CLEAR_ORANGE: [Cue; 5] = [
    Name("MP_ANNOUNCER_0150A_CLEAR_DISC_RED"),
    Name("MP_ANNOUNCER_0150B_CLEAR_DISC_RED"),
    Name("MP_ANNOUNCER_0150C_CLEAR_DISC_RED"),
    Name("MP_ANNOUNCER_0150D_CLEAR_DISC_RED"),
    Name("MP_ANNOUNCER_0150E_CLEAR_DISC_RED"),
];
pub const CLEAR_ANY: [Cue; 7] = [
    Name("MPL_ARENA_CLEAR_A1"),
    Name("MPL_ARENA_CLEAR_B1"),
    Name("MPL_ARENA_CLEAR_C1"),
    Name("MPL_ARENA_CLEAR_D1"),
    Name("MPL_ARENA_CLEAR_E1"),
    Name("MPL_ARENA_CLEAR_F1"),
    Name("MPL_ARENA_CLEAR_G1"),
];

/// A disc nobody had held for `POSSESSION_LOOSE_S` or more, picked up by
/// the other team ("Blue picks up the disc." has no cue).
pub const POSSESSION_BLUE: [Cue; 7] = [
    Name("MP_ANNOUNCER_0140A_CHANGE_POSSESSION_BLUE"),
    Name("MP_ANNOUNCER_0140C_CHANGE_POSSESSION_BLUE"),
    Name("MP_ANNOUNCER_0140D_CHANGE_POSSESSION_BLUE"),
    Name("MP_ANNOUNCER_0140E_CHANGE_POSSESSION_BLUE"),
    Name("MP_ANNOUNCER_0140F_CHANGE_POSSESSION_BLUE"),
    Name("MP_ANNOUNCER_0140G_CHANGE_POSSESSION_BLUE"),
    Name("MP_ANNOUNCER_0140H_CHANGE_POSSESSION_BLUE"),
];
pub const POSSESSION_ORANGE: [Cue; 8] = [
    Name("MP_ANNOUNCER_0130A_CHANGE_POSSESSION_RED"),
    Name("MP_ANNOUNCER_0130B_CHANGE_POSSESSION_RED"),
    Name("MP_ANNOUNCER_0130C_CHANGE_POSSESSION_RED"),
    Name("MP_ANNOUNCER_0130D_CHANGE_POSSESSION_RED"),
    Name("MP_ANNOUNCER_0130E_CHANGE_POSSESSION_RED"),
    Name("MP_ANNOUNCER_0130F_CHANGE_POSSESSION_RED"),
    Name("MP_ANNOUNCER_0130G_CHANGE_POSSESSION_RED"),
    Name("MP_ANNOUNCER_0130H_CHANGE_POSSESSION_RED"),
];
pub const POSSESSION_LOOSE_S: f32 = 2.0;

/// A stun: the stock punch lines and the `MPL_ARENA_PUNCH` scenes.
pub const PUNCH: [Cue; 17] = [
    Name("MP_ANNOUNCER_0040A_PUNCH"),
    Name("MP_ANNOUNCER_0040B_PUNCH"),
    Name("MP_ANNOUNCER_0040C_PUNCH"),
    Name("MP_ANNOUNCER_0040D_PUNCH"),
    Name("MP_ANNOUNCER_0040E_PUNCH"),
    Name("MP_ANNOUNCER_0040F_PUNCH"),
    Name("MP_ANNOUNCER_0040G_PUNCH"),
    Name("MP_ANNOUNCER_0040H_PUNCH"),
    Name("MP_ANNOUNCER_0040I_PUNCH"),
    Name("MPL_ARENA_PUNCH_A1"),
    Name("MPL_ARENA_PUNCH_B1"),
    Name("MPL_ARENA_PUNCH_C1"),
    Name("MPL_ARENA_PUNCH_D1"),
    Name("MPL_ARENA_PUNCH_E1"),
    Name("MPL_ARENA_PUNCH_F1"),
    Name("MPL_ARENA_PUNCH_G1"),
    Name("MPL_ARENA_PUNCH_H1"),
];
/// Native's delay before a punch call.
pub const PUNCH_DELAY_S: f32 = 0.5;

/// `RAMPAGE_STUNS` stuns within `RAMPAGE_WINDOW_S`: a rampage call instead
/// of the punch call, then no other for `RAMPAGE_COOLDOWN_S`.
pub const RAMPAGE: [Cue; 9] = [
    Name("MP_ANNOUNCER_0090A_RAMPAGE"),
    Name("MP_ANNOUNCER_0090B_RAMPAGE"),
    Name("MP_ANNOUNCER_0090C_RAMPAGE"),
    Name("MP_ANNOUNCER_0090D_RAMPAGE"),
    Name("MP_ANNOUNCER_0090E_RAMPAGE"),
    Name("MP_ANNOUNCER_0090F_RAMPAGE"),
    Name("MP_ANNOUNCER_0090G_RAMPAGE"),
    Name("MP_ANNOUNCER_0090H_RAMPAGE"),
    Name("MP_ANNOUNCER_0090I_RAMPAGE"),
];
pub const RAMPAGE_STUNS: usize = 4;
pub const RAMPAGE_WINDOW_S: f32 = 6.0;
pub const RAMPAGE_COOLDOWN_S: f32 = 15.0;

/// One more save line the stock table leaves out.
pub const SAVE_EXTRA: [Cue; 1] = [Name("MPL_ARENA_SAVE_F1")]; // Not this time!

/// A shot's call waits this long for a goal, save or post first.
pub const SHOT_WAIT_S: f32 = 3.5;
/// Clears, possession, misses and jousts give way to other calls: none
/// within this long of another.
pub const MINOR_GAP_S: f32 = 2.0;

/// One uniform pick across `tables`.
pub fn pick_cue(tables: &[&[Cue]]) -> u64 {
    let n: usize = tables.iter().map(|t| t.len()).sum();
    let mut i = rand::random::<usize>() % n.max(1);
    for t in tables {
        if i < t.len() {
            return t[i].symbol();
        }
        i -= t.len();
    }
    0
}

/// A save: native's table and the line it leaves out.
pub fn save() -> u64 {
    if rand::random::<usize>() % (SAVE.len() + 1) == 0 { SAVE_EXTRA[0].symbol() } else { pick(&SAVE) }
}

/// A slam dunk's call: the favourite `DUNK_FAVOURITE_SHARE` of the time.
pub fn dunk() -> u64 {
    if rand::random::<f32>() < DUNK_FAVOURITE_SHARE { DUNK_FAVOURITE.symbol() } else { pick_cue(&[&DUNK]) }
}

/// The call for a goal: a hat trick first, then a dunk, a bounce shot, an
/// assisted goal (`PASS_SHARE` of them), else native's call for its points.
/// Own goals keep native's.
pub fn goal_call(points: u32, goal_type: u16, assisted: bool, hat_trick: bool) -> u64 {
    use super::stats::{GOAL_BOUNCE_SHOT, GOAL_LONG_BOUNCE_SHOT, GOAL_SELF, GOAL_SLAM_DUNK};
    if goal_type == GOAL_SELF {
        return pick(goal_table(points));
    }
    if hat_trick {
        return pick_cue(&[&TRICK]);
    }
    match goal_type {
        GOAL_SLAM_DUNK => dunk(),
        GOAL_BOUNCE_SHOT | GOAL_LONG_BOUNCE_SHOT => pick_cue(&[&BOUNCE]),
        _ if assisted && rand::random::<f32>() < PASS_SHARE => pick_cue(&[&PASS]),
        _ => pick(goal_table(points)),
    }
}

/// The joust call for the team that won it (true: blue).
pub fn joust(blue: bool) -> u64 {
    pick_cue(&[if blue { &JOUST_BLUE } else { &JOUST_ORANGE }, &JOUST_ANY])
}

/// The clear call for the clearing team (true: blue).
pub fn clear(blue: bool) -> u64 {
    pick_cue(&[if blue { &CLEAR_BLUE } else { &CLEAR_ORANGE }, &CLEAR_ANY])
}

/// The possession call for the team that picked the disc up (true: blue).
pub fn possession(blue: bool) -> u64 {
    pick_cue(&[if blue { &POSSESSION_BLUE } else { &POSSESSION_ORANGE }])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The named entries are the symbols of their names (the comments).
    #[test]
    fn named_entries_hash_to_their_names() {
        let s = |n: &str| crate::sns::symbol(n);
        assert_eq!(SAVE[5], s("MP_ANNOUNCER_0110C_SAVE"));
        assert_eq!(SAVE[18], s("MPL_ARENA_SAVE_A1"));
        assert_eq!(INTERCEPTION_BLUE[1], s("MP_ANNOUNCER_0020A_INTERCEPTION_BLUE"));
        assert_eq!(INTERCEPTION_ORANGE[4], s("MP_ANNOUNCER_0010A_INTERCEPTION_RED"));
        assert_eq!(STEAL[4], s("MP_ANNOUNCER_0030A_DISC_STOLEN"));
        assert_eq!(STEAL_SOUND, s("steal_sound"));
        assert_eq!(GOAL_2[11], crate::engine::msg::EV_ANNOUNCER_GOAL_2);
        assert_eq!(GOAL_3[6], crate::engine::msg::EV_ANNOUNCER_GOAL_3);
        assert_eq!(GOAL_1[6], s("MP_ANNOUNCER_0180A_GOAL_1_POINT"));
    }

    /// Our own lines' cues are the names the client scripts listen for,
    /// each line once.
    #[test]
    fn own_cues_are_the_lines_names() {
        assert_eq!(DUNK_FAVOURITE.symbol(), crate::sns::symbol("MPL_ARENA_DUNK_G1"));
        assert_eq!(PASS[4].symbol(), 0x047b_2111_8ee4_bdd0);
        let all: [&[Cue]; 17] = [
            &DUNK,
            &BOUNCE,
            &TRICK,
            &PASS,
            &SHOT_ON_GOAL,
            &HIT_POST,
            &JOUST_BLUE,
            &JOUST_ORANGE,
            &JOUST_ANY,
            &CLEAR_BLUE,
            &CLEAR_ORANGE,
            &CLEAR_ANY,
            &POSSESSION_BLUE,
            &POSSESSION_ORANGE,
            &PUNCH,
            &RAMPAGE,
            &SAVE_EXTRA,
        ];
        let mut seen = std::collections::HashSet::new();
        for t in all {
            for c in t {
                assert!(seen.insert(c.symbol()), "{c:?} twice");
            }
        }
        assert!(seen.insert(DUNK_FAVOURITE.symbol()));
        for stock in SAVE.iter().chain(&STEAL).chain(&GOAL_1).chain(&GOAL_2).chain(&GOAL_3) {
            assert!(!seen.contains(stock), "{stock:#x} is native's already");
        }
    }

    /// "Dipsy-doo Dunk-a-roo." is about half the dunk calls; every other
    /// dunk line still comes up. A hat trick outranks the dunk, and own
    /// goals keep native's call.
    #[test]
    fn dipsy_doo_takes_half_the_dunks() {
        use crate::engine::stats::{GOAL_BOUNCE_SHOT, GOAL_SELF, GOAL_SLAM_DUNK};
        let fav = DUNK_FAVOURITE.symbol();
        let n = 4000;
        let calls: Vec<u64> = (0..n).map(|_| goal_call(2, GOAL_SLAM_DUNK, false, false)).collect();
        let share = calls.iter().filter(|&&c| c == fav).count() as f32 / n as f32;
        assert!((0.44..0.56).contains(&share), "{share}");
        for c in DUNK {
            assert!(calls.contains(&c.symbol()), "{c:?}");
        }
        let trick = goal_call(2, GOAL_SLAM_DUNK, false, true);
        assert!(TRICK.iter().any(|c| c.symbol() == trick));
        assert!(GOAL_2.contains(&goal_call(2, GOAL_SELF, true, true)));
        let bounce = goal_call(3, GOAL_BOUNCE_SHOT, false, false);
        assert!(BOUNCE.iter().any(|c| c.symbol() == bounce));
    }

    #[test]
    fn picks_come_from_the_table() {
        for _ in 0..200 {
            assert!(SAVE.contains(&pick(&SAVE)));
        }
        assert_eq!(goal_table(3).len(), 19);
        assert_eq!(goal_table(2).len(), 14);
    }
}
