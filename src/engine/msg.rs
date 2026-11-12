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
pub const UPDATE_PINGS: u64 = h("SNSLobbyUpdatePings");
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
/// Team stat record hashes, in native's order. Each record is
/// `[hash u64][mantissa i32][exponent i32][count u32]`, value = m * 2^e.
pub const TEAM_STATS: [u64; 12] = [
    0x41d2_d732_132d_130d, // Points
    0x2fd6_9c8c_5767_4399,
    0xe32d_c7c9_d899_41a3,
    0xe32d_c7c9_cd9a_4aa3,
    0xea7c_6201_ea1d_29f5, // PossessionTime (2^-6 s)
    0x41d2_d429_1f22_0b0d,
    0x2afc_a8a0_4d9e_f31c, // ShotsOnGoal
    0x4dd2_9d01_fecc_667b,
    0xe32d_c7dd_d68e_48a3, // Goals
    0x41d2_c531_1520_0c0d,
    0x2fd4_8e8b_5d7c_5299,
    0x41d2_d73c_0930_020d,
];
pub const TS_POINTS: usize = 0;
pub const TS_POSSESSION: usize = 4;
pub const TS_SHOTS: usize = 6;
pub const TS_GOALS: usize = 8;
/// TopAwards record constants (one MVP record in the native game).
pub const AWARD_MVP_ID: u64 = 0xc8e8_d0b1_a89d_f4fa;
pub const AWARD_MVP_STAT: u64 = 0xe32d_c7c9_da80_56b5;

/// Raw (unencrypted) 16-byte ping datagrams: `[symbol u64][token u64]`.
pub const RAW_PING_REQUEST: u64 = 0x9972_79de_065a_03b0;
pub const RAW_PING_ACK: u64 = 0x4f7a_e556_e0b7_7891;

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
pub const EV_RESTART_CELEBRATION: u64 = 0x9ebb_9a61_909b_5440;
pub const EV_MATCH_NOT_READY: u64 = 0x5ca9_e3d2_7753_75f5;
pub const EV_RESTART_GOALS: u64 = 0x0f20_dd3d_f2d9_e5a1;
pub const EV_RESTART_DISC: u64 = 0xa4ef_0120_5f50_62a1;
pub const EV_LIGHT_WHITE: u64 = 0x4666_89ab_41e6_49db;
pub const EV_FX_WHITE: u64 = 0x440e_7479_9f15_2d2b;
pub const EV_DISABLE_LOCAL_DISC: u64 = 0x7cff_6e8b_60cd_61d7;
pub const EV_ROUND_START: u64 = 0x8fea_3716_c99b_8122;
pub const EV_NEW_ROUND_STARTED: u64 = 0x07fd_afd6_80ee_3a56;
pub const EV_ANNOUNCER_ROUND_START: u64 = 0xba47_472b_3f67_118e;
pub const EV_GAME_SCORE: u64 = 0x92a7_734b_005d_e071;
pub const EV_ROUND_RESTART: u64 = 0x6f88_4020_dc2e_8d0c;
pub const EV_REALLY_RESTART: u64 = 0xcd87_4f73_cab7_5974;
pub const EV_MATCH_OVER: u64 = 0x0772_d3c5_391b_a59e;
pub const EV_TELEPORT_CELEBRATION: u64 = 0xe9df_6ad8_e934_f69c;
pub const EV_PERSONAL_STATS: u64 = 0x28c4_e143_bd2a_895f;
pub const EV_RETURN_TO_LOBBY: u64 = 0x6574_2a42_0357_6093;
/// Sent (filter 13, twice) with every goal, just before game_score.
pub const EV_GOAL_A: u64 = 0xe145_49f2_f4c4_0266;
pub const EV_GOAL_B: u64 = 0xfa94_15c1_1a6a_3b6c;
/// Announcer goal calls (filter 13): native sent the first for a 2-point
/// goal and the second for a 3-point goal.
pub const EV_ANNOUNCER_GOAL_2: u64 = 0x66f4_61df_61ae_a120;
pub const EV_ANNOUNCER_GOAL_3: u64 = 0x6cfb_6cd1_5bd1_90a7;
/// Disc light/FX in orange, used for the relaunch after blue scores (the
/// disc goes to orange).
pub const EV_LIGHT_ORANGE: u64 = 0x54cd_84e5_4d95_66f2;
pub const EV_FX_ORANGE: u64 = 0x52ae_1390_2597_cf66;

/// `SR15NetComponentEvent`: symbol, module, handle, filter, origin.
/// Net component handle an event is routed to. The arena IdsResponse we send
/// (native's, byte for byte) hands the client these handles: generation 10,
/// match graph index 45, disc-FX component index 27, the same ones native
/// addresses. The 0x0fff sentinel only reaches generic listeners and skips the
/// node graph, which is what runs the round start (opening the tunnels).
fn component_handle(module: u64) -> u16 {
    match module {
        MOD_MATCH | MOD_ANNOUNCER => 0xa02d,
        MOD_DISC_FX => 0xa01b,
        _ => 0x0fff,
    }
}

pub fn component_event(symbol: u64, module: u64, filter: u16) -> [u8; 24] {
    let mut b = [0u8; 24];
    b[0..8].copy_from_slice(&symbol.to_le_bytes());
    b[8..16].copy_from_slice(&module.to_le_bytes());
    b[16..18].copy_from_slice(&component_handle(module).to_le_bytes());
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
    pub baselines: Vec<(u64, Vec<Vec<u8>>)>,
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
        ],
        balance: vec![
            (LEVEL_SOCIAL, hex(include_str!("data/balance_settings_mass_update.hex"))),
            (LEVEL_ARENA, hex(include_str!("data/balance_settings_arena_4bbc.hex"))),
        ],
        baselines: vec![
            (LEVEL_SOCIAL, vec![hex(include_str!("data/replicated_delta_unreliable_vars_045c.hex"))]),
            (
                LEVEL_ARENA,
                vec![
                    hex(include_str!("data/replicated_delta_arena_4bbc_s0.hex")),
                    hex(include_str!("data/replicated_delta_arena_4bbc_s1.hex")),
                ],
            ),
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

pub fn baseline(level: u64) -> Option<&'static [Vec<u8>]> {
    captured().baselines.iter().find(|(l, _)| *l == level).map(|(_, b)| b.as_slice())
}

/// Look up a variable's raw value in a replicated-vars payload.
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
