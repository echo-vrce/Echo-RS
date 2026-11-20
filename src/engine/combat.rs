//! Combat matches: the four maps' round flow and the payload.
//!
//! Native's combat server runs the maps' scripts; this server runs none, so
//! the flow is rebuilt from a native public Gauss match
//! (`captures/client-combat-20261116-b`, NOTES.md there) and the maps' own
//! settings (`mpl_combat_<map>_map_settings`, `mp_combat_rules`):
//!
//! - Game state: the same `CR15NetGame` states as the arena (`ST_*`). A
//!   native public Gauss match from its start, recorded on both sides
//!   (`captures/netrec-gauss-20261116*`): two load events, `pre_match` for
//!   `pre_match_wait_time` (30 s), then `pre_match_done` and the team room
//!   (two states, its event, `team_room_wait_time` 15 s), then `round_start`
//!   (one state before it, its event, `pre_launch` 45 s: players are held in
//!   the war room and the orb shows the count), then `main_loop_enter` /
//!   `playing` with the clock at `round_timers.start`. The end runs
//!   `round_over` (15 s), `post_match_celebration` (10 s), `personal_stats`
//!   (15 s) and back to the lobby, with native's match-script events (module
//!   `6fa8cc9cd721a563`, the level's last net component, the same actor
//!   `9550158657ffada8` on every map); the first end events say who won
//!   (orange at the end of the track, blue when time runs out).
//! - Payload (`SR15NetPayload*`, handle of the payload component): the server
//!   counts who claims it and sends `UpdateMiscState [handle u16][0 u16]
//!   [attackers u8][0][direction i8][0]` on a change, and
//!   `TrackpointReached` + `PosStateUpdate` `[handle u16][trackpoint u16]`
//!   each time it gets further than ever; clients move it themselves between
//!   those. Orange attacks. Speeds (recorded): `payload_speed_max_mps` x the
//!   attacker multiplier x the section's multiplier; nobody on it for 5 s and
//!   it rolls back at 0.25 m/s, never past the last checkpoint. A checkpoint
//!   adds `checkpoint_N` seconds and holds the clock and the payload for the
//!   celebration (`cN_celebration_timer`). Attackers on it at 0:00 keep it
//!   alive (`overtime`, refreshed while they claim). The end of the track
//!   wins for orange; time up wins for blue.
//! - Tracks (Gauss, Fission): `data/combat_<map>_track.txt`, from each
//!   level's own track points (`tools/gen_combat_maps.py`): trackpoint, metres,
//!   position, flags (0x60 checkpoint, 0x10 end). The points are the level's
//!   first net components, so the payload is the next one (handle `0x1b + n`:
//!   Gauss 0xd0 as recorded, Fission 0x60).
//! - Capture point (Dyson, Combustion): native's server runs
//!   `CR15NetCaptureVolumeCS::Update` and broadcasts `SR15NetCaptureVolumeUpdate`
//!   `[space u64][pool index u16][generation u16][kind u32 = 1][possession u16]
//!   [flags u16: 1 contested, 2 locked][converting team u16, 0xffff none][0 u16]`
//!   then per team `[claimer bits u16][claimers u16][progress f32]`; clients
//!   set the volume from it and play its events. Possession 0 blue, 1 orange, 2
//!   neutral (`ResetVolume`). A team alone on the point converts it: progress
//!   counts up while it is alone there and down otherwise, and at
//!   `time_to_capture` the point is theirs. The point is locked for
//!   `capture_point_lock_time` at round start. Shapes: `data/combat_<map>_
//!   capture.txt` (`tools/gen_combat_maps.py`). Scoring is the maps' scripts
//!   natively (not in the binary): here the holder earns 1 point a second while
//!   on it, `unclaimed_accrual_multiplier` of that while off it, nothing while
//!   contested, and `points_to_win` wins; the points go in the arena's team
//!   score vars.
//! - Script vars (native server recordings of all four maps, 2026-11-16;
//!   `tools/gen_combat_vars.py`): the HUD, the capture point's look and
//!   sounds, the doors and the war room are driven by replicated vars that
//!   native's server-side level scripts set. Each level has its own keys
//!   (per actor), so each map's baseline is native's own first full var
//!   message. The server sets: the match sub-phase symbol, the team room
//!   flag, the round over flag, the post-match flag, the rounds won and last
//!   round's winner, and on capture maps the point's owner and who is on it
//!   (symbols), each team's head count, conversion progress, points and
//!   whether it is scoring, and the lock count. A client's server-only
//!   (filter 5) events set level vars exactly as native's scripts did
//!   (`data/combat_<map>_reflex.txt`, learned from the recordings), and
//!   barrier vars follow the barriers (`data/combat_<map>_barrier_vars.txt`).

use rad_protocol::symbol::MatSym;
use serde_json::Value;

use super::msg::{
    ST_MAIN_LOOP_ENTER, ST_MATCH_END, ST_PLAYING, ST_PRE_MATCH, ST_PRE_MATCH_DONE, ST_ROUND_START, Val, VAR_BLUE_READY,
    VAR_BLUE_SCORE, VAR_CELEBRATION, VAR_COUNTDOWN, VAR_GAME_CLOCK, VAR_GAME_STATE, VAR_ORANGE_READY, VAR_RED_SCORE,
};

// ── Private matches ─────────────────────────────────────────────────────────
// `echo_combat_private`: native waits in `pre_match_wait_for_ready` until
// every team with players has pressed its terminal's ready (an empty team does
// not hold the other up), then plays as a public match does. A payload map
// plays two halves: orange attacks, the payload's furthest point is kept as a
// marker, the teams swap sides (blue becomes orange and attacks) and the new
// attackers win by getting the payload as far; if their time runs out short
// of it, the first attackers win. (No native private combat recording: the
// halves follow the game's own rules as players describe them; the marker
// vars are the level's names from the symbol dictionary, unverified.)
const ST_WAIT_FOR_READY: u64 = 0x1c79_024a_0c34_e549;
const VAR_FURTHEST_ROUND: [u64; 2] = [0x00e5_031f_be4a_d639, 0x00e5_031f_be4a_d63a];
const VAR_TEAM_DISTANCE: [u64; 2] = [0xe19b_4f91_8bf8_7b83, 0xdefa_0c74_7357_0171];
const VAR_TEAM_A_MARKER_POS: u64 = 0x07e7_3ff9_4812_9a0a;
const VAR_TEAM_A_MARKER_SHOWN: u64 = 0x107e_ce0c_1520_cc7d;
const VAR_PROGRESS_MARKER_POS: u64 = 0xe873_948c_32c9_94a0;
const VAR_PROGRESS_MARKER_SHOWN: u64 = 0x89fb_46c6_d1b1_293b;
const VAR_DEFENDING_TEAM: u64 = 0xf881_55a6_6a6b_05f8;
const VAR_TOTAL_ROUNDS: u64 = 0x9042_518a_de2d_74b2;
const VAR_ROUNDS_PLAYED: u64 = 0x50ba_16e5_7e95_8204;

// The private terminal and match restarts, from the stock match script
// (`mp_combat_server_state`, 6fa8cc9cd721a563, lifted 2026-11-19):
// - ready: each press flips the team's `*_team_ready` flag (only while
//   waiting for ready). Once every team with players is ready (an empty team
//   does not hold the others up): `nevt_round_restart`, 3 s later
//   `nevt_match_ready`, 0.2 s later `round_start`, then the team room.
// - restart request: sets the team's `*_team_restart_request` flag (a second
//   press does nothing). Once every team with players asked, in the team
//   room, pre-launch or play: the match restarts (as a rematch does).
// - rematch (`nevt_match_restart_request`, in the post-match celebration or
//   personal stats): `nevt_game_restart_confirmed`, `nevt_round_restart`,
//   `nevt_game_restart`, the stats reset and sent, `nevt_mpl_refresh_
//   scoreboard2`, and 0.2 s later back to waiting for ready.
// - A private match does not go back to the lobby by itself after the
//   personal stats (a public one does); the lobby button
//   (`nevt_return_to_lobby_request`) sends everyone back.
const EV_MATCH_READY: u64 = 0xa25c_369c_8347_706f;
const EV_GAME_RESTART_CONFIRMED: u64 = 0x449e_f6be_2f77_41e2;
const EV_GAME_RESTART: u64 = 0x8b7d_7cb9_c1b3_9fe4;
const EV_REFRESH_SCOREBOARD: u64 = 0xc35b_4c6c_3ffd_3a2a;
const VAR_RESTART_REQUEST: [u64; 2] = [0xef31_e81d_7ccf_9310, 0x0b64_c6bf_815b_9764];
/// After every team is ready: `nevt_match_ready`, then `round_start`.
const READY_TO_MATCH_READY: f32 = 3.0;
const READY_TO_ROUND_START: f32 = 3.2;
/// A rematch or restart: back to waiting for ready this long after.
const RESTART_TO_WAITING: f32 = 0.2;

// ── Payload HUD (native Gauss and Fission server recordings, 2026-11-16) ────
/// Who holds the payload: `Unclaimed`, `attackerPossessed`, `contested` (a
/// defender alone stays `Unclaimed`). Each level keeps a copy of its own.
const VAR_CLAIM_STATE: u64 = 0x8ba9_857e_5c38_d27d;
const VAR_CLAIM_STATE_FISSION: u64 = 0x24ee_4033_c210_54ed;
const VAR_CLAIM_STATE_GAUSS: u64 = 0xa8fc_5db5_1329_d7ad;
const CLAIM_NONE: u64 = 0xf558_bbb6_0dc3_bb9a;
const CLAIM_ATTACKERS: u64 = 0x8b67_f8fc_b701_88a4;
const CLAIM_CONTESTED: u64 = 0xba11_136c_4dd2_e646;
/// The payload's motion (`still`, `forward`, `backward`), its direction
/// (`backward` only while it rolls back) and moving (1, 0, -1).
const VAR_PAYLOAD_MOTION: u64 = 0x70ab_e536_3113_cdf7;
const VAR_PAYLOAD_DIRECTION: u64 = 0xc78a_fe76_b073_4e3a;
const VAR_PAYLOAD_MOVING: u64 = 0xd52d_ae83_702e_6fb3;
const MOTION_STILL: u64 = 0xe32d_c7c9_cd86_48bc;
const MOTION_FORWARD: u64 = 0x2fd1_808d_4975_458e;
const MOTION_BACKWARD: u64 = 0x34ec_971c_3efe_9662;
/// Attackers on it (0/1, and the same as a flag byte), defenders on it (count).
const VAR_ATTACKER_CLAIM: u64 = 0xa157_0bd3_e2ea_aac7;
const VAR_ATTACKER_FLAG: u64 = 0x5faf_3b26_2449_647a;
const VAR_DEFENDER_CLAIM: u64 = 0x21e8_feec_3e84_4a3d;
/// `overtime_progress`: 1.0 from the attackers' first claim on (both maps).
const VAR_OVERTIME_PROGRESS: u64 = 0x9d89_a48a_426b_a48f;
/// Gauss's door at its first checkpoint: whole seconds left, 20 down to 0.
const VAR_GAUSS_DOOR: u64 = 0xe68b_8118_ed84_d0de;
const GAUSS_DOOR_SECS: f32 = 20.0;

/// Someone on a team: team (0 blue, 1 orange), alive, body position, slot.
pub type Player = (u8, bool, Option<[f32; 3]>, u8);

const fn h(name: &str) -> u64 {
    MatSym::hash(name).data
}

pub const PAYLOAD_MISC: u64 = h("SR15NetPayloadUpdateMiscStateReliable");
pub const PAYLOAD_TRACKPOINT: u64 = h("SR15NetPayloadTrackpointReachedReliable");
pub const PAYLOAD_POS: u64 = h("SR15NetPayloadPosStateUpdateReliable");
/// `[handle u16][on u16][speed f32]` (native Fission: on at its first
/// checkpoint, off when it reaches the track's 0x20 point).
pub const PAYLOAD_AUTODRIVE: u64 = h("SR15NetPayloadUpdatedAutodriveReliable");
pub const CAPTURE_UPDATE: u64 = h("SR15NetCaptureVolumeUpdate");
pub const CAPTURE_RESET: u64 = h("SR15NetCaptureVolumeReset");

pub const LEVEL_DYSON: u64 = 0x43e2_da79_1464_2604;
pub const LEVEL_COMBUSTION: u64 = 0x4267_0f2b_ed45_703c;
pub const LEVEL_FISSION: u64 = 0xdf5c_a7b7_dfa3_83d4;
pub const LEVEL_GAUSS: u64 = 0x43e2_da7a_0c62_3a19;
pub const LEVELS: [u64; 4] = [LEVEL_DYSON, LEVEL_COMBUSTION, LEVEL_FISSION, LEVEL_GAUSS];

/// Native's match-script module (`SR15NetComponentEvent` bytes 8..16).
const MOD_COMBAT_MATCH: u64 = 0x6fa8_cc9c_d721_a563;
const EV_ROUND_BEGIN: u64 = 0x07fd_afd6_80ee_3a56;
/// Sent when the match loads, and at the team room and round start.
const EV_LOAD_A: u64 = 0x9ebb_9a61_909b_5440;
const EV_LOAD_B: u64 = 0x5ca9_e3d2_7753_75f5;
const EV_TEAM_ROOM: u64 = 0x18c4_b28b_0f48_12c4;
const EV_ROUND_START: u64 = 0x0e5c_e3dc_ce8c_0872;
/// Time up (blue wins): instead of `EV_OVER_A`..`C`.
const EV_TIME_UP_A: u64 = 0xff20_bef7_0ee3_749c;
const EV_TIME_UP_B: u64 = 0xe145_49f2_f4c4_0266;
/// Event filters (`SR15NetComponentEvent` bytes 18..20).
const FILTER_ALL: u16 = 5;
const FILTER_ALL_ALT: u16 = 13;
/// The states native passes through into the team room and round start.
const ST_TEAM_ROOM: [u64; 3] = [ST_PRE_MATCH_DONE, 0xe1c1_6321_def4_cc6c, 0xfbc4_6082_c080_fbcf];
const ST_BEFORE_ROUND_START: u64 = 0x3775_6d88_77ed_5768;
const EV_OVER_C: u64 = 0xf7b1_de94_6009_b29e;
/// A payload's clock ran out with attackers on it: overtime begins (native
/// Gauss, twice in one match, each the moment the clock hit 0; the wire bytes
/// `23370169fde82c7d`, `8944e17a6a6fd420`).
const EV_OVERTIME_A: u64 = 0x7d2c_e8fd_6901_3723;
const EV_OVERTIME_B: u64 = 0x20d4_6f6a_7ae1_4489;
const EV_ROUND_OVER_A: u64 = 0x2734_410e_d952_8b14;
const EV_ROUND_OVER_B: u64 = 0xe2d5_eb48_e0d8_a7c4;
const EV_CELEBRATION: u64 = 0xe9df_6ad8_e934_f69c;
const EV_PERSONAL_STATS: u64 = 0x28c4_e143_bd2a_895f;
const EV_RETURN_TO_LOBBY: u64 = 0x6574_2a42_0357_6093;
/// Native Dyson, 15 s after a round over that is not the last: before the
/// team room event (`0c8d2edc2040886f` on the wire).
const EV_NEXT_ROUND: u64 = 0x6f88_4020_dc2e_8d0c;
/// Native Dyson: every frame while both teams are on the unlocked point, on
/// the capture volume's handle, filter 5 (`eae1d95d61eb7a20` on the wire).
const EV_CAPTURE_CONTESTED: u64 = 0x207a_eb61_5dd9_e1ea;
/// The level's game state actor (its announcer and music): native's events
/// on it, filter 5, when a round or the match is won and at a checkpoint
/// (server recordings, 2026-11-16). Its handle is the level's own.
const MOD_GAME_STATE: u64 = 0x8641_1bf2_6906_b560;
/// Who won the round (blue, orange): first, on a capture map.
const GS_ROUND_WON: [u64; 2] = [0x321e_65da_7451_5d6f, 0x83ee_1817_fcd8_bbb9];
/// ... then this when another round follows (Dyson round 1, Combustion round 1),
const GS_NEXT_ROUND: [u64; 2] = [0x22cc_233f_8980_2b89, 0x3a8c_7d4a_4ff8_546f];
/// ... or this for the match (every map).
const GS_MATCH_WON: [u64; 2] = [0x72de_b4a7_7e83_7f5f, 0x5543_743e_603a_6665];
/// A payload map's end, after `GS_MATCH_WON` (Fission home, Gauss time up).
const GS_PAYLOAD_OVER: u64 = 0x22f3_486f_1745_6e6d;
/// The payload reached checkpoint 1, 2 (Fission both, Gauss 1).
const GS_CHECKPOINT: [u64; 2] = [0xc768_c939_9081_44b4, 0xba83_a3cc_7939_f10a];
/// Gauss's own at its checkpoint 1, on the same handle in another module.
const MOD_GAUSS_CHECKPOINT: u64 = 0x3e75_99b2_76e2_031a;
const GAUSS_CHECKPOINT: [u64; 2] = [0xe37e_071c_0a59_9abb, 0x85fd_88ca_2c40_93e0];
/// ... and in the same module 21.36 s after it, as its door has opened
/// (native Gauss, 479.30 s and 500.67 s).
const GAUSS_DOOR_OPEN: u64 = 0x4e31_3900_d6bd_9f8c;
const GAUSS_DOOR_OPEN_AFTER: f32 = 21.36;
/// Native, in the team room after a capture round: a global event (module
/// and handle all ones, filter 5) 3.07 s and 4.05 s in (Dyson and Combustion
/// both, 18.57 s and 19.55 s after the round's win).
const EV_TEAM_ROOM_CUE: u64 = 0xcd87_4f73_cab7_5974;
const TEAM_ROOM_CUES: [f32; 2] = [3.07, 4.05];

