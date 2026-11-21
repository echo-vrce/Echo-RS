//! Message type ids (MatSym hashes of the C++ type names), replicated-variable
//! and event symbols, and the captured payloads the client needs verbatim.

use rad_protocol::message::MsgType as M;
use rad_protocol::symbol::MatSym;

const fn h(name: &str) -> u64 {
    MatSym::hash(name).data
}

pub const CONNECT: u64 = M::SBROADCASTER_CONNECT_EVENT.data();
pub const ENCODING_SUPPORT: u64 = M::SBROADCASTER_MSG_ENCODING_SUPPORT_EVENT_1.data();
pub const ENCODING_FAIL: u64 = M::SBROADCASTER_MSG_ENCODING_FAIL_EVENT.data();
pub const ENABLE_COMPRESSION: u64 = M::SBROADCASTER_ENABLE_COMPRESSION_EVENT_1.data();
pub const PING_UPDATE: u64 = M::SBROADCASTER_PING_UPDATE.data();
pub const KEEPALIVE: u64 = M::SBROADCASTER_NOOP_KEEPALIVE.data();
pub const DISCONNECT: u64 = M::SBROADCASTER_DISCONNECT_EVENT.data();
pub const RELIABLE_BATCH: u64 = h("SBroadcasterReliableBatchEvent");

pub const JOIN_REQUEST: u64 = M::SNS_LOBBY_JOIN_REQUEST_V4.data();
pub const JOIN_REJECTED: u64 = M::SNS_LOBBY_JOIN_REJECTED_INTERNAL.data();
pub const JOIN_SETUP: u64 = h("SNSLobbyJoinSetup");
pub const JOIN_ACCEPTED: u64 = h("SNSLobbyJoinAcceptedv2");
pub const JOIN_COMPLETE: u64 = h("SNSLobbyJoinComplete");
pub const OWNER_STATE: u64 = h("SNSLobbyOwnerState");
pub const HOST_STATE: u64 = h("SNSLobbyHostState");
pub const ENTRANT_STATE: u64 = M::SNS_LOBBY_ENTRANT_STATE_V2.data();
pub const ENTRANT_REMOVED: u64 = M::SNS_LOBBY_ENTRANT_REMOVED.data();
/// A lobby entrant's key/value data. Client -> server `[slot u64][key u64][data][u16]`;
/// server -> everyone (the sender too) `[sender slot u64][key u64][data]`
/// (netrec-native-srv-1201 and its client netrec-lobby-ours-1959; netrec-lobby-native-20261118).
pub const ENTRANT_DATA: u64 = h("SNSLobbyEntrantDataChanged");
/// The entrant-data header native's server gives each newcomer for its own slot (key + 8 key
/// hashes, identical in every capture). The client publishes its own entrant data only after it.
pub const ENTRANT_DATA_HEADER: [u8; 72] = [
    0xcc, 0x24, 0xf1, 0x5e, 0x6e, 0x1a, 0x0f, 0xb1, 0xa2, 0x08, 0x7d, 0x33, 0x48, 0x3e, 0xc3, 0xc8, 0x63, 0x89, 0xfe, 0x27,
    0x1a, 0x95, 0xe8, 0x22, 0xdf, 0x32, 0x32, 0x13, 0x1b, 0x17, 0x62, 0x2f, 0x8e, 0x5e, 0x6d, 0x4a, 0x8d, 0x8e, 0xc7, 0x2f,
    0x63, 0x94, 0xe6, 0x3d, 0x0e, 0x90, 0xe2, 0x34, 0x88, 0x31, 0x93, 0x80, 0x82, 0x93, 0x76, 0x5d, 0x5e, 0x98, 0xfd, 0xb4,
    0x85, 0x63, 0xce, 0x64, 0x7f, 0x0c, 0x5f, 0x0b, 0x63, 0x28, 0xe2, 0x6f,
];
/// Native's per-entrant slot entry: an earlier entrant's is its slot (u16), a newcomer's own empty.
pub const ENTRANT_DATA_SLOT_KEY: u64 = 0xa019_1e7a_56fe_32c3;
/// Entrant data native's server sets itself (peer -1 of netrec-native-srv-1201; the same bytes
/// in every client capture): `afk` with its value right after the join header, `spawned`
/// (empty) and `mm|status` once the player's first equipment update arrives.
pub const ENTRANT_KEY_AFK: u64 = 0xc8e8_d0b1_a891_e4e1;
pub const ENTRANT_AFK_VALUE: [u8; 16] = [0x7d, 0x06, 0x1f, 0x48, 0x0c, 0xf3, 0x1c, 0x19, 0x15, 0x28, 0xa1, 0x76, 0xce, 0x52, 0xe9, 0x70];
pub const ENTRANT_KEY_SPAWNED: u64 = 0x2fc4_9f9e_497a_528e;
pub const ENTRANT_KEY_MM_STATUS: u64 = 0x191c_f30c_481f_067d;
pub const ENTRANT_MM_STATUS_VALUE: [u8; 8] = [0xc4, 0xbe, 0xda, 0xed, 0x35, 0xd9, 0xf3, 0x2d];
pub const UPDATE_PINGS: u64 = h("SNSLobbyUpdatePings");
/// Client -> server on each cosmetic equip: `[slot u64][instance][slot type][item]`.
pub const ITEM_EQUIPPED: u64 = h("SR15NetItemEquippedNotification");
/// Client -> server after an equip (4 bytes); native sends nothing back.
pub const SAVE_LOADOUT_REQUEST: u64 = h("SR15NetSaveLoadoutRequest");
pub const REFRESH_PROFILE: u64 = M::SNS_REFRESH_PROFILE_FROM_SERVER.data();

