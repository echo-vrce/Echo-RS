# Arena announcer audio: every file in `vo_announcer_arena`

All 279 files in `J:\EchoAudio\vo_announcer_arena`, named from the game's own announcer scripts.
276 are played by a script; 3 are referenced by no script and not by the engine.

## How the game plays them

- `mp_announcer_server_state` (`e28785a6428603f9`) runs **on the server**. It listens for the engine's `nevt_*` events
  (save, interceptions, steal, clears, possession changes, post hit, punch, rampage, score, round/match results) and
  sends a cue to every client: `SR15NetComponentEvent` with the cue's CSymbol64, module `949c090e2472d440`, handle
  `0x2f`, filter 13, twice. Seen in native's public overtime recording (`H:\EVR\captures\netrec-pub-ot-20261119`).
- The clients' `mp_announcer_client_*` scripts each listen for cues (`OnEvent`) and play the matching line
  (`DialogueSceneNode` -> `PlayTimeline` -> `PlayDialogueNode`, which names the Wwise event, i.e. the file):
  `client_disc_events` (`c11303ba08ebcab3`), `client_points` (`a1991b50876efaa1`), `client_attacks`
  (`75e48e0e6041f259`), `client_attacks_lobby` (`8944549b62e28d27`), `client_progress` (`7b4a91f0d45a06d1`).