/// The game state actor's handle (from the recordings).
fn game_state_handle(level: u64) -> Option<u16> {
    match level {
        LEVEL_DYSON => Some(0xaf),
        LEVEL_COMBUSTION => Some(0xaa),
        LEVEL_FISSION => Some(0xee),
        LEVEL_GAUSS => Some(0x15b),
        _ => None,
    }
}

/// The module of the capture volume's and the barriers' events.
const OBJECT_MODULE: u64 = 0x6aea_99ee_b2ba_b6c4;
/// Rounds a team must win on a capture map. Native Dyson played two rounds,
/// orange won both and the match ended (`round_count` 1 notwithstanding):
/// first to two is the guess that fits.
const CAPTURE_ROUNDS_TO_WIN: u8 = 2;
/// Native Dyson: the capture volume is reset this long before the round begins.
const CAPTURE_RESET_BEFORE: f32 = 5.0;

/// A player by the payload counts as on the objective for the match stats
/// (what native counts there is not recorded; the claim itself is by touch).
const CLAIM_RADIUS: f32 = 3.5;
/// A client's `SR15NetTouchStateEvent`: `[space u32][slot u16][u16][sequence
/// 10 bits][hand 7 bits][hand 7 bits]`, each hand the touch entry it holds
/// (the level's count for none: Gauss 110, Fission 109).
pub const TOUCH_STATE: u64 = h("SR15NetTouchStateEvent");
/// Whether a touch state has a hand on the payload: its slot, and so.
pub fn payload_touch(d: &[u8]) -> Option<(u8, bool)> {
    if d.len() < 12 {
        return None;
    }
    let w = (u32::from(d[9]) | u32::from(d[10]) << 8 | u32::from(d[11]) << 16) >> 2;
    let hands = [(w & 0x7f) as u8, ((w >> 7) & 0x7f) as u8];
    Some((d[4], hands.contains(&PAYLOAD_TOUCH)))
}

/// The payload's touch entry on both maps. Native counts a player on the
/// payload exactly while a hand holds it: every attacker count change in the
/// Gauss and Fission recordings is 1-2 ms after that player's touch state
/// with a hand going on or off 105 (a player standing on the cart 0.7 m from
/// its middle flipped it 1, 0, 1 without moving).
const PAYLOAD_TOUCH: u8 = 105;
/// The payload's own numbers: the level's `CR15NetPayloadCR` row (type
/// `3850c0283892236e`, the same on Gauss and Fission) as the engine's server
/// update uses it (`echovr.exe` 0x140d84ff0):
///
/// - an inner speed moves toward its target by `PAYLOAD_ACCEL` m/s each second
///   (row +0x1c);
/// - attackers alone: the target is the max speed x their multiplier (1 to 4);
///   both teams, a pause or an autodrive: stopped at once, the idle timer 0;
/// - nobody: the idle timer runs, the payload stopped, and past
///   `ROLLBACK_AFTER` (row +0x20: 6.0; a defender's hand: at once) the target is
///   -(`REVERSE_BASE` + `REVERSE_PER_DEFENDER` x defenders) x the max speed
///   (row +0x34, +0x38);
/// - a negative speed moves it x `SLOW_REVERSE` (row +0x3c, with the row's
///   SlowReverse flag 0x20 at +0x10).
///
/// The recordings agree: 3.558 m in the first 3.458 s; rollback 6.000 s after
/// the last attacker (three times) at 0.25 m/s; an attacker's hand on a rolling payload turned it 1.2365 s
/// later (1.25 at 1 m/s each second) and the next trackpoint came 2 ms from this.
const PAYLOAD_ACCEL: f32 = 1.0;
const ROLLBACK_AFTER: f32 = 6.0;
const REVERSE_BASE: f32 = 1.0;
const REVERSE_PER_DEFENDER: f32 = 0.1;
const SLOW_REVERSE: f32 = 0.2;
/// One speed the whole track on every map: `payload_speed_max_mps`, times
/// `payload_speed_multi_attacker_N` for N attackers on it. (The levels'
/// per-section `sNN_payload_speed_multiplier` settings are not applied.)
/// Misc state: an empty payload rolled back onto its last checkpoint (native
/// Gauss 521.042, with the trackpoint before the checkpoint's); it stays set
/// until a later push back ends (561.990).
const PAYLOAD_FLOORED: u16 = 16;
/// Native Fission: from its first checkpoint the payload drives itself to the
/// track's next 0x20 point (2.79 m in 18.5 s), the clock held meanwhile; the
/// misc state turns 2 five seconds in.
const AUTODRIVE_MPS: f32 = 0.15;
const AUTODRIVE_STATE_AFTER: f32 = 5.0;
/// Payload misc states (`UpdateMiscState` bytes 2..4): 1 a checkpoint's
/// celebration, 2 autodriving, 8 the match is over.
const PAYLOAD_CELEBRATING: u16 = 1;
const PAYLOAD_AUTODRIVING: u16 = 2;
const PAYLOAD_OVER: u16 = 8;
/// Native locks entrants before the end once the match is nearly decided:
/// Fission at 90.0% of the track (163.63 of 181.81 m), Combustion's deciding
/// round at 75% of the points (56.27 of 75; Dyson's about the same). A first
/// round past 75% did not lock.
const LOCK_AT_TRACK: f32 = 0.9;
const LOCK_AT_POINTS: f32 = 0.75;
/// Capture volume possession: nobody's (blue 0, orange 1).
const NEUTRAL: u16 = 2;
/// No team converting the point.
const NOBODY: u16 = 0xffff;
/// While a team converts, its progress goes out this often.
const CAPTURE_SEND_EVERY: f32 = 0.1;
// ── Script vars (see the module notes) ─────────────────────────────────────
/// The match's sub-phase symbol: before the team room, in it, a payload's
/// round start (war room count), the round itself.
const VAR_SUBPHASE: u64 = 0x9de6_06e4_96ce_e8a3;
const SUB_TEAM_ROOM: u64 = 0xd503_9cdb_4e56_22f9;
const SUB_ROUND_START: u64 = 0x541c_3aa0_6a22_8bee;
const SUB_PLAYING: u64 = 0x1509_ee27_4504_1c7e;
/// 1 until the round begins (and again in the next round's team room).
const VAR_TEAM_ROOM: u64 = 0x1694_0a2d_58ba_1e0a;
/// 1 from a round's end until 4 s into the next team room (and after the match).
const VAR_ROUND_OVER: u64 = 0x0eae_8218_0440_d42d;
const ROUND_OVER_CLEARS: f32 = 4.0;
/// 1 from 3.04 s into the celebration until the return to the lobby.
const VAR_POST_MATCH: u64 = 0x5ef7_7e9e_acc1_27c0;
const POST_MATCH_AFTER: f32 = 3.04;
const VAR_ROUND_COUNTER: u64 = 0x4eb3_e1b1_52a1_ba7a;
/// Capture maps: rounds won (blue, orange) and the last round's winner.
const VAR_ROUNDS_WON: [u64; 2] = [0x72c1_5861_6250_640b, 0xb546_f3ac_eb9c_95ab];
const VAR_LAST_ROUND_WINNER: u64 = 0xc5c2_fc48_ab26_03d7;
/// Capture point: its owner (locked, neutral, blue, orange) and who is on it
/// (the same, or both).
const VAR_CAP_OWNER: u64 = 0xe919_2a0c_cdcc_f498;
const VAR_CAP_ON: u64 = 0x34f3_a75d_bb96_c0b3;
const CAP_LOCKED: u64 = 0x3fe3_9514_3df6_9263;
const CAP_NEUTRAL: u64 = 0xf558_bbb6_0dc3_bb9a;
const CAP_TEAM: [u64; 2] = [0xc8c3_3e48_387f_0bb5, 0x41d2_c82f_1b2d_001b];
const CAP_BOTH: u64 = 0xba11_136c_4dd2_e646;
/// Per team (blue, orange): how many are on the point, conversion progress
/// (0..1), points (fraction of `points_to_win`, and raw), scoring now.
const VAR_CAP_COUNT: [u64; 2] = [0x4500_0053_ae43_fe8b, 0xed17_977a_b05d_bd33];
const VAR_CAP_PROGRESS: [u64; 2] = [0xa01b_c18b_5719_726b, 0x8ad4_206d_d36b_bc57];
const VAR_CAP_POINTS_FRAC: [u64; 2] = [0x8bd5_41a7_f856_5a03, 0xba4a_488e_fd29_847b];
const VAR_CAP_POINTS: [u64; 2] = [0xc3c1_c808_a637_4775, 0x5aa6_7dae_23fb_4993];
const VAR_CAP_SCORING: [u64; 2] = [0x4f8a_a683_cc60_b185, 0xdf54_2064_982b_cdf7];
/// Seconds left on the point's lock.
const VAR_CAP_LOCK: u64 = 0xeb40_dd00_966d_72fe;

/// Players needed before the pre-match count starts (public: one is enough to
/// test; native waits for its matchmaker's teams).
const PRE_MATCH_PLAYERS: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Waiting,
    PreMatch,
    TeamRoom,
    RoundStart,
    Playing,
    /// A capture map's round is won and another follows.
    RoundOver,
    Over,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Winner {
    Blue,
    Orange,
    Draw,
}

/// A message for every joined client.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Reliable(u64, Vec<u8>),
    /// A component event: symbol, module, handle, filter.
    Event(u64, u64, u16, u16),
    /// The match is over: top awards now.
    Awards,
    /// Lock the match (no more joins).
    Lock,
    /// Take joins again (the next round of a capture map).
    Unlock,
    /// Every barrier back up (a capture map's next round).
    RestoreBarriers,
    /// A private payload match's half time: every blue player goes orange
    /// and every orange player blue.
    SwapTeams,
    /// A private match restarts: every player's stats back to 0, and sent.
    ResetStats,
}

/// What native's level scripts do that the server repeats: a client's
/// server-only event sets level vars; barriers drive vars of their own.
struct Script {
    /// (event symbol, module, handle) -> (var, value).
    reflexes: Vec<(u64, u64, u16, u64, u64)>,
    /// (barrier handle, var): the var is 0 while a barrier of its set is down.
    barrier_vars: Vec<(u16, u64)>,
    /// Level vars set so far (sent with the mode's own).
    set: Vec<(u64, u64)>,
    down: Vec<u16>,
}

impl Script {
    fn of(level: u64) -> Self {
        let (reflex, barrier) = match level {
            LEVEL_DYSON => (include_str!("data/combat_dyson_reflex.txt"), include_str!("data/combat_dyson_barrier_vars.txt")),
            LEVEL_COMBUSTION => (include_str!("data/combat_combustion_reflex.txt"), include_str!("data/combat_combustion_barrier_vars.txt")),
            LEVEL_FISSION => (include_str!("data/combat_fission_reflex.txt"), include_str!("data/combat_fission_barrier_vars.txt")),
            LEVEL_GAUSS => (include_str!("data/combat_gauss_reflex.txt"), include_str!("data/combat_gauss_barrier_vars.txt")),
            _ => ("", ""),
        };
        let hex64 = |x: &str| u64::from_str_radix(x.trim_start_matches("0x"), 16).ok();
        let wire = |x: &str| hex64(x).map(u64::swap_bytes);
        let lines = |t: &'static str| t.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty());
        let reflexes = lines(reflex)
            .filter_map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                Some((wire(w.first()?)?, wire(w.get(1)?)?, hex64(w.get(2)?)? as u16, hex64(w.get(3)?)?, hex64(w.get(4)?)?))
            })
            .collect();
        let barrier_vars = lines(barrier)
            .filter_map(|l| {
                let w: Vec<&str> = l.split_whitespace().collect();
                Some((hex64(w.first()?)? as u16, hex64(w.get(1)?)?))
            })
            .collect();
        Self { reflexes, barrier_vars, set: Vec::new(), down: Vec::new() }
    }

    fn event(&mut self, sym: u64, module: u64, handle: u16) {
        for &(s, m, h, var, value) in &self.reflexes {
            if (s, m, h) == (sym, module, handle) {
                match self.set.iter_mut().find(|e| e.0 == var) {
                    Some(e) => e.1 = value,
                    None => self.set.push((var, value)),
                }
            }
        }
    }

    fn vars(&self, v: &mut Vec<(u64, Val)>) {
        v.extend(self.set.iter().map(|&(k, x)| (k, Val::Raw(x))));
        let mut seen: Vec<u64> = Vec::new();
        for &(_, var) in &self.barrier_vars {
            if seen.contains(&var) {
                continue;
            }
            seen.push(var);
            let down = self.barrier_vars.iter().any(|&(h, k)| k == var && self.down.contains(&h));
            v.push((var, Val::Lo32(u32::from(!down))));
        }
    }
}

struct Track {
    /// (trackpoint, metres, position)
    points: Vec<(u16, f32, [f32; 3])>,
    checkpoints: Vec<f32>,
    /// Points flagged 0x20 (Fission's one, after its first checkpoint): where
    /// an autodrive from the checkpoint before them stops.
    autodrive: Vec<f32>,
}

impl Track {
    /// A level's payload track, if it has one.
    fn of(level: u64) -> Option<Self> {
        let text = match level {
            LEVEL_GAUSS => include_str!("data/combat_gauss_track.txt"),
            LEVEL_FISSION => include_str!("data/combat_fission_track.txt"),
            _ => return None,
        };
        Some(Self::parse(text))
    }

    /// `trackpoint metres x y z flags` lines (flags 0x40: a checkpoint).
    fn parse(text: &str) -> Self {
        let mut points = Vec::new();
        let mut checkpoints = Vec::new();
        let mut autodrive = Vec::new();
        for l in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let w: Vec<&str> = l.split_whitespace().collect();
            let v: Vec<f32> = w.iter().take(5).filter_map(|x| x.parse().ok()).collect();
            if v.len() != 5 {
                continue;
            }
            let flags = w.get(5).and_then(|x| u32::from_str_radix(x.trim_start_matches("0x"), 16).ok()).unwrap_or(0);
            if flags & 0x40 != 0 {
                checkpoints.push(v[1]);
            } else if flags & 0x20 != 0 {
                autodrive.push(v[1]);
            }
            points.push((v[0] as u16, v[1], [v[2], v[3], v[4]]));
        }
        // The end is the last checkpoint.
        checkpoints.push(points.last().map_or(0.0, |p| p.1));
        Self { points, checkpoints, autodrive }
    }

    #[cfg(test)]
    fn gauss() -> Self {
        Self::of(LEVEL_GAUSS).expect("gauss track")
    }

    fn end(&self) -> f32 {
        self.checkpoints.last().copied().unwrap_or(0.0)
    }

    fn pos(&self, d: f32) -> [f32; 3] {
        let p = &self.points;
        let i = p.iter().position(|e| e.1 > d).unwrap_or(p.len() - 1).max(1);
        let (a, b) = (p[i - 1], p[i]);
        let t = ((d - a.1) / (b.1 - a.1).max(0.01)).clamp(0.0, 1.0);
        [0, 1, 2].map(|k| a.2[k] + (b.2[k] - a.2[k]) * t)
    }

    /// The furthest trackpoint at or before `d`.
    fn trackpoint(&self, d: f32) -> u16 {
        self.points.iter().take_while(|e| e.1 <= d).last().map_or(0, |e| e.0)
    }
}

/// A capture volume's shape (world space, y up).
#[derive(Clone, Copy, Debug)]
enum Shape {
    Sphere([f32; 3], f32),
    /// Centre, rotation, half extents.
    Box([f32; 3], [f32; 4], [f32; 3]),
}

impl Shape {
    fn contains(&self, p: [f32; 3]) -> bool {
        match *self {
            Shape::Sphere(c, r) => dist(p, c) <= r,
            Shape::Box(c, q, h) => {
                let l = unrotate(q, [p[0] - c[0], p[1] - c[1], p[2] - c[2]]);
                (0..3).all(|k| l[k].abs() <= h[k])
            }
        }
    }
}

/// `v` turned by the inverse of the unit quaternion `q` (x y z w).
fn unrotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let (u, w) = ([-q[0], -q[1], -q[2]], q[3]);
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = cross(u, v).map(|x| 2.0 * x);
    let ut = cross(u, t);
    [0, 1, 2].map(|k| v[k] + w * t[k] + ut[k])
}

struct Capture {
    shapes: Vec<Shape>,
    /// The volume's net handle (the `CR15NetCaptureVolumeCR` actor in the
    /// level's IdsResponse: Dyson 0xae, as recorded; Combustion 0xa9).
    handle: u16,
    possession: u16,
    converting: u16,
    /// Each team's conversion progress (seconds).
    progress: [f32; 2],
    locked: bool,
    lock_left: f32,
    contested: bool,
    /// Who of each team is on it (bit = slot), and how many.
    claims: [u16; 2],
    counts: [u16; 2],
    points: [f32; 2],
    sent: Option<(u16, u16, [u16; 2], [u16; 2])>,
    since_send: f32,
    /// Unlocked this round (native: the owner symbol stays "locked" until then).
    unlocked_once: bool,
}

impl Capture {
    /// A level's capture volume, if it has one.
    fn of(level: u64) -> Option<Self> {
        let (text, handle) = match level {
            LEVEL_DYSON => (include_str!("data/combat_dyson_capture.txt"), 0xae),
            LEVEL_COMBUSTION => (include_str!("data/combat_combustion_capture.txt"), 0xa9),
            _ => return None,
        };
        let shapes = text
            .lines()
            .filter_map(|l| {
                let mut w = l.split_whitespace();
                let kind = w.next()?;
                let v: Vec<f32> = w.filter_map(|x| x.parse().ok()).collect();
                match (kind, v.len()) {
                    ("sphere", 4) => Some(Shape::Sphere([v[0], v[1], v[2]], v[3])),
                    ("box", 10) => Some(Shape::Box([v[0], v[1], v[2]], [v[3], v[4], v[5], v[6]], [v[7], v[8], v[9]])),
                    _ => None,
                }
            })
            .collect();
        Some(Self {
            shapes,
            handle,
            possession: NEUTRAL,
            converting: NOBODY,
            progress: [0.0; 2],
            locked: true,
            lock_left: 0.0,
            contested: false,
            claims: [0; 2],
            counts: [0; 2],
            points: [0.0; 2],
            sent: None,
            since_send: 0.0,
            unlocked_once: false,
        })
    }