pub const SERVER_INFO: u64 = M::SR15_NET_SERVER_INFO.data();
pub const SERVER_FRAME: u64 = M::SR15_NET_SERVER_FRAME_MSG.data();
pub const USER_PING_EST: u64 = M::SR15_NET_USER_PING_EST.data();
pub const CLIENT_PING_RESPONSE: u64 = M::SR15_NET_CLIENT_PING_RESPONSE.data();
pub const IDS_REQUEST: u64 = M::SR15_NET_IDS_REQUEST.data();
pub const IDS_RESPONSE: u64 = M::SR15_NET_IDS_RESPONSE.data();
pub const BALANCE_READY: u64 = M::SR15_NET_BALANCE_SETTINGS_CLIENT_READY.data();
pub const BALANCE_UPDATE: u64 = M::SR15_NET_BALANCE_SETTINGS_MASS_UPDATE.data();
pub const REPLICATED_READY: u64 = M::SR15_NET_REPLICATED_CLIENT_READY.data();
pub const REPLICATED_VARS: u64 = M::SR15_NET_REPLICATED_DELTA_UNRELIABLE_VARS.data();
pub const REPLICATED_ACK: u64 = M::SR15_NET_REPLICATED_DELTA_ACK.data();
pub const VOIP: u64 = M::SR15_NET_USER_VOIP_EVENT.data();
pub const PLAYER_STATE: u64 = M::SR15_NET_PLAYER_STATE_MSG.data();
pub const PHYSICS: u64 = M::SR15_NET_PHYSICS_SIM_STATE_EVENT.data();
pub const EQUIPMENT: u64 = M::SR15_NET_PLAYER_EQUIPMENT_UPDATE.data();
pub const LOADOUT_REQUEST: u64 = M::SR15_NET_CURRENT_LOADOUT_REQUEST.data();
pub const LOADOUT_RESPONSE: u64 = M::SR15_NET_CURRENT_LOADOUT_RESPONSE.data();
pub const LOADOUT_NUMBER: u64 = M::SR15_NET_LOADOUT_NUMBER_NOTIFICATION.data();
pub const STUN: u64 = M::SR15_STUN_PLAYER_MSG.data();
pub const EMOTE: u64 = M::SR15_NET_PLAY_EMOTE.data();
pub const ACTOR_CREATE: u64 = M::SR15_NET_POOLED_ACTOR_CREATE.data();
pub const ACTOR_DESTROY: u64 = M::SR15_NET_POOLED_ACTOR_DESTROY.data();
pub const FORCE_PUSH: u64 = M::SR15_FORCE_PUSH_PLAYER.data();
pub const NEW_UNLOCKS: u64 = M::SR15_NET_NEW_UNLOCKS.data();
pub const COMPONENT_EVENT: u64 = M::SR15_NET_COMPONENT_EVENT.data();
pub const TOUCH_XFER_RELIABLE: u64 = M::SR15_NET_TOUCH_OWNERSHIP_TRANSFER_RELIABLE.data();
pub const TOUCH_XFER_MAILBOX: u64 = M::SR15_NET_TOUCH_OWNERSHIP_TRANSFER_MAILBOX.data();
pub const TOUCH_GRANTED: u64 = M::SR15_NET_TOUCH_OWNERSHIP_GRANTED.data();
pub const TOUCH_DECLINED: u64 = M::SR15_NET_TOUCH_OWNERSHIP_DECLINED.data();
pub const POST_SCORE: u64 = M::SR15_NET_RELIABLE_POST_SCORE_UPDATE.data();
pub const STAT_UPDATE: u64 = h("SR15NetReliableStatUpdate");
pub const TEAM_STAT_UPDATE: u64 = h("SR15NetReliableTeamStatUpdate");
pub const TOP_AWARDS: u64 = h("SR15NetTopAwardsMsg");

// ── Match stats (layouts from a recorded native game) ─────────────────────
/// Team stat record hashes, in native's order: the arena screen-stat list
/// (`stats::STAT_NAMES`). Each record is
/// `[hash u64][mantissa i32][exponent i32][count u32]`, value = m * 2^e.
pub const TEAM_STATS: [u64; 12] = [
    0x41d2_d732_132d_130d, // Points
    0x2fd6_9c8c_5767_4399, // Assists
    0xe32d_c7c9_d899_41a3, // Saves
    0xe32d_c7c9_cd9a_4aa3, // Stuns
    0xea7c_6201_ea1d_29f5, // PossessionTime (2^-6 s)
    0x41d2_d429_1f22_0b0d, // Steals
    0x2afc_a8a0_4d9e_f31c, // ShotsOnGoal
    0x4dd2_9d01_fecc_667b, // Interceptions
    0xe32d_c7dd_d68e_48a3, // Goals
    0x41d2_c531_1520_0c0d, // Blocks
    0x2fd4_8e8b_5d7c_5299, // Catches
    0x41d2_d73c_0930_020d, // Passes
];
pub const TS_POSSESSION: usize = 4;
/// TopAwards: the MVP's award and stat (`score`); the others: `awards`.
pub const AWARD_MVP_ID: u64 = 0xc8e8_d0b1_a89d_f4fa;
pub const AWARD_MVP_STAT: u64 = 0xe32d_c7c9_da80_56b5;
/// `SR15NetTierRewardMsg` `[stat u64][tier u64][slot u64][xp u64]`, to the
/// player only (native: gold/silver/bronze in Goals, Assists, Saves).
pub const TIER_REWARD: u64 = h("SR15NetTierRewardMsg");
pub const TIER_GOLD: u64 = 0xc8c3_3e48_3d7c_12b4;
pub const TIER_SILVER: u64 = 0x41d2_d434_1635_020c;
pub const TIER_BRONZE: u64 = 0x41d2_c52f_152d_1d1b;

/// Raw (unencrypted) 16-byte ping datagrams: `[symbol u64][token u64]`.
pub const RAW_PING_REQUEST: u64 = 0x9972_79de_065a_03b0;
pub const RAW_PING_ACK: u64 = 0x4f7a_e556_e0b7_7891;
/// The Ipv6Bridge plugin's question, over IPv6 to a game port:
/// `[BRIDGE_QUERY][token u64]`. The answer names the IPv4 address players
/// are given for this host: `[BRIDGE_ANSWER][token u64][ipv4 4]`. With it the
/// plugin sends the game's packets for that IPv4 to us over IPv6 (a host
/// behind CGNAT has no reachable IPv4 port). Not game messages.
pub const BRIDGE_QUERY: u64 = 0x7136_6267_6469_7262; // "brdigb6q"
pub const BRIDGE_ANSWER: u64 = 0x6136_6267_6469_7262; // "brdigb6a"

// ── Levels and modes ───────────────────────────────────────────────────────
pub const LEVEL_SOCIAL: u64 = 0xd09a_fd15_b1c7_5c04; // mpl_lobby_b2
pub const LEVEL_ARENA: u64 = 0x576e_d3f8_428e_bc4b; // mpl_arena_a
pub const SOCIAL_MODES: [u64; 3] = [0x042d_9cf9_cfdd_cf76, 0x305d_6e37_c158_9c45, 0x1639_56e6_e431_e617];