- `client_countdowns` (`5db0735a60bb1093`) needs no cue: each client plays the numbers 1-15 itself from the
  replicated clock vars. Its 15 files have no graph container; the numbers come from their Wwise ids (two runs of
  ids that differ only in the name's last digit; only 1-9 and 10-15 fit the files present).
- Any line here with a cue can be played by the server at any time by sending that cue. That includes lines the
  stock server never triggers (the `MPL_ARENA_*` scenes).

The files were `<Wwise event id>_<media id>.wav`; they are now renamed to their cue (or scene) name, and
`_rename_map.csv` in the folder maps each new name back to the original (the id form is what
`J:\EchoAudio\audio_manifest.json` still uses). Source: EchoConstruct's
`lift_dump --graph` of each script container, names from its `data/echo_symbols.tsv`.

| Renamed file | Original file | Cue the client listens for | Scene | Subtitle | Script |
|---|---|---|---|---|---|
| MPL_ARENA_BLOCK_A1.wav | 478581274_1060082228.wav | `MPL_ARENA_BLOCK_A1` | MPL_ARENA_BLOCK_A1 | Shut down! | client_attacks |
| MPL_ARENA_BLOCK_B1.wav | 1366463173_469193065.wav | `MPL_ARENA_BLOCK_B1` | MPL_ARENA_BLOCK_B1 | Deflected! | client_attacks |
| MPL_ARENA_BLOCK_C1.wav | 3537354438_449574024.wav | `MPL_ARENA_BLOCK_C1` | MPL_ARENA_BLOCK_C1 | What a block! | client_attacks |
| MPL_ARENA_BLOCK_D1.wav | 2052523728_1003880643.wav | `MPL_ARENA_BLOCK_D1` | MPL_ARENA_BLOCK_D1 | A perfect parry! | client_attacks |
| MPL_ARENA_BLOCK_E1.wav | 873312726_475103260.wav | `MPL_ARENA_BLOCK_E1` | MPL_ARENA_BLOCK_E1 | The ultimate defense! | client_attacks |
| MPL_ARENA_BLOCK_F1.wav | 426759111_695306539.wav | `MPL_ARENA_BLOCK_F1` | MPL_ARENA_BLOCK_F1 | Can't touch 'em! | client_attacks |
| MPL_ARENA_BLOCK_G1.wav | 469897402_205969995.wav | `MPL_ARENA_BLOCK_G1` | MPL_ARENA_BLOCK_G1 | Hold's 'em off! | client_attacks |
| MPL_ARENA_PUNCH_A1.wav | 2713581526_994593551.wav | `MPL_ARENA_PUNCH_A1` | mp_arena_punch_a1 | Gonna feel that one in the morning! | client_attacks |
| MPL_ARENA_PUNCH_B1.wav | 4231431083_1024839491.wav | `MPL_ARENA_PUNCH_B1` | mp_arena_punch_b1 | What a hit! | client_attacks |
| MPL_ARENA_PUNCH_C1.wav | 715541653_201757402.wav | `MPL_ARENA_PUNCH_C1` | mp_arena_punch_c1 | BLASTED! | client_attacks |
| MPL_ARENA_PUNCH_D1.wav | 2085763573_149912128.wav | `MPL_ARENA_PUNCH_D1` | mp_arena_punch_d1 | GOT 'EM! | client_attacks |
| MPL_ARENA_PUNCH_E1.wav | 1776678194_638707189.wav | `MPL_ARENA_PUNCH_E1` | mp_arena_punch_e1 | Gonna need recalibrating after that hit. | client_attacks |
| MPL_ARENA_PUNCH_F1.wav | 547165608_1043200145.wav | `MPL_ARENA_PUNCH_F1` | mp_arena_punch_f1 | Ohh, knocked 'em back to beta! | client_attacks |
| MPL_ARENA_PUNCH_G1.wav | 2191351404_433985271.wav | `MPL_ARENA_PUNCH_G1` | mp_arena_punch_g1 | Ohh ouch! | client_attacks |
| MPL_ARENA_PUNCH_H1.wav | 2582782174_397195329.wav | `MPL_ARENA_PUNCH_H1` | mp_arena_punch_h1 | Ring... Ding... Dong... | client_attacks |
| MP_ANNOUNCER_0040A_PUNCH.wav | 2702077534_55952568.wav | `MP_ANNOUNCER_0040A_PUNCH` | MP_ANNOUNCER_0040A_PUNCH | Knockout! | client_attacks |
| MP_ANNOUNCER_0040B_PUNCH.wav | 251666981_310641793.wav | `MP_ANNOUNCER_0040B_PUNCH` | MP_ANNOUNCER_0040B_PUNCH | K.O.! | client_attacks |
| MP_ANNOUNCER_0040C_PUNCH.wav | 2955982207_463844117.wav | `MP_ANNOUNCER_0040C_PUNCH` | MP_ANNOUNCER_0040C_PUNCH | Great hit! | client_attacks |
| MP_ANNOUNCER_0040D_PUNCH.wav | 713627269_240894736.wav | `MP_ANNOUNCER_0040D_PUNCH` | MP_ANNOUNCER_0040D_PUNCH | Scrambled his circuits! | client_attacks |
| MP_ANNOUNCER_0040E_PUNCH.wav | 1456731320_292476901.wav | `MP_ANNOUNCER_0040E_PUNCH` | MP_ANNOUNCER_0040E_PUNCH | Somebody call the repair shop! | client_attacks |
| MP_ANNOUNCER_0040F_PUNCH.wav | 845377818_360771205.wav | `MP_ANNOUNCER_0040F_PUNCH` | MP_ANNOUNCER_0040F_PUNCH | Brutal takedown! | client_attacks |
| MP_ANNOUNCER_0040G_PUNCH.wav | 1741636743_665445094.wav | `MP_ANNOUNCER_0040G_PUNCH` | MP_ANNOUNCER_0040G_PUNCH | Rock 'em, sock 'em! | client_attacks |
| MP_ANNOUNCER_0040H_PUNCH.wav | 1347416066_513587933.wav | `MP_ANNOUNCER_0040H_PUNCH` | MP_ANNOUNCER_0040H_PUNCH | What a beating! | client_attacks |
| MP_ANNOUNCER_0040I_PUNCH.wav | 533347113_178080344.wav | `MP_ANNOUNCER_0040I_PUNCH` | MP_ANNOUNCER_0040I_PUNCH | That's gotta hurt! | client_attacks |
| MP_ANNOUNCER_0090A_RAMPAGE.wav | 2379340667_406222523.wav | `MP_ANNOUNCER_0090A_RAMPAGE` | MP_ANNOUNCER_0090A_RAMPAGE | It's chaos! | client_attacks |
| MP_ANNOUNCER_0090B_RAMPAGE.wav | 1513616798_269729166.wav | `MP_ANNOUNCER_0090B_RAMPAGE` | MP_ANNOUNCER_0090B_RAMPAGE | We've got pandemonium! | client_attacks |
| MP_ANNOUNCER_0090C_RAMPAGE.wav | 1212006657_913688808.wav | `MP_ANNOUNCER_0090C_RAMPAGE` | MP_ANNOUNCER_0090C_RAMPAGE | Rampage! | client_attacks |
| MP_ANNOUNCER_0090D_RAMPAGE.wav | 624274111_696247697.wav | `MP_ANNOUNCER_0090D_RAMPAGE` | MP_ANNOUNCER_0090D_RAMPAGE | Is there anybody left standing! | client_attacks |
| MP_ANNOUNCER_0090E_RAMPAGE.wav | 1958788656_456424512.wav | `MP_ANNOUNCER_0090E_RAMPAGE` | MP_ANNOUNCER_0090E_RAMPAGE | Things are getting messy... | client_attacks |
| MP_ANNOUNCER_0090F_RAMPAGE.wav | 2708106472_306226012.wav | `MP_ANNOUNCER_0090F_RAMPAGE` | MP_ANNOUNCER_0090F_RAMPAGE | Brutal melee! | client_attacks |
| MP_ANNOUNCER_0090G_RAMPAGE.wav | 2708147871_268177418.wav | `MP_ANNOUNCER_0090G_RAMPAGE` | MP_ANNOUNCER_0090G_RAMPAGE | It's getting ugly... | client_attacks |
| MP_ANNOUNCER_0090H_RAMPAGE.wav | 1635757861_666899367.wav | `MP_ANNOUNCER_0090H_RAMPAGE` | MP_ANNOUNCER_0090H_RAMPAGE | The fists are flying... | client_attacks |
| MP_ANNOUNCER_0090I_RAMPAGE.wav | 14957318_403842342.wav | `MP_ANNOUNCER_0090I_RAMPAGE` | MP_ANNOUNCER_0090I_RAMPAGE | It's total anarchy! | client_attacks |
| MPL_ARENA_BLOCK_A1.wav | 478581274_1060082228.wav | `MPL_ARENA_BLOCK_A1_Lobby` | MPL_ARENA_BLOCK_A1 | Shut down! | client_attacks_lobby |
| MPL_ARENA_BLOCK_B1.wav | 1366463173_469193065.wav | `MPL_ARENA_BLOCK_B1_Lobby` | MPL_ARENA_BLOCK_B1 | Deflected! | client_attacks_lobby |
| MPL_ARENA_BLOCK_C1.wav | 3537354438_449574024.wav | `MPL_ARENA_BLOCK_C1_Lobby` | MPL_ARENA_BLOCK_C1 | What a block! | client_attacks_lobby |
| MPL_ARENA_BLOCK_D1.wav | 2052523728_1003880643.wav | `MPL_ARENA_BLOCK_D1_Lobby` | MPL_ARENA_BLOCK_D1 | A perfect parry! | client_attacks_lobby |
| MPL_ARENA_BLOCK_E1.wav | 873312726_475103260.wav | `MPL_ARENA_BLOCK_E1_Lobby` | MPL_ARENA_BLOCK_E1 | The ultimate defense! | client_attacks_lobby |
| MPL_ARENA_BLOCK_F1.wav | 426759111_695306539.wav | `MPL_ARENA_BLOCK_F1_Lobby` | MPL_ARENA_BLOCK_F1 | Can't touch 'em! | client_attacks_lobby |
| MPL_ARENA_BLOCK_G1.wav | 469897402_205969995.wav | `MPL_ARENA_BLOCK_G1_Lobby` | MPL_ARENA_BLOCK_G1 | Hold's 'em off! | client_attacks_lobby |
| MPL_ARENA_PUNCH_A1.wav | 2713581526_994593551.wav | `MPL_ARENA_PUNCH_A1_Lobby` | mp_arena_punch_a1 | Gonna feel that one in the morning! | client_attacks_lobby |
| MPL_ARENA_PUNCH_B1.wav | 4231431083_1024839491.wav | `MPL_ARENA_PUNCH_B1_Lobby` | mp_arena_punch_b1 | What a hit! | client_attacks_lobby |
| MPL_ARENA_PUNCH_C1.wav | 715541653_201757402.wav | `MPL_ARENA_PUNCH_C1_Lobby` | mp_arena_punch_c1 | BLASTED! | client_attacks_lobby |
| MPL_ARENA_PUNCH_D1.wav | 2085763573_149912128.wav | `MPL_ARENA_PUNCH_D1_Lobby` | mp_arena_punch_d1 | GOT 'EM! | client_attacks_lobby |
| MPL_ARENA_PUNCH_E1.wav | 1776678194_638707189.wav | `MPL_ARENA_PUNCH_E1_Lobby` | mp_arena_punch_e1 | Gonna need recalibrating after that hit. | client_attacks_lobby |
| MPL_ARENA_PUNCH_F1.wav | 547165608_1043200145.wav | `MPL_ARENA_PUNCH_F1_Lobby` | mp_arena_punch_f1 | Ohh, knocked 'em back to beta! | client_attacks_lobby |
| MPL_ARENA_PUNCH_G1.wav | 2191351404_433985271.wav | `MPL_ARENA_PUNCH_G1_Lobby` | mp_arena_punch_g1 | Ohh ouch! | client_attacks_lobby |
| MPL_ARENA_PUNCH_H1.wav | 2582782174_397195329.wav | `MPL_ARENA_PUNCH_H1_Lobby` | mp_arena_punch_h1 | Ring... Ding... Dong... | client_attacks_lobby |
| MP_ANNOUNCER_0040A_PUNCH.wav | 2702077534_55952568.wav | `MP_ANNOUNCER_0040A_PUNCH_Lobby` | MP_ANNOUNCER_0040A_PUNCH_Lobby | Knockout! | client_attacks_lobby |
| MP_ANNOUNCER_0040B_PUNCH.wav | 251666981_310641793.wav | `MP_ANNOUNCER_0040B_PUNCH_Lobby` | MP_ANNOUNCER_0040B_PUNCH_Lobby | K.O.! | client_attacks_lobby |
| MP_ANNOUNCER_0040C_PUNCH.wav | 2955982207_463844117.wav | `MP_ANNOUNCER_0040C_PUNCH_Lobby` | MP_ANNOUNCER_0040C_PUNCH_Lobby | Great hit! | client_attacks_lobby |
| MP_ANNOUNCER_0040D_PUNCH.wav | 713627269_240894736.wav | `MP_ANNOUNCER_0040D_PUNCH_Lobby` | MP_ANNOUNCER_0040D_PUNCH_Lobby | Scrambled his circuits! | client_attacks_lobby |
| MP_ANNOUNCER_0040E_PUNCH.wav | 1456731320_292476901.wav | `MP_ANNOUNCER_0040E_PUNCH_Lobby` | MP_ANNOUNCER_0040E_PUNCH_Lobby | Somebody call the repair shop! | client_attacks_lobby |
| MP_ANNOUNCER_0040F_PUNCH.wav | 845377818_360771205.wav | `MP_ANNOUNCER_0040F_PUNCH_Lobby` | MP_ANNOUNCER_0040F_PUNCH_Lobby | Brutal takedown! | client_attacks_lobby |
| MP_ANNOUNCER_0040G_PUNCH.wav | 1741636743_665445094.wav | `MP_ANNOUNCER_0040G_PUNCH_Lobby` | MP_ANNOUNCER_0040G_PUNCH_Lobby | Rock 'em, sock 'em! | client_attacks_lobby |
| MP_ANNOUNCER_0040H_PUNCH.wav | 1347416066_513587933.wav | `MP_ANNOUNCER_0040H_PUNCH_Lobby` | MP_ANNOUNCER_0040H_PUNCH_Lobby | What a beating! | client_attacks_lobby |
| MP_ANNOUNCER_0040I_PUNCH.wav | 533347113_178080344.wav | `MP_ANNOUNCER_0040I_PUNCH_Lobby` | MP_ANNOUNCER_0040I_PUNCH_Lobby | That's gotta hurt! | client_attacks_lobby |
| MP_ANNOUNCER_0090A_RAMPAGE.wav | 2379340667_406222523.wav | `MP_ANNOUNCER_0090A_RAMPAGE_Lobby` | MP_ANNOUNCER_0090A_RAMPAGE_Lobby | It's chaos! | client_attacks_lobby |
| MP_ANNOUNCER_0090B_RAMPAGE.wav | 1513616798_269729166.wav | `MP_ANNOUNCER_0090B_RAMPAGE_Lobby` | MP_ANNOUNCER_0090B_RAMPAGE_Lobby | We've got pandemonium! | client_attacks_lobby |
| MP_ANNOUNCER_0090C_RAMPAGE.wav | 1212006657_913688808.wav | `MP_ANNOUNCER_0090C_RAMPAGE_Lobby` | MP_ANNOUNCER_0090C_RAMPAGE_Lobby | Rampage! | client_attacks_lobby |
| MP_ANNOUNCER_0090D_RAMPAGE.wav | 624274111_696247697.wav | `MP_ANNOUNCER_0090D_RAMPAGE_Lobby` | MP_ANNOUNCER_0090D_RAMPAGE_Lobby | Is there anybody left standing! | client_attacks_lobby |
| MP_ANNOUNCER_0090E_RAMPAGE.wav | 1958788656_456424512.wav | `MP_ANNOUNCER_0090E_RAMPAGE_Lobby` | MP_ANNOUNCER_0090E_RAMPAGE_Lobby | Things are getting messy... | client_attacks_lobby |
| MP_ANNOUNCER_0090F_RAMPAGE.wav | 2708106472_306226012.wav | `MP_ANNOUNCER_0090F_RAMPAGE_Lobby` | MP_ANNOUNCER_0090F_RAMPAGE_Lobby | Brutal melee! | client_attacks_lobby |
| MP_ANNOUNCER_0090G_RAMPAGE.wav | 2708147871_268177418.wav | `MP_ANNOUNCER_0090G_RAMPAGE_Lobby` | MP_ANNOUNCER_0090G_RAMPAGE_Lobby | It's getting ugly... | client_attacks_lobby |
| MP_ANNOUNCER_0090H_RAMPAGE.wav | 1635757861_666899367.wav | `MP_ANNOUNCER_0090H_RAMPAGE_Lobby` | MP_ANNOUNCER_0090H_RAMPAGE_Lobby | The fists are flying... | client_attacks_lobby |
| MP_ANNOUNCER_0090I_RAMPAGE.wav | 14957318_403842342.wav | `MP_ANNOUNCER_0090I_RAMPAGE_Lobby` | MP_ANNOUNCER_0090I_RAMPAGE_Lobby | It's total anarchy! | client_attacks_lobby |
| COUNTDOWN_01.wav | 3404556318_6560560.wav | — (client plays it from the match clock) | countdown 1 | 1 | client_countdowns |
| COUNTDOWN_10.wav | 3387778732_953362583.wav | — (client plays it from the match clock) | countdown 10 | 10 | client_countdowns |
| COUNTDOWN_11.wav | 3387778733_965410831.wav | — (client plays it from the match clock) | countdown 11 | 11 | client_countdowns |
| COUNTDOWN_12.wav | 3387778734_839453756.wav | — (client plays it from the match clock) | countdown 12 | 12 | client_countdowns |
| COUNTDOWN_13.wav | 3387778735_292319466.wav | — (client plays it from the match clock) | countdown 13 | 13 | client_countdowns |
| COUNTDOWN_14.wav | 3387778728_534400515.wav | — (client plays it from the match clock) | countdown 14 | 14 | client_countdowns |
| COUNTDOWN_15.wav | 3387778729_256882699.wav | — (client plays it from the match clock) | countdown 15 | 15 | client_countdowns |
| COUNTDOWN_02.wav | 3404556317_960047422.wav | — (client plays it from the match clock) | countdown 2 | 2 | client_countdowns |
| COUNTDOWN_03.wav | 3404556316_809088768.wav | — (client plays it from the match clock) | countdown 3 | 3 | client_countdowns |
| COUNTDOWN_04.wav | 3404556315_829265542.wav | — (client plays it from the match clock) | countdown 4 | 4 | client_countdowns |
| COUNTDOWN_05.wav | 3404556314_435166651.wav | — (client plays it from the match clock) | countdown 5 | 5 | client_countdowns |
| COUNTDOWN_06.wav | 3404556313_336117411.wav | — (client plays it from the match clock) | countdown 6 | 6 | client_countdowns |
| COUNTDOWN_07.wav | 3404556312_701539818.wav | — (client plays it from the match clock) | countdown 7 | 7 | client_countdowns |
| COUNTDOWN_08.wav | 3404556311_6280018.wav | — (client plays it from the match clock) | countdown 8 | 8 | client_countdowns |
| COUNTDOWN_09.wav | 3404556310_256287296.wav | — (client plays it from the match clock) | countdown 9 | 9 | client_countdowns |
| MPL_ARENA_PASS_E1.wav | 1482322209_975390908.wav | `47b21118ee4bdd0` (unnamed) | mpl_arena_pass_e1 | Passes down range. | client_disc_events |
| MPL_ARENA_JOUST_E1.wav | 971877839_633693853.wav | `61aa93e1405d2d86` (unnamed) | mpl_arena_joust_e1 | High speed getaway! | client_disc_events |
| MPL_ARENA_CLEAR_A1.wav | 3644678797_222066657.wav | `MPL_ARENA_CLEAR_A1` | MPL_ARENA_CLEAR_A1 | What a clear! | client_disc_events |
| MPL_ARENA_CLEAR_B1.wav | 3980774943_474420994.wav | `MPL_ARENA_CLEAR_B1` | MPL_ARENA_CLEAR_B1 | Sends it back the other way! | client_disc_events |
| MPL_ARENA_CLEAR_C1.wav | 3668947858_768389262.wav | `MPL_ARENA_CLEAR_C1` | MPL_ARENA_CLEAR_C1 | Disc cleared! | client_disc_events |
| MPL_ARENA_CLEAR_D1.wav | 4137192624_298248286.wav | `MPL_ARENA_CLEAR_D1` | MPL_ARENA_CLEAR_D1 | The disc is moving down-field! | client_disc_events |
| MPL_ARENA_CLEAR_E1.wav | 3842138583_338216595.wav | `MPL_ARENA_CLEAR_E1` | MPL_ARENA_CLEAR_E1 | Look at that clear! | client_disc_events |
| MPL_ARENA_CLEAR_F1.wav | 2922406634_749541279.wav | `MPL_ARENA_CLEAR_F1` | MPL_ARENA_CLEAR_F1 | And we're headed back the other way. | client_disc_events |
| MPL_ARENA_CLEAR_G1.wav | 3101979128_269689590.wav | `MPL_ARENA_CLEAR_G1` | MPL_ARENA_CLEAR_G1 | Cleared the disc! | client_disc_events |
| MPL_ARENA_INTERCEPTION_A1.wav | 2320888729_796538503.wav | `MPL_ARENA_INTERCEPTION_A1` | MPL_ARENA_INTERCEPTION_A1 | INTERCEPTED! | client_disc_events |
| MPL_ARENA_INTERCEPTION_B1.wav | 213394148_422094450.wav | `MPL_ARENA_INTERCEPTION_B1` | MPL_ARENA_INTERCEPTION_B1 | What a pick! | client_disc_events |
| MPL_ARENA_JOUST_A1.wav | 589328974_789309594.wav | `MPL_ARENA_JOUST_A1` | MPL_ARENA_JOUST_A1 | Blue Team wins the Joust! | client_disc_events |
| MPL_ARENA_JOUST_B1.wav | 3720049791_640258601.wav | `MPL_ARENA_JOUST_B1` | MPL_ARENA_JOUST_B1 | Orange team wins the Joust! | client_disc_events |
| MPL_ARENA_JOUST_C1.wav | 3219857166_23307920.wav | `MPL_ARENA_JOUST_C1` | MPL_ARENA_JOUST_C1 | Blue team snags the disc! | client_disc_events |
| MPL_ARENA_JOUST_D1.wav | 2693956120_468545016.wav | `MPL_ARENA_JOUST_D1` | MPL_ARENA_JOUST_D1 | Orange team snags the disc! | client_disc_events |
| MPL_ARENA_JOUST_F1.wav | 788967509_570590452.wav | `MPL_ARENA_JOUST_F1` | MPL_ARENA_JOUST_F1 | Snatched by a blur! | client_disc_events |
| MPL_ARENA_JOUST_G1.wav | 2283237975_1028978341.wav | `MPL_ARENA_JOUST_G1` | MPL_ARENA_JOUST_G1 | What a Joust! | client_disc_events |
| MPL_ARENA_JOUST_H1.wav | 3674816358_350944910.wav | `MPL_ARENA_JOUST_H1` | MPL_ARENA_JOUST_H1 | The other team couldn't keep up! | client_disc_events |
| MPL_ARENA_PASS_A1.wav | 2995639875_529612605.wav | `MPL_ARENA_PASS_A1` | MPL_ARENA_PASS_A1 | Look at the teamwork! | client_disc_events |
| MPL_ARENA_PASS_B1.wav | 1130884264_18378291.wav | `MPL_ARENA_PASS_B1` | MPL_ARENA_PASS_B1 | Right to their teammate! | client_disc_events |
| MPL_ARENA_PASS_C1.wav | 623882362_945050588.wav | `MPL_ARENA_PASS_C1` | MPL_ARENA_PASS_C1 | A perfect pass! | client_disc_events |
| MPL_ARENA_PASS_D1.wav | 1542856472_158665535.wav | `MPL_ARENA_PASS_D1` | MPL_ARENA_PASS_D1 | What a pass! | client_disc_events |
| MPL_ARENA_PASS_F1.wav | 485564329_334262630.wav | `MPL_ARENA_PASS_F1` | MPL_ARENA_PASS_F1 | A Clean pass. | client_disc_events |
| MPL_ARENA_PASS_G1.wav | 1069955877_524448249.wav | `MPL_ARENA_PASS_G1` | MPL_ARENA_PASS_G1 | Textbook pass. | client_disc_events |
| MPL_ARENA_PASS_H1.wav | 4012050183_481980252.wav | `MPL_ARENA_PASS_H1` | MPL_ARENA_PASS_H1 | And the pass. | client_disc_events |
| MPL_ARENA_SAVE_A1.wav | 3128847550_5543243.wav | `MPL_ARENA_SAVE_A1` | MPL_ARENA_SAVE_A1 | What a save! | client_disc_events |
| MPL_ARENA_SAVE_B1.wav | 3726580796_385326609.wav | `MPL_ARENA_SAVE_B1` | MPL_ARENA_SAVE_B1 | Denied! | client_disc_events |
| MPL_ARENA_SAVE_C1.wav | 3016494939_133839426.wav | `MPL_ARENA_SAVE_C1` | MPL_ARENA_SAVE_C1 | Not today! | client_disc_events |
| MPL_ARENA_SAVE_D1.wav | 1081384650_894370598.wav | `MPL_ARENA_SAVE_D1` | MPL_ARENA_SAVE_D1 | That's what we call defense! | client_disc_events |
| MPL_ARENA_SAVE_E1.wav | 243295953_57873082.wav | `MPL_ARENA_SAVE_E1` | MPL_ARENA_SAVE_E1 | Now that's a Goalie! | client_disc_events |
| MPL_ARENA_SAVE_F1.wav | 330778333_321253358.wav | `MPL_ARENA_SAVE_F1` | MPL_ARENA_SAVE_F1 | Not this time! | client_disc_events |
| MPL_ARENA_SAVE_G1.wav | 3537507428_705644484.wav | `MPL_ARENA_SAVE_G1` | MPL_ARENA_SAVE_G1 | Who says D is a lost art? | client_disc_events |
| MPL_ARENA_SAVE_I1.wav | 588113106_904891877.wav | `MPL_ARENA_SAVE_I1` | MPL_ARENA_SAVE_I1 | Defense! Learn it, live it, love it. | client_disc_events |
| MPL_ARENA_SAVE_J1.wav | 363485948_173403515.wav | `MPL_ARENA_SAVE_J1` | MPL_ARENA_SAVE_J1 | Get that out of here! | client_disc_events |
| MPL_ARENA_SAVE_K1.wav | 1331750764_194539429.wav | `MPL_ARENA_SAVE_K1` | MPL_ARENA_SAVE_K1 | Holdin' down the fort. | client_disc_events |
| MPL_ARENA_SAVE_L1.wav | 4030802667_539796062.wav | `MPL_ARENA_SAVE_L1` | MPL_ARENA_SAVE_L1 | Shut 'em down! | client_disc_events |
| MPL_ARENA_SAVE_M1.wav | 3571031284_79706887.wav | `MPL_ARENA_SAVE_M1` | MPL_ARENA_SAVE_M1 | Not today, and probably not tomorrow! | client_disc_events |
| MPL_ARENA_SAVE_N1.wav | 1367348050_449204239.wav | `MPL_ARENA_SAVE_N1` | MPL_ARENA_SAVE_N1 | Not in my house. | client_disc_events |
| MPL_ARENA_SAVE_O1.wav | 2027501632_830619041.wav | `MPL_ARENA_SAVE_O1` | MPL_ARENA_SAVE_O1 | Return to sender. | client_disc_events |
| MPL_ARENA_SHOTONGOAL_A1.wav | 2930130532_1024011897.wav | `MPL_ARENA_SHOTONGOAL_A1` | MPL_ARENA_SHOTONGOAL_A1 | That's a shot on goal! | client_disc_events |
| MPL_ARENA_SHOTONGOAL_B1.wav | 1573997707_373996101.wav | `MPL_ARENA_SHOTONGOAL_B1` | MPL_ARENA_SHOTONGOAL_B1 | So close! | client_disc_events |
| MPL_ARENA_SHOTONGOAL_C1.wav | 2417015527_120678179.wav | `MPL_ARENA_SHOTONGOAL_C1` | MPL_ARENA_SHOTONGOAL_C1 | Only inches away! | client_disc_events |
| MPL_ARENA_SHOTONGOAL_D1.wav | 1726142511_930458036.wav | `MPL_ARENA_SHOTONGOAL_D1` | MPL_ARENA_SHOTONGOAL_D1 | Close doesn't cut it! | client_disc_events |
| MPL_ARENA_SHOTONGOAL_E1.wav | 4117014191_493066995.wav | `MPL_ARENA_SHOTONGOAL_E1` | MPL_ARENA_SHOTONGOAL_E1 | Ohh, so close! | client_disc_events |
| MPL_ARENA_SHOTONGOAL_F2.wav | 1656861286_291800588.wav | `MPL_ARENA_SHOTONGOAL_F2` | MPL_ARENA_SHOTONGOAL_F2 | Ohhh, I thought that was in! | client_disc_events |
| MPL_ARENA_STEAL_B1.wav | 1828366146_115880898.wav | `MPL_ARENA_STEAL_B1` | MPL_ARENA_STEAL_B1 | Looks like we've got a thief! | client_disc_events |
| MPL_ARENA_STEAL_C1.wav | 885196860_1031184621.wav | `MPL_ARENA_STEAL_C1` | MPL_ARENA_STEAL_C1 | Stolen! | client_disc_events |
| MPL_ARENA_STEAL_D1.wav | 3743279856_464480580.wav | `MPL_ARENA_STEAL_D1` | MPL_ARENA_STEAL_D1 | Practically gave it away! | client_disc_events |
| MPL_ARENA_STEAL_E1.wav | 2854439817_811869601.wav | `MPL_ARENA_STEAL_E1` | MPL_ARENA_STEAL_E1 | It's open season! | client_disc_events |
| MP_ANNOUNCER_0010A_INTERCEPTION_GENERIC.wav | 81730156_529728632.wav | `MP_ANNOUNCER_0010A_INTERCEPTION_GENERIC` | MP_ANNOUNCER_0010A_INTERCEPTION_RED | Interception! | client_disc_events |
| MP_ANNOUNCER_0010A_INTERCEPTION_RED.wav | 84373885_163056666.wav | `MP_ANNOUNCER_0010A_INTERCEPTION_RED` | mp_announcer_0010d_interception_red | Red catches the enemy pass! | client_disc_events |
| MP_ANNOUNCER_0010B_INTERCEPTION_GENERIC.wav | 3515238745_184112096.wav | `MP_ANNOUNCER_0010B_INTERCEPTION_GENERIC` | MP_ANNOUNCER_0010B_INTERCEPTION_RED | Picked off! | client_disc_events |
| MP_ANNOUNCER_0010B_INTERCEPTION_RED.wav | 123626504_986366715.wav | `MP_ANNOUNCER_0010B_INTERCEPTION_RED` | 7da99a5ae20ef992 | Red yanks it away! | client_disc_events |
| MP_ANNOUNCER_0010C_INTERCEPTION_GENERIC.wav | 519300714_577364211.wav | `MP_ANNOUNCER_0010C_INTERCEPTION_GENERIC` | mp_announcer_0010c_interception_red | Right to the opposing team! | client_disc_events |
| MP_ANNOUNCER_0010D_INTERCEPTION_GENERIC.wav | 3581142946_290722553.wav | `MP_ANNOUNCER_0010D_INTERCEPTION_GENERIC` | 45e75842a34c69c3 | Pulled down by the other team! | client_disc_events |
| MP_ANNOUNCER_0020A_INTERCEPTION_BLUE.wav | 595819990_407527841.wav | `MP_ANNOUNCER_0020A_INTERCEPTION_BLUE` | 653ec57c6c9ed9ad | Blue catches the enemy pass! | client_disc_events |
| MP_ANNOUNCER_0020B_INTERCEPTION_BLUE.wav | 2572733686_283696891.wav | `MP_ANNOUNCER_0020B_INTERCEPTION_BLUE` | 10d08f3fa789554b | Blue yanks it away! | client_disc_events |
| MP_ANNOUNCER_0030A_DISC_STOLEN.wav | 3400360370_455683225.wav | `MP_ANNOUNCER_0030A_DISC_STOLEN` | MP_ANNOUNCER_0030A_DISC_STOLEN | And there's the steal! | client_disc_events |
| MP_ANNOUNCER_0030B_DISC_STOLEN.wav | 3999310176_719344029.wav | `MP_ANNOUNCER_0030B_DISC_STOLEN` | MP_ANNOUNCER_0030B_DISC_STOLEN | Steal! | client_disc_events |
| MP_ANNOUNCER_0030C_DISC_STOLEN.wav | 2065839447_960518619.wav | `MP_ANNOUNCER_0030C_DISC_STOLEN` | MP_ANNOUNCER_0030C_DISC_STOLEN | Disc stolen! | client_disc_events |
| MP_ANNOUNCER_0030D_DISC_STOLEN.wav | 1968506946_965996224.wav | `MP_ANNOUNCER_0030D_DISC_STOLEN` | MP_ANNOUNCER_0030D_DISC_STOLEN | Snatched away! | client_disc_events |
| MP_ANNOUNCER_0030E_DISC_STOLEN.wav | 4186438155_134761562.wav | `MP_ANNOUNCER_0030E_DISC_STOLEN` | MP_ANNOUNCER_0030E_DISC_STOLEN | Couldn't hold onto it! | client_disc_events |
| MP_ANNOUNCER_0030F_DISC_STOLEN.wav | 2935787660_211502164.wav | `MP_ANNOUNCER_0030F_DISC_STOLEN` | MP_ANNOUNCER_0030F_DISC_STOLEN | Looks like its heading the other way! | client_disc_events |
| MP_ANNOUNCER_0110A_SAVE.wav | 631815166_942892273.wav | `MP_ANNOUNCER_0110A_SAVE` | MP_ANNOUNCER_0110A_SAVE | Save! | client_disc_events |
| MP_ANNOUNCER_0110B_SAVE.wav | 1099653048_312348647.wav | `MP_ANNOUNCER_0110B_SAVE` | MP_ANNOUNCER_0110B_SAVE | Blocked! | client_disc_events |
| MP_ANNOUNCER_0110D_SAVE.wav | 1504299662_696595924.wav | `MP_ANNOUNCER_0110D_SAVE` | MP_ANNOUNCER_0110D_SAVE | Rejected! | client_disc_events |
| MP_ANNOUNCER_0110E_SAVE.wav | 754194867_919077591.wav | `MP_ANNOUNCER_0110E_SAVE` | MP_ANNOUNCER_0110E_SAVE | The shot was blocked! | client_disc_events |
| MP_ANNOUNCER_0110F_SAVE.wav | 1710322667_288425356.wav | `MP_ANNOUNCER_0110F_SAVE` | MP_ANNOUNCER_0110F_SAVE | What a save! | client_disc_events |
| MP_ANNOUNCER_0110G_SAVE.wav | 2491935063_473893653.wav | `MP_ANNOUNCER_0110G_SAVE` | MP_ANNOUNCER_0110G_SAVE | Get that out of here! | client_disc_events |
| MP_ANNOUNCER_0110H_SAVE.wav | 2829325885_687807223.wav | `MP_ANNOUNCER_0110H_SAVE` | MP_ANNOUNCER_0110H_SAVE | Knocked wide. | client_disc_events |
| MP_ANNOUNCER_0110I_SAVE.wav | 2091250556_815660110.wav | `MP_ANNOUNCER_0110I_SAVE` | MP_ANNOUNCER_0110I_SAVE | Not this time... | client_disc_events |
| MP_ANNOUNCER_0110J_SAVE.wav | 148163971_919769879.wav | `MP_ANNOUNCER_0110J_SAVE` | MP_ANNOUNCER_0110J_SAVE | Great job guarding the goal. | client_disc_events |
| MP_ANNOUNCER_0110K_SAVE.wav | 2536237543_843475632.wav | `MP_ANNOUNCER_0110K_SAVE` | MP_ANNOUNCER_0110K_SAVE | Perfect defense! | client_disc_events |
| MP_ANNOUNCER_0130A_CHANGE_POSSESSION_RED.wav | 3715243134_753205522.wav | `MP_ANNOUNCER_0130A_CHANGE_POSSESSION_RED` | MP_ANNOUNCER_0130A_CHANGE_POSSESSION_RED | Red team has the disc. | client_disc_events |
| MP_ANNOUNCER_0130B_CHANGE_POSSESSION_RED.wav | 3414659588_334693713.wav | `MP_ANNOUNCER_0130B_CHANGE_POSSESSION_RED` | MP_ANNOUNCER_0130B_CHANGE_POSSESSION_RED | Red picks up the disc. | client_disc_events |
| MP_ANNOUNCER_0130C_CHANGE_POSSESSION_RED.wav | 3863151142_1039449330.wav | `MP_ANNOUNCER_0130C_CHANGE_POSSESSION_RED` | MP_ANNOUNCER_0130C_CHANGE_POSSESSION_RED | Red with the disc. | client_disc_events |
| MP_ANNOUNCER_0130D_CHANGE_POSSESSION_RED.wav | 419084236_443951188.wav | `MP_ANNOUNCER_0130D_CHANGE_POSSESSION_RED` | MP_ANNOUNCER_0130D_CHANGE_POSSESSION_RED | Red team takes possession. | client_disc_events |
| MP_ANNOUNCER_0130E_CHANGE_POSSESSION_RED.wav | 3155557204_97412260.wav | `MP_ANNOUNCER_0130E_CHANGE_POSSESSION_RED` | MP_ANNOUNCER_0130E_CHANGE_POSSESSION_RED | Disc into the hands of red. | client_disc_events |
| MP_ANNOUNCER_0130F_CHANGE_POSSESSION_RED.wav | 2500476960_571909770.wav | `MP_ANNOUNCER_0130F_CHANGE_POSSESSION_RED` | MP_ANNOUNCER_0130F_CHANGE_POSSESSION_RED | Possession moves to red team. | client_disc_events |
| MP_ANNOUNCER_0130G_CHANGE_POSSESSION_RED.wav | 771480687_128401811.wav | `MP_ANNOUNCER_0130G_CHANGE_POSSESSION_RED` | MP_ANNOUNCER_0130G_CHANGE_POSSESSION_RED | Great pickup by red. | client_disc_events |
| MP_ANNOUNCER_0130H_CHANGE_POSSESSION_RED.wav | 1044366089_147612971.wav | `MP_ANNOUNCER_0130H_CHANGE_POSSESSION_RED` | MP_ANNOUNCER_0130H_CHANGE_POSSESSION_RED | Red gets their hands on it. | client_disc_events |
| MP_ANNOUNCER_0140A_CHANGE_POSSESSION_BLUE.wav | 1377419127_695810909.wav | `MP_ANNOUNCER_0140A_CHANGE_POSSESSION_BLUE` | MP_ANNOUNCER_0140A_CHANGE_POSSESSION_BLUE | Blue team has the disc. | client_disc_events |
| MP_ANNOUNCER_0140C_CHANGE_POSSESSION_BLUE.wav | 24258001_794548899.wav | `MP_ANNOUNCER_0140C_CHANGE_POSSESSION_BLUE` | MP_ANNOUNCER_0140C_CHANGE_POSSESSION_BLUE | Blue with the disc. | client_disc_events |
| MP_ANNOUNCER_0140D_CHANGE_POSSESSION_BLUE.wav | 3600984073_718091776.wav | `MP_ANNOUNCER_0140D_CHANGE_POSSESSION_BLUE` | MP_ANNOUNCER_0140D_CHANGE_POSSESSION_BLUE | Blue team takes possession. | client_disc_events |
| MP_ANNOUNCER_0140E_CHANGE_POSSESSION_BLUE.wav | 1627845798_919827823.wav | `MP_ANNOUNCER_0140E_CHANGE_POSSESSION_BLUE` | MP_ANNOUNCER_0140E_CHANGE_POSSESSION_BLUE | Disc into the hands of blue. | client_disc_events |
| MP_ANNOUNCER_0140F_CHANGE_POSSESSION_BLUE.wav | 2612665109_232891612.wav | `MP_ANNOUNCER_0140F_CHANGE_POSSESSION_BLUE` | MP_ANNOUNCER_0140F_CHANGE_POSSESSION_BLUE | Possession moves to blue team. | client_disc_events |
| MP_ANNOUNCER_0140G_CHANGE_POSSESSION_BLUE.wav | 941219011_1012155564.wav | `MP_ANNOUNCER_0140G_CHANGE_POSSESSION_BLUE` | MP_ANNOUNCER_0140G_CHANGE_POSSESSION_BLUE | Great pickup by blue. | client_disc_events |
| MP_ANNOUNCER_0140H_CHANGE_POSSESSION_BLUE.wav | 856745199_412233770.wav | `MP_ANNOUNCER_0140H_CHANGE_POSSESSION_BLUE` | MP_ANNOUNCER_0140H_CHANGE_POSSESSION_BLUE | Blue gets their hands on it. | client_disc_events |
| MP_ANNOUNCER_0150A_CLEAR_DISC_RED.wav | 861259927_852200813.wav | `MP_ANNOUNCER_0150A_CLEAR_DISC_RED` | MP_ANNOUNCER_0150A_CLEAR_DISC_RED | Red clears it out. | client_disc_events |
| MP_ANNOUNCER_0150B_CLEAR_DISC_RED.wav | 4021203951_140414174.wav | `MP_ANNOUNCER_0150B_CLEAR_DISC_RED` | MP_ANNOUNCER_0150B_CLEAR_DISC_RED | Red throws it way down field. | client_disc_events |
| MP_ANNOUNCER_0150C_CLEAR_DISC_RED.wav | 457193336_710186581.wav | `MP_ANNOUNCER_0150C_CLEAR_DISC_RED` | MP_ANNOUNCER_0150C_CLEAR_DISC_RED | Red clears it out of their zone. | client_disc_events |
| MP_ANNOUNCER_0150D_CLEAR_DISC_BLUE.wav | 81745644_907504341.wav | `MP_ANNOUNCER_0150D_CLEAR_DISC_BLUE` | MP_ANNOUNCER_0150D_CLEAR_DISC_BLUE | It's headed back down field. | client_disc_events |
| MP_ANNOUNCER_0150D_CLEAR_DISC_RED.wav | 478244926_785902632.wav | `MP_ANNOUNCER_0150D_CLEAR_DISC_RED` | MP_ANNOUNCER_0150D_CLEAR_DISC_RED | It's headed back down field. | client_disc_events |
| MP_ANNOUNCER_0150E_CLEAR_DISC_RED.wav | 2938512585_694492099.wav | `MP_ANNOUNCER_0150E_CLEAR_DISC_RED` | MP_ANNOUNCER_0150E_CLEAR_DISC_RED | Red sends it back the other way. | client_disc_events |
| MP_ANNOUNCER_0160A_CLEAR_DISC_BLUE.wav | 848085028_781953649.wav | `MP_ANNOUNCER_0160A_CLEAR_DISC_BLUE` | MP_ANNOUNCER_0160A_CLEAR_DISC_BLUE | Blue clears it out. | client_disc_events |
| MP_ANNOUNCER_0160B_CLEAR_DISC_BLUE.wav | 1274606390_100708163.wav | `MP_ANNOUNCER_0160B_CLEAR_DISC_BLUE` | MP_ANNOUNCER_0160B_CLEAR_DISC_BLUE | Blue throws it way down field. | client_disc_events |
| MP_ANNOUNCER_0160C_CLEAR_DISC_BLUE.wav | 1256948779_883318100.wav | `MP_ANNOUNCER_0160C_CLEAR_DISC_BLUE` | MP_ANNOUNCER_0160C_CLEAR_DISC_BLUE | Blue clears it out of their zone. | client_disc_events |
| MP_ANNOUNCER_0160E_CLEAR_DISC_BLUE.wav | 45447077_808698022.wav | `MP_ANNOUNCER_0160E_CLEAR_DISC_BLUE` | MP_ANNOUNCER_0160E_CLEAR_DISC_BLUE | Blue sends it back the other way. | client_disc_events |
| MP_ANNOUNCER_0170A_HIT_POST.wav | 3680239332_708844020.wav | `MP_ANNOUNCER_0170A_HIT_POST` | MP_ANNOUNCER_0170A_HIT_POST | Ohhh... | client_disc_events |
| MP_ANNOUNCER_0170B_HIT_POST.wav | 1381890601_862518666.wav | `MP_ANNOUNCER_0170B_HIT_POST` | MP_ANNOUNCER_0170B_HIT_POST | Shot wide... | client_disc_events |
| MP_ANNOUNCER_0170C_HIT_POST.wav | 3443884701_624338906.wav | `MP_ANNOUNCER_0170C_HIT_POST` | MP_ANNOUNCER_0170C_HIT_POST | No good... | client_disc_events |
| MP_ANNOUNCER_0170D_HIT_POST.wav | 1031103902_957138691.wav | `MP_ANNOUNCER_0170D_HIT_POST` | MP_ANNOUNCER_0170D_HIT_POST | No dice... | client_disc_events |
| MP_ANNOUNCER_0170E_HIT_POST.wav | 2991880028_210466124.wav | `MP_ANNOUNCER_0170E_HIT_POST` | MP_ANNOUNCER_0170E_HIT_POST | So close... | client_disc_events |
| MP_ANNOUNCER_0170F_HIT_POST.wav | 2393911649_17769936.wav | `MP_ANNOUNCER_0170F_HIT_POST` | MP_ANNOUNCER_0170F_HIT_POST | Not quite... | client_disc_events |
| MP_ANNOUNCER_0170G_HIT_POST.wav | 3958125395_34581705.wav | `MP_ANNOUNCER_0170G_HIT_POST` | MP_ANNOUNCER_0170G_HIT_POST | Doesn't quite come up with it... | client_disc_events |
| MP_ANNOUNCER_0170H_HIT_POST.wav | 2989642598_619725895.wav | `MP_ANNOUNCER_0170H_HIT_POST` | MP_ANNOUNCER_0170H_HIT_POST | Comes up short... | client_disc_events |
| MP_ANNOUNCER_0170I_HIT_POST.wav | 571119254_379572303.wav | `MP_ANNOUNCER_0170I_HIT_POST` | MP_ANNOUNCER_0170I_HIT_POST | Not this time. | client_disc_events |
| MP_ANNOUNCER_0170J_HIT_POST.wav | 1589251887_493489245.wav | `MP_ANNOUNCER_0170J_HIT_POST` | MP_ANNOUNCER_0170J_HIT_POST | Hits the post! | client_disc_events |
| MP_ANNOUNCER_0170K_HIT_POST.wav | 3701837075_883692122.wav | `MP_ANNOUNCER_0170K_HIT_POST` | MP_ANNOUNCER_0170K_HIT_POST | Bounces off! | client_disc_events |
| MP_ANNOUNCER_0170L_HIT_POST.wav | 2134176184_833968729.wav | `MP_ANNOUNCER_0170L_HIT_POST` | MP_ANNOUNCER_0170L_HIT_POST | Off the rim! | client_disc_events |
| NOCUE_BLUE_PICKS_UP_THE_DISC.wav | 3672010789_350503264.wav | — | ffffffffffffffff | Blue picks up the disc. | client_disc_events |
| MPL_ARENA_2POINT_A1.wav | 2092338177_675755510.wav | `MPL_ARENA_2POINT_A1` | MPL_ARENA_2POINT_A1 | 2 points on the board! | client_points |
| MPL_ARENA_2POINT_B1.wav | 1242380213_522189239.wav | `MPL_ARENA_2POINT_B1` | MPL_ARENA_2POINT_B1 | Mark it down for 2 points! | client_points |
| MPL_ARENA_2POINT_C1.wav | 1427544740_76174328.wav | `MPL_ARENA_2POINT_C1` | MPL_ARENA_2POINT_C1 | Put 'em down for 2! | client_points |
| MPL_ARENA_2POINT_D1.wav | 4291475056_488612460.wav | `MPL_ARENA_2POINT_D1` | MPL_ARENA_2POINT_D1 | Biiiingo! | client_points |
| MPL_ARENA_3POINT_A1.wav | 426615352_484820591.wav | `MPL_ARENA_3POINT_A1` | MPL_ARENA_3POINT_A1 | That's a long shot for 3 points! | client_points |
| MPL_ARENA_3POINT_B1.wav | 4099613898_130578247.wav | `MPL_ARENA_3POINT_B1` | MPL_ARENA_3POINT_B1 | From downtown for 3 points! | client_points |
| MPL_ARENA_3POINT_C1.wav | 1947107395_936929027.wav | `MPL_ARENA_3POINT_C1` | MPL_ARENA_3POINT_C1 | Sniper shot for 3 points! | client_points |
| MPL_ARENA_3POINT_D1.wav | 2788153211_129132048.wav | `MPL_ARENA_3POINT_D1` | MPL_ARENA_3POINT_D1 | 3 points never looked so pretty! | client_points |
| MPL_ARENA_3POINT_E1.wav | 3945827738_1026643417.wav | `MPL_ARENA_3POINT_E1` | MPL_ARENA_3POINT_E1 | From way downtown for three! | client_points |
| MPL_ARENA_3POINT_F1.wav | 130249026_495860259.wav | `MPL_ARENA_3POINT_F1` | MPL_ARENA_3POINT_F1 | I think he shot that from orbit! | client_points |
| MPL_ARENA_3POINT_G1.wav | 2193920001_1039310232.wav | `MPL_ARENA_3POINT_G1` | MPL_ARENA_3POINT_G1 | See the three.  Be the three. | client_points |
| MPL_ARENA_3POINT_H1.wav | 461750884_603526636.wav | `MPL_ARENA_3POINT_H1` | MPL_ARENA_3POINT_H1 | He eyes it.  He tries it.  He buys it. | client_points |
| MPL_ARENA_3POINT_I1.wav | 2619607742_422926461.wav | `MPL_ARENA_3POINT_I1` | MPL_ARENA_3POINT_I1 | No hocus pocus, just focus on that shot. | client_points |
| MPL_ARENA_3POINT_J1.wav | 2661447843_764249655.wav | `MPL_ARENA_3POINT_J1` | MPL_ARENA_3POINT_J1 | Boo-yah! | client_points |
| MPL_ARENA_3POINT_K1.wav | 2149928665_401260035.wav | `MPL_ARENA_3POINT_K1` | MPL_ARENA_3POINT_K1 | Gooooooooooaaaaaaaaaaaaallll! | client_points |
| MPL_ARENA_3POINT_L1.wav | 2751235172_413114059.wav | `MPL_ARENA_3POINT_L1` | MPL_ARENA_3POINT_L1 | That bot will not be denied! | client_points |
| MPL_ARENA_BOUNCE_A1.wav | 1175878152_937868690.wav | `MPL_ARENA_BOUNCE_A1` | MPL_ARENA_BOUNCE_A1 | Welcome to the bounce house! | client_points |
| MPL_ARENA_BOUNCE_B1.wav | 3658236802_80316290.wav | `MPL_ARENA_BOUNCE_B1` | MPL_ARENA_BOUNCE_B1 | That required some practice! | client_points |
| MPL_ARENA_BOUNCE_C1.wav | 1428013800_66580826.wav | `MPL_ARENA_BOUNCE_C1` | MPL_ARENA_BOUNCE_C1 | Someone studied geometry! | client_points |
| MPL_ARENA_BOUNCE_D1.wav | 1833798477_254237752.wav | `MPL_ARENA_BOUNCE_D1` | MPL_ARENA_BOUNCE_D1 | What do androids dream of? Shots like that!  | client_points |
| MPL_ARENA_BOUNCE_E1.wav | 4098520935_849663950.wav | `MPL_ARENA_BOUNCE_E1` | MPL_ARENA_BOUNCE_E1 | That's one for the highlight reel! | client_points |
| MPL_ARENA_BOUNCE_F1.wav | 469749302_982468869.wav | `MPL_ARENA_BOUNCE_F1` | MPL_ARENA_BOUNCE_F1 | I'm not sure what that shot was, but it tastes like chicken. | client_points |
| MPL_ARENA_BOUNCE_G1.wav | 557192631_714967492.wav | `MPL_ARENA_BOUNCE_G1` | MPL_ARENA_BOUNCE_G1 | Oh me oh my! | client_points |
| MPL_ARENA_BOUNCE_H1.wav | 276245463_296470085.wav | `MPL_ARENA_BOUNCE_H1` | MPL_ARENA_BOUNCE_H1 | If you don't like that, you don't like VR sports. | client_points |
| MPL_ARENA_BOUNCE_I1.wav | 525054825_283284658.wav | `MPL_ARENA_BOUNCE_I1` | MPL_ARENA_BOUNCE_I1 | Threaded the needle! | client_points |
| MPL_ARENA_DUNK_A1.wav | 465632466_516092837.wav | `MPL_ARENA_DUNK_A1` | MPL_ARENA_DUNK_A1 | What a dunk! | client_points |
| MPL_ARENA_DUNK_B1.wav | 57125158_1033062882.wav | `MPL_ARENA_DUNK_B1` | MPL_ARENA_DUNK_B1 | That's a dunk! | client_points |
| MPL_ARENA_DUNK_C1.wav | 555593518_863865276.wav | `MPL_ARENA_DUNK_C1` | MPL_ARENA_DUNK_C1 | 2 points for the dunk! | client_points |
| MPL_ARENA_DUNK_D1.wav | 3551695516_538011452.wav | `MPL_ARENA_DUNK_D1` | MPL_ARENA_DUNK_D1 | BOOM. | client_points |
| MPL_ARENA_DUNK_E1.wav | 3662765401_109199024.wav | `MPL_ARENA_DUNK_E1` | MPL_ARENA_DUNK_E1 | Special delivery! | client_points |
| MPL_ARENA_DUNK_F1.wav | 888871049_639995033.wav | `MPL_ARENA_DUNK_F1` | MPL_ARENA_DUNK_F1 | Straight to the goal for two! | client_points |
| MPL_ARENA_DUNK_G1.wav | 4257569918_893169925.wav | `MPL_ARENA_DUNK_G1` | MPL_ARENA_DUNK_G1 | Dipsy-doo Dunk-a-roo. | client_points |
| MPL_ARENA_DUNK_H1.wav | 2269279600_901861800.wav | `MPL_ARENA_DUNK_H1` | MPL_ARENA_DUNK_H1 | Boom goes the dynamite! | client_points |
| MPL_ARENA_DUNK_I1.wav | 3019854868_457726092.wav | `MPL_ARENA_DUNK_I1` | MPL_ARENA_DUNK_I1 | BAAAAANG!  That one goes in. | client_points |
| MPL_ARENA_TRICK_A1.wav | 231526688_748472821.wav | `MPL_ARENA_TRICK_A1` | MPL_ARENA_TRICK_A1 | Hat Trick! | client_points |
| MPL_ARENA_TRICK_B1.wav | 3491092290_126422729.wav | `MPL_ARENA_TRICK_B1` | MPL_ARENA_TRICK_B1 | Looks like we've got a show-off! | client_points |
| MPL_ARENA_TRICK_C1.wav | 3867978788_506727972.wav | `MPL_ARENA_TRICK_C1` | MPL_ARENA_TRICK_C1 | UNSTOPPABLE! | client_points |
| MPL_ARENA_TRICK_D1.wav | 1351757679_123746744.wav | `MPL_ARENA_TRICK_D1` | MPL_ARENA_TRICK_D1 | Three in a row!  | client_points |
| MPL_ARENA_TRICK_E1.wav | 3603499687_165415460.wav | `MPL_ARENA_TRICK_E1` | MPL_ARENA_TRICK_E1 | Ok now that's just showing off! | client_points |
| MP_ANNOUNCER_0070A_RED_TEAM_WINS.wav | 3659254474_920350323.wav | `MP_ANNOUNCER_0070A_RED_TEAM_WINS` | MP_ANNOUNCER_0070A_RED_TEAM_WINS | Red team wins! | client_points |
| MP_ANNOUNCER_0070B_RED_TEAM_WINS.wav | 4088933334_516592391.wav | `MP_ANNOUNCER_0070B_RED_TEAM_WINS` | MP_ANNOUNCER_0070B_RED_TEAM_WINS | Red team is victorious! | client_points |
| MP_ANNOUNCER_0070C_RED_TEAM_WINS.wav | 2978548606_102287017.wav | `MP_ANNOUNCER_0070C_RED_TEAM_WINS` | MP_ANNOUNCER_0070C_RED_TEAM_WINS | Red has it this time! | client_points |
| MP_ANNOUNCER_0070D_RED_TEAM_WINS.wav | 1927350903_365965126.wav | `MP_ANNOUNCER_0070D_RED_TEAM_WINS` | MP_ANNOUNCER_0070D_RED_TEAM_WINS | And red takes the win! | client_points |
| MP_ANNOUNCER_0070E_RED_TEAM_WINS.wav | 3286123828_770354784.wav | `MP_ANNOUNCER_0070E_RED_TEAM_WINS` | MP_ANNOUNCER_0070E_RED_TEAM_WINS | Red is walking away with the W. | client_points |
| MP_ANNOUNCER_0070F_RED_TEAM_WINS.wav | 1066496163_54400885.wav | `MP_ANNOUNCER_0070F_RED_TEAM_WINS` | MP_ANNOUNCER_0070F_RED_TEAM_WINS | Chalk up a win for red! | client_points |
| MP_ANNOUNCER_0070G_RED_TEAM_WINS.wav | 3087854552_399602010.wav | `MP_ANNOUNCER_0070G_RED_TEAM_WINS` | MP_ANNOUNCER_0070G_RED_TEAM_WINS | Red takes the win! | client_points |
| MP_ANNOUNCER_0070H_RED_TEAM_WINS.wav | 1414012771_397184504.wav | `MP_ANNOUNCER_0070H_RED_TEAM_WINS` | MP_ANNOUNCER_0070H_RED_TEAM_WINS | Red comes out on top! | client_points |
| MP_ANNOUNCER_0080A_BLUE_TEAM_WINS.wav | 468675032_1009266453.wav | `MP_ANNOUNCER_0080A_BLUE_TEAM_WINS` | MP_ANNOUNCER_0080A_BLUE_TEAM_WINS | Blue team wins! | client_points |
| MP_ANNOUNCER_0080B_BLUE_TEAM_WINS.wav | 3556913197_613892843.wav | `MP_ANNOUNCER_0080B_BLUE_TEAM_WINS` | MP_ANNOUNCER_0080B_BLUE_TEAM_WINS | Blue team is victorious! | client_points |
| MP_ANNOUNCER_0080C_BLUE_TEAM_WINS.wav | 1548658223_49509098.wav | `MP_ANNOUNCER_0080C_BLUE_TEAM_WINS` | MP_ANNOUNCER_0080C_BLUE_TEAM_WINS | Blue has it this time! | client_points |
| MP_ANNOUNCER_0080D_BLUE_TEAM_WINS.wav | 3397332015_849767799.wav | `MP_ANNOUNCER_0080D_BLUE_TEAM_WINS` | MP_ANNOUNCER_0080D_BLUE_TEAM_WINS | And blue takes the win! | client_points |
| MP_ANNOUNCER_0080E_BLUE_TEAM_WINS.wav | 1830836914_447512344.wav | `MP_ANNOUNCER_0080E_BLUE_TEAM_WINS` | MP_ANNOUNCER_0080E_BLUE_TEAM_WINS | Blue is walking away with the W. | client_points |
| MP_ANNOUNCER_0080F_BLUE_TEAM_WINS.wav | 1836669950_180611828.wav | `MP_ANNOUNCER_0080F_BLUE_TEAM_WINS` | MP_ANNOUNCER_0080F_BLUE_TEAM_WINS | Chalk up a win for blue! | client_points |
| MP_ANNOUNCER_0080G_BLUE_TEAM_WINS.wav | 3562963086_876483186.wav | `MP_ANNOUNCER_0080G_BLUE_TEAM_WINS` | MP_ANNOUNCER_0080G_BLUE_TEAM_WINS | Blue takes the win! | client_points |
| MP_ANNOUNCER_0080H_BLUE_TEAM_WINS.wav | 1946793390_1056691158.wav | `MP_ANNOUNCER_0080H_BLUE_TEAM_WINS` | MP_ANNOUNCER_0080H_BLUE_TEAM_WINS | Blue comes out on top! | client_points |
| MP_ANNOUNCER_0100A_DRAW.wav | 378614861_131420554.wav | `MP_ANNOUNCER_0100A_DRAW` | MP_ANNOUNCER_0100A_DRAW | It's a tie! | client_points |
| MP_ANNOUNCER_0100B_DRAW.wav | 2145437044_167961075.wav | `MP_ANNOUNCER_0100B_DRAW` | MP_ANNOUNCER_0100B_DRAW | Draw game! | client_points |
| MP_ANNOUNCER_0100C_DRAW.wav | 455331593_838356575.wav | `MP_ANNOUNCER_0100C_DRAW` | MP_ANNOUNCER_0100C_DRAW | Very even match... | client_points |
| MP_ANNOUNCER_0100D_DRAW.wav | 1894196707_377295643.wav | `MP_ANNOUNCER_0100D_DRAW` | MP_ANNOUNCER_0100D_DRAW | The scores are tied... | client_points |
| MP_ANNOUNCER_0100E_DRAW.wav | 2552430596_506599329.wav | `MP_ANNOUNCER_0100E_DRAW` | MP_ANNOUNCER_0100E_DRAW | No winner this time... | client_points |
| MP_ANNOUNCER_0100F_DRAW.wav | 1518919212_760526163.wav | `MP_ANNOUNCER_0100F_DRAW` | MP_ANNOUNCER_0100F_DRAW | This one ends in a draw... | client_points |
| MP_ANNOUNCER_0180A_GOAL_1_POINT.wav | 874977314_603296851.wav | `MP_ANNOUNCER_0180A_GOAL_1_POINT` | MP_ANNOUNCER_0180A_GOAL_1_POINT | It's good! | client_points |
| MP_ANNOUNCER_0180B_GOAL_1_POINT.wav | 2527792713_912845050.wav | `MP_ANNOUNCER_0180B_GOAL_1_POINT` | MP_ANNOUNCER_0180B_GOAL_1_POINT | Goal! | client_points |
| MP_ANNOUNCER_0180C_GOAL_1_POINT.wav | 1937294921_898389737.wav | `MP_ANNOUNCER_0180C_GOAL_1_POINT` | MP_ANNOUNCER_0180C_GOAL_1_POINT | Across the line! | client_points |
| MP_ANNOUNCER_0180D_GOAL_1_POINT.wav | 3730013163_777825760.wav | `MP_ANNOUNCER_0180D_GOAL_1_POINT` | MP_ANNOUNCER_0180D_GOAL_1_POINT | It's in there! | client_points |
| MP_ANNOUNCER_0180E_GOAL_1_POINT.wav | 1480562999_429561243.wav | `MP_ANNOUNCER_0180E_GOAL_1_POINT` | MP_ANNOUNCER_0180E_GOAL_1_POINT | And it hits the target! | client_points |
| MP_ANNOUNCER_0180F_GOAL_1_POINT.wav | 4046683007_981819688.wav | `MP_ANNOUNCER_0180F_GOAL_1_POINT` | MP_ANNOUNCER_0180F_GOAL_1_POINT | Score! | client_points |
| MP_ANNOUNCER_0180G_GOAL_1_POINT.wav | 2536222642_304539611.wav | `MP_ANNOUNCER_0180G_GOAL_1_POINT` | MP_ANNOUNCER_0180G_GOAL_1_POINT | Way to bring it home! | client_points |
| MP_ANNOUNCER_0180H_GOAL_1_POINT.wav | 2044982400_777295190.wav | `MP_ANNOUNCER_0180H_GOAL_1_POINT` | MP_ANNOUNCER_0180H_GOAL_1_POINT | Great throw! | client_points |
| MP_ANNOUNCER_0180I_GOAL_1_POINT.wav | 1744622362_749395622.wav | `MP_ANNOUNCER_0180I_GOAL_1_POINT` | MP_ANNOUNCER_0180I_GOAL_1_POINT | Tipped in! | client_points |
| MP_ANNOUNCER_0180J_GOAL_1_POINT.wav | 760795942_184239176.wav | `MP_ANNOUNCER_0180J_GOAL_1_POINT` | MP_ANNOUNCER_0180J_GOAL_1_POINT | From point blank! | client_points |
| MP_ANNOUNCER_0190A_GOAL_2_POINT.wav | 392406748_552018562.wav | `MP_ANNOUNCER_0190A_GOAL_2_POINT` | MP_ANNOUNCER_0190A_GOAL_2_POINT | It's good! | client_points |
| MP_ANNOUNCER_0190B_GOAL_2_POINT.wav | 2145102487_232665064.wav | `MP_ANNOUNCER_0190B_GOAL_2_POINT` | MP_ANNOUNCER_0190B_GOAL_2_POINT | Goal! | client_points |
| MP_ANNOUNCER_0190C_GOAL_2_POINT.wav | 1530933981_1008019736.wav | `MP_ANNOUNCER_0190C_GOAL_2_POINT` | MP_ANNOUNCER_0190C_GOAL_2_POINT | Across the line! | client_points |
| MP_ANNOUNCER_0190D_GOAL_2_POINT.wav | 992162122_204195449.wav | `MP_ANNOUNCER_0190D_GOAL_2_POINT` | MP_ANNOUNCER_0190D_GOAL_2_POINT | It's in there! | client_points |
| MP_ANNOUNCER_0190E_GOAL_2_POINT.wav | 2099842083_1013707069.wav | `MP_ANNOUNCER_0190E_GOAL_2_POINT` | MP_ANNOUNCER_0190E_GOAL_2_POINT | And it hits the target! | client_points |
| MP_ANNOUNCER_0190F_GOAL_2_POINT.wav | 1140801118_501962526.wav | `MP_ANNOUNCER_0190F_GOAL_2_POINT` | MP_ANNOUNCER_0190F_GOAL_2_POINT | Score! | client_points |
| MP_ANNOUNCER_0190G_GOAL_2_POINT.wav | 4046653046_880766077.wav | `MP_ANNOUNCER_0190G_GOAL_2_POINT` | MP_ANNOUNCER_0190G_GOAL_2_POINT | Way to bring it home! | client_points |
| MP_ANNOUNCER_0190H_GOAL_2_POINT.wav | 1520673377_14752462.wav | `MP_ANNOUNCER_0190H_GOAL_2_POINT` | MP_ANNOUNCER_0190H_GOAL_2_POINT | Great throw! | client_points |
| MP_ANNOUNCER_0190I_GOAL_2_POINT.wav | 4284706672_380468742.wav | `MP_ANNOUNCER_0190I_GOAL_2_POINT` | MP_ANNOUNCER_0190I_GOAL_2_POINT | Two points! | client_points |
| MP_ANNOUNCER_0190J_GOAL_2_POINT.wav | 2283319784_1043809305.wav | `MP_ANNOUNCER_0190J_GOAL_2_POINT` | MP_ANNOUNCER_0190J_GOAL_2_POINT | Two more on the board! | client_points |
| MP_ANNOUNCER_0200A_GOAL_3_POINT.wav | 984382760_420147128.wav | `MP_ANNOUNCER_0200A_GOAL_3_POINT` | MP_ANNOUNCER_0200A_GOAL_3_POINT | Three points! | client_points |
| MP_ANNOUNCER_0200B_GOAL_3_POINT.wav | 3004468858_658778739.wav | `MP_ANNOUNCER_0200B_GOAL_3_POINT` | MP_ANNOUNCER_0200B_GOAL_3_POINT | What a shot! | client_points |
| MP_ANNOUNCER_0200C_GOAL_3_POINT.wav | 3387701577_215507830.wav | `MP_ANNOUNCER_0200C_GOAL_3_POINT` | MP_ANNOUNCER_0200C_GOAL_3_POINT | Special delivery! | client_points |
| MP_ANNOUNCER_0200D_GOAL_3_POINT.wav | 3146224869_929981552.wav | `MP_ANNOUNCER_0200D_GOAL_3_POINT` | MP_ANNOUNCER_0200D_GOAL_3_POINT | That's three! | client_points |
| MP_ANNOUNCER_0200E_GOAL_3_POINT.wav | 263011340_863131813.wav | `MP_ANNOUNCER_0200E_GOAL_3_POINT` | MP_ANNOUNCER_0200E_GOAL_3_POINT | Three more on the board! | client_points |
| MP_ANNOUNCER_0200F_GOAL_3_POINT.wav | 793282167_751329092.wav | `MP_ANNOUNCER_0200F_GOAL_3_POINT` | MP_ANNOUNCER_0200F_GOAL_3_POINT | Oh-em-zero-gee! | client_points |
| MP_ANNOUNCER_0200G_GOAL_3_POINT.wav | 3406973773_209288124.wav | `MP_ANNOUNCER_0200G_GOAL_3_POINT` | MP_ANNOUNCER_0200G_GOAL_3_POINT | It's in! Epic throw! | client_points |
| MP_ANNOUNCER_001A_OVERTIME.wav | 1578145190_743758748.wav | — | ffffffffffffffff | OT | client_points |
| MP_ANNOUNCER_001A_OVERTIME.wav | 1578145190_743758748.wav | `MP_ANNOUNCER_001A_OVERTIME` | MP_ANNOUNCER_001A_OVERTIME | O.T. | client_progress |
| MP_ANNOUNCER_0050A_ROUND_START.wav | 2269512976_660506849.wav | `MP_ANNOUNCER_0050A_ROUND_START` | MP_ANNOUNCER_0050A_ROUND_START | The round has begun... | client_progress |
| MP_ANNOUNCER_0050B_ROUND_START.wav | 1241832046_246091317.wav | `MP_ANNOUNCER_0050B_ROUND_START` | MP_ANNOUNCER_0050B_ROUND_START | Here we go... | client_progress |
| MP_ANNOUNCER_0050C_ROUND_START.wav | 277829360_1015239073.wav | `MP_ANNOUNCER_0050C_ROUND_START` | MP_ANNOUNCER_0050C_ROUND_START | And we're under way... | client_progress |
| MP_ANNOUNCER_0050D_ROUND_START.wav | 730735660_606917111.wav | `MP_ANNOUNCER_0050D_ROUND_START` | MP_ANNOUNCER_0050D_ROUND_START | Teams are entering the arena. | client_progress |
| MP_ANNOUNCER_0050E_ROUND_START.wav | 3168940348_56574528.wav | `MP_ANNOUNCER_0050E_ROUND_START` | MP_ANNOUNCER_0050E_ROUND_START | It's game on... | client_progress |
| MP_ANNOUNCER_0050F_ROUND_START.wav | 2053519961_646164462.wav | `MP_ANNOUNCER_0050F_ROUND_START` | MP_ANNOUNCER_0050F_ROUND_START | Teams have cleared the launchers! | client_progress |
| MP_ANNOUNCER_0060A_MATCH_OVER.wav | 3060918919_809431569.wav | `MP_ANNOUNCER_0060A_MATCH_OVER` | MP_ANNOUNCER_0060A_MATCH_OVER | And that's the match! | client_progress |
| MP_ANNOUNCER_0060B_MATCH_OVER.wav | 1230803398_394869175.wav | `MP_ANNOUNCER_0060B_MATCH_OVER` | MP_ANNOUNCER_0060B_MATCH_OVER | That's all she wrote... | client_progress |
| MP_ANNOUNCER_0060C_MATCH_OVER.wav | 3575853839_715321887.wav | `MP_ANNOUNCER_0060C_MATCH_OVER` | MP_ANNOUNCER_0060C_MATCH_OVER | And that's game... | client_progress |
| MP_ANNOUNCER_0060D_MATCH_OVER.wav | 516304102_501766822.wav | `MP_ANNOUNCER_0060D_MATCH_OVER` | MP_ANNOUNCER_0060D_MATCH_OVER | Game over! | client_progress |
| MP_ANNOUNCER_0060E_MATCH_OVER.wav | 1776241938_799358519.wav | `MP_ANNOUNCER_0060E_MATCH_OVER` | MP_ANNOUNCER_0060E_MATCH_OVER | And the clock's run out! | client_progress |
| MP_ANNOUNCER_0060F_MATCH_OVER.wav | 1887349570_820856060.wav | `MP_ANNOUNCER_0060F_MATCH_OVER` | MP_ANNOUNCER_0060F_MATCH_OVER | We're ready to call this one! | client_progress |
| MP_ANNOUNCER_0060G_MATCH_OVER.wav | 3160270630_897849908.wav | `MP_ANNOUNCER_0060G_MATCH_OVER` | MP_ANNOUNCER_0060G_MATCH_OVER | This one's all over! | client_progress |
| MP_ANNOUNCER_0120A_ROUND_OVER.wav | 564554878_113098378.wav | `MP_ANNOUNCER_0120A_ROUND_OVER` | MP_ANNOUNCER_0120A_ROUND_OVER | And the round comes to a close... | client_progress |
| MP_ANNOUNCER_0120B_ROUND_OVER.wav | 2908796745_902093230.wav | `MP_ANNOUNCER_0120B_ROUND_OVER` | MP_ANNOUNCER_0120B_ROUND_OVER | That's the end of this round... | client_progress |
| MP_ANNOUNCER_0120C_ROUND_OVER.wav | 214210652_1010208086.wav | `MP_ANNOUNCER_0120C_ROUND_OVER` | MP_ANNOUNCER_0120C_ROUND_OVER | That'll do it for the round... | client_progress |
| MP_ANNOUNCER_0120D_ROUND_OVER.wav | 2721114525_265310579.wav | `MP_ANNOUNCER_0120D_ROUND_OVER` | MP_ANNOUNCER_0120D_ROUND_OVER | This round is over... | client_progress |
| MP_ANNOUNCER_0120E_ROUND_OVER.wav | 2980075347_90892285.wav | `MP_ANNOUNCER_0120E_ROUND_OVER` | MP_ANNOUNCER_0120E_ROUND_OVER | Another round in the books... | client_progress |
| MP_ANNOUNCER_0120F_ROUND_OVER.wav | 2538449067_78820907.wav | `MP_ANNOUNCER_0120F_ROUND_OVER` | MP_ANNOUNCER_0120F_ROUND_OVER | That wraps up the round... | client_progress |
| MP_ANNOUNCER_0120G_ROUND_OVER.wav | 2755043106_444934860.wav | `MP_ANNOUNCER_0120G_ROUND_OVER` | MP_ANNOUNCER_0120G_ROUND_OVER | And that's the round... | client_progress |
| MP_ANNOUNCER_0120H_ROUND_OVER.wav | 815244573_336589598.wav | `MP_ANNOUNCER_0120H_ROUND_OVER` | MP_ANNOUNCER_0120H_ROUND_OVER | This round's over and done... | client_progress |
| NEVENT_GLOBAL_15_SEC_REMAINING.wav | 514740022_813986464.wav | `nevent_global_15_sec_remaining` | 45b6c9bfc23df6c1 | 15 seconds remaining. | client_progress |
| NEVENT_GLOBAL_30_SEC_REMAINING.wav | 136064785_654542474.wav | `nevent_global_30_sec_remaining` | 559e01c3cfcca9c5 | 30 seconds remaining. | client_progress |
| NEVENT_GLOBAL_BLUE_TEAM_READY.wav | 1864002821_347805481.wav | `nevent_global_blue_team_ready` | 713c18e2fb12c6c7 | Blue Team - Ready. | client_progress |
| NEVENT_GLOBAL_GET_TO_LAUNCHERS.wav | 3970680089_967885561.wav | `nevent_global_get_to_launchers` | fb0a6db7cc7f670f | Get to your launchers. | client_progress |
| NEVENT_GLOBAL_RED_TEAM_READY.wav | 544731908_620563277.wav | `nevent_global_red_team_ready` | 2eb214342960242b | Red Team - Ready. | client_progress |
| NEVENT_GLOBAL_ROUND_STARTING_IN.wav | 1993545314_612036646.wav | `nevent_global_round_starting_in` | d9bda30e58c5fe36 | Round starting in. . . | client_progress |
| NEVENT_GLOBAL_ROUND_TIED.wav | 2696778814_866677394.wav | `nevent_global_round_tied` | 1539adf1ebc0cc70 | Round tied. | client_progress |
| UNUSED_3536117903_374300019.wav | 3536117903_374300019.wav | — | not used by any script or the engine |  | — |
| UNUSED_75672086_652792720.wav | 75672086_652792720.wav | — | not used by any script or the engine |  | — |
| UNUSED_1938136332_948526918.wav | 1938136332_948526918.wav | — | not used by any script or the engine |  | — |

## When native calls a line (mp_announcer_server_state)

Each send in the stock server script picks **one line at random** from a fixed table. The tables were read by running
each pick expression from the script DLL; `src/engine/announcer.rs` holds them entry for entry.

| Engine event | Lines it picks from | Delay | Our server |
|---|---|---|---|
| `nevt_player_save` | 0110A-K_SAVE, MPL_ARENA_SAVE_A1-E1, G1, I1-O1, 1 silent | none | yes |
| `nevt_blue_intercepted` | 0010A/B_GENERIC, 0020A_INTERCEPTION_BLUE, MPL_ARENA_INTERCEPTION_A1/B1 | 0.75 s | yes |
| `nevt_orange_intercepted` | 0010A/B_GENERIC, 0010A_INTERCEPTION_RED, MPL_ARENA_INTERCEPTION_A1/B1 | 0.75 s | yes |
| `nevt_disc_stolen` | `steal_sound` at once, then 0030A-F_DISC_STOLEN, MPL_ARENA_STEAL_B1-E1 | 0.75 s | yes |
| `nevt_game_score` (arena) | 1 pt: 0180A-F, H-J, 1 silent; 2 pt: 0190A-J, MPL_ARENA_2POINT_A1-D1; 3 pt: 0200A-G, MPL_ARENA_3POINT_A1-L1 | none | yes |
| `nevt_generic_intercepted` | 0010A-D_GENERIC, MPL_ARENA_INTERCEPTION_A1/B1 | 0.75 s | no: when the engine raises it is not known |
| `nevt_blue/orange_cleared_disc` | 0160A-E_CLEAR_DISC_BLUE / 0150A-E_CLEAR_DISC_RED | none | no: the engine's clear test is not decoded yet |
| `nevt_blue/orange_gains_possession` | 0140A-H_CHANGE_POSSESSION_BLUE / 0130A-H_..._RED | none | no: fires on far fewer owner changes than a turnover; rule not decoded |
| `nevt_disc_hit_post` | 0170A-L_HIT_POST | 0.75 s | no: no post-hit detection |
| `nevt_player_punched` | arena: 0040A-I_PUNCH, MPL_ARENA_PUNCH_A1/B1, 6 silent; social: the same + `_Lobby` | 0.5 s | no |
| `nevt_rampage` | arena: 0090A-I_RAMPAGE; social: the same + `_Lobby` | none | no |
| round/match results, round start, round over, overtime | 0050, 0060, 0070, 0080, 0100, 0120, OT_ROUND | none | yes (before this change) |

"Silent" entries are hashes that no client announcer script listens for. Native sends them as well, and nothing plays.

Native's public overtime recording (`H:\EVR\captures\netrec-pub-ot-20261119`) ties the save, interception and steal
calls to the stats. Every call falls on that player's Saves, Interceptions or Steals going up, in the next 3 s stat
batch. Both `INTERCEPTION_RED` calls were on orange (team 1) interceptions. So our server makes the call where it
counts the stat (`DiscTracker::change_owner`).

In the engine (`echovr.exe`), clears, steals, interceptions and possession changes are all raised from one
disc-ownership routine around `0x140330000` (FunBuild build). Its clear and possession conditions are still to be
decoded.