    /// A new round: neutral, nobody converting, no points.
    fn reset(&mut self) {
        self.possession = NEUTRAL;
        self.converting = NOBODY;
        self.progress = [0.0; 2];
        self.contested = false;
        self.claims = [0; 2];
        self.counts = [0; 2];
        self.points = [0.0; 2];
        self.sent = None;
        self.since_send = 0.0;
        self.unlocked_once = false;
    }

    fn inside(&self, p: [f32; 3]) -> bool {
        self.shapes.iter().any(|s| s.contains(p))
    }

    /// The volume's id: the only one of its kind on the level, so pool index
    /// 0, generation 0 (a fresh level's handles, as the recorded bullet
    /// targets show), kind 1.
    fn id(space: u64) -> Vec<u8> {
        let mut b = space.to_le_bytes().to_vec();
        b.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0]);
        b
    }

    fn update(&self, space: u64) -> Vec<u8> {
        let mut b = Self::id(space);
        b.extend_from_slice(&self.possession.to_le_bytes());
        // Native: then a flags byte whose bit 0 is set while both teams are
        // on it (the client's "contested" announcer and effects); its other
        // bits and the next byte are left uninitialised (Dyson 178.05: 01,
        // 95, 85, b9 contested; 00, 94, b8, c4 not).
        b.extend_from_slice(&[u8::from(self.contested), 0]);
        b.extend_from_slice(&self.converting.to_le_bytes());
        b.extend_from_slice(&[0, 0]);
        for t in 0..2 {
            b.extend_from_slice(&self.claims[t].to_le_bytes());
            b.extend_from_slice(&self.counts[t].to_le_bytes());
            b.extend_from_slice(&self.progress[t].to_le_bytes());
        }
        b
    }
}

struct Payload {
    track: Track,
    handle: u16,
    dist: f32,
    best_tp: u16,
    /// The trackpoint it is on now (native sends `PosStateUpdate` on every
    /// change, back as well as forward).
    cur_tp: u16,
    /// Next checkpoint (index into `track.checkpoints`).
    next: usize,
    idle: f32,
    /// The inner speed (m/s; a negative one moves it x `SLOW_REVERSE`).
    speed: f32,
    /// Celebration: payload and clock held.
    hold: f32,
    /// Autodriving to this distance, and for how long so far.
    autodrive_to: Option<f32>,
    auto_for: f32,
    /// The misc state (0, `PAYLOAD_CELEBRATING`, ...).
    state: u16,
    sent: Option<(u16, u8, u8, i8)>,
    /// The furthest it got (metres).
    furthest: f32,
    /// For the HUD: attackers and defenders on it, moving (1, 0, -1; 0
    /// while held or driving itself), claimed by attackers yet.
    hud: (u8, u8, i8),
    claimed_once: bool,
    /// Gauss's door count, from its first checkpoint.
    door: Option<f32>,
}

impl Payload {
    fn new(track: Track) -> Self {
        // Native never announces the starting trackpoint (Fission's first is 0x1c).
        let first = track.points.first().map_or(0, |e| e.0);
        Self {
            // The payload is the net component right after the track points.
            handle: 0x1b + track.points.len() as u16,
            track,
            dist: 0.0,
            best_tp: first,
            cur_tp: first,
            next: 0,
            idle: 0.0,
            speed: 0.0,
            hold: 0.0,
            autodrive_to: None,
            auto_for: 0.0,
            state: 0,
            sent: None,
            furthest: 0.0,
            hud: (0, 0, 0),
            claimed_once: false,
            door: None,
        }
    }
}

/// `data/combat_<map>_track_vars.txt` (`tools/gen_track_vars.py`): level vars
/// native's scripts set as the payload first reaches a trackpoint (the
/// checkpoint counter and symbol, the level's load progression, ...):
/// (trackpoint, var, value, value before the match).
fn track_vars(level: u64) -> Vec<(u16, u64, u64, u64)> {
    let text = match level {
        LEVEL_GAUSS => include_str!("data/combat_gauss_track_vars.txt"),
        LEVEL_FISSION => include_str!("data/combat_fission_track_vars.txt"),
        _ => return Vec::new(),
    };
    let hex = |x: &str| u64::from_str_radix(x, 16).ok();
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            Some((hex(w.first()?)? as u16, hex(w.get(1)?)?, hex(w.get(2)?)?, hex(w.get(3)?)?))
        })
        .collect()
}

pub struct Combat {
    pub level: u64,
    pub phase: Phase,
    pub state: u64,
    pub clock: f32,
    pub countdown: f32,
    overtime: f32,
    match_handle: u16,
    payload: Option<Payload>,
    capture: Option<Capture>,
    settings: Value,
    rules: Value,
    /// Seconds since the match ended (the end sequence runs on it).
    over_for: f32,
    end_step: usize,
    pub winner: Option<Winner>,
    /// States still to go out, one a step (native sends each in its own frame).
    states: std::collections::VecDeque<u64>,
    /// Capture maps: rounds each team has won.
    rounds_won: [u8; 2],
    /// Capture maps: the round being played (1 = first).
    round: u8,
    capture_reset_sent: bool,
    /// In overtime (its events sent once on the way in).
    in_overtime: bool,
    /// Entrants locked early (nearly decided).
    early_lock: bool,
    script: Script,
    /// Slots with a hand on the payload (their touch states).
    holders: Vec<u8>,
    /// Each player's place in their team (slot, team slot): the capture
    /// point's claimer bits are by team slot.
    team_slots: Vec<(u8, u8)>,
    subphase: Option<u64>,
    team_room: bool,
    /// The round over flag, and how long until it clears (in a team room).
    round_over: bool,
    round_over_clears: f32,
    /// Seconds into a between-rounds team room, and the cues still to send.
    team_room_for: f32,
    cues_left: usize,
    post_match: bool,
    celebrating: bool,
    last_round_winner: Option<u8>,
    /// Capture maps: the points vars exist (native creates them with the
    /// first point, then resets them in the next team room).
    points_shown: bool,
    /// A private match (see the notes at the top): teams ready up.
    private: bool,
    ready: [bool; 2],
    /// Seconds since every team was ready (the start is under way).
    starting: Option<f32>,
    /// The teams' restart requests.
    restart_req: [bool; 2],
    /// Seconds until a rematch or restart goes back to waiting for ready.
    restarting: Option<f32>,
    /// The match-load events went out (a private match waiting for ready).
    loaded: bool,
    /// Private payload: the half (1, 2) and the first attackers' furthest
    /// point, which the second must reach.
    half: u8,
    to_beat: Option<f32>,
    track_vars: Vec<(u16, u64, u64, u64)>,
    /// Level vars from `track_vars` set so far.
    track_set: Vec<(u64, Val)>,
}

fn f(v: &Value, path: &str, def: f32) -> f32 {
    v.pointer(path).and_then(Value::as_f64).map_or(def, |x| x as f32)
}

impl Combat {
    /// `components`: how many net components the level's IdsResponse has
    /// (the match script is the last, handles from 0x1b).
    pub fn new(level: u64, components: usize, settings: Value, rules: Value) -> Option<Self> {
        if !LEVELS.contains(&level) {
            return None;
        }
        let payload = Track::of(level).map(Payload::new);
        let mut c = Self {
            level,
            phase: Phase::Waiting,
            state: ST_PRE_MATCH,
            clock: 0.0,
            countdown: 0.0,
            overtime: 0.0,
            match_handle: 0x1b + components.saturating_sub(1) as u16,
            payload,
            capture: Capture::of(level),
            settings,
            rules,
            over_for: 0.0,
            end_step: 0,
            winner: None,
            states: std::collections::VecDeque::new(),
            rounds_won: [0; 2],
            round: 1,
            capture_reset_sent: false,
            in_overtime: false,
            early_lock: false,
            script: Script::of(level),
            holders: Vec::new(),
            team_slots: Vec::new(),
            subphase: None,
            team_room: true,
            round_over: false,
            round_over_clears: 0.0,
            team_room_for: 0.0,
            cues_left: 0,
            post_match: false,
            celebrating: false,
            last_round_winner: None,
            points_shown: false,
            private: false,
            ready: [false; 2],
            starting: None,
            restart_req: [false; 2],
            restarting: None,
            loaded: false,
            half: 1,
            to_beat: None,
            track_vars: track_vars(level),
            track_set: Vec::new(),
        };
        c.clock = c.round_secs();
        c.countdown = f(&c.rules, "/round_rules/pre_match_wait_time", 30.0);
        Some(c)
    }

    fn round_secs(&self) -> f32 {
        f(&self.settings, "/round_timers/start", 180.0)
    }

    /// Replicated vars this mode owns.
    pub fn vars(&self) -> Vec<(u64, Val)> {
        let mut v = vec![
            (VAR_GAME_STATE, Val::Raw(self.state)),
            (VAR_GAME_CLOCK, Val::F32(self.clock.max(0.0))),
            (VAR_COUNTDOWN, Val::F32(self.countdown.max(0.0))),
        ];
        // Native, all four maps' server recordings: the winner's team score
        // goes to 1 as the match is won (the round scores are never set). A
        // capture point's points stay with the maps' scripts: clients count
        // them from the volume updates.
        match self.winner {
            Some(Winner::Blue) => v.push((VAR_BLUE_SCORE, Val::Lo32(1))),
            Some(Winner::Orange) => v.push((VAR_RED_SCORE, Val::Lo32(1))),
            _ => {}
        }
        if let Some(s) = self.subphase {
            v.push((VAR_SUBPHASE, Val::Raw(s)));
        }
        v.extend([
            (VAR_TEAM_ROOM, Val::Lo8(u8::from(self.team_room))),
            (VAR_ROUND_OVER, Val::Lo32(u32::from(self.round_over))),
            (VAR_POST_MATCH, Val::Lo32(u32::from(self.post_match))),
            (VAR_CELEBRATION, Val::Lo8(u8::from(self.celebrating))),
            (VAR_ROUND_COUNTER, Val::Lo32(u32::from(self.round))),
        ]);
        if let Some(c) = &self.capture {
            for t in 0..2 {
                if self.rounds_won[t] > 0 {
                    v.push((VAR_ROUNDS_WON[t], Val::Lo32(u32::from(self.rounds_won[t]))));
                }
            }
            if let Some(w) = self.last_round_winner {
                v.push((VAR_LAST_ROUND_WINNER, Val::Lo32(u32::from(w))));
            }
            v.extend(self.capture_vars(c));
        }
        if let Some(p) = &self.payload {
            v.extend(self.payload_vars(p));
        }
        if self.private {
            v.push((VAR_BLUE_READY, Val::Lo8(u8::from(self.ready[0]))));
            v.push((VAR_ORANGE_READY, Val::Lo8(u8::from(self.ready[1]))));
            v.push((VAR_RESTART_REQUEST[0], Val::Lo8(u8::from(self.restart_req[0]))));
            v.push((VAR_RESTART_REQUEST[1], Val::Lo8(u8::from(self.restart_req[1]))));
        }
        self.script.vars(&mut v);
        v.extend(self.track_set.iter().copied());
        v
    }

    /// The payload's HUD vars (see their notes), and a private match's
    /// halves.
    fn payload_vars(&self, p: &Payload) -> Vec<(u64, Val)> {
        let (att, def, moving) = p.hud;
        let claim = match (att > 0, def > 0) {
            (true, true) => CLAIM_CONTESTED,
            (true, false) => CLAIM_ATTACKERS,
            _ => CLAIM_NONE,
        };
        // Native Fission: a self-driving payload stays `forward`.
        let motion = match moving {
            1 => MOTION_FORWARD,
            -1 => MOTION_BACKWARD,
            _ if p.autodrive_to.is_some() => MOTION_FORWARD,
            _ => MOTION_STILL,
        };
        let mut v = vec![
            (VAR_CLAIM_STATE, Val::Raw(claim)),
            (VAR_PAYLOAD_MOTION, Val::Raw(motion)),
            (VAR_PAYLOAD_DIRECTION, Val::Raw(if moving < 0 { MOTION_BACKWARD } else { MOTION_FORWARD })),
            (VAR_PAYLOAD_MOVING, Val::Lo32(moving as i32 as u32)),
            (VAR_ATTACKER_CLAIM, Val::Lo32(u32::from(att > 0))),
            (VAR_ATTACKER_FLAG, Val::Lo8(u8::from(att > 0))),
            (VAR_DEFENDER_CLAIM, Val::Lo32(u32::from(def))),
            (VAR_OVERTIME_PROGRESS, Val::F32(if p.claimed_once { 1.0 } else { 0.0 })),
        ];
        match self.level {
            LEVEL_FISSION => v.push((VAR_CLAIM_STATE_FISSION, Val::Raw(claim))),
            LEVEL_GAUSS => v.push((VAR_CLAIM_STATE_GAUSS, Val::Raw(claim))),
            _ => {}
        }
        if let Some(d) = p.door {
            v.push((VAR_GAUSS_DOOR, Val::Lo32(d.max(0.0).ceil() as u32)));
        }
        if self.private {
            // Team a: the first attackers (orange in the first half).
            let first = self.to_beat.unwrap_or(p.furthest);
            let second = if self.to_beat.is_some() { p.furthest } else { 0.0 };
            v.extend([
                (VAR_TOTAL_ROUNDS, Val::Lo32(2)),
                (VAR_ROUNDS_PLAYED, Val::Lo32(u32::from(self.half - 1))),
                (VAR_DEFENDING_TEAM, Val::Lo32(0)),
                (VAR_FURTHEST_ROUND[0], Val::F32(first)),
                (VAR_FURTHEST_ROUND[1], Val::F32(second)),
                (VAR_TEAM_DISTANCE[0], Val::F32(first)),
                (VAR_TEAM_DISTANCE[1], Val::F32(second)),
            ]);
            if let Some(d) = self.to_beat {
                let shown = Val::Lo8(1);
                v.extend([
                    (VAR_TEAM_A_MARKER_POS, Val::F32(d)),
                    (VAR_TEAM_A_MARKER_SHOWN, shown),
                    (VAR_PROGRESS_MARKER_POS, Val::F32(d)),
                    (VAR_PROGRESS_MARKER_SHOWN, shown),
                ]);
            }
        }
        v
    }

    /// A private match: teams ready up before it starts.
    pub fn set_private(&mut self) {
        self.private = true;
        self.state = ST_WAIT_FOR_READY;
    }

    pub fn is_private(&self) -> bool {
        self.private
    }

    /// A private match's team (0 blue, 1 orange) pressed ready: native flips
    /// the flag (a second press un-readies). `Some(now ready)` if it counted
    /// (waiting for ready, the start not under way yet).
    pub fn team_ready(&mut self, team: usize) -> Option<bool> {
        if !self.private || self.phase != Phase::Waiting || team > 1 || self.starting.is_some() || self.restarting.is_some() {
            return None;
        }
        self.ready[team] = !self.ready[team];
        Some(self.ready[team])
    }

    /// A private match's team asked for a restart: true if the flag went up
    /// (native sets it, a second press does nothing).
    pub fn restart_request(&mut self, team: usize) -> bool {
        if !self.private || team > 1 || self.restart_req[team] {
            return false;
        }
        self.restart_req[team] = true;
        true
    }

    /// A private match's rematch button: in the post-match celebration or
    /// the personal stats, the match starts over (back to waiting for ready).
    pub fn rematch(&mut self) -> Vec<Out> {
        let at_end = self.phase == Phase::Over && (self.state == ST_MATCH_END[3] || self.state == ST_MATCH_END[5]);
        if !self.private || !at_end || self.restarting.is_some() {
            return Vec::new();
        }
        self.restart()
    }

    /// The lobby button: everyone goes back to the lobby (native's
    /// `ReturnToLobby`: `nevt_return_to_lobby`, waiting_to_return_to_lobby).
    pub fn return_to_lobby(&mut self) -> Vec<Out> {
        if self.state == ST_MATCH_END[6] {
            return Vec::new();
        }
        self.states.clear();
        self.state = ST_MATCH_END[6];
        self.post_match = false;
        vec![self.event(EV_RETURN_TO_LOBBY)]
    }

    /// Native's match restart (rematch, or both teams' restart requests):
    /// its events and the stats reset now, waiting for ready 0.2 s later.
    fn restart(&mut self) -> Vec<Out> {
        tracing::info!("private combat match: restart");
        self.restarting = Some(RESTART_TO_WAITING);
        vec![
            self.event(EV_GAME_RESTART_CONFIRMED),
            self.event(EV_NEXT_ROUND),
            self.event(EV_GAME_RESTART),
            Out::ResetStats,
            self.event(EV_REFRESH_SCOREBOARD),
        ]
    }

    /// Every team with players (`on`: blue, orange) has `flags` set, and
    /// at least one is set (native's test, an empty team does not count).
    fn all_teams(flags: [bool; 2], on: (bool, bool)) -> bool {
        (flags[0] || flags[1]) && (!on.0 || flags[0]) && (!on.1 || flags[1])
    }