// ── Replicated variables (CSymbol64 of the variable name) ──────────────────
pub const VAR_GAME_CLOCK: u64 = 0xb299_6ad4_84f1_d1e8;
pub const VAR_GAME_STATE: u64 = 0xb299_6ad3_99fd_c0ff;
pub const VAR_BLUE_SCORE: u64 = 0x1c97_052a_21e0_9f11;
pub const VAR_RED_SCORE: u64 = 0x2ed0_7b1c_8c27_548d;
pub const VAR_BLUE_ROUND_SCORE: u64 = 0x1907_9a53_4e20_8dc1;
pub const VAR_RED_ROUND_SCORE: u64 = 0xf7df_d2d5_5b87_1001;
pub const VAR_ROUND_COUNTER: u64 = 0x4eb3_e1b1_52a1_ba7a;
pub const VAR_OVERTIME: u64 = 0x5faf_3c37_3f5a_7771;
pub const VAR_TOTAL_ROUNDS: u64 = 0x9042_518a_de2d_74b2;
pub const VAR_DISC_OWNER: u64 = 0x1b10_bdc9_62b4_7fcc;
pub const VAR_COUNTDOWN: u64 = 0x3e76_6a9a_fe6c_97b0;
// Flag vars the native server flips through a match (low byte 0/1; the rest
// of the value comes from the baseline). Timings from a recorded native game.
/// Spawn-room tunnel doors: open when the pre-match count reaches 0.
pub const VAR_TUNNEL_DOORS: u64 = 0xe405_6c13_b804_cba2;
/// Set alongside the doors, one frame later; stays set.
pub const VAR_ROUND_BEGUN: u64 = 0x1ec7_83ef_243d_a0c4;
/// Launch-tube gates: open for play, closed on a goal.
pub const VAR_LAUNCH_GATES: u64 = 0x7265_2e2c_e806_d370;
/// Set during a goal celebration / match end; cleared ~0.5 s before pre_launch.
pub const VAR_HOLD: u64 = 0x098a_a551_580b_0386;
/// Set until play first begins; set again at match end.
pub const VAR_NOT_PLAYING: u64 = 0xcd17_8a7a_e176_4792;
/// Opens at match end.
pub const VAR_END_DOORS: u64 = 0x8619_87b5_4865_c378;
/// Where `restart_disc` respawns the disc: 2 centre (match start and end),
/// 1 orange side (after blue scores), 0 blue side (assumed, after orange).
pub const VAR_DISC_SPAWN: u64 = 0x7d0a_6658_1bb4_5536;
/// Set when the post-match celebration starts.
pub const VAR_CELEBRATION: u64 = 0xf5bb_086c_1435_b529;

// ── Arena game_state values ────────────────────────────────────────────────
pub const ST_PRE_MATCH: u64 = 0xd69c_c3ed_9f1e_31bf;
/// First state after the pre-match count hits 0 (before round_start_enter).
pub const ST_PRE_MATCH_DONE: u64 = 0xdad7_65f3_754f_6262;
pub const ST_ROUND_START: u64 = 0x7b54_c48f_ed3e_cd93;
pub const ST_MAIN_LOOP_ENTER: u64 = 0x3b52_9c35_6a11_8cc6;
pub const ST_PLAYING: u64 = 0xab08_d245_b707_b56b;
pub const ST_GOAL: [u64; 9] = [
    0x56ed_ca90_65f2_9296,
    0x75c7_a473_9d7a_1739,
    0xf916_2deb_2772_3fa6,
    0xc3d1_fdd4_0bb6_9c3f,
    0xe1c1_6321_def4_cc6c,
    0xfbc4_6082_c080_fbcf,
    0x51df_5264_2db6_7278,
    0x97d0_a383_527c_012f,
    0x3775_6d88_77ed_5768,
];
pub const ST_MATCH_END: [u64; 7] = [
    0xccea_96e4_8128_f05e, // round_over_enter
    0x67e1_e232_f03b_e11d, // round_over_update
    0x9567_032e_22cd_c734, // post_match_celebration_enter
    0xf83f_cdbb_25ba_6f07, // post_match_celebration_update
    0x344d_080e_a700_b7ea, // personal_stats_enter
    0x3eda_28aa_e966_fa2d, // personal_stats_update
    0xfc61_40cc_02f9_c63d, // waiting_to_return_to_lobby
];

// ── Component events ───────────────────────────────────────────────────────
pub const MOD_MATCH: u64 = 0x0831_7e89_cc3a_e611;
pub const MOD_ANNOUNCER: u64 = 0x949c_090e_2472_d440;
pub const MOD_DISC_FX: u64 = 0xb9e1_3691_0995_a482;
pub const FILTER_ALL: u16 = 5;
pub const FILTER_ALL_ALT: u16 = 13;

