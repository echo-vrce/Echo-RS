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

    #[test]
    fn picks_come_from_the_table() {
        for _ in 0..200 {
            assert!(SAVE.contains(&pick(&SAVE)));
        }
        assert_eq!(goal_table(3).len(), 19);
        assert_eq!(goal_table(2).len(), 14);
    }
}