    /// Back to the start: waiting for players (public) or for ready
    /// (private), the payload back at the start when `reset_payload`.
    fn reset_to_waiting(&mut self, reset_payload: bool) -> Vec<Out> {
        let mut out = Vec::new();
        self.states.clear();
        self.rounds_won = [0; 2];
        self.round = 1;
        self.capture_reset_sent = false;
        self.early_lock = false;
        self.clock = self.round_secs();
        if let Some(c) = self.capture.as_mut() {
            c.reset();
        }
        self.state = if self.private { ST_WAIT_FOR_READY } else { ST_PRE_MATCH };
        self.countdown = f(&self.rules, "/round_rules/pre_match_wait_time", 30.0);
        self.subphase = None;
        self.team_room = true;
        self.round_over = false;
        self.last_round_winner = None;
        self.points_shown = false;
        self.cues_left = 0;
        self.ready = [false; 2];
        self.loaded = false;
        if reset_payload {
            // Left at half time, or a restart: from the start of the track.
            out.extend(self.reset_payload());
        }
        self.phase = Phase::Waiting;
        self.half = 1;
        self.to_beat = None;
        self.starting = None;
        self.restart_req = [false; 2];
        out
    }

    /// A private match's restart lands: everything as at the start, the
    /// end sequence and the winner cleared, the payload at the start.
    fn reset_match(&mut self) -> Vec<Out> {
        self.winner = None;
        self.over_for = 0.0;
        self.end_step = 0;
        self.post_match = false;
        self.celebrating = false;
        self.in_overtime = false;
        self.overtime = 0.0;
        let reset_payload = self.payload.is_some();
        self.reset_to_waiting(reset_payload)
    }

    /// The level vars a trackpoint's first reach sets (all those passed, in
    /// order).
    fn reach_track_vars(&mut self, from: u16, to: u16) {
        for &(tp, var, value, _) in &self.track_vars {
            if tp > from && tp <= to {
                upsert(&mut self.track_set, var, Val::Raw(value));
            }
        }
    }

    /// Back to the start of the track (a private match's second half): the
    /// level vars as before the match, the payload where it began.
    fn reset_payload(&mut self) -> Vec<Out> {
        for &(_, var, _, before) in &self.track_vars {
            // A var the match created keeps its type tag.
            let v = if before == 0 { Val::Lo32(0) } else { Val::Raw(before) };
            upsert(&mut self.track_set, var, v);
        }
        let Some(p) = self.payload.as_mut() else { return Vec::new() };
        let track = std::mem::replace(&mut p.track, Track { points: Vec::new(), checkpoints: Vec::new(), autodrive: Vec::new() });
        *p = Payload::new(track);
        let mut b = p.handle.to_le_bytes().to_vec();
        b.extend_from_slice(&p.cur_tp.to_le_bytes());
        p.sent = Some((0, 0, 0, 0));
        vec![Out::Reliable(PAYLOAD_POS, b), Out::Reliable(PAYLOAD_MISC, misc(p.handle, (0, 0, 0, 0)))]
    }

    /// The capture point's script vars (native Dyson and Combustion, to the
    /// frame): see the module notes.
    fn capture_vars(&self, c: &Capture) -> Vec<(u64, Val)> {
        let to_capture = f(&self.settings, "/capture_point/time_to_capture", 4.0).max(0.1);
        let to_win = f(&self.settings, "/capture_point/points_to_win", 75.0).max(1.0);
        let playing = matches!(self.phase, Phase::Playing);
        let open = playing && !c.locked;
        let owner = if !c.unlocked_once {
            CAP_LOCKED
        } else if c.possession < NEUTRAL {
            CAP_TEAM[usize::from(c.possession)]
        } else {
            CAP_NEUTRAL
        };
        let on = if !open {
            // Locked, or the round is over.
            CAP_LOCKED
        } else {
            match (c.counts[0] > 0, c.counts[1] > 0) {
                (true, true) => CAP_BOTH,
                (true, false) => CAP_TEAM[0],
                (false, true) => CAP_TEAM[1],
                _ => CAP_NEUTRAL,
            }
        };
        let mut v = vec![(VAR_CAP_OWNER, Val::Raw(owner)), (VAR_CAP_ON, Val::Raw(on))];
        let lock = if playing && c.locked { c.lock_left.max(0.0) } else { 0.0 };
        v.push((VAR_CAP_LOCK, Val::F32(lock)));
        for t in 0..2 {
            let counts = if open { c.counts[t] } else { 0 };
            v.push((VAR_CAP_COUNT[t], Val::Lo32(u32::from(counts))));
            v.push((VAR_CAP_PROGRESS[t], Val::F32((c.progress[t] / to_capture).clamp(0.0, 1.0))));
            let scoring = open && c.possession == t as u16 && c.counts[1 - t] == 0;
            v.push((VAR_CAP_SCORING[t], Val::Lo8(u8::from(scoring))));
            if self.points_shown {
                v.push((VAR_CAP_POINTS_FRAC[t], Val::F32((c.points[t] / to_win).clamp(0.0, 1.0))));
                v.push((VAR_CAP_POINTS[t], Val::F32(c.points[t])));
            }
        }
        v
    }

    /// A client's `SR15NetComponentEvent` (server-only, filter 5): what the
    /// level's scripts do with it natively.
    pub fn client_event(&mut self, sym: u64, module: u64, handle: u16) {
        self.script.event(sym, module, handle);
    }

    /// The barriers that are down now (their handles).
    pub fn set_barriers_down(&mut self, down: Vec<u16>) {
        self.script.down = down;
    }

    /// The players (slots) with a hand on the payload now.
    pub fn set_payload_holders(&mut self, slots: Vec<u8>) {
        self.holders = slots;
    }

    /// Who is where in their team: (slot, team slot).
    pub fn set_team_slots(&mut self, slots: Vec<(u8, u8)>) {
        self.team_slots = slots;
    }

    fn event(&self, sym: u64) -> Out {
        Out::Event(sym, MOD_COMBAT_MATCH, self.match_handle, FILTER_ALL)
    }

    fn event_alt(&self, sym: u64) -> Out {
        Out::Event(sym, MOD_COMBAT_MATCH, self.match_handle, FILTER_ALL_ALT)
    }

    /// A player here is on the objective (match stats): in the capture
    /// volume, or by the payload, while the round is played.
    pub fn on_objective(&self, pos: [f32; 3]) -> bool {
        if self.phase != Phase::Playing {
            return false;
        }
        if let Some(c) = &self.capture {
            return c.inside(pos);
        }
        self.payload.as_ref().is_some_and(|p| dist(pos, p.track.pos(p.dist)) <= CLAIM_RADIUS)
    }

    /// A capture point map (else a payload map).
    pub fn is_capture(&self) -> bool {
        self.capture.is_some()
    }

    /// The team that holds the capture point (0 blue, 1 orange), if any.
    pub fn capture_owner(&self) -> Option<u8> {
        self.capture.as_ref().filter(|c| c.possession < NEUTRAL).map(|c| c.possession as u8)
    }

    /// The match has begun and is not over yet: stats count and are sent.
    pub fn counting(&self) -> bool {
        matches!(self.phase, Phase::Playing | Phase::RoundOver | Phase::TeamRoom | Phase::RoundStart) && !(self.phase == Phase::TeamRoom && self.round == 1)
    }

    /// An event on the level's game state actor (none on an unknown level).
    fn announce(&self, module: u64, syms: &[u64]) -> Vec<Out> {
        game_state_handle(self.level).map_or_else(Vec::new, |h| syms.iter().map(|&s| Out::Event(s, module, h, FILTER_ALL)).collect())
    }

    /// Go through `states` one step each, ending on the last.
    fn to_states(&mut self, states: &[u64]) {
        if let Some((first, rest)) = states.split_first() {
            self.state = *first;
            self.states.extend(rest);
        }
    }

    /// One step of `dt` seconds. `players`: (team: 0 blue / 1 orange, alive,
    /// body position) of everyone on a team.
    pub fn step(&mut self, dt: f32, players: &[Player]) -> Vec<Out> {
        let mut out = Vec::new();
        if let Some(s) = self.states.pop_front() {
            self.state = s;
        }
        let on = (players.iter().any(|p| p.0 == 0), players.iter().any(|p| p.0 == 1));
        if let Some(left) = self.restarting.as_mut() {
            *left -= dt;
            if *left <= 0.0 {
                self.restarting = None;
                out.extend(self.reset_match());
            }
            return out;
        }
        // Native's midgame restart: every team with players asked, in the
        // team room, pre-launch or play.
        if self.private
            && matches!(self.phase, Phase::TeamRoom | Phase::RoundStart | Phase::Playing)
            && Self::all_teams(self.restart_req, on)
        {
            out.extend(self.restart());
            return out;
        }
        match self.phase {
            Phase::Waiting if self.private => {
                if !players.is_empty() && !self.loaded {
                    self.loaded = true;
                    out.push(self.event(EV_LOAD_A));
                    out.push(self.event(EV_LOAD_B));
                }
                if self.starting.is_none() && Self::all_teams(self.ready, on) {
                    tracing::info!("private combat match: all teams ready");
                    self.starting = Some(0.0);
                    out.push(self.event(EV_NEXT_ROUND));
                }
                let Some(t) = self.starting.as_mut() else { return out };
                let was = *t;
                *t += dt;
                let now = *t;
                if was < READY_TO_MATCH_READY && now >= READY_TO_MATCH_READY {
                    out.push(self.event(EV_MATCH_READY));
                }
                if now >= READY_TO_ROUND_START {
                    self.starting = None;
                    // round_start, then the team room, as a public match
                    // after its pre-match wait.
                    self.phase = Phase::TeamRoom;
                    self.to_states(&ST_TEAM_ROOM);
                    self.subphase = Some(SUB_TEAM_ROOM);
                    out.push(self.event(EV_TEAM_ROOM));
                    self.countdown = f(&self.rules, "/round_rules/team_room_wait_time", 15.0);
                }
            }
            Phase::Waiting => {
                if players.len() >= PRE_MATCH_PLAYERS {
                    self.phase = Phase::PreMatch;
                    self.state = ST_PRE_MATCH;
                    out.push(self.event(EV_LOAD_A));
                    out.push(self.event(EV_LOAD_B));
                }
            }
            Phase::PreMatch | Phase::TeamRoom | Phase::RoundStart | Phase::RoundOver if players.is_empty() => {
                // Everyone left before the round began: start over.
                let moved = self.payload.is_some() && (self.half > 1 || self.phase == Phase::RoundOver);
                out.extend(self.reset_to_waiting(moved));
            }
            Phase::PreMatch => {
                self.countdown -= dt;
                if self.countdown <= 0.0 {
                    self.phase = Phase::TeamRoom;
                    self.to_states(&ST_TEAM_ROOM);
                    self.subphase = Some(SUB_TEAM_ROOM);
                    out.push(self.event(EV_TEAM_ROOM));
                    self.countdown = f(&self.rules, "/round_rules/team_room_wait_time", 15.0);
                }
            }
            Phase::TeamRoom => {
                self.countdown -= dt;
                self.team_room_for += dt;
                while self.cues_left > 0 && self.team_room_for >= TEAM_ROOM_CUES[TEAM_ROOM_CUES.len() - self.cues_left] {
                    self.cues_left -= 1;
                    out.push(Out::Event(EV_TEAM_ROOM_CUE, u64::MAX, 0x0fff, FILTER_ALL));
                }
                if self.round_over {
                    self.round_over_clears -= dt;
                    if self.round_over_clears <= 0.0 {
                        self.round_over = false;
                    }
                }
                if self.capture.is_some() && self.round > 1 && !self.capture_reset_sent && self.countdown <= CAPTURE_RESET_BEFORE {
                    self.capture_reset_sent = true;
                    if let Some(c) = self.capture.as_mut() {
                        c.reset();
                    }
                    out.push(Out::Reliable(CAPTURE_RESET, Capture::id(self.level)));
                }
                if self.countdown <= 0.0 && self.capture.is_some() {
                    // Native Dyson: no round start count, the team room opens
                    // onto the round.
                    self.countdown = 0.0;
                    out.extend(self.begin_round());
                } else if self.countdown <= 0.0 {
                    self.phase = Phase::RoundStart;
                    self.to_states(&[ST_BEFORE_ROUND_START, ST_ROUND_START]);
                    self.subphase = Some(SUB_ROUND_START);
                    out.push(self.event(EV_ROUND_START));
                    self.countdown = f(&self.settings, "/round_timers/pre_launch", 45.0).min(99.0);
                }
            }
            Phase::RoundStart => {
                self.countdown -= dt;
                if self.countdown <= 0.0 {
                    self.countdown = 0.0;
                    out.extend(self.begin_round());
                }
            }
            Phase::Playing => {
                if self.state == ST_MAIN_LOOP_ENTER {
                    self.state = ST_PLAYING;
                }
                out.extend(self.play(dt, players));
            }
            Phase::RoundOver => out.extend(self.between_rounds(dt)),
            Phase::Over => out.extend(self.end_sequence(dt)),
        }
        out
    }

    fn begin_round(&mut self) -> Vec<Out> {
        self.phase = Phase::Playing;
        let lock = f(&self.settings, "/capture_point/capture_point_lock_time", 45.0);
        if let Some(c) = self.capture.as_mut() {
            // Locked for its time from the round's begin, nothing sent
            // meanwhile (native's first update: 45.0 s after EV_ROUND_BEGIN,
            // both rounds).
            c.locked = true;
            c.lock_left = lock;
        }
        // Native goes main_loop_enter -> playing in one frame.
        self.state = ST_MAIN_LOOP_ENTER;
        self.subphase = Some(SUB_PLAYING);
        self.team_room = false;
        self.round_over = false;
        vec![self.event(EV_ROUND_BEGIN)]
    }

    /// A capture map's round is won but not the match (native Dyson, from the
    /// 75th point: OVER_C twice at +0, ROUND_OVER_A and the round over states
    /// at +0.5, the next round's event and the team room at +15.5).
    fn end_round(&mut self, w: Winner) -> Vec<Out> {
        tracing::info!(round = self.round, winner = ?w, rounds = ?self.rounds_won, "capture round over");
        self.phase = Phase::RoundOver;
        self.over_for = 0.0;
        self.end_step = 0;
        self.last_round_winner = Some(u8::from(w == Winner::Orange));
        let t = usize::from(w == Winner::Orange);
        let mut v = vec![Out::Lock];
        v.extend(self.announce(MOD_GAME_STATE, &[GS_ROUND_WON[t], GS_NEXT_ROUND[t]]));
        v.extend(self.win_events(w));
        v
    }

    /// A private payload match's first half is over (time up, or the payload
    /// home): its furthest point is the mark to beat; the round over runs as
    /// a capture map's does, then the teams swap.
    fn end_half(&mut self) -> Vec<Out> {
        self.phase = Phase::RoundOver;
        self.over_for = 0.0;
        self.end_step = 0;
        self.in_overtime = false;
        self.overtime = 0.0;
        self.clock = 0.0;
        let Some(p) = self.payload.as_mut() else { return Vec::new() };
        self.to_beat = Some(p.furthest);
        tracing::info!(furthest = p.furthest, "private payload: first half over, teams swap");
        p.state = PAYLOAD_OVER;
        let (_, att, def, dir) = p.sent.unwrap_or((0, 0, 0, 0));
        p.sent = Some((PAYLOAD_OVER, att, def, dir));
        vec![Out::Reliable(PAYLOAD_MISC, misc(p.handle, (PAYLOAD_OVER, att, def, dir)))]
    }

    /// Native's events for who won (server recordings, 2026-11-16): orange
    /// (Dyson's rounds, Fission's payload home) OVER_C twice; blue
    /// (Combustion's rounds) TIME_UP_B twice, after TIME_UP_A when a payload's
    /// time ran out (Gauss).
    fn win_events(&self, w: Winner) -> Vec<Out> {
        if w == Winner::Orange {
            return vec![self.event_alt(EV_OVER_C), self.event_alt(EV_OVER_C)];
        }
        let mut v = Vec::new();
        if self.payload.is_some() {
            v.push(self.event(EV_TIME_UP_A));
        }
        v.extend([self.event_alt(EV_TIME_UP_B), self.event_alt(EV_TIME_UP_B)]);
        v
    }

    fn between_rounds(&mut self, dt: f32) -> Vec<Out> {
        self.over_for += dt;
        let round_over = f(&self.rules, "/round_rules/round_over_time", 15.0);
        let mut out = Vec::new();
        if self.end_step == 0 && self.over_for >= 0.5 {
            self.end_step = 1;
            self.to_states(&ST_MATCH_END[..2]);
            out.push(self.event(EV_ROUND_OVER_A));
            self.countdown = round_over;
            self.round_over = true;
        }
        if self.end_step == 1 && self.over_for >= 0.5 + round_over {
            self.end_step = 0;
            self.round += 1;
            self.capture_reset_sent = false;
            self.early_lock = false;
            self.clock = self.round_secs();
            // Native Dyson: the points go back to 0 in the team room; the
            // volume itself (owner, progress) at its reset, 5 s before the round.
            if let Some(c) = self.capture.as_mut() {
                c.points = [0.0; 2];
            }
            out.extend([self.event(EV_NEXT_ROUND), Out::Unlock, self.event(EV_TEAM_ROOM), Out::RestoreBarriers]);
            if self.payload.is_some() {
                // A private payload match's second half: sides swap, the
                // payload goes back to the start.
                self.half = 2;
                out.push(Out::SwapTeams);
                out.extend(self.reset_payload());
            }
            self.phase = Phase::TeamRoom;
            self.to_states(&ST_TEAM_ROOM);
            // Native: the team room flag at once, the sub-phase with the
            // last team room state, the points reset, the round over flag 4 s on.
            self.team_room = true;
            self.subphase = Some(SUB_TEAM_ROOM);
            self.round_over_clears = ROUND_OVER_CLEARS;
            self.team_room_for = 0.0;
            self.cues_left = TEAM_ROOM_CUES.len();
            self.countdown = f(&self.rules, "/round_rules/team_room_wait_time", 15.0);
            return out;
        }
        self.countdown = (self.countdown - dt).max(0.0);
        out
    }