pub const EV_ENABLE_SCORING: u64 = 0x521f_717a_2cb8_0aa9;
/// The social lobby's scoring module (the quarter court): native sends
/// `EV_ENABLE_SCORING` on it 3 s after each goal, on net handle 0x2e in a
/// session whose IdsResponse is our `ids_response_045c` (captures/netrec-info-20261118).
pub const MOD_LOBBY_SCORING: u64 = 0x5578_07c7_3b06_750f;
pub const LOBBY_SCORING_HANDLE: u16 = 0x002e;
pub const EV_RESTART_CELEBRATION: u64 = 0x9ebb_9a61_909b_5440;
pub const EV_MATCH_NOT_READY: u64 = 0x5ca9_e3d2_7753_75f5;
pub const EV_RESTART_GOALS: u64 = 0x0f20_dd3d_f2d9_e5a1;
pub const EV_RESTART_DISC: u64 = 0xa4ef_0120_5f50_62a1;
pub const EV_LIGHT_WHITE: u64 = 0x4666_89ab_41e6_49db;
pub const EV_FX_WHITE: u64 = 0x440e_7479_9f15_2d2b;
pub const EV_DISABLE_LOCAL_DISC: u64 = 0x7cff_6e8b_60cd_61d7;
pub const EV_ROUND_START: u64 = 0x8fea_3716_c99b_8122;
pub const EV_NEW_ROUND_STARTED: u64 = 0x07fd_afd6_80ee_3a56;
pub const EV_GAME_SCORE: u64 = 0x92a7_734b_005d_e071;
pub const EV_ROUND_RESTART: u64 = 0x6f88_4020_dc2e_8d0c;
pub const EV_REALLY_RESTART: u64 = 0xcd87_4f73_cab7_5974;
pub const EV_MATCH_OVER: u64 = 0x0772_d3c5_391b_a59e;
pub const EV_TELEPORT_CELEBRATION: u64 = 0xe9df_6ad8_e934_f69c;
pub const EV_PERSONAL_STATS: u64 = 0x28c4_e143_bd2a_895f;
pub const EV_RETURN_TO_LOBBY: u64 = 0x6574_2a42_0357_6093;
/// Sent (filter 13, twice) with every goal, just before game_score.
/// First goal event: blue's goal (native, every blue goal in two matches).
pub const EV_GOAL_A: u64 = 0xe145_49f2_f4c4_0266;
/// Its orange counterpart (every orange goal): the celebration's team.
pub const EV_GOAL_A_ORANGE: u64 = 0xf7b1_de94_6009_b29e;
pub const EV_GOAL_B: u64 = 0xfa94_15c1_1a6a_3b6c;
/// Announcer goal calls (filter 13): native sent the first for a 2-point
/// goal and the second for a 3-point goal.
#[cfg(test)]
pub const EV_ANNOUNCER_GOAL_2: u64 = 0x66f4_61df_61ae_a120;
#[cfg(test)]
pub const EV_ANNOUNCER_GOAL_3: u64 = 0x6cfb_6cd1_5bd1_90a7;
/// Disc light/FX in orange, used for the relaunch after blue scores (the
/// disc goes to orange).
pub const EV_LIGHT_ORANGE: u64 = 0x54cd_84e5_4d95_66f2;
pub const EV_FX_ORANGE: u64 = 0x52ae_1390_2597_cf66;

// ── Private match (echo_arena_private) ─────────────────────────────────────
pub const MODE_ARENA_PRIVATE: u64 = 0x0999_0965_f4db_8c03;
/// `echo_combat_private`: teams ready up at their terminals.
pub const MODE_COMBAT_PRIVATE: u64 = 0x33bb_f684_2df9_7a3f;
/// Module of the private-match terminal panels (rules, start/pause, restart).
pub const MOD_RULES_PANEL: u64 = 0x6aea_99ee_b2ba_b6c4;
pub const UI_INC_SETTING: u64 = h("SR15NetUIIncSettingMsg");
pub const UI_DEC_SETTING: u64 = h("SR15NetUIDecSettingMsg");
pub const UI_CHANGED_SETTINGS: u64 = h("SR15NetUIUserChangedSettings");
pub const ASSIGN_TEAM_REQUEST: u64 = h("SR15NetUserAssignTeamRequest");
pub const ASSIGN_TEAM_RESPONSE: u64 = h("SR15NetUserAssignTeamResponse");
pub const ENTRANT_TEAM_CHANGED: u64 = h("SNSLobbyEntrantTeamChanged");
/// Terminal buttons (c2s component events on MOD_RULES_PANEL).
/// Team ready buttons (each team's terminal): orange's pressed by an
/// orange player in the capture, blue's by a blue player live.
pub const BTN_READY_ORANGE: u64 = 0x1d10_8d26_ada8_c1dd;
pub const BTN_READY_BLUE: u64 = 0x70f8_cfcd_41c4_de99;
/// `nevt_pause_game_orange` / `nevt_pause_game_blue` (toggle).
pub const BTN_PAUSE: u64 = 0x0137_90bf_f1ab_f487;
pub const BTN_PAUSE_BLUE: u64 = 0xe292_0479_4d3d_c819;
/// End-of-match terminal: `nevt_match_restart_request` (Rematch) and
/// `nevt_return_to_lobby_request` (Lobby).
pub const BTN_REMATCH: u64 = 0x8ef2_3f7c_6f2d_03e6;
pub const BTN_LOBBY: u64 = 0xc778_b098_5921_1cea;
/// Sent by the client by itself (twice) as post_match_celebration_update
/// begins; not a button. Name unknown.
pub const EV_CLIENT_AT_CELEBRATION: u64 = 0x184d_b492_6b1d_09de;
/// Rules set through the client's HTTP API (type name unknown):
/// `[u32 0][u32 1][level u64]` + JSON `{"<RULE NAME>":<value>}` + NUL.
pub const API_SET_RULE: u64 = 0xd69f_b582_0373_ee18;
/// Announcer lines are the symbol of the dialogue scene's name.
pub const EV_ANNOUNCER_LAUNCHERS_CLEARED: u64 = 0x4fe2_738a_eb34_b29c; // MP_ANNOUNCER_0050F_ROUND_START
pub const EV_ANNOUNCER_ROUND_OVER: u64 = 0xfc6c_2c91_b40d_c994; // MP_ANNOUNCER_0120F_ROUND_OVER
pub const EV_ANNOUNCER_BLUE_WINS: u64 = 0xcb64_87d9_f71b_9145; // MP_ANNOUNCER_0080A_BLUE_TEAM_WINS
/// `nevt_round_over`: a round of a multi-round private match ended.
pub const EV_ROUND_OVER: u64 = 0x2734_410e_d952_8b14;
/// `is_mercy_win` (bool): the round ended on the mercy rule.
pub const VAR_MERCY_WIN: u64 = 0x2060_26b9_2d9c_8244;
/// Restart request buttons: a blue player pressed the first in the
/// capture (it set `blue_team_restart_request`), an orange one the second.
pub const BTN_RESTART_BLUE: u64 = 0xf289_24a0_9729_c910;
pub const BTN_RESTART_ORANGE: u64 = 0x9f27_d43c_604d_c4e8;
/// Sent on MOD_MATCH for every rules press.
pub const EV_SETTING_CHANGED: u64 = 0x565a_200a_f9a1_1f49;
/// Sent to the rules panel (IdsResponse entry 19) for a page 1 row 2 press.
pub const EV_RULES_PANEL_1_2: u64 = 0x38fb_0207_b7d5_b358;
pub const EV_START_LAUNCH: u64 = 0xa25c_369c_8347_706f;
pub const EV_PAUSED: u64 = 0x068d_263e_bc3c_07e4;
pub const EV_RESTART_A: u64 = 0x449e_f6be_2f77_41e2;
pub const EV_RESTART_B: u64 = 0xc481_cb71_b0ea_0671;
pub const EV_RESTART_C: u64 = 0xc35b_4c6c_3ffd_3a2a;
// Names below are the game's own (their symbol hashes match).
/// `red_team_ready` / `blue_team_ready` (bool).
pub const VAR_ORANGE_READY: u64 = 0x77c6_f68a_7ded_4ff7;
pub const VAR_BLUE_READY: u64 = 0x1795_439c_46cf_fedd;
/// Set (raw 1) when the start launch begins, 4 s after Start.
pub const VAR_START_LAUNCHED: u64 = 0xf761_6479_3f93_932f;
/// `blue_team_restart_request` / `red_team_restart_request` (bool).
pub const VAR_BLUE_RESTART_REQUEST: u64 = 0xef31_e81d_7ccf_9310;
pub const VAR_ORANGE_RESTART_REQUEST: u64 = 0x0b64_c6bf_815b_9764;
/// Team symbol of who paused / who unpaused.
pub const VAR_PAUSED_BY: u64 = 0xe713_95ce_7a46_1593;
pub const VAR_UNPAUSED_BY: u64 = 0x80ed_f387_7b6a_e9c9;
/// Pause state symbol (`unpaused` / `paused`; a third unnamed value for one
/// frame while pausing).
pub const VAR_PAUSE_STATE: u64 = 0x02fe_0aa4_f263_7309;
/// f32: seconds paused, counting up.
pub const VAR_PAUSE_TIME: u64 = 0x70d8_6d49_e454_1bb0;
/// f32: 14.5 at pause, counts to 0 after unpause, then play resumes.
pub const VAR_RESUME_COUNT: u64 = 0x9946_59a6_5cfa_75d3;
pub const SYM_ORANGE: u64 = 0x41d2_c82f_1b2d_001b;
pub const SYM_BLUE: u64 = 0xc8c3_3e48_387f_0bb5;
pub const SYM_NONE: u64 = 0xc8c3_3e48_347c_10b5;
pub const SYM_PAUSED: u64 = 0x41d2_d73c_0f30_021a;
pub const SYM_UNPAUSED: u64 = 0x23e3_8416_3cec_8162;
pub const SYM_PAUSING: u64 = 0xb24c_4239_e89e_103e;
pub const ST_RESTART_A: u64 = 0x6c19_a962_2753_9fac;
pub const ST_RESTART_B: u64 = 0x3979_ba68_6488_ba64;

