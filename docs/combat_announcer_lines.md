# Combat announcer audio: every file in `vo_announcer_combat`

All 126 files in `J:\EchoAudio\vo_announcer_combat`. 84 are named; the files are renamed
to their Wwise event name and `_rename_map.csv` in the folder maps each back to the original `<event id>_<media id>.wav`.

## How the game plays them

- One client script plays them all: `mp_combat_voice_announcer` (`526ae906c49de696`). Each of its `OnEvent`
  listeners (`evt_vo_anc_*`) plays one Wwise event with a `PlaySoundNode`.
- Native's combat server sends none of these events (its recordings of all four maps, 2026-11-16). Each game raises
  them itself from its level scripts (`mp_combat_gamemode_capturepoint`, `mp_combat_gamemode_payload`,
  `mp_combat_capturable_point`, `veh_payload_core`, `mp_combat_server_state`), reacting to the replicated vars and the
  server's game-state events. So a line plays on our server as long as we send what native does.
- Names: a Wwise event id is FNV-1 (32-bit) of the lower-cased event name, and every line is `vo_anc_<line>`
  (51 of the script's 55 match `vo_anc_` + its `evt_vo_anc_` listener). The rest were found by trying the game's own
  symbol names and announcer words, one to three words long.
- "Raised by nobody": no other script container and not the game's executable holds the event's id.

## Counts

- **34 lines** the announcer plays and a level script raises: they play in native's matches.
- **20 lines** the announcer listens for but nothing raises: never heard in a stock match. A server could
  only play them if it sent the `evt_vo_anc_*` event to the announcer itself (not tested: whether a client script's
  `OnEvent` answers a network event on that actor).
- **72 lines** no script plays at all (no listener): they cannot be played by any event. Of these,
  40 are not named yet (no name tried matched; listening to them would name them).

| File | Wwise event | Listener | Status |
|---|---|---|---|
| VO_ANC_ATTACK_VICTORY.wav | 1674904415 | `evt_vo_anc_attack_victory` | raised by mp_combat_gamemode_payload, tst_dsn_cbt_payload_clock |
| VO_ANC_CAPTURE_POINT.wav | 401820849 | `evt_vo_anc_capture_point` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_CAPTURE_THE_POINT.wav | 1742265977 | `evt_vo_anc_capture_the_point` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_COUNTDOWN_01.wav | 4194757161 | `evt_vo_anc_countdown_01` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_COUNTDOWN_02.wav | 4194757162 | `evt_vo_anc_countdown_02` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_COUNTDOWN_03.wav | 4194757163 | `evt_vo_anc_countdown_03` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_COUNTDOWN_04.wav | 4194757164 | `evt_vo_anc_countdown_04` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_COUNTDOWN_05.wav | 4194757165 | `evt_vo_anc_countdown_05` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_DEFEND_VICTORY.wav | 1120877703 | `evt_vo_anc_defend_victory` | raised by mp_combat_gamemode_payload, tst_dsn_cbt_payload_clock |
| VO_ANC_DEFENSE_SETUP_ATTACKER.wav | 1469270880 | `evt_vo_anc_defense_setup_attacker` | raised by mp_combat_gamemode_payload |
| VO_ANC_DEFENSE_SETUP_DEFENDER.wav | 3525362004 | `evt_vo_anc_defense_setup_defender` | raised by mp_combat_gamemode_payload |
| VO_ANC_OVERTIME.wav | 843044405 | `evt_vo_anc_overtime` | raised by mp_combat_server_state |
| VO_ANC_PAYLOAD_ATTACK_ACCEL.wav | 3730028584 | `evt_vo_anc_payload_attack_accel` | raised by veh_payload_core |
| VO_ANC_PAYLOAD_ATTACK_START.wav | 246988596 | `evt_vo_anc_payload_attack_start` | raised by veh_payload_core |
| VO_ANC_PAYLOAD_CHECKPOINT.wav | 2305603607 | `evt_vo_anc_payload_checkpoint` | raised by veh_payload_core |
| VO_ANC_PAYLOAD_CONTESTED.wav | 230703748 | `evt_vo_anc_payload_contested` | raised by veh_payload_a_fx, veh_payload_core |
| VO_ANC_PAYLOAD_DEFEND_START.wav | 3434550792 | `evt_vo_anc_payload_defend_start` | raised by veh_payload_a_fx, veh_payload_core |
| VO_ANC_PAYLOAD_OBJ_ATTACK.wav | 2347158459 | `evt_vo_anc_payload_obj_attack` | raised by mp_combat_gamemode_payload |
| VO_ANC_PAYLOAD_OBJ_DEFEND.wav | 2710547271 | `evt_vo_anc_payload_obj_defend` | raised by mp_combat_gamemode_payload |
| VO_ANC_PAYLOAD_RETURNED.wav | 804256164 | `evt_vo_anc_payload_returned` | raised by veh_payload_core |
| VO_ANC_POINT_CAPTURED.wav | 3924190611 | `evt_vo_anc_point_captured` | raised by mp_combat_capturable_point |
| VO_ANC_POINT_CONTESTED.wav | 89710316 | `evt_vo_anc_point_contested` | raised by mp_combat_capturable_point |
| VO_ANC_POINT_STOLEN.wav | 2490897932 | `evt_vo_anc_point_stolen` | raised by mp_combat_capturable_point |
| VO_ANC_POINT_UNLOCKING_IN.wav | 2181387335 | `evt_vo_anc_point_unlocking_in` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_PREPARE_CAPTURE_POINT.wav | 2134412277 | `evt_vo_anc_prepare_capture_point` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_READY_UP_ATTACKERS.wav | 73634826 | `evt_vo_anc_ready_up_attackers` | raised by mp_combat_gamemode_payload |
| VO_ANC_READY_UP_DEFENDERS.wav | 72974410 | `evt_vo_anc_ready_up_defenders` | raised by mp_combat_gamemode_payload |
| VO_ANC_ROUND_COMPLETE.wav | 2806407610 | `evt_vo_anc_round_complete` | raised by mp_combat_gamemode_capturepoint, mp_combat_gamemode_payload |
| VO_ANC_SQUAD_READY_BLUE.wav | 2558879465 | `evt_vo_anc_squad_ready_blue` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_SQUAD_READY_ORANGE.wav | 2965118631 | `evt_vo_anc_squad_ready_orange` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_SQUAD_VICTORY_BLUE.wav | 1750728028 | `evt_vo_anc_victory_blue` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_SQUAD_VICTORY_ORANGE.wav | 249273094 | `evt_vo_anc_victory_orange` | raised by mp_combat_gamemode_capturepoint |
| 2501083752_417493019.wav | 2501083752 | `evt_vo_anc_won_match_orange` | raised by mp_combat_gamemode_capturepoint |
| 535462322_412777373.wav | 535462322 | `evt_vo_anc_won_match_blue` | raised by mp_combat_gamemode_capturepoint |
| VO_ANC_COUNTDOWN_06.wav | 4194757166 | `evt_vo_anc_countdown_06` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_COUNTDOWN_07.wav | 4194757167 | `evt_vo_anc_countdown_07` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_COUNTDOWN_08.wav | 4194757152 | `evt_vo_anc_countdown_08` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_COUNTDOWN_09.wav | 4194757153 | `evt_vo_anc_countdown_09` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_COUNTDOWN_10.wav | 4211534747 | `evt_vo_anc_countdown_10` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_MATCH_COMPLETE.wav | 2664968815 | `evt_vo_anc_match_complete` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_MATCH_STARTED.wav | 153171357 | `evt_vo_anc_match_started` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_MATCH_STARTING.wav | 4133843374 | `evt_vo_anc_match_starting` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_MATCH_TIME_EXTENDED.wav | 3577281707 | `evt_vo_anc_match_time_extended` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_PAYLOAD_ATTACK_WIN.wav | 3201383226 | `evt_vo_anc_payload_attack_win` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_PAYLOAD_DEFEND_ACCEL.wav | 2215197164 | `evt_vo_anc_payload_defend_accel` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_PAYLOAD_DEFEND_WIN.wav | 3304277342 | `evt_vo_anc_payload_defend_win` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_ROUND_01.wav | 3955722018 | `evt_vo_anc_round_01` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_ROUND_02.wav | 3955722017 | `evt_vo_anc_round_02` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_ROUND_03.wav | 3955722016 | `evt_vo_anc_round_03` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_ROUND_04.wav | 3955722023 | `evt_vo_anc_round_04` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_ROUND_05.wav | 3955722022 | `evt_vo_anc_round_05` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_ROUND_06.wav | 3955722021 | `evt_vo_anc_round_06` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_ROUND_STARTING.wav | 4022091895 | `evt_vo_anc_round_starting` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_THIRTY_SEC_REMAIN.wav | 1810373753 | `evt_vo_thirty_sec_remain` | UNUSED: the announcer listens, nothing raises it |
| VO_ANC_CAPTURE_STARTED.wav | 2375423504 | — | UNUSED: no script plays it |
| VO_ANC_CAPTURING.wav | 2432016427 | — | UNUSED: no script plays it |
| VO_ANC_CAPTURING_POINT.wav | 599934228 | — | UNUSED: no script plays it |
| VO_ANC_CLOCK_PAUSED.wav | 3887658355 | — | UNUSED: no script plays it |
| VO_ANC_COMBAT_PAUSED.wav | 3661072109 | — | UNUSED: no script plays it |
| VO_ANC_CONVERTING.wav | 777403255 | — | UNUSED: no script plays it |
| VO_ANC_CONVERTING_POINT.wav | 1952784528 | — | UNUSED: no script plays it |
| VO_ANC_DRAW.wav | 260993154 | — | UNUSED: no script plays it |
| VO_ANC_FINAL_ROUND.wav | 557126125 | — | UNUSED: no script plays it |
| VO_ANC_GAME_PAUSED.wav | 814437455 | — | UNUSED: no script plays it |
| VO_ANC_MATCH_DRAW.wav | 455293136 | — | UNUSED: no script plays it |
| VO_ANC_MATCH_OVER.wav | 3850160404 | — | UNUSED: no script plays it |
| VO_ANC_MATCH_PAUSED.wav | 3368801620 | — | UNUSED: no script plays it |
| VO_ANC_OVERTIME_PUSH.wav | 3998456092 | — | UNUSED: no script plays it |
| VO_ANC_PAUSE.wav | 2347167834 | — | UNUSED: no script plays it |
| VO_ANC_PAYLOAD_PAUSED.wav | 448861475 | — | UNUSED: no script plays it |
| VO_ANC_POINT_LOST.wav | 1572708583 | — | UNUSED: no script plays it |
| VO_ANC_POINT_TAKEN.wav | 2814210530 | — | UNUSED: no script plays it |
| VO_ANC_POINT_UNLOCKED.wav | 2890553010 | — | UNUSED: no script plays it |
| VO_ANC_PREPARE_TAKE_POINT.wav | 434205646 | — | UNUSED: no script plays it |
| VO_ANC_PROGRESS_STOPPED.wav | 3897558131 | — | UNUSED: no script plays it |
| VO_ANC_ROUND_DRAW.wav | 31605733 | — | UNUSED: no script plays it |
| VO_ANC_ROUND_OVER.wav | 3440992865 | — | UNUSED: no script plays it |
| VO_ANC_SCORE_HALTED.wav | 2120760891 | — | UNUSED: no script plays it |
| VO_ANC_TAKING_POINT.wav | 549190131 | — | UNUSED: no script plays it |
| VO_ANC_TIE_BREAKER.wav | 3020664231 | — | UNUSED: no script plays it |
| VO_ANC_TIMER_PAUSED.wav | 461384732 | — | UNUSED: no script plays it |
| VO_ANC_VICTORY_BLUE.wav | 1793721929 | — | UNUSED: no script plays it |
| VO_ANC_VICTORY_ORANGE.wav | 783603719 | — | UNUSED: no script plays it |
| VO_ANC_WINS_MATCH.wav | 1207116951 | — | UNUSED: no script plays it |
| VO_ANC_WON_MATCH_BLUE.wav | 3983484569 | — | UNUSED: no script plays it |
| VO_ANC_WON_MATCH_ORANGE.wav | 1769035479 | — | UNUSED: no script plays it |
| 1443381939_108560131.wav | 1443381939 | — | UNUSED: no script plays it |
| 1544135365_353506924.wav | 1544135365 | — | UNUSED: no script plays it |
| 158128186_476860293.wav | 158128186 | — | UNUSED: no script plays it |
| 1606967575_687449346.wav | 1606967575 | — | UNUSED: no script plays it |
| 1689441546_607024719.wav | 1689441546 | — | UNUSED: no script plays it |
| 1810888815_287673166.wav | 1810888815 | — | UNUSED: no script plays it |
| 184088113_528148317.wav | 184088113 | — | UNUSED: no script plays it |
| 1887954697_25811981.wav | 1887954697 | — | UNUSED: no script plays it |
| 1963056324_720404194.wav | 1963056324 | — | UNUSED: no script plays it |
| 2092801294_19531070.wav | 2092801294 | — | UNUSED: no script plays it |
| 2425042426_591862299.wav | 2425042426 | — | UNUSED: no script plays it |
| 2504419893_91404266.wav | 2504419893 | — | UNUSED: no script plays it |
| 2511428978_591872819.wav | 2511428978 | — | UNUSED: no script plays it |
| 2581802314_272908878.wav | 2581802314 | — | UNUSED: no script plays it |
| 2618612983_311939007.wav | 2618612983 | — | UNUSED: no script plays it |
| 2765933555_1035459978.wav | 2765933555 | — | UNUSED: no script plays it |
| 277114807_942815335.wav | 277114807 | — | UNUSED: no script plays it |
| 2807589983_953317798.wav | 2807589983 | — | UNUSED: no script plays it |
| 2854246940_834886333.wav | 2854246940 | — | UNUSED: no script plays it |
| 2878288453_496300786.wav | 2878288453 | — | UNUSED: no script plays it |
| 2931271129_222526411.wav | 2931271129 | — | UNUSED: no script plays it |
| 3117740815_930979776.wav | 3117740815 | — | UNUSED: no script plays it |
| 3172174774_851339067.wav | 3172174774 | — | UNUSED: no script plays it |
| 3216747643_774839533.wav | 3216747643 | — | UNUSED: no script plays it |
| 3255755804_104622785.wav | 3255755804 | — | UNUSED: no script plays it |
| 3298178972_782077972.wav | 3298178972 | — | UNUSED: no script plays it |
| 3504381384_514448573.wav | 3504381384 | — | UNUSED: no script plays it |
| 3513483399_727223969.wav | 3513483399 | — | UNUSED: no script plays it |
| 3544795292_970175925.wav | 3544795292 | — | UNUSED: no script plays it |
| 375812966_156650541.wav | 375812966 | — | UNUSED: no script plays it |
| 3879545557_529411543.wav | 3879545557 | — | UNUSED: no script plays it |
| 3915025702_418931627.wav | 3915025702 | — | UNUSED: no script plays it |
| 3991645846_732455402.wav | 3991645846 | — | UNUSED: no script plays it |
| 4050683332_881322887.wav | 4050683332 | — | UNUSED: no script plays it |
| 4073870678_1059348693.wav | 4073870678 | — | UNUSED: no script plays it |
| 424690278_250790703.wav | 424690278 | — | UNUSED: no script plays it |
| 471125205_886638494.wav | 471125205 | — | UNUSED: no script plays it |
| 679360176_614500660.wav | 679360176 | — | UNUSED: no script plays it |
| 757494527_130840142.wav | 757494527 | — | UNUSED: no script plays it |
| 871435746_1029175497.wav | 871435746 | — | UNUSED: no script plays it |