    fn play(&mut self, dt: f32, players: &[Player]) -> Vec<Out> {
        let mut out = Vec::new();
        let held = self.payload.as_ref().is_some_and(|p| p.hold > 0.0 || p.autodrive_to.is_some());
        if !held {
            self.clock -= dt;
        }
        let mut claimed = false;
        let speed_max = f(&self.settings, "/payload/payload_speed_max_mps", 1.25);
        let multi = |n: u8| match n {
            0 | 1 => f(&self.settings, "/payload/payload_speed_multi_attacker_1", 1.0),
            2 => f(&self.settings, "/payload/payload_speed_multi_attacker_2", 1.2),
            3 => f(&self.settings, "/payload/payload_speed_multi_attacker_3", 1.3),
            _ => f(&self.settings, "/payload/payload_speed_multi_attacker_4", 1.35),
        };
        let celebrate = [f(&self.settings, "/payload/c1_celebration_timer", 15.0), f(&self.settings, "/payload/c2_celebration_timer", 10.0)];
        let added = [f(&self.settings, "/round_timers/checkpoint_1", 120.0), f(&self.settings, "/round_timers/checkpoint_2", 120.0)];
        let mut finished = false;
        let mut reached: Option<usize> = None;
        let mut first_reach: Option<(u16, u16)> = None;
        let mut door_open = false;
        if let Some(p) = self.payload.as_mut() {
            let holders = &self.holders;
            let near = |team: u8| players.iter().filter(|(t, alive, _, slot)| *t == team && *alive && holders.contains(slot)).count() as u8;
            let (att, def) = (near(1), near(0));
            claimed = att > 0;
            let last_cp = p.next.checked_sub(1).map_or(0.0, |i| p.track.checkpoints[i]);
            // Rolled back onto a checkpoint it sits just before it (native
            // reports the trackpoint before the checkpoint's).
            let floor = if last_cp > 0.0 { last_cp - 0.002 } else { 0.0 };
            let was = p.sent.map_or(0, |s| s.3);
            let max = speed_max;
            // The inner speed's target; None: stopped at once.
            let mut auto = false;
            let target: Option<f32> = if let Some(to) = p.autodrive_to {
                auto = true;
                p.auto_for += dt;
                p.hold -= dt;
                p.idle = 0.0;
                p.speed = 0.0;
                p.dist = (p.dist + AUTODRIVE_MPS * dt).min(to);
                if p.auto_for >= AUTODRIVE_STATE_AFTER {
                    p.state = PAYLOAD_AUTODRIVING;
                }
                if p.dist >= to - 1e-4 {
                    // There: the clock runs again (native: at once, the
                    // celebration's 15 s being over by then).
                    p.autodrive_to = None;
                    p.hold = 0.0;
                    p.state = 0;
                    out.push(Out::Reliable(PAYLOAD_AUTODRIVE, autodrive(p.handle, false)));
                }
                None
            } else if p.hold > 0.0 {
                p.hold -= dt;
                p.idle = 0.0;
                if p.hold <= 0.0 {
                    p.state = 0;
                }
                None
            } else if att > 0 && def > 0 {
                p.idle = 0.0;
                None
            } else if att > 0 {
                p.idle = 0.0;
                Some(max * multi(att.min(4)))
            } else {
                p.idle += dt;
                if p.dist <= floor || (p.idle <= ROLLBACK_AFTER && def == 0) {
                    None
                } else {
                    Some(-(REVERSE_BASE + REVERSE_PER_DEFENDER * f32::from(def)) * max)
                }
            };
            match target {
                None => p.speed = 0.0,
                Some(t) if p.speed > t => p.speed = (p.speed - PAYLOAD_ACCEL * dt).max(t),
                Some(t) => p.speed = (p.speed + PAYLOAD_ACCEL * dt).min(t),
            }
            let moving = if p.speed < 0.0 { p.speed * SLOW_REVERSE } else { p.speed };
            if moving > 0.0 {
                // (From the floor it is on the checkpoint again at once.)
                p.dist = (p.dist.max(last_cp) + moving * dt).min(p.track.end());
            } else if moving < 0.0 {
                p.dist += moving * dt;
                if p.dist <= floor {
                    p.dist = floor;
                    p.speed = 0.0;
                    if last_cp > 0.0 {
                        p.state = PAYLOAD_FLOORED;
                    }
                }
            }
            // The direction is the speed's sign (native's own test).
            let dir: i8 = if auto {
                1
            } else if p.speed > 0.0 {
                1
            } else if p.speed < 0.0 {
                -1
            } else {
                0
            };
            // Native Gauss: the floored state went when a push back ended.
            if p.state == PAYLOAD_FLOORED && was == -1 && dir == 0 && p.dist > floor + 0.01 {
                p.state = 0;
            }
            // The HUD: held or driving itself it shows still (native).
            let shown: i8 = if auto || p.hold > 0.0 {
                0
            } else if moving > 0.0 {
                1
            } else if moving < 0.0 {
                -1
            } else {
                0
            };
            p.hud = (att, def, shown);
            p.claimed_once |= att > 0;
            p.furthest = p.furthest.max(p.dist);
            if let Some(d) = p.door.as_mut() {
                let at = GAUSS_DOOR_SECS - GAUSS_DOOR_OPEN_AFTER;
                door_open |= *d > at && *d - dt <= at;
                *d -= dt;
            }
            // Native Fission: `[handle][state u16][attackers u8][defenders u8]
            // [direction i8][0]` (1 attacker + 1 defender: `0101`, stopped).
            let state = (p.state, att, def, dir);
            if p.sent != Some(state) {
                p.sent = Some(state);
                out.push(Out::Reliable(PAYLOAD_MISC, misc(p.handle, state)));
            }
            if !self.early_lock && p.dist >= LOCK_AT_TRACK * p.track.end() {
                self.early_lock = true;
                tracing::info!(dist = p.dist, "payload nearly home: entrants locked");
                out.push(Out::Lock);
            }
            let tp = p.track.trackpoint(p.dist);
            if tp != p.cur_tp {
                p.cur_tp = tp;
                let mut b = p.handle.to_le_bytes().to_vec();
                b.extend_from_slice(&tp.to_le_bytes());
                if tp > p.best_tp {
                    first_reach = Some((p.best_tp, tp));
                    p.best_tp = tp;
                    out.push(Out::Reliable(PAYLOAD_TRACKPOINT, b.clone()));
                }
                out.push(Out::Reliable(PAYLOAD_POS, b));
            }
            if p.next < p.track.checkpoints.len() && p.dist >= p.track.checkpoints[p.next] - 0.01 {
                if p.next + 1 == p.track.checkpoints.len() {
                    finished = true;
                } else {
                    // In overtime the clock is 0: native Gauss's went to 120.00.
                    self.clock = self.clock.max(0.0) + added[p.next.min(1)];
                    self.in_overtime = false;
                    p.hold = celebrate[p.next.min(1)];
                    let cp = p.track.checkpoints[p.next];
                    let upto = p.track.checkpoints.get(p.next + 1).copied().unwrap_or(f32::MAX);
                    if let Some(&to) = p.track.autodrive.iter().find(|&&d| d > cp && d < upto) {
                        p.autodrive_to = Some(to);
                        p.auto_for = 0.0;
                        out.push(Out::Reliable(PAYLOAD_AUTODRIVE, autodrive(p.handle, true)));
                    } else {
                        p.state = PAYLOAD_CELEBRATING;
                    }
                    tracing::info!(checkpoint = p.next + 1, clock = self.clock, "payload checkpoint");
                    reached = Some(p.next);
                }
                p.next += 1;
            }
        }
        if let Some((from, to)) = first_reach {
            self.reach_track_vars(from, to);
        }
        if let Some(i) = reached {
            out.extend(self.announce(MOD_GAME_STATE, &GS_CHECKPOINT[i.min(1)..=i.min(1)]));
            if self.level == LEVEL_GAUSS && i == 0 {
                out.extend(self.announce(MOD_GAUSS_CHECKPOINT, &GAUSS_CHECKPOINT));
                if let Some(p) = self.payload.as_mut() {
                    p.door = Some(GAUSS_DOOR_SECS);
                }
            }
        }
        if door_open {
            out.extend(self.announce(MOD_GAUSS_CHECKPOINT, &[GAUSS_DOOR_OPEN]));
        }
        let mut won = finished.then_some(Winner::Orange);
        // A private match's second half: the new attackers (orange now) win
        // the moment the payload is as far as the first attackers got it.
        if let (Some(to_beat), Some(p)) = (self.to_beat, self.payload.as_ref())
            && p.dist >= to_beat - 0.01
            && p.dist > 0.0
        {
            won = Some(Winner::Orange);
        }
        if finished && self.private && self.half == 1 {
            return [out, self.end_half()].concat();
        }
        if self.capture.is_some() {
            let (o, w) = self.run_capture(dt, players);
            out.extend(o);
            if let Some(w) = w {
                let t = usize::from(w == Winner::Orange);
                self.rounds_won[t] += 1;
                if self.rounds_won[t] < CAPTURE_ROUNDS_TO_WIN {
                    out.extend(self.end_round(w));
                    return out;
                }
            }
            won = won.or(w);
        }
        if let Some(w) = won {
            out.extend(self.finish(w));
        } else if self.clock <= 0.0 {
            self.clock = 0.0;
            let grace = f(&self.settings, "/round_timers/overtime", 0.0);
            if claimed && grace > 0.0 {
                self.overtime = grace;
                if !self.in_overtime {
                    self.in_overtime = true;
                    tracing::info!("payload overtime");
                    out.extend([self.event(EV_OVERTIME_A), self.event(EV_OVERTIME_B)]);
                }
            } else if self.overtime > 0.0 {
                self.overtime -= dt;
            }
            if self.overtime <= 0.0 && self.private && self.payload.is_some() && self.half == 1 {
                out.extend(self.end_half());
            } else if self.overtime <= 0.0 {
                let w = match &self.capture {
                    _ if self.payload.is_some() => Winner::Blue,
                    Some(c) if c.points[0] > c.points[1] => Winner::Blue,
                    Some(c) if c.points[1] > c.points[0] => Winner::Orange,
                    _ => Winner::Draw,
                };
                out.extend(self.finish(w));
            }
        }
        out
    }

    /// The capture point: claims, conversion, points (see the module notes).
    fn run_capture(&mut self, dt: f32, players: &[Player]) -> (Vec<Out>, Option<Winner>) {
        let to_capture = f(&self.settings, "/capture_point/time_to_capture", 4.0).max(0.1);
        let to_win = f(&self.settings, "/capture_point/points_to_win", 75.0);
        let unclaimed = f(&self.settings, "/capture_point/unclaimed_accrual_multiplier", 0.5);
        let needs_claim = self.settings.pointer("/capture_point/point_accrual_requires_claim").and_then(Value::as_bool).unwrap_or(false);
        let space = self.level;
        let deciding = self.rounds_won.map(|r| r + 1 >= CAPTURE_ROUNDS_TO_WIN);
        let team_slots = &self.team_slots;
        let Some(c) = self.capture.as_mut() else { return (Vec::new(), None) };
        let (mut claims, mut counts) = ([0u16; 2], [0u16; 2]);
        for &(team, alive, pos, slot) in players {
            let t = usize::from(team.min(1));
            if alive && pos.is_some_and(|p| c.inside(p)) {
                // Native: bit n is the team's player n (Dyson: blue 1, 2 and
                // 3, orange 1 alongside), so each game sees itself on it.
                let bit = team_slots.iter().find(|(s, _)| *s == slot).map_or(slot, |(_, t)| *t);
                claims[t] |= 1 << (bit & 15);
                counts[t] += 1;
            }
        }
        c.claims = claims;
        c.counts = counts;
        c.contested = counts[0] > 0 && counts[1] > 0;
        let alone = |t: usize| counts[t] > 0 && counts[1 - t] == 0;
        if c.locked {
            c.lock_left -= dt;
            if c.lock_left <= 0.0 {
                c.locked = false;
                c.unlocked_once = true;
                tracing::info!("capture point unlocked");
            }
        }
        // Native's first update after the lock already has the converter.
        if !c.locked {
            if c.converting == NOBODY {
                if c.possession != 0 && alone(0) {
                    c.converting = 0;
                } else if c.possession != 1 && alone(1) {
                    c.converting = 1;
                }
            } else {
                let t = usize::from(c.converting);
                c.progress[t] += if alone(t) { dt } else { -dt };
                if c.progress[t] <= 0.0 {
                    c.progress[t] = 0.0;
                    c.converting = NOBODY;
                } else if c.progress[t] >= to_capture {
                    c.progress[t] = 0.0;
                    c.possession = t as u16;
                    c.converting = NOBODY;
                    tracing::info!(team = t, "capture point taken");
                }
            }
            // Native Dyson (both rounds, to the frame): the holder earns
            // nothing while the other team is on the point at all.
            if c.possession < NEUTRAL && counts[1 - usize::from(c.possession)] == 0 {
                let t = usize::from(c.possession);
                let rate = if counts[t] > 0 {
                    1.0
                } else if needs_claim {
                    0.0
                } else {
                    unclaimed
                };
                c.points[t] = (c.points[t] + rate * dt).min(to_win);
                self.points_shown = true;
            }
        }
        let mut out = Vec::new();
        if !self.early_lock && (0..2).any(|t| deciding[t] && c.points[t] >= LOCK_AT_POINTS * to_win) {
            self.early_lock = true;
            tracing::info!(points = ?c.points, "capture match nearly decided: entrants locked");
            out.push(Out::Lock);
        }
        if c.locked {
            // Native sends nothing about a locked point.
            return (out, None);
        }
        if c.contested {
            out.push(Out::Event(EV_CAPTURE_CONTESTED, OBJECT_MODULE, c.handle, FILTER_ALL));
        }
        c.since_send += dt;
        let state = (c.possession, c.converting, c.claims, c.counts);
        if c.sent != Some(state) || (c.converting != NOBODY && c.since_send >= CAPTURE_SEND_EVERY) {
            c.sent = Some(state);
            c.since_send = 0.0;
            out.push(Out::Reliable(CAPTURE_UPDATE, c.update(space)));
        }
        let won = if c.points[0] >= to_win {
            Some(Winner::Blue)
        } else if c.points[1] >= to_win {
            Some(Winner::Orange)
        } else {
            None
        };
        (out, won)
    }

    fn finish(&mut self, w: Winner) -> Vec<Out> {
        tracing::info!(level = %format!("{:016x}", self.level), winner = ?w, "combat match over");
        self.phase = Phase::Over;
        self.winner = Some(w);
        self.over_for = 0.0;
        self.end_step = 0;
        self.state = ST_MATCH_END[0];
        if self.capture.is_some() && w != Winner::Draw {
            self.last_round_winner = Some(u8::from(w == Winner::Orange));
        }
        let mut v = vec![Out::Lock];
        if w != Winner::Draw {
            let t = usize::from(w == Winner::Orange);
            let syms: Vec<u64> = if self.capture.is_some() { vec![GS_ROUND_WON[t], GS_MATCH_WON[t]] } else { vec![GS_MATCH_WON[t], GS_PAYLOAD_OVER] };
            v.extend(self.announce(MOD_GAME_STATE, &syms));
        }
        v.extend(self.win_events(w));
        // Native (Fission home, Gauss time up): the payload's state goes to 8.
        if let Some(p) = self.payload.as_mut() {
            p.state = PAYLOAD_OVER;
            let (_, att, def, dir) = p.sent.unwrap_or((0, 0, 0, 0));
            p.sent = Some((PAYLOAD_OVER, att, def, dir));
            v.push(Out::Reliable(PAYLOAD_MISC, misc(p.handle, (PAYLOAD_OVER, att, def, dir))));
        }
        v
    }

    /// Native, after the payload's end: events at +0, +0.8 (round over, the
    /// top awards), +15.8 (celebration), +25.8 (personal stats), +40.8
    /// (return to lobby).
    fn end_sequence(&mut self, dt: f32) -> Vec<Out> {
        self.over_for += dt;
        let round_over = f(&self.rules, "/round_rules/round_over_time", 15.0);
        let celebration = f(&self.rules, "/round_rules/celebration_time", 10.0);
        let stats = f(&self.rules, "/round_rules/celebration_personal_stats_time", 15.0);
        let steps = [0.8, 0.8 + round_over, 0.8 + round_over + celebration, 0.8 + round_over + celebration + stats];
        let mut out = Vec::new();
        while self.end_step < steps.len() && self.over_for >= steps[self.end_step] {
            match self.end_step {
                0 => {
                    self.state = ST_MATCH_END[1];
                    self.round_over = true;
                    out.push(self.event(EV_ROUND_OVER_A));
                    out.push(self.event(EV_ROUND_OVER_B));
                    out.push(Out::Awards);
                    self.countdown = round_over;
                }
                1 => {
                    // Native: enter then update, a frame apart.
                    self.to_states(&ST_MATCH_END[2..4]);
                    self.celebrating = true;
                    out.push(self.event(EV_CELEBRATION));
                    self.countdown = celebration;
                }
                2 => {
                    self.to_states(&ST_MATCH_END[4..6]);
                    out.push(self.event(EV_PERSONAL_STATS));
                    self.countdown = stats;
                }
                _ if self.private => {
                    // Native: a private match stays on the personal stats
                    // until a rematch or the lobby button.
                }
                _ => {
                    self.state = ST_MATCH_END[6];
                    self.post_match = false;
                    out.push(self.event(EV_RETURN_TO_LOBBY));
                }
            }
            self.end_step += 1;
        }
        if self.end_step == 2 && self.over_for >= steps[1] + POST_MATCH_AFTER {
            self.post_match = true;
        }
        self.countdown = (self.countdown - dt).max(0.0);
        out
    }

    #[cfg(test)]
    fn capture_points(&self) -> [f32; 2] {
        self.capture.as_ref().map_or([0.0; 2], |c| c.points)
    }

    #[cfg(test)]
    pub fn payload_dist(&self) -> Option<f32> {
        self.payload.as_ref().map(|p| p.dist)
    }