/// A replicated value relative to the baseline's: the high bits (type tag)
/// come from the baseline, the low part from here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Val {
    Raw(u64),
    Lo32(u32),
    Lo8(u8),
    F32(f32),
}

impl Val {
    pub fn resolve(self, old: u64) -> u64 {
        match self {
            Val::Raw(v) => v,
            Val::Lo32(v) => (old & 0xffff_ffff_0000_0000) | u64::from(v),
            Val::Lo8(v) => (old & !0xff) | u64::from(v),
            Val::F32(v) => with_f32(old, v),
        }
    }
}

/// Net component handles the client got from the IdsResponse we sent:
/// u16 entries from offset 176. Entry 20 is the match graph (public 0xa02d,
/// private 0x002f), entry 0 the disc FX, entry 19 the rules panel.
#[derive(Clone, Copy, Debug)]
pub struct Handles {
    pub match_graph: u16,
    pub disc_fx: u16,
    pub rules_panel: u16,
}

impl Handles {
    pub const PUBLIC_ARENA: Handles = Handles { match_graph: 0xa02d, disc_fx: 0xa01b, rules_panel: 0xa02e };

    pub fn from_ids(ids: &[u8]) -> Option<Self> {
        let e = |i: usize| ids.get(176 + 2 * i..178 + 2 * i).map(|b| u16::from_le_bytes([b[0], b[1]]));
        Some(Self { match_graph: e(20)?, disc_fx: e(0)?, rules_panel: e(19)? })
    }

    /// The 0x0fff sentinel only reaches generic listeners and skips the node
    /// graph, which is what runs the round start (opening the tunnels).
    pub fn for_module(&self, module: u64) -> u16 {
        match module {
            MOD_MATCH | MOD_ANNOUNCER => self.match_graph,
            MOD_DISC_FX => self.disc_fx,
            MOD_RULES_PANEL => self.rules_panel,
            _ => 0x0fff,
        }
    }
}

/// `SR15NetComponentEvent`: symbol, module, handle, filter, origin.
pub fn component_event(symbol: u64, module: u64, handle: u16, filter: u16) -> [u8; 24] {
    let mut b = [0u8; 24];
    b[0..8].copy_from_slice(&symbol.to_le_bytes());
    b[8..16].copy_from_slice(&module.to_le_bytes());
    b[16..18].copy_from_slice(&handle.to_le_bytes());
    b[18..20].copy_from_slice(&filter.to_le_bytes());
    b[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
    b
}

// ── Mailbox keys: base + (frame % 90) * stride + index ─────────────────────
pub const KEY_BUCKETS: u64 = 90;
pub fn key_server_frame(frame: u64) -> u64 {
    frame % KEY_BUCKETS
}
pub fn key_ping_est(frame: u64, slot: u64) -> u64 {
    0x05fa + (frame % KEY_BUCKETS) * 0x10 + (slot & 0xf)
}
pub fn key_replicated(stream: usize, frame: u64, slot: u64) -> u64 {
    let base = if stream == 0 { 0x1cd4 } else { 0x2814 };
    base + (frame % KEY_BUCKETS) * 0x10 + (slot & 0xf)
}
pub fn key_world_physics(frame: u64, index: u64) -> u64 {
    0x3354 + (frame % KEY_BUCKETS) * 0x11 + index
}
pub const KEY_UPDATE_PINGS: u64 = 0xe32d_c7ca_d081_43a3;
pub const VOICE_SLOT_STRIDE: u64 = 0x400;

// ── Captured payloads ──────────────────────────────────────────────────────
fn hex(s: &str) -> Vec<u8> {
    let s: Vec<u8> = s.bytes().filter(u8::is_ascii_hexdigit).collect();
    s.chunks_exact(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap_or("00"), 16).unwrap_or(0)).collect()
}

pub struct Captured {
    pub ids: Vec<(u64, Vec<u8>)>,
    pub balance: Vec<(u64, Vec<u8>)>,
    /// The same for games without EchoCombat (stock gear tables).
    pub balance_stock: Vec<(u64, Vec<u8>)>,
    pub baselines: Vec<(u64, Vec<Vec<u8>>)>,
    pub private_ids: Vec<u8>,
    pub private_baseline: Vec<Vec<u8>>,
    pub new_unlocks: Vec<u8>,
    pub refresh_profile: Vec<u8>,
    pub disc_physics: [u8; 64],
}