    /// The match area's object set with the payload where it is now, stamped
    /// `now_us` (the server frame's time), for the physics mailbox.
    pub fn payload_physics(&self, now_us: u64) -> Option<Vec<u8>> {
        let p = self.payload.as_ref()?;
        let (template, index) = match self.level {
            LEVEL_GAUSS => (PAYLOAD_PHYS_GAUSS, 0),
            LEVEL_FISSION => (PAYLOAD_PHYS_FISSION, 3),
            _ => return None,
        };
        let mut d: Vec<u8> = (0..template.len() / 2).map(|i| u8::from_str_radix(&template[2 * i..2 * i + 2], 16).unwrap_or(0)).collect();
        d[8..16].copy_from_slice(&(now_us & 0xffff_ffff).to_le_bytes());
        let at = p.track.pos(p.dist);
        let (a, b) = (p.track.pos((p.dist - 0.5).max(0.0)), p.track.pos(p.dist + 0.5));
        let f = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let r = PHYS_HEAD_BITS + index * PHYS_RECORD_BITS;
        let pos = [at[0], at[1] + PAYLOAD_RIDE, at[2]];
        for (k, x) in pos.iter().enumerate() {
            phys_set(&mut d, r + PHYS_POS + 19 * k, 19, (x * 1000.0 + 250_000.0).round().clamp(0.0, 524_287.0) as u32);
        }
        phys_set(&mut d, r + PHYS_ROT, 32, rotation_bits(look_along(f)));
        // Its speed along the track (rolling back: x SLOW_REVERSE), 0 held.
        let moving = p.hud.2 != 0 || p.autodrive_to.is_some();
        let v = if !moving || p.hold > 0.0 { 0.0 } else if p.speed < 0.0 { p.speed * SLOW_REVERSE } else { p.speed };
        let l = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt().max(1e-6);
        for k in 0..3 {
            phys_set(&mut d, r + PHYS_VEL + 12 * k, 12, (f[k] / l * v * 32.0 + 1536.0).round().clamp(0.0, 4095.0) as u32);
        }
        Some(d)
    }
}

// ── The payload's physics (native Gauss and Fission recordings, 2026-11-16) ──
// The native server sends the match area's object set (owner ffffffff, set id
// = the area, 4 records) with each other server frame, its time = that frame's,
// mailbox key 0x2824 + (frame % 90) x 0x11. Only the payload's record ever
// changes (Gauss record 0, Fission record 3): position 0.651 m above the track,
// its own +Z along the track and +Y up, velocity along it. Without it the
// clients' payload never moved, whatever the trackpoints said.
const PAYLOAD_PHYS_GAUSS: &str = "ffffffff193a620cce0cd104000000004001000000000000004903000008000000f0e618b8f4d247d87fff510c600006609020416ae518c9f3fe4808103500008000000000d90b42ab6efc75ff47f17f000660000609122448e841420f127a005903000008000000a8b8a445f2c25ed87fff510c6000066090204182841e24f420a1071036000080000000800adac23b0f867ffdf7dfff000660000609122448e841420f127a0000";
const PAYLOAD_PHYS_FISSION: &str = "ffffffffd483a3dff8b1d20400000000400100000000000000490300000800000070c19d98fdf4a0d77fff01006000066090204182841e24f420a1071035000080000000806f3e8a29af127afdf71f00000660000609122448e841420f127a00590300000800000008c9aa9df716a1d77fff01006000066090204182841e24f420a1071036000080000000802e7489048f1d78fd07e0ff00066000060912a406745dfa4e1e780000";
const PHYS_HEAD_BITS: usize = 192;
const PHYS_RECORD_BITS: usize = 284;
const PHYS_POS: usize = 75;
const PHYS_ROT: usize = 132;
const PHYS_VEL: usize = 164;
/// The payload stands this far above its track's points.
const PAYLOAD_RIDE: f32 = 0.651;
pub const PAYLOAD_PHYS_KEY: u64 = 0x2824;

fn phys_set(d: &mut [u8], at: usize, n: usize, v: u32) {
    for k in 0..n {
        let i = at + k;
        if v >> k & 1 == 1 {
            d[i >> 3] |= 1 << (i & 7);
        } else {
            d[i >> 3] &= !(1 << (i & 7));
        }
    }
}

/// A rotation (x, y, z, w) whose +Z is `f` and +Y as near world up as it can be.
fn look_along(f: [f32; 3]) -> [f32; 4] {
    let norm = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let z = norm(f);
    let x = norm(cross([0.0, 1.0, 0.0], z));
    let y = cross(z, x);
    // Columns x, y, z of the rotation matrix to a quaternion.
    let (m00, m11, m22) = (x[0], y[1], z[2]);
    let tr = m00 + m11 + m22;
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [(y[2] - z[1]) / s, (z[0] - x[2]) / s, (x[1] - y[0]) / s, 0.25 * s]
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        [0.25 * s, (y[0] + x[1]) / s, (z[0] + x[2]) / s, (y[2] - z[1]) / s]
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        [(y[0] + x[1]) / s, 0.25 * s, (z[1] + y[2]) / s, (z[0] - x[2]) / s]
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        [(z[0] + x[2]) / s, (z[1] + y[2]) / s, 0.25 * s, (x[1] - y[0]) / s]
    };
    q
}

/// A rotation in a physics record: the index of the largest component (left
/// out, made positive), then the other three, 10 bits each over +-1/sqrt 2.
fn rotation_bits(q: [f32; 4]) -> u32 {
    let largest = (0..4).max_by(|a, b| q[*a].abs().total_cmp(&q[*b].abs())).unwrap_or(3);
    let sign = if q[largest] < 0.0 { -1.0 } else { 1.0 };
    let mut raw = largest as u32;
    for (i, c) in (0..4).filter(|i| *i != largest).enumerate() {
        let v = ((q[c] * sign * std::f32::consts::SQRT_2 + 1.0) / 2.0 * 1023.0).round().clamp(0.0, 1023.0) as u32;
        raw |= v << (2 + 10 * i);
    }
    raw
}

/// `UpdateMiscState`: `[handle][state u16][attackers u8][defenders u8][direction i8][0]`.
fn misc(handle: u16, (state, att, def, dir): (u16, u8, u8, i8)) -> Vec<u8> {
    let mut b = handle.to_le_bytes().to_vec();
    b.extend_from_slice(&state.to_le_bytes());
    b.extend_from_slice(&[att, def, dir as u8, 0]);
    b
}

/// `UpdatedAutodrive`: `[handle][on u16][speed f32]` (native: 0.15 both ways).
fn autodrive(handle: u16, on: bool) -> Vec<u8> {
    let mut b = handle.to_le_bytes().to_vec();
    b.extend_from_slice(&u16::from(on).to_le_bytes());
    b.extend_from_slice(&AUTODRIVE_MPS.to_le_bytes());
    b
}