pub fn captured() -> &'static Captured {
    static C: std::sync::OnceLock<Captured> = std::sync::OnceLock::new();
    C.get_or_init(|| Captured {
        ids: vec![
            (LEVEL_ARENA, hex(include_str!("data/ids_response_arena_4bbc.hex"))),
            (LEVEL_SOCIAL, hex(include_str!("data/ids_response_045c.hex"))),
            (0x3f99_15d3_001d_c28e, hex(include_str!("data/ids_response_8ec2.hex"))),
            (0x6daa_00a6_d33d_44b7, hex(include_str!("data/ids_response_b744.hex"))),
            (0xcb99_77f7_fc2b_4526, hex(include_str!("data/ids_response_2645.hex"))),
            (super::combat::LEVEL_DYSON, hex(include_str!("data/combat_dyson_ids.hex"))),
            (super::combat::LEVEL_COMBUSTION, hex(include_str!("data/combat_combustion_ids.hex"))),
            (super::combat::LEVEL_FISSION, hex(include_str!("data/combat_fission_ids.hex"))),
            // Fission's `mpl_combat_fission_climax` sublevel asks for its own
            // (native Fission, 2026-11-16): without them the client waits on
            // it forever and the match never loads.
            (LEVEL_FISSION_CLIMAX, hex(include_str!("data/combat_fission_climax_ids.hex"))),
            (super::combat::LEVEL_GAUSS, hex(include_str!("data/combat_gauss_ids.hex"))),
        ],
        // Clients play with these values; the host's changes are merged in.
        balance: vec![
            (LEVEL_SOCIAL, super::balance::apply(hex(include_str!("data/balance_settings_mass_update.hex")))),
            (LEVEL_ARENA, super::balance::apply(hex(include_str!("data/balance_settings_arena_4bbc.hex")))),
            (super::combat::LEVEL_DYSON, super::balance::apply(hex(include_str!("data/combat_dyson_balance.hex")))),
            (super::combat::LEVEL_COMBUSTION, super::balance::apply(hex(include_str!("data/combat_combustion_balance.hex")))),
            (super::combat::LEVEL_FISSION, super::balance::apply(hex(include_str!("data/combat_fission_balance.hex")))),
            (super::combat::LEVEL_GAUSS, super::balance::apply(hex(include_str!("data/combat_gauss_balance.hex")))),
        ],
        balance_stock: vec![
            (LEVEL_SOCIAL, super::balance::apply_stock(hex(include_str!("data/balance_settings_mass_update.hex")))),
            (LEVEL_ARENA, super::balance::apply_stock(hex(include_str!("data/balance_settings_arena_4bbc.hex")))),
            (super::combat::LEVEL_DYSON, super::balance::apply_stock(hex(include_str!("data/combat_dyson_balance.hex")))),
            (super::combat::LEVEL_COMBUSTION, super::balance::apply_stock(hex(include_str!("data/combat_combustion_balance.hex")))),
            (super::combat::LEVEL_FISSION, super::balance::apply_stock(hex(include_str!("data/combat_fission_balance.hex")))),
            (super::combat::LEVEL_GAUSS, super::balance::apply_stock(hex(include_str!("data/combat_gauss_balance.hex")))),
        ],
        baselines: vec![
            (LEVEL_SOCIAL, vec![hex(include_str!("data/replicated_delta_unreliable_vars_045c.hex"))]),
            (super::combat::LEVEL_DYSON, vec![hex(include_str!("data/combat_dyson_rep_s0.hex"))]),
            (super::combat::LEVEL_COMBUSTION, vec![hex(include_str!("data/combat_combustion_rep_s0.hex"))]),
            (super::combat::LEVEL_FISSION, vec![hex(include_str!("data/combat_fission_rep_s0.hex"))]),
            (super::combat::LEVEL_GAUSS, vec![hex(include_str!("data/combat_gauss_rep_s0.hex"))]),
            (
                LEVEL_ARENA,
                vec![
                    hex(include_str!("data/replicated_delta_arena_4bbc_s0.hex")),
                    hex(include_str!("data/replicated_delta_arena_4bbc_s1.hex")),
                ],
            ),
        ],
        private_ids: hex(include_str!("data/ids_response_arena_private.hex")),
        private_baseline: vec![
            hex(include_str!("data/replicated_delta_arena_private_s0.hex")),
            hex(include_str!("data/replicated_delta_arena_private_s1.hex")),
        ],
        new_unlocks: hex(include_str!("data/new_unlocks.hex")),
        refresh_profile: hex(include_str!("data/refresh_profile_from_server.hex")),
        disc_physics: [
            0xff, 0xff, 0xff, 0xff, 0x4b, 0xbc, 0x8e, 0x42, 0x00, 0x00, 0x00, 0x00, 0x12, 0x00, 0x00, 0x00, 0x50, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x80, 0x84, 0x1e, 0x24, 0xf4,
            0x20, 0xa1, 0xf7, 0x7f, 0xff, 0xfd, 0x07, 0x60, 0x00, 0x06, 0x60, 0x90, 0x20, 0x41, 0x82, 0x84, 0x1e, 0x24, 0xf4,
            0x20, 0xa1, 0x07, 0x00, 0x00, 0x00, 0x00,
        ],
    })
}

/// The public arena timings and rules, from the `mp_arena_rules` clients get.
pub fn arena_public() -> super::balance::ArenaPublic {
    static A: std::sync::OnceLock<super::balance::ArenaPublic> = std::sync::OnceLock::new();
    *A.get_or_init(|| {
        captured()
            .balance
            .iter()
            .find(|(l, _)| *l == LEVEL_ARENA)
            .and_then(|(_, b)| super::balance::document(b, "mp_arena_rules"))
            .map(|r| super::balance::ArenaPublic::from_rules(&r))
            .unwrap_or_default()
    })
}

/// A private match's starting rules, from the `mp_arena_rules` clients get.
pub fn arena_private_rules() -> super::rules::Rules {
    static R: std::sync::OnceLock<super::rules::Rules> = std::sync::OnceLock::new();
    R.get_or_init(|| {
        let server = super::balance::server_arena();
        let none = serde_json::Value::Null;
        captured()
            .balance
            .iter()
            .find(|(l, _)| *l == LEVEL_ARENA)
            .and_then(|(_, b)| super::balance::document(b, "mp_arena_rules"))
            .map_or_else(|| super::rules::Rules::from_private(&none, &server), |r| super::rules::Rules::from_private(&r, &server))
    })
    .clone()
}

fn private_arena(level: u64, mode: u64) -> bool {
    level == LEVEL_ARENA && mode == MODE_ARENA_PRIVATE
}

/// Replicated baseline for a session (private arena has its own).
pub fn baseline(level: u64, mode: u64) -> Option<&'static [Vec<u8>]> {
    if private_arena(level, mode) {
        return Some(captured().private_baseline.as_slice());
    }
    captured().baselines.iter().find(|(l, _)| *l == level).map(|(_, b)| b.as_slice())
}

/// IdsResponse body for a level; the session's own level gets its mode's.
/// `mpl_combat_fission_climax`, a Fission sublevel with net ids of its own.
pub const LEVEL_FISSION_CLIMAX: u64 = 0xf919_f210_bbda_872c;

pub fn ids_response(level: u64, session_level: u64, mode: u64) -> Option<&'static [u8]> {
    if level == session_level && private_arena(level, mode) {
        return Some(captured().private_ids.as_slice());
    }
    captured().ids.iter().find(|(l, _)| *l == level).map(|(_, b)| b.as_slice())
}

/// Look up a variable's raw value in a replicated-vars payload.
/// The lobby's `VAR_HOLD` as native sends it: the flag in the low word, `6aea99ee` in the high one (every lobby
/// capture: native-lobby/native-det/native-20261116/native-match, baseline and each per-player change). The high
/// word is the var's version (the lobby's other vars carry one near the message sequence): a client keeps its own
/// value against a lower one, so a hold sent at version 0 (our captured baseline's) never overrode a client that
/// had lifted it itself, and players stunned and blocked anywhere.
pub const LOBBY_HOLD_VERSION: u64 = 0x6aea_99ee_0000_0000;

/// `payload` with the var `hash` set to `value` (unchanged if the payload has no such var).
pub fn with_var(payload: &[u8], hash: u64, value: u64) -> Vec<u8> {
    let mut p = payload.to_vec();
    if p.len() < 30 {
        return p;
    }
    let n = u16::from_le_bytes([p[28], p[29]]) as usize;
    for i in 0..n {
        let o = 30 + i * 16;
        if o + 16 > p.len() {
            break;
        }
        if u64::from_le_bytes(p[o..o + 8].try_into().expect("8")) == hash {
            p[o + 8..o + 16].copy_from_slice(&value.to_le_bytes());
        }
    }
    p
}

/// `with_var`, adding the var when the payload lacks it.
pub fn with_var_added(payload: &[u8], hash: u64, value: u64) -> Vec<u8> {
    if payload.len() < 30 || var_raw(payload, hash).is_some() {
        return with_var(payload, hash, value);
    }
    let mut p = payload.to_vec();
    let n = u16::from_le_bytes([p[28], p[29]]).wrapping_add(1);
    p[28..30].copy_from_slice(&n.to_le_bytes());
    p.extend_from_slice(&hash.to_le_bytes());
    p.extend_from_slice(&value.to_le_bytes());
    p
}

/// The cease fire var of entrant `(gen, slot)`: every entrant has its own, and
/// a client reads only its own entrant's (native sends all of them to everyone,
/// in the snapshot and as a delta when someone joins or moves). `VAR_HOLD` is
/// entrant (0, 0)'s. The name differs from `VAR_HOLD`'s only in the generation
/// and slot digits (`<gen>?<slot>` then 15 more characters), and the hash is
/// affine in the bytes for names of one length, so H(name) = VAR_HOLD ^
/// fold0(name ^ name00). Pinned: (0, 1) = 0x79ac0fb75468917e (native-2p-c1,
/// player 2's join), (2, 4) = 0x9f65696aa4ce186c (netrec-lobby-native-20261118,
/// mode 0x042d; once thought a per-mode var).
///
/// Slots 10-19 have a longer name, still unknown, but in it the generation and
/// the slot's ones digit sit at the same distance from the end. So the same
/// formula works from entrant (0, 10)'s var, `VAR_HOLD_SLOT10`. Native server
/// recording netrec-info-20261118 (part_003.zip) pins (0, 10), (0, 11), (1, 10)
/// and (1, 11): each var first appears in a delta as that entrant joins.
/// Two-digit generations are unknown: `None`.
pub fn entrant_hold_var(generation: u16, slot: usize) -> Option<u64> {
    let base = match slot {
        0..=9 => VAR_HOLD,
        10..=19 => VAR_HOLD_SLOT10,
        _ => return None,
    };
    if generation > 9 {
        return None;
    }
    let mut d = [0u8; 18];
    d[0] = generation as u8;
    d[2] = (slot % 10) as u8;
    let diff = std::str::from_utf8(&d).ok()?;
    Some(base ^ rad_protocol::symbol::Symbol::calculate(diff, 0) as u64)
}

/// Entrant (0, 10)'s cease fire var (see `entrant_hold_var`).
pub const VAR_HOLD_SLOT10: u64 = 0x1ed1_36b5_f1c0_fc6a;

pub fn var_raw(payload: &[u8], hash: u64) -> Option<u64> {
    if payload.len() < 30 {
        return None;
    }
    payload[30..].chunks_exact(16).find_map(|r| {
        (u64::from_le_bytes(r[..8].try_into().ok()?) == hash).then(|| u64::from_le_bytes(r[8..].try_into().unwrap_or([0; 8])))
    })
}

pub fn with_f32(value: u64, v: f32) -> u64 {
    (value & 0xffff_ffff_0000_0000) | u64::from(v.to_bits())
}

// ── Disc transform inside SR15NetPhysicsSimStateEvent (64 bytes) ───────────
pub const DISC_OBJECT_ID: u32 = 0x428e_bc4b;
const DISC_BIT: usize = 267;
const DISC_WIDTH: usize = 19;
const DISC_BIAS: i32 = 250_000;

fn read_field(p: &[u8], bit: usize) -> i32 {
    let mut v = 0u32;
    for k in 0..DISC_WIDTH {
        let i = bit + k;
        v |= u32::from((p[i >> 3] >> (i & 7)) & 1) << k;
    }
    v as i32 - DISC_BIAS
}