fn upsert(set: &mut Vec<(u64, Val)>, var: u64, v: Val) {
    match set.iter_mut().find(|e| e.0 == var) {
        Some(e) => e.1 = v,
        None => set.push((var, v)),
    }
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gauss() -> Combat {
        let settings: Value = serde_json::from_str(
            r#"{"payload":{"c1_celebration_timer":35.0,"c2_celebration_timer":20.0,"payload_speed_max_mps":1.25,
            "payload_speed_multi_attacker_2":1.2,"s02_payload_speed_multiplier":0.75},
            "round_timers":{"pre_launch":45.0,"start":180.0,"checkpoint_1":120.0,"checkpoint_2":120.0,"overtime":4.0}}"#,
        )
        .expect("json");
        let rules: Value = serde_json::from_str(r#"{"round_rules":{"pre_match_wait_time":30.0}}"#).expect("json");
        Combat::new(LEVEL_GAUSS, 352, settings, rules).expect("combat level")
    }

    fn fission() -> Combat {
        let settings: Value = serde_json::from_str(
            r#"{"payload":{"c1_celebration_timer":15.0,"c2_celebration_timer":10.0,"payload_speed_max_mps":1.25},
            "round_timers":{"pre_launch":45.0,"start":180.0,"checkpoint_1":120.0,"checkpoint_2":120.0,"overtime":4.0}}"#,
        )
        .expect("json");
        Combat::new(LEVEL_FISSION, 229, settings, Value::Null).expect("fission")
    }

    fn playing(mut c: Combat) -> Combat {
        c.phase = Phase::Playing;
        c.state = ST_PLAYING;
        c
    }

    /// A player at the payload with a hand on it.
    fn on_payload(c: &mut Combat, team: u8) -> Player {
        c.holders = vec![0];
        let p = c.payload.as_ref().expect("payload");
        (team, true, Some(p.track.pos(p.dist)), 0)
    }

    /// Native Fission: checkpoint 1 (56.52 m) starts an autodrive at 0.15 m/s
    /// to the 0x20 point (59.31 m), the clock held, state 2 from 5 s in.
    #[test]
    fn fission_autodrives_from_its_first_checkpoint() {
        let mut c = playing(fission());
        c.payload.as_mut().expect("p").dist = 56.4;
        let mut v = Vec::new();
        for _ in 0..20 {
            let me = on_payload(&mut c, 1);
            v.extend(c.step(0.1, &[me]));
        }
        assert!(v.contains(&Out::Reliable(PAYLOAD_AUTODRIVE, autodrive(0x60, true))), "{v:?}");
        let clock = c.clock;
        let nobody = [(1u8, true, Some([500.0, 0.0, 0.0]), 0)];
        let v = run(&mut c, 6.0, &nobody);
        assert!((c.clock - clock).abs() < 0.01, "held");
        assert!(v.iter().any(|o| matches!(o, Out::Reliable(PAYLOAD_MISC, b) if b[2] == 2 && b[6] == 1)), "{v:?}");
        let v = run(&mut c, 14.0, &nobody);
        assert!(v.contains(&Out::Reliable(PAYLOAD_AUTODRIVE, autodrive(0x60, false))), "{v:?}");
        assert!((c.payload_dist().expect("d") - 59.305).abs() < 0.01);
        run(&mut c, 1.0, &nobody);
        assert!(c.clock < clock - 0.5, "the clock runs again");
    }

    /// Native Gauss: the clock hits 0 with attackers on it: overtime events
    /// once; a checkpoint then sets the clock to 120.
    #[test]
    fn overtime_events_and_a_checkpoint_in_overtime() {
        let mut c = playing(gauss());
        c.clock = 0.05;
        c.payload.as_mut().expect("p").dist = 76.0;
        let mut v = Vec::new();
        for _ in 0..30 {
            let me = on_payload(&mut c, 1);
            v.extend(c.step(0.1, &[me]));
        }
        let ot = |v: &[Out]| v.iter().filter(|o| matches!(o, Out::Event(EV_OVERTIME_A | EV_OVERTIME_B, ..))).count();
        assert_eq!(ot(&v), 2, "{v:?}");
        assert!(c.winner.is_none());
        // 76 m + 1.25 m/s: checkpoint 1 (77.8 m) within overtime.
        assert!((c.clock - 120.0).abs() < 0.5, "{}", c.clock);
        assert_eq!(c.payload.as_ref().expect("p").state, PAYLOAD_CELEBRATING);
    }

    /// Native Fission: entrants lock at 90% of the track, before the end;
    /// the end sets the payload's state to 8.
    #[test]
    fn nearly_home_locks_entrants_then_the_end_is_state_8() {
        let mut c = playing(fission());
        {
            let p = c.payload.as_mut().expect("p");
            p.dist = 160.0;
            p.next = 2;
        }
        let mut v = Vec::new();
        let mut locked_at = None;
        for _ in 0..400 {
            let me = on_payload(&mut c, 1);
            let o = c.step(0.1, &[me]);
            if locked_at.is_none() && o.contains(&Out::Lock) {
                locked_at = c.payload_dist();
            }
            v.extend(o);
            if c.phase == Phase::Over {
                break;
            }
        }
        let at = locked_at.expect("locked");
        assert!((at - 0.9 * 181.81).abs() < 0.2, "{at}");
        assert_eq!(c.winner, Some(Winner::Orange));
        assert!(v.iter().any(|o| matches!(o, Out::Reliable(PAYLOAD_MISC, b) if b[2] == 8)));
    }

    /// Native Combustion: the deciding round locks entrants at 75% of the
    /// points; the first round does not.
    #[test]
    fn a_deciding_capture_round_locks_entrants_at_75_percent() {
        let mut c = playing(capture_map(LEVEL_DYSON));
        let orange = [(1u8, true, Some([0.0, 0.0, 0.0]), 3)];
        c.capture.as_mut().expect("c").locked = false;
        c.capture.as_mut().expect("c").possession = 1;
        c.capture.as_mut().expect("c").points = [0.0, 56.0];
        let v = run(&mut c, 1.0, &orange);
        assert!(!v.contains(&Out::Lock), "first round");
        c.rounds_won = [0, 1];
        c.capture.as_mut().expect("c").points = [0.0, 56.0];
        let v = run(&mut c, 1.0, &orange);
        assert!(v.contains(&Out::Lock));
        assert!(c.winner.is_none());
    }

    fn run(c: &mut Combat, secs: f32, players: &[Player]) -> Vec<Out> {
        let mut v = Vec::new();
        for _ in 0..(secs * 10.0) as usize {
            // In these tests a player at the payload has a hand on it.
            if let Some(at) = c.payload.as_ref().map(|p| p.track.pos(p.dist)) {
                c.holders = players.iter().filter(|(_, _, pos, _)| pos.is_some_and(|q| dist(q, at) <= CLAIM_RADIUS)).map(|p| p.3).collect();
            }
            v.extend(c.step(0.1, players));
        }
        v
    }

    #[test]
    fn tracks_come_from_the_levels() {
        let g = Track::gauss();
        assert_eq!(g.points.len(), 181);
        assert_eq!(g.points[0].0, 27);
        assert_eq!(g.points[180].0, 207);
        assert!((g.checkpoints[0] - 77.8).abs() < 0.5 && (g.checkpoints[1] - 126.0).abs() < 0.5, "{:?}", g.checkpoints);
        assert!((g.end() - 206.8).abs() < 0.1);
        let f = Track::of(LEVEL_FISSION).expect("fission");
        assert_eq!(f.points.len(), 69);
        assert_eq!(f.checkpoints.len(), 3);
        assert!((f.end() - 181.8).abs() < 0.1);
        let c = Combat::new(LEVEL_FISSION, 229, Value::Null, Value::Null).expect("fission");
        assert_eq!(c.payload.as_ref().map(|p| p.handle), Some(0x60));
        assert_eq!(gauss().payload.as_ref().map(|p| p.handle), Some(0xd0));
    }

    fn capture_map(level: u64) -> Combat {
        let settings: Value = serde_json::from_str(
            r#"{"capture_point":{"time_to_capture":4.0,"capture_point_lock_time":45.0,"point_accrual_requires_claim":false,
            "points_to_win":75.0,"unclaimed_accrual_multiplier":0.5},
            "round_timers":{"pre_launch":99.0,"start":4000.0,"overtime":0.0}}"#,
        )
        .expect("json");
        let rules: Value = serde_json::from_str(r#"{"round_rules":{"pre_match_wait_time":30.0}}"#).expect("json");
        Combat::new(level, 173, settings, rules).expect("combat level")
    }

    #[test]
    fn capture_shapes_come_from_the_levels() {
        let d = Capture::of(LEVEL_DYSON).expect("dyson");
        assert_eq!(d.shapes.len(), 1);
        assert!(d.inside([0.0, 1.0, 0.0]) && d.inside([4.0, 0.0, 3.0]) && !d.inside([6.0, 0.0, 2.0]));
        let c = Capture::of(LEVEL_COMBUSTION).expect("combustion");
        assert_eq!(c.shapes.len(), 7);
        assert!(c.inside([8.4, 2.3, 0.0]) && !c.inside([8.4, 6.0, 0.0]) && !c.inside([30.0, 2.3, 0.0]));
        assert!(Capture::of(LEVEL_GAUSS).is_none() && Capture::of(LEVEL_FISSION).is_none());
        // A box turned 90 degrees about y: its x half extent lies along z.
        let b = Shape::Box([0.0; 3], [0.0, std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2], [1.0, 1.0, 3.0]);
        assert!(b.contains([2.5, 0.0, 0.0]) && !b.contains([0.0, 0.0, 2.5]));
    }

    fn updates(v: &[Out]) -> Vec<Vec<u8>> {
        v.iter().filter_map(|o| if let Out::Reliable(CAPTURE_UPDATE, b) = o { Some(b.clone()) } else { None }).collect()
    }

    fn contested(v: &[Out]) -> usize {
        v.iter().filter(|o| matches!(o, Out::Event(EV_CAPTURE_CONTESTED, OBJECT_MODULE, 0xae, FILTER_ALL))).count()
    }

    /// Native Dyson (netrec-dyson-20261116-server): no round start, a locked
    /// point says nothing, 4 s to take it, 1 point a second on it alone, 0.5
    /// empty, none with an enemy on it, 75 wins the round, two rounds.
    /// The claimer bits are by team slot, as native's (Dyson: blue 1, 2, 3
    /// and orange 1 at once): players in slots 3 and 4, each their team's
    /// first, are both bit 0, so each one's game sees itself on the point.
    #[test]
    fn capture_claims_are_by_team_slot() {
        let mut c = capture_map(LEVEL_DYSON);
        c.set_team_slots(vec![(3, 0), (4, 0), (6, 1)]);
        let orange = (1u8, true, Some([0.0, 0.0, 0.0]), 3);
        let blue = (0u8, true, Some([1.0, 0.0, 0.0]), 4);
        let blue2 = (0u8, true, Some([1.0, 0.0, 0.0]), 6);
        run(&mut c, 30.5, &[orange]);
        run(&mut c, 15.0, &[orange]);
        run(&mut c, 44.5, &[orange]);
        let v = run(&mut c, 1.0, &[orange, blue, blue2]);
        let u = updates(&v).into_iter().last().expect("update");
        let claims = |t: usize| u16::from_le_bytes([u[24 + 8 * t], u[25 + 8 * t]]);
        assert_eq!((claims(0), claims(1)), (0b11, 0b01));
    }

    #[test]
    fn dyson_rounds_follow_the_native_match() {
        let mut c = capture_map(LEVEL_DYSON);
        let orange = (1u8, true, Some([0.0, 0.0, 0.0]), 3);
        let blue = (0u8, true, Some([1.0, 0.0, 0.0]), 4);
        let away = (0u8, true, Some([50.0, 0.0, 0.0]), 4);
        run(&mut c, 30.5, &[orange]);
        assert_eq!(c.phase, Phase::TeamRoom);
        let v = run(&mut c, 15.0, &[orange]);
        assert_eq!(c.phase, Phase::Playing, "the team room opens onto the round");
        assert!(v.contains(&Out::Event(EV_ROUND_BEGIN, MOD_COMBAT_MATCH, c.match_handle, FILTER_ALL)));
        // Locked for 45 s: nothing sent, contested or not.
        let v = run(&mut c, 44.5, &[orange, blue]);
        assert!(updates(&v).is_empty() && contested(&v) == 0);
        // Unlocked: orange alone converts from neutral.
        let v = run(&mut c, 1.0, &[orange, away]);
        let first = updates(&v).into_iter().next().expect("update");
        assert_eq!(first.len(), 40);
        assert_eq!(&first[..16], &unhex8(LEVEL_DYSON, &[0, 0, 0, 0, 1, 0, 0, 0])[..]);
        // Neutral, no flags, orange converting; orange's claim: slot 3, one player.
        assert_eq!(&first[16..24], &[2, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(&first[32..36], &[8, 0, 1, 0]);
        run(&mut c, 4.0, &[orange, away]);
        assert_eq!(c.capture.as_ref().expect("c").possession, 1);
        // Both on it: contested, the event every step, no points.
        let before = c.capture_points()[1];
        let v = run(&mut c, 5.0, &[orange, blue]);
        assert_eq!(contested(&v), 50);
        assert!((c.capture_points()[1] - before).abs() < 0.01);
        // Blue alone converting: still nothing for orange.
        run(&mut c, 2.0, &[(1, true, Some([50.0, 0.0, 0.0]), 3), blue]);
        assert!((c.capture_points()[1] - before).abs() < 0.01);
        // Nobody on it: half rate.
        run(&mut c, 10.0, &[(1, true, Some([50.0, 0.0, 0.0]), 3), away]);
        assert!((c.capture_points()[1] - before - 5.0).abs() < 0.2, "{}", c.capture_points()[1]);
        // On it alone to 75: the round, not the match.
        let v = run(&mut c, 75.0, &[orange, away]);
        assert_eq!(c.phase, Phase::RoundOver);
        assert!(v.contains(&Out::Lock) && v.contains(&Out::Event(EV_OVER_C, MOD_COMBAT_MATCH, c.match_handle, FILTER_ALL_ALT)));
        // Native's game state actor (handle 0xaf): orange won the round, more follow.
        assert!(v.contains(&Out::Event(GS_ROUND_WON[1], MOD_GAME_STATE, 0xaf, FILTER_ALL)));
        assert!(v.contains(&Out::Event(GS_NEXT_ROUND[1], MOD_GAME_STATE, 0xaf, FILTER_ALL)));
        assert_eq!(c.winner, None);
        let v: Vec<Out> = v.into_iter().chain(run(&mut c, 16.0, &[orange, away])).collect();
        assert_eq!(c.phase, Phase::TeamRoom);
        assert!(v.contains(&Out::Unlock) && v.contains(&Out::RestoreBarriers));
        assert!(v.contains(&Out::Event(EV_ROUND_OVER_A, MOD_COMBAT_MATCH, c.match_handle, FILTER_ALL)));
        assert!(v.contains(&Out::Event(EV_NEXT_ROUND, MOD_COMBAT_MATCH, c.match_handle, FILTER_ALL)));
        let cue = Out::Event(EV_TEAM_ROOM_CUE, u64::MAX, 0x0fff, FILTER_ALL);
        assert_eq!(v.iter().filter(|o| **o == cue).count(), 2, "the team room's two cues");
        // Native: the points go in the team room, the owner with the reset.
        assert_eq!(c.capture.as_ref().expect("c").possession, 1);
        assert_eq!(c.capture_points(), [0.0; 2]);
        let vars = c.vars();
        // (The round ended mid-run: over 4 s into the team room, its round over flag is down.)
        assert!(vars.contains(&(VAR_TEAM_ROOM, Val::Lo8(1))) && vars.contains(&(VAR_ROUND_OVER, Val::Lo32(0))));
        assert!(vars.contains(&(VAR_ROUNDS_WON[1], Val::Lo32(1))) && vars.contains(&(VAR_LAST_ROUND_WINNER, Val::Lo32(1))));
        assert!(vars.contains(&(VAR_ROUND_COUNTER, Val::Lo32(2))) && vars.contains(&(VAR_CAP_OWNER, Val::Raw(CAP_TEAM[1]))));
        // Reset 5 s before the round, then round two.
        let v = run(&mut c, 15.0, &[orange, away]);
        assert!(v.contains(&Out::Reliable(CAPTURE_RESET, unhex8(LEVEL_DYSON, &[0, 0, 0, 0, 1, 0, 0, 0]))));
        assert_eq!(c.capture.as_ref().expect("c").possession, NEUTRAL);
        assert_eq!(c.phase, Phase::Playing);
        assert!(c.vars().contains(&(VAR_CAP_OWNER, Val::Raw(CAP_LOCKED))));
        let v = run(&mut c, 45.0 + 4.5 + 76.0, &[orange, away]);
        assert!(v.contains(&Out::Event(GS_MATCH_WON[1], MOD_GAME_STATE, 0xaf, FILTER_ALL)));
        assert_eq!(c.winner, Some(Winner::Orange));
        assert_eq!(c.phase, Phase::Over);
        assert!(c.vars().contains(&(VAR_RED_SCORE, Val::Lo32(1))));
    }

    fn var(c: &Combat, h: u64) -> Option<Val> {
        c.vars().into_iter().find(|e| e.0 == h).map(|e| e.1)
    }

    /// Native Dyson's capture vars through a round's first minute: locked
    /// with the lock count, then neutral, blue alone converting, both on it,
    /// orange owning and scoring.
    #[test]
    fn capture_vars_follow_the_native_point() {
        let mut c = playing(capture_map(LEVEL_DYSON));
        c.begin_round();
        let orange = (1u8, true, Some([0.0, 0.0, 0.0]), 3);
        let blue = (0u8, true, Some([1.0, 0.0, 0.0]), 4);
        let away = (0u8, true, Some([50.0, 0.0, 0.0]), 4);
        assert_eq!(var(&c, VAR_SUBPHASE), Some(Val::Raw(SUB_PLAYING)));
        assert_eq!(var(&c, VAR_TEAM_ROOM), Some(Val::Lo8(0)));
        run(&mut c, 10.0, &[orange, blue]);
        assert_eq!(var(&c, VAR_CAP_OWNER), Some(Val::Raw(CAP_LOCKED)));
        assert_eq!(var(&c, VAR_CAP_ON), Some(Val::Raw(CAP_LOCKED)));
        assert_eq!(var(&c, VAR_CAP_COUNT[0]), Some(Val::Lo32(0)), "nothing while locked");
        assert!(matches!(var(&c, VAR_CAP_LOCK), Some(Val::F32(x)) if (x - 35.0).abs() < 0.2));
        run(&mut c, 35.5, &[blue]);
        assert_eq!(var(&c, VAR_CAP_OWNER), Some(Val::Raw(CAP_NEUTRAL)));
        assert_eq!(var(&c, VAR_CAP_ON), Some(Val::Raw(CAP_TEAM[0])));
        assert_eq!(var(&c, VAR_CAP_COUNT[0]), Some(Val::Lo32(1)));
        assert!(matches!(var(&c, VAR_CAP_PROGRESS[0]), Some(Val::F32(x)) if x > 0.05 && x < 0.2));
        let v = run(&mut c, 0.5, &[orange, blue]);
        assert_eq!(var(&c, VAR_CAP_ON), Some(Val::Raw(CAP_BOTH)));
        // Native's update: the contested bit after the possession.
        assert_eq!(updates(&v).last().expect("update")[18] & 1, 1, "contested");
        let v = run(&mut c, 0.5, &[blue]);
        assert_eq!(updates(&v).last().expect("update")[18] & 1, 0, "no longer contested");
        run(&mut c, 0.1, &[orange, blue]);
        assert_eq!(var(&c, VAR_CAP_COUNT[1]), Some(Val::Lo32(1)));
        run(&mut c, 6.0, &[orange, away]);
        assert_eq!(var(&c, VAR_CAP_OWNER), Some(Val::Raw(CAP_TEAM[1])));
        assert_eq!(var(&c, VAR_CAP_SCORING[1]), Some(Val::Lo8(1)));
        assert_eq!(var(&c, VAR_CAP_SCORING[0]), Some(Val::Lo8(0)));
        assert!(matches!(var(&c, VAR_CAP_POINTS[1]), Some(Val::F32(x)) if x > 0.5));
        run(&mut c, 0.5, &[orange, blue]);
        assert_eq!(var(&c, VAR_CAP_SCORING[1]), Some(Val::Lo8(0)), "an enemy on the point stops it");
    }

    /// Native Dyson: a client's door trigger events set the door's var; a
    /// broken barrier's vars are 0 until it is back.
    #[test]
    fn level_vars_follow_client_events_and_barriers() {
        let mut c = capture_map(LEVEL_DYSON);
        let objects = OBJECT_MODULE;
        c.client_event(u64::from_le_bytes([0x9e, 0x91, 0x11, 0xc2, 0x7a, 0x42, 0xc0, 0xfb]), objects, 0x2b);
        assert_eq!(var(&c, 0x24f0_bb7d_e399_2aee), Some(Val::Raw(0x0000_01dd_0000_0000)));
        c.client_event(u64::from_le_bytes([0x9f, 0x91, 0x11, 0xc2, 0x7a, 0x42, 0xc0, 0xfb]), objects, 0x2b);
        assert_eq!(var(&c, 0x24f0_bb7d_e399_2aee), Some(Val::Raw(0x0000_01dd_3f80_0000)));
        assert_eq!(var(&c, 0x8f8e_39e4_e67b_5496), Some(Val::Lo32(1)));
        c.set_barriers_down(vec![0xbb]);
        assert_eq!(var(&c, 0x8f8e_39e4_e67b_5496), Some(Val::Lo32(0)));
        assert_eq!(var(&c, 0xc86e_56e1_4609_87d8), Some(Val::Lo32(1)));
    }

    /// Native Dyson and Combustion: the round over flag goes up with the
    /// round over states and down 4.0 s into the next team room.
    #[test]
    fn the_round_over_flag_clears_in_the_next_team_room() {
        let mut c = playing(capture_map(LEVEL_DYSON));
        let away = [(0u8, true, Some([50.0, 0.0, 0.0]), 4)];
        c.rounds_won = [0, 1];
        c.end_round(Winner::Orange);
        run(&mut c, 0.6, &away);
        assert_eq!(var(&c, VAR_ROUND_OVER), Some(Val::Lo32(1)));
        run(&mut c, 15.0, &away);
        assert_eq!(c.phase, Phase::TeamRoom);
        assert_eq!(var(&c, VAR_ROUND_OVER), Some(Val::Lo32(1)));
        assert_eq!(var(&c, VAR_SUBPHASE), Some(Val::Raw(SUB_TEAM_ROOM)));
        run(&mut c, 4.1, &away);
        assert_eq!(var(&c, VAR_ROUND_OVER), Some(Val::Lo32(0)));
    }

    fn unhex8(space: u64, rest: &[u8]) -> Vec<u8> {
        [space.to_le_bytes().as_slice(), rest].concat()
    }

    #[test]
    fn event_symbols_are_the_recorded_bytes() {
        // Recording: 563aee80d6affd07 63a521d79ccca86f 7a01 ...
        assert_eq!(EV_ROUND_BEGIN.to_le_bytes(), [0x56, 0x3a, 0xee, 0x80, 0xd6, 0xaf, 0xfd, 0x07]);
        assert_eq!(MOD_COMBAT_MATCH.to_le_bytes(), [0x63, 0xa5, 0x21, 0xd7, 0x9c, 0xcc, 0xa8, 0x6f]);
        assert_eq!(EV_RETURN_TO_LOBBY.to_le_bytes(), [0x93, 0x60, 0x57, 0x03, 0x42, 0x2a, 0x74, 0x65]);
        assert_eq!(gauss().match_handle, 0x17a);
    }

    #[test]
    fn gauss_round_flow_checkpoints_and_win() {
        let mut c = gauss();
        let start = Track::gauss().pos(0.0);
        let mut on = [(1u8, true, Some(start), 0)];
        run(&mut c, 30.5, &on);
        assert_eq!(c.phase, Phase::TeamRoom);
        run(&mut c, 15.0, &on);
        assert_eq!(c.phase, Phase::RoundStart);
        let v = run(&mut c, 45.5, &on);
        assert_eq!(c.phase, Phase::Playing);
        assert!(v.contains(&Out::Event(EV_ROUND_BEGIN, MOD_COMBAT_MATCH, 0x17a, FILTER_ALL)));
        // One attacker pushes at 1.25 m/s: a misc state, then trackpoints.
        let mut seen_tp = false;
        let mut cp_clock = None;
        for _ in 0..2000 {
            let p = c.payload.as_ref().expect("payload");
            on[0].2 = Some(p.track.pos(p.dist));
            for o in c.step(0.1, &on) {
                if let Out::Reliable(PAYLOAD_TRACKPOINT, b) = &o {
                    assert_eq!(&b[..2], &[0xd0, 0]);
                    seen_tp = true;
                }
            }
            if cp_clock.is_none() && c.payload.as_ref().expect("p").next == 1 {
                cp_clock = Some(c.clock);
                break;
            }
        }
        assert!(seen_tp);
        // 77.8 m at 1.25 m/s is 62.2 s: 180 - 62.2 + 120.
        let clock = cp_clock.expect("checkpoint 1");
        assert!((clock - 237.5).abs() < 1.0, "{clock}");
        // Held for the 35 s celebration.
        run(&mut c, 30.0, &on);
        assert!((c.clock - clock).abs() < 0.01);
        // Walk it home.
        for _ in 0..20_000 {
            let p = c.payload.as_ref().expect("payload");
            on[0].2 = Some(p.track.pos(p.dist));
            c.step(0.1, &on);
            if c.phase == Phase::Over {
                break;
            }
        }
        assert_eq!(c.winner, Some(Winner::Orange));
        let v = run(&mut c, 45.0, &on);
        assert!(v.contains(&Out::Awards));
        assert!(v.contains(&Out::Event(EV_RETURN_TO_LOBBY, MOD_COMBAT_MATCH, 0x17a, FILTER_ALL)));
        assert_eq!(c.state, ST_MATCH_END[6]);
    }

    /// One attacker pushes Gauss at the max speed the whole track (no
    /// per-section speeds); a second one adds `payload_speed_multi_attacker_2`.
    #[test]
    fn gauss_payload_speed_is_the_same_along_the_track() {
        for (from, n, want) in [(0.0, 1usize, 1.25), (90.0, 1, 1.25), (90.0, 2, 1.5)] {
            let mut c = playing(gauss());
            {
                let p = c.payload.as_mut().expect("p");
                p.dist = from;
                p.next = p.track.checkpoints.iter().filter(|&&d| d <= from).count();
            }
            c.set_payload_holders((0..n as u8).collect());
            let mut me: Vec<Player> = (0..n as u8).map(|s| (1u8, true, None, s)).collect();
            for _ in 0..50 {
                let p = c.payload.as_ref().expect("p");
                let at = Some(p.track.pos(p.dist));
                me.iter_mut().for_each(|m| m.2 = at);
                c.step(0.1, &me);
            }
            let a = c.payload_dist().expect("d");
            for _ in 0..20 {
                let p = c.payload.as_ref().expect("p");
                let at = Some(p.track.pos(p.dist));
                me.iter_mut().for_each(|m| m.2 = at);
                c.step(0.1, &me);
            }
            let v = (c.payload_dist().expect("d") - a) / 2.0;
            assert!((v - want).abs() < 0.02, "from {from} with {n}: {v}");
        }
    }

    /// Native Gauss 411.5-412.9: slot 2's touch states as the attacker count
    /// went 1, 0, 1 (a hand on entry 105, either hand).
    /// The payload's physics record at the start of each track matches the
    /// native server's first frame: the same position (to the millimetres the
    /// track file keeps) and rotation, and only the payload's record and the
    /// time differ from the template.
    #[test]
    fn payload_physics_matches_native_at_the_start() {
        let get = |d: &[u8], at: usize, n: usize| (0..n).fold(0u32, |v, k| v | u32::from((d[(at + k) >> 3] >> ((at + k) & 7)) & 1) << k);
        let quat = |raw: u32| {
            let largest = (raw & 3) as usize;
            let mut rest = (0..3).map(|i| ((raw >> (2 + 10 * i) & 1023) as f32 / 1023.0 * 2.0 - 1.0) / std::f32::consts::SQRT_2);
            let mut q = [0.0f32; 4];
            let mut sum = 0.0;
            for (i, c) in q.iter_mut().enumerate() {
                if i != largest {
                    *c = rest.next().unwrap_or(0.0);
                    sum += *c * *c;
                }
            }
            q[largest] = (1.0 - sum).max(0.0).sqrt();
            q
        };
        for (c, template, index) in [(gauss(), PAYLOAD_PHYS_GAUSS, 0usize), (fission(), PAYLOAD_PHYS_FISSION, 3)] {
            let native: Vec<u8> = (0..template.len() / 2).map(|i| u8::from_str_radix(&template[2 * i..2 * i + 2], 16).expect("hex")).collect();
            let ours = c.payload_physics(0x1234_5678).expect("payload map");
            assert_eq!(ours.len(), 168);
            assert_eq!(&ours[8..12], &0x1234_5678u32.to_le_bytes());
            let r = PHYS_HEAD_BITS + index * PHYS_RECORD_BITS;
            for k in 0..3 {
                let (a, b) = (get(&native, r + PHYS_POS + 19 * k, 19) as i64, get(&ours, r + PHYS_POS + 19 * k, 19) as i64);
                assert!((a - b).abs() < 120, "axis {k}: native {a}, ours {b}");
            }
            let (qa, qb) = (quat(get(&native, r + PHYS_ROT, 32)), quat(get(&ours, r + PHYS_ROT, 32)));
            let dot: f32 = (0..4).map(|i| qa[i] * qb[i]).sum();
            assert!(dot.abs() > 0.995, "rotation: native {qa:?}, ours {qb:?}");
            // Nothing outside the payload's record changes (bar the time).
            for i in (128..PHYS_HEAD_BITS).chain(PHYS_HEAD_BITS..168 * 8).filter(|i| !(r..r + PHYS_RECORD_BITS).contains(i)) {
                assert_eq!(get(&native, i, 1), get(&ours, i, 1), "bit {i}");
            }
        }
    }

    #[test]
    fn a_hand_on_the_payload_is_the_claim() {
        let t = |tail: [u8; 3]| payload_touch(&[0x19, 0x3a, 0x62, 0x0c, 2, 0, 1, 0, tail[0], tail[1], tail[2], 0, 0, 0, 0, 0]);
        assert_eq!(t([0x99, 0xb8, 0xd3]), Some((2, true)));
        assert_eq!(t([0x9c, 0xb8, 0xdd]), Some((2, false)));
        assert_eq!(t([0x9d, 0xa4, 0xdd]), Some((2, true)));
        // Standing at the payload without a hand on it claims nothing.
        let mut c = playing(gauss());
        let me = [(1u8, true, Some(Track::gauss().pos(0.0)), 0)];
        for _ in 0..30 {
            c.step(0.1, &me);
        }
        assert_eq!(c.payload_dist(), Some(0.0));
        c.set_payload_holders(vec![0]);
        for _ in 0..30 {
            c.step(0.1, &me);
        }
        // Native Fission: 3.558 m in the first 3.458 s (the ramp).
        let d = c.payload_dist().expect("d");
        assert!((d - 2.98).abs() < 0.1, "3 s from rest: {d}");
    }

    /// Native Gauss 520.7-521.7: the empty payload rolled back onto its
    /// checkpoint: the trackpoint before it, state 16, stopped; a hand on it
    /// and it is on the checkpoint's trackpoint again.
    #[test]
    fn a_rollback_onto_a_checkpoint_is_state_16() {
        let mut c = playing(gauss());
        let cp = Track::gauss().checkpoints[0];
        {
            let p = c.payload.as_mut().expect("p");
            p.dist = cp + 0.01;
            p.next = 1;
            p.cur_tp = 83;
            p.best_tp = 83;
            p.idle = ROLLBACK_AFTER;
        }
        let away = [(1u8, true, Some([500.0, 0.0, 0.0]), 0)];
        let v = run(&mut c, 2.0, &away);
        assert!(v.contains(&Out::Reliable(PAYLOAD_POS, vec![0xd0, 0, 82, 0])), "{v:?}");
        assert!(v.contains(&Out::Reliable(PAYLOAD_MISC, vec![0xd0, 0, 16, 0, 0, 0, 0, 0])), "{v:?}");
        let me = on_payload(&mut c, 1);
        let v = c.step(0.1, &[me]);
        assert!(v.contains(&Out::Reliable(PAYLOAD_MISC, vec![0xd0, 0, 16, 0, 1, 0, 1, 0])), "{v:?}");
        assert!(v.contains(&Out::Reliable(PAYLOAD_POS, vec![0xd0, 0, 83, 0])), "{v:?}");
    }

    #[test]
    fn an_empty_payload_rolls_back_to_the_checkpoint_and_time_up_wins_for_blue() {
        let mut c = gauss();
        let start = Track::gauss().pos(0.0);
        let on = [(1u8, true, Some(start), 0)];
        run(&mut c, 91.0, &on);
        assert_eq!(c.phase, Phase::Playing);
        let p = c.payload.as_mut().expect("p");
        p.dist = 10.0;
        p.idle = 0.0;
        let away = [(1u8, true, Some([500.0, 0.0, 0.0]), 0)];
        run(&mut c, 5.9, &away);
        assert!((c.payload_dist().expect("d") - 10.0).abs() < 0.01, "waits 6 s");
        // Native Fission: 1.31 m in the first 5.84 s of a rollback (the ramp).
        run(&mut c, 5.9, &away);
        let d = c.payload_dist().expect("d");
        assert!((d - (10.0 - 1.31)).abs() < 0.06, "5.8 s of rollback: {d}");
        // Native Fission: an attacker's hand on it turns it 1.24 s later.
        let hand = [(1u8, true, Some(Track::gauss().pos(d)), 0)];
        let v = run(&mut c, 1.0, &hand);
        assert!(v.contains(&Out::Reliable(PAYLOAD_MISC, vec![0xd0, 0, 0, 0, 1, 0, 0xff, 0])), "{v:?}");
        assert!(!v.contains(&Out::Reliable(PAYLOAD_MISC, vec![0xd0, 0, 0, 0, 1, 0, 1, 0])));
        let v = run(&mut c, 0.5, &hand);
        assert!(v.contains(&Out::Reliable(PAYLOAD_MISC, vec![0xd0, 0, 0, 0, 1, 0, 1, 0])), "{v:?}");
        run(&mut c, 6.5, &away);
        // A defender alone pushes it back at once (native Fission), to the
        // start at most; the misc state says 0 attackers, 1 defender, back.
        let p = c.payload.as_mut().expect("p");
        p.dist = 7.75;
        let def = [(0u8, true, Some(Track::gauss().pos(7.75)), 1)];
        let v = run(&mut c, 1.0, &def);
        assert!(v.contains(&Out::Reliable(PAYLOAD_MISC, vec![0xd0, 0, 0, 0, 0, 1, 0xff, 0])), "{v:?}");
        assert!((c.payload_dist().expect("d") - 7.5).abs() < 0.05);
        // Time runs out: blue wins.
        run(&mut c, 200.0, &away);
        assert_eq!(c.winner, Some(Winner::Blue));
    }

    /// Native Gauss: the claim state, motion and claims follow who holds it.
    #[test]
    fn payload_hud_follows_the_claims() {
        let mut c = playing(gauss());
        c.payload.as_mut().expect("p").dist = 10.0;
        assert_eq!(var(&c, VAR_CLAIM_STATE), Some(Val::Raw(CLAIM_NONE)));
        assert_eq!(var(&c, VAR_PAYLOAD_MOTION), Some(Val::Raw(MOTION_STILL)));
        assert_eq!(var(&c, VAR_OVERTIME_PROGRESS), Some(Val::F32(0.0)));
        let me = on_payload(&mut c, 1);
        for _ in 0..5 {
            c.step(0.1, &[me]);
        }
        assert_eq!(var(&c, VAR_CLAIM_STATE), Some(Val::Raw(CLAIM_ATTACKERS)));
        assert_eq!(var(&c, VAR_CLAIM_STATE_GAUSS), Some(Val::Raw(CLAIM_ATTACKERS)));
        assert_eq!(var(&c, VAR_PAYLOAD_MOTION), Some(Val::Raw(MOTION_FORWARD)));
        assert_eq!(var(&c, VAR_PAYLOAD_MOVING), Some(Val::Lo32(1)));
        assert_eq!(var(&c, VAR_ATTACKER_CLAIM), Some(Val::Lo32(1)));
        assert_eq!(var(&c, VAR_OVERTIME_PROGRESS), Some(Val::F32(1.0)));
        // A defender's hand too: contested, stopped.
        c.holders = vec![0, 1];
        let both = [me, (0u8, true, me.2, 1)];
        c.step(0.1, &both);
        assert_eq!(var(&c, VAR_CLAIM_STATE), Some(Val::Raw(CLAIM_CONTESTED)));
        assert_eq!(var(&c, VAR_DEFENDER_CLAIM), Some(Val::Lo32(1)));
        assert_eq!(var(&c, VAR_PAYLOAD_MOVING), Some(Val::Lo32(0)));
        // The defender alone rolls it back: unclaimed, backward, -1.
        c.holders = vec![1];
        for _ in 0..20 {
            c.step(0.1, &both);
        }
        assert_eq!(var(&c, VAR_CLAIM_STATE), Some(Val::Raw(CLAIM_NONE)));
        assert_eq!(var(&c, VAR_PAYLOAD_DIRECTION), Some(Val::Raw(MOTION_BACKWARD)));
        assert_eq!(var(&c, VAR_PAYLOAD_MOVING), Some(Val::Lo32(u32::MAX)));
    }

    /// Native Gauss: checkpoint 1 sets its level vars (the counter) and starts
    /// the door's 20 s count.
    #[test]
    fn a_checkpoint_sets_its_level_vars_and_gauss_door() {
        let mut c = playing(gauss());
        c.payload.as_mut().expect("p").dist = 77.0;
        let me = on_payload(&mut c, 1);
        run(&mut c, 2.0, &[me]);
        assert_eq!(var(&c, 0x1856_51ce_8e5f_1fb4), Some(Val::Raw(0x0000_022c_0000_0002)));
        assert!(matches!(var(&c, VAR_GAUSS_DOOR), Some(Val::Lo32(19 | 20))));
        let away = [(1u8, true, Some([500.0, 0.0, 0.0]), 0)];
        let open = |v: &[Out]| v.iter().filter(|o| matches!(o, Out::Event(GAUSS_DOOR_OPEN, MOD_GAUSS_CHECKPOINT, 0x15b, _))).count();
        assert_eq!(open(&run(&mut c, 18.0, &away)), 0, "not before 21.36 s");
        assert_eq!(open(&run(&mut c, 7.0, &away)), 1, "once, as the door opens");
        assert_eq!(var(&c, VAR_GAUSS_DOOR), Some(Val::Lo32(0)));
        assert_eq!(open(&run(&mut c, 10.0, &away)), 0);
    }

    /// A private match waits for every team with players to be ready
    /// (native mp_combat_server_state): a press flips the flag, and once
    /// all are ready, round restart, 3 s later match ready, 0.2 s later the
    /// round starts.
    #[test]
    fn a_private_match_waits_for_ready() {
        let mut c = gauss();
        c.set_private();
        let both = [(0u8, true, None, 0), (1u8, true, None, 1)];
        let v = run(&mut c, 60.0, &both);
        assert_eq!(c.phase, Phase::Waiting);
        assert_eq!(c.state, ST_WAIT_FOR_READY);
        assert_eq!(v.iter().filter(|o| matches!(o, Out::Event(EV_LOAD_A, ..))).count(), 1);
        assert_eq!(c.team_ready(1), Some(true));
        assert_eq!(c.team_ready(1), Some(false), "a second press un-readies");
        assert_eq!(c.team_ready(1), Some(true));
        run(&mut c, 1.0, &both);
        assert_eq!(c.phase, Phase::Waiting);
        assert_eq!(var(&c, VAR_ORANGE_READY), Some(Val::Lo8(1)));
        assert_eq!(c.team_ready(0), Some(true));
        let v = run(&mut c, 0.1, &both);
        assert!(v.iter().any(|o| matches!(o, Out::Event(EV_NEXT_ROUND, ..))), "{v:?}");
        assert_eq!(c.team_ready(0), None, "locked in once starting");
        let v = run(&mut c, 2.8, &both);
        assert!(!v.iter().any(|o| matches!(o, Out::Event(EV_MATCH_READY, ..))));
        assert_eq!(c.phase, Phase::Waiting);
        let v = run(&mut c, 0.2, &both);
        assert!(v.iter().any(|o| matches!(o, Out::Event(EV_MATCH_READY, ..))), "{v:?}");
        assert_eq!(c.phase, Phase::Waiting);
        run(&mut c, 0.3, &both);
        assert_eq!(c.phase, Phase::TeamRoom);
        // Only orange there: orange alone is enough.
        let mut c = fission();
        c.set_private();
        let orange = [(1u8, true, None, 0)];
        c.team_ready(1);
        run(&mut c, 3.5, &orange);
        assert_eq!(c.phase, Phase::TeamRoom);
        // A public match does not wait.
        let mut c = gauss();
        assert_eq!(c.team_ready(0), None);
        run(&mut c, 0.2, &orange);
        assert_eq!(c.phase, Phase::PreMatch);
    }

    /// Native: a private match stays on the personal stats; a rematch goes
    /// back to waiting for ready with the stats reset; the lobby button
    /// sends everyone back.
    #[test]
    fn a_private_match_end_waits_for_rematch_or_lobby() {
        let mut c = playing(gauss());
        c.set_private();
        c.state = ST_PLAYING;
        let away = [(0u8, true, Some([500.0, 0.0, 0.0]), 0), (1u8, true, Some([500.0, 0.0, 0.0]), 1)];
        c.finish(Winner::Orange);
        let v = run(&mut c, 120.0, &away);
        assert_eq!(c.phase, Phase::Over);
        assert!(!v.iter().any(|o| matches!(o, Out::Event(EV_RETURN_TO_LOBBY, ..))), "private: no return");
        assert_eq!(c.state, ST_MATCH_END[5]);
        let v = c.rematch();
        assert!(v.iter().any(|o| matches!(o, Out::Event(EV_GAME_RESTART_CONFIRMED, ..))));
        assert!(v.iter().any(|o| matches!(o, Out::Event(EV_GAME_RESTART, ..))));
        assert!(v.contains(&Out::ResetStats));
        assert!(c.rematch().is_empty(), "once");
        run(&mut c, 0.3, &away);
        assert_eq!(c.phase, Phase::Waiting);
        assert_eq!(c.state, ST_WAIT_FOR_READY);
        assert!(c.winner.is_none());
        assert_eq!(c.payload_dist(), Some(0.0));
        assert_eq!(c.team_ready(0), Some(true), "ready again");
        // The lobby button.
        let v = c.return_to_lobby();
        assert!(v.iter().any(|o| matches!(o, Out::Event(EV_RETURN_TO_LOBBY, ..))));
        assert_eq!(c.state, ST_MATCH_END[6]);
        // A public match goes back by itself and has no rematch.
        let mut c = playing(gauss());
        c.finish(Winner::Orange);
        let v = run(&mut c, 120.0, &away);
        assert!(v.iter().any(|o| matches!(o, Out::Event(EV_RETURN_TO_LOBBY, ..))));
        assert!(c.rematch().is_empty());
    }

    /// Native: every team with players asks for a restart, in play: the
    /// match restarts. One team alone does not.
    #[test]
    fn private_restart_requests_restart_the_match() {
        let mut c = playing(fission());
        c.set_private();
        c.state = ST_PLAYING;
        let both = [(0u8, true, None, 0), (1u8, true, None, 1)];
        assert!(c.restart_request(0));
        assert!(!c.restart_request(0), "a second press does nothing");
        run(&mut c, 0.1, &both);
        assert_eq!(c.phase, Phase::Playing);
        assert_eq!(var(&c, VAR_RESTART_REQUEST[0]), Some(Val::Lo8(1)));
        assert!(c.restart_request(1));
        let v = run(&mut c, 0.1, &both);
        assert!(v.iter().any(|o| matches!(o, Out::Event(EV_GAME_RESTART, ..))), "{v:?}");
        run(&mut c, 0.3, &both);
        assert_eq!(c.phase, Phase::Waiting);
        assert_eq!(var(&c, VAR_RESTART_REQUEST[0]), Some(Val::Lo8(0)));
        assert_eq!(c.payload_dist(), Some(0.0));
    }

    /// A private payload match: orange's furthest point in the first half is
    /// the mark; teams swap; the new attackers win by reaching it.
    #[test]
    fn private_payload_halves_swap_and_beat_the_mark() {
        let mut c = playing(fission());
        c.set_private();
        c.state = ST_PLAYING;
        c.payload.as_mut().expect("p").dist = 20.0;
        c.clock = 1.0;
        let away = [(1u8, true, Some([500.0, 0.0, 0.0]), 0)];
        let v = run(&mut c, 2.0, &away);
        assert_eq!(c.phase, Phase::RoundOver);
        assert!(c.winner.is_none());
        assert_eq!(c.to_beat, Some(20.0));
        assert!(v.iter().any(|o| matches!(o, Out::Reliable(PAYLOAD_MISC, b) if b[2] == 8)));
        // Round over, the team room: sides swap, the payload back at the start.
        let v = run(&mut c, 17.0, &away);
        assert!(v.contains(&Out::SwapTeams), "{v:?}");
        assert_eq!(c.phase, Phase::TeamRoom);
        assert_eq!(c.payload_dist(), Some(0.0));
        assert_eq!(var(&c, VAR_TEAM_A_MARKER_POS), Some(Val::F32(20.0)));
        assert_eq!(var(&c, VAR_ROUND_COUNTER), Some(Val::Lo32(2)));
        // The second half: just short at time up loses, ...
        let mut short = playing(std::mem::replace(&mut c, fission()));
        short.payload.as_mut().expect("p").dist = 19.0;
        short.clock = 0.5;
        run(&mut short, 2.0, &away);
        assert_eq!(short.winner, Some(Winner::Blue));
        // ... reaching the mark wins.
        let mut c = playing(fission());
        c.set_private();
        c.half = 2;
        c.to_beat = Some(20.0);
        c.payload.as_mut().expect("p").dist = 19.5;
        let me = on_payload(&mut c, 1);
        run(&mut c, 3.0, &[me]);
        assert_eq!(c.winner, Some(Winner::Orange));
    }
}