fn write_field(p: &mut [u8; 64], bit: usize, value: i32) {
    let raw = (value + DISC_BIAS) as u32;
    for k in 0..DISC_WIDTH {
        let i = bit + k;
        let m = 1u8 << (i & 7);
        if (raw >> k) & 1 == 1 {
            p[i >> 3] |= m;
        } else {
            p[i >> 3] &= !m;
        }
    }
}

/// `(owner slot, x, y, depth)` when the payload is the arena disc's state.
pub fn disc_transform(p: &[u8]) -> Option<(u16, i32, i32, i32)> {
    if p.len() != 64 || u32::from_le_bytes([p[4], p[5], p[6], p[7]]) != DISC_OBJECT_ID {
        return None;
    }
    Some((
        u16::from_le_bytes([p[0], p[1]]),
        read_field(p, DISC_BIT),
        read_field(p, DISC_BIT + DISC_WIDTH),
        read_field(p, DISC_BIT + 2 * DISC_WIDTH),
    ))
}

pub fn write_disc_transform(p: &mut [u8; 64], x: i32, y: i32, depth: i32) {
    write_field(p, DISC_BIT, x);
    write_field(p, DISC_BIT + DISC_WIDTH, y);
    write_field(p, DISC_BIT + 2 * DISC_WIDTH, depth);
}

/// A `SR15NetCurrentLoadoutResponse` (452 bytes) as the instance it carries:
/// its name (`unified`) at 0x44, the item count at 0x74 and (slot, item)
/// symbol pairs from 0x84.
pub fn parse_loadout(d: &[u8]) -> Option<nevr_proto::rtapi::v1::LoadoutInstance> {
    let q = |o: usize| d.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().expect("8")));
    let name = q(0x44)?;
    let count = d.get(0x74..0x78).map(|b| u32::from_le_bytes(b.try_into().expect("4")))? as usize;
    let fits = d.len().saturating_sub(0x84) / 16;
    if name == 0 || count == 0 || count > fits {
        return None;
    }
    let items = (0..count)
        .filter_map(|i| {
            let o = 0x84 + i * 16;
            let (slot_type, equipped_item) = (q(o)?, q(o + 8)?);
            (slot_type != 0 && equipped_item != 0).then_some(nevr_proto::rtapi::v1::LoadoutItem { slot_type, equipped_item })
        })
        .collect();
    Some(nevr_proto::rtapi::v1::LoadoutInstance { instance_name: name, items })
}

/// Put `item` in `slot_type` of a `SR15NetCurrentLoadoutResponse`, if that
/// slot is in it. Returns whether the loadout changed.
pub fn equip_in_loadout(d: &mut [u8], slot_type: u64, item: u64) -> bool {
    let count = d.get(0x74..0x78).map_or(0, |b| u32::from_le_bytes(b.try_into().expect("4")) as usize);
    for i in 0..count.min(d.len().saturating_sub(0x84) / 16) {
        let o = 0x84 + i * 16;
        if u64::from_le_bytes(d[o..o + 8].try_into().expect("8")) == slot_type {
            let cur = &mut d[o + 8..o + 16];
            let new = item.to_le_bytes();
            let changed = *cur != new;
            cur.copy_from_slice(&new);
            return changed;
        }
    }
    false
}

#[cfg(test)]
mod loadout_tests {
    use super::*;

    #[test]
    fn equip_symbols_match_the_capture() {
        // Symbols seen in a native lobby recording.
        assert_eq!(ITEM_EQUIPPED, 0xed0f_ba84_9bad_c7bf);
        assert_eq!(SAVE_LOADOUT_REQUEST, 0xa374_9626_d6d2_0ef7);
    }

    #[test]
    fn equip_replaces_the_slot_item() {
        let mut d = vec![0u8; 452];
        d[0x44..0x4c].copy_from_slice(&0x2fc2_8196_587d_528e_u64.to_le_bytes());
        d[0x74..0x78].copy_from_slice(&2u32.to_le_bytes());
        d[0x84..0x8c].copy_from_slice(&11u64.to_le_bytes());
        d[0x8c..0x94].copy_from_slice(&12u64.to_le_bytes());
        d[0x94..0x9c].copy_from_slice(&21u64.to_le_bytes());
        d[0x9c..0xa4].copy_from_slice(&22u64.to_le_bytes());
        assert!(equip_in_loadout(&mut d, 21, 99));
        assert!(!equip_in_loadout(&mut d, 21, 99));
        let inst = parse_loadout(&d).expect("parses");
        assert_eq!(inst.items[1].equipped_item, 99);
        assert_eq!(inst.items[0].equipped_item, 12);
    }
}

#[cfg(test)]
mod lobby_hold_tests {
    use super::*;

    // Native's lobby hold (every lobby capture): 0x6aea99ee00000001 on, 0x6aea99ee00000000 lifted.
    #[test]
    fn lobby_hold_carries_natives_version() {
        assert_eq!(entrant_hold_var(0, 0), Some(VAR_HOLD));
        assert_eq!(entrant_hold_var(0, 1), Some(0x79ac_0fb7_5468_917e));
        assert_eq!(entrant_hold_var(0, 2), Some(0xddbc_5ca1_9ab6_2b1c));
        assert_eq!(entrant_hold_var(0, 9), Some(0xbac1_65a3_3f1e_4608));
        assert_eq!(entrant_hold_var(2, 4), Some(0x9f65_696a_a4ce_186c));
        assert_eq!(entrant_hold_var(0, 10), Some(0x1ed1_36b5_f1c0_fc6a));
        assert_eq!(entrant_hold_var(0, 11), Some(0x6ef7_9c53_fda3_6e92));
        assert_eq!(entrant_hold_var(1, 10), Some(0x8137_9f72_0986_0106));
        assert_eq!(entrant_hold_var(1, 11), Some(0xf111_3594_05e5_93fe));
        assert_eq!(entrant_hold_var(0, 20), None);
        assert_eq!(entrant_hold_var(10, 0), None);
        assert_eq!(LOBBY_HOLD_VERSION | 1, 0x6aea_99ee_0000_0001);
        let base = &baseline(LEVEL_SOCIAL, 0).expect("lobby baseline")[0];
        let fixed = with_var(base, VAR_HOLD, LOBBY_HOLD_VERSION | 1);
        assert_eq!(var_raw(&fixed, VAR_HOLD), Some(0x6aea_99ee_0000_0001));
        assert_eq!(fixed.len(), base.len());
    }
}
