# EchoRustServer: hand-off notes

State as of 2026-11-16. Read this before working here; `README.md` has the engine
and protocol detail. Nothing is committed past the initial commit: everything below
is in the working tree (commit only when the user asks).

## What this is

A Rust replacement for Echo VR's native game server (`echovr.exe -server -headless`).
One `echo-rust-server` process runs many game servers (one UDP port each) and
registers them with a backend: EchoRelay (`ws://…/serverdb`), nakama (same config
format, detected automatically), or its own built-in services. A small GUI,
`launcher/` (`echo-server-launcher`), starts it for hosts.

| Path | What |
|---|---|
| `src/main.rs` | CLI, startup, instance pool wiring |
| `src/upstream.rs` | backend bridges (EchoRelay ServerDB / nakama), registration, loadout saves |
| `src/lobby.rs`, `pool.rs`, `services.rs`, `store.rs` | built-in backend, matchmaking, game-server pool |
| `src/engine/` | the game-server engine: `transport.rs` (crypto, channels), `session.rs` (join, relay, replicated state, arena rules), `goal.rs`, `rules.rs`, `stats.rs`, `filter.rs` (mode/map filters), `msg.rs` (symbols + captured data), `veilnet.rs` (EchoCombat) |
| `src/engine/data/` | captured native bytes the client needs (ids responses, balance settings, replicated baselines) + `cosmetic_names.txt` |
| `src/decap.rs` | `--decode-capture <dir>`: decrypts a recording into `msgs.csv` |
| `tools/record_native.py` | records a native server session (ws proxy :7781 + UDP relay). `REC_UPSTREAM` / `REC_UDP_PORT` env override the defaults (`ws://127.0.0.1:777`, 6800) |
| `captures/` | recordings (git-ignored): `native-20261111`, `native-2p-20261115`, `native-lobby-20261115`, `native-private-20261115` (private arena, see its NOTES.md), `native-nakama-20261115` (registration only) |
| `launcher/` | eframe/egui GUI, own Cargo project and target dir |
| `dist/` | packaged builds + `echo-rust-server-windows.zip` / `-linux.zip` (git-ignored) |

## Build

- Windows: `cargo build --release` (server) and `cd launcher && cargo build --release`.
  `.cargo/config.toml` builds the C runtime in (`+crt-static`): the exes need no
  Visual C++ redistributable. Keep it that way.
- Linux (cross from Windows, WSL is broken here):
  `ZIGDIR=$(python -c "import ziglang,os;print(os.path.dirname(ziglang.__file__))"); export PATH="$(cygpath -u "$ZIGDIR"):$PATH"`
  then `cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.31` (both projects).
- Package: copy both exes into `dist/windows` / `dist/linux` and zip them as
  `echo-rust-server/<file>` (Linux entries with mode 0755). Never include
  `launcher-setup.json`, `server-data/` or `balance-settings.json`: the first two hold the user's credentials.
- `cargo test --release`: 64 server tests + 2 launcher tests, all passing.

## Done (and confirmed by the user unless noted)

- Arena: disc networking, goals, rounds, rematch, pub rules (no self-goals, mercy at 12).
- Cosmetics saved from the game server for any player: EchoRelay via
  `SNSUserServerProfileUpdateRequest` on the login service (names from
  `cosmetic_names.txt`), nakama via its `0x7777777777770B00` save request. Never send
  0B00 to EchoRelay (it means something else there).
- **nakama hosting works** (confirmed on a port-forwarded PC). Key facts:
  - Registration layout differs: nakama reads 10 pad bytes after the port, EchoRelay 2.
    The server sends nakama's first and falls back to EchoRelay's only if a backend
    ends registration without ever sending a nakama protobuf frame (`retry_layout`).
  - nakama then pings `external_ip:port` over UDP (`0x997279DE065A03B0`, 500 ms
    timeout; we answer with `0x4F7AE556E0B77891`). Without UDP port forwarding it
    fails and nakama DMs the owner on Discord.
  - nakama sends players' session keys (SessionSuccessV5) just before
    LobbySessionCreate: the engine now holds early keys and applies them when the
    session starts (untested live).
- Launcher: pasted `config.json` (its `serverdb_host` is the upstream, `loginservice_host`
  the login), game server count / kept running / first UDP port / frame rate (10-120 Hz,
  the wire clock is 120 Hz: do not raise it) / mode and map filters / region / public IP.
  Hide to tray (Windows tray-icon, Linux ksni; Linux untested). It renders with OpenGL,
  then DX12 on a GPU, then WARP (software): tested all three; errors show in a box and
  `launcher-error.log`.
- **EchoCombat VeilNet, server side** (`src/engine/veilnet.rs`, ported from
  `J:\EchoVR-Tools-Launcher\EchoCombat\plugin\veilnet.cpp`). Unit-tested and tested
  over real UDP with fake clients; **not yet tested with the game**.
  - 16-byte packets on the game socket (`ECVL`, type, pad, u32 id, `VEIL`), taken out
    before the session sees them.
  - Hello `H` -> `A` (flag 1 = MEDIGUN) + `G` (rules word). Events `V`/`P`/`T`/`O`
    (and `M` when MEDIGUN is on) are deduplicated and relayed twice to every other
    client heard from in the last 30 s.
  - Private matches (`echo_combat_private`, `echo_arena_private`): LOADOUTS, SERIES-10
    ONLY, POINT RATE / PAYLOAD SPEED (Fission, Gauss), PAUSE with the 5 s resume.
    Rules reset on each new session.
  - **MEDIGUN is only ever on from a file**: `echocombat.ini` next to the server exe,
    `[server]` / `medigun=1`. The user wants no other switch (no CLI flag, no launcher option).
  - The launcher's **EchoCombat** checkbox (default on) passes `--no-veilnet` when off:
    VeilNet packets are dropped unanswered, so clients treat the server as stock and
    hide the mod's gear.
  - Not ported (they need the map's scripts, which this server does not run): `R`
    (RULES page-1 point unlock / capture time; sending it would wipe client values),
    the map-switch buttons (`C` WANT_MAP, `L`), pausing a payload only at a
    checkpoint, server-side cease fire / capture point reset on pause, drone kills `K`.

## Balance settings block (decoded 2026-11-15)

After `SR15NetBalanceSettingsClientReady` the server sends `SR15NetBalanceSettingsMassUpdate`
(`src/engine/data/balance_settings_*.hex`): u32 size + zstd frame of NUL-terminated JSON
documents plus an index of their ids (CSymbol64 of the game's balance file name).
`tools/balance_block.py decode|encode` splits / rebuilds it (round trip byte-identical);
decoded copies are in `captures/balance/`.

- Social block: `mp_equipment` (gear table), `mp_weapon_settings` (all weapons, damage
  table, tac mods incl. `buff` = OVERCHARGE, `smoke`, `point`, grenades incl. `dot`,
  `drone`), punch, movement, disc/team colours, aim assist.
- Arena block: `mp_arena_rules` (round times, mercy spread, self goals, private values),
  `mp_arena_ai_difficulty`, punch, movement, colours. No weapon settings.
- Combat maps also have `mpl_combat_<map>_map_settings` (spawn timers); EchoCombat's
  note that changing them in its files "did NOT take" suggests the client uses the
  server's copy.
- Our lobby/arena captures (from the local native server) carry **stock** values. A
  public EchoVRCE Gauss match (`captures/client-combat-20261116-b`, see its NOTES.md)
  showed **EchoVRCE's combat servers send EchoCombat's tuning in the block**: DOT radius
  3.5, OVERCHARGE dr 0.09, plus the mod's extra keys (`buff.link_window_ms`,
  `cheat_death_*`, `damage_bonus_*`, a `medigun` section, `grenade_properties.loc`,
  UMBRAL VEIL `veil_*` ...). The Gauss block also has `mp_combat_rules`
  (`03f14649abfb4fa3`) and `mpl_combat_gauss_map_settings`.
- So with EchoCombat on (default) this server now sends that EchoCombat
  `mp_weapon_settings` (`src/engine/data/mp_weapon_settings_echocombat.json`) in place
  of the stock one (`balance::set_base`); with `--no-veilnet` it sends stock.
  `medigun.enabled` is always forced to `echocombat.ini`'s value, even over `--balance`.
- Client recordings: `tools/record_client.py <out> <game dir> [echovr args]` proxies the
  client's websocket services (patches and restores `_local/config.json`), rewrites
  LobbySessionSuccessV5 to a local UDP relay, logs `/session` API polls;
  `--decode-capture` reads its `sessions.jsonl` for keys. No admin needed.

### Editing it: `--balance` and the launcher's Settings page (2026-11-16)

- Server: `--balance <file>` (env `ERS_BALANCE`, `src/engine/balance.rs`) loads
  `{"<doc file name or 16-hex id>": {partial JSON}}` and merges it into every captured
  block that has that document before sending (only keys the document already has;
  unknown ones are warned and skipped). Public arena flow (round time, pre-match wait,
  launch wait, mercy spread, self goals) reads the same `mp_arena_rules`, so edits
  there change the server's match flow too (`msg::arena_public()`).
- Launcher: a **Settings** page (header selector Server | Settings, `launcher/src/settings.rs`).
  Combat: Weapons, Tac mods, Ordnance, Damage (per-gear tabs with in-game names), Player,
  Movement. Arena: Match rules, Disc & grab, Movement, Punch. Search box, changed values
  in blue with ↺, Reset all. Disabled while the server runs.
- Defaults are bundled from `launcher/defaults/*.json`: `mp_weapon_settings` is the
  EchoCombat one (same file as the server's), the rest are stock values decoded from
  the captured blocks. `medigun` is not editable there. Only differences are stored, in `launcher-setup.json` → `balance`;
  on Start they are written to `balance-settings.json` next to the launcher and passed
  as `--balance`. The disc & grab document's file name is unknown, so it is keyed by
  its id `e3830436d1d46da9`.
- Combat values reach players only in the social lobby today (combat maps are not
  hosted). Whether clients really use the server's values is still unverified in game.

## Server logs (2026-11-16)

The launcher saves every server run to `logs/server-<unix time>.log` next to it (last 20
kept, keys redacted) and starts the server with
`ERS_LOG=info,echo_rust_server::engine::session=debug` unless ERS_LOG is set, so the
arena's disc / touch decisions are in the file. Open bug waiting on such a log (public
arena): a headbutted disc was not shared; after a goal the spectator's disc stayed in
the goal, and after the relaunch it stayed in one place instead of the conceding
team's spawn.

## Lobby combat area (2026-11-16)

From a native H-server recording of the social lobby's combat area
(`captures/native-20261116`, NOTES.md there). `src/engine/gear.rs` answers what native
answers there: orb teleports (`GameplayTeleportPlayerRequest` -> `GameplayTeleportPlayer`,
same bytes), the area's bit field (`BitFieldRequest` for `26452bfcf77799cb` =
`mpl_lobby_b_combat`; `data/bitfield_response_2645.hex`, `SetBits` updates and relays),
FireGun / tac mod / heal bubble relays, bullet hits -> `LocallyTrackedDamage` (base x
damage table: part 2 arm, 4 torso, 5 head), ordnance magazines (end after the item's
lifetime, CooldownActive, destroy 2.5 s later).
Splash (2026-11-16, after the user found METEOR / COMET / DETONATOR did nothing):
player positions come from each client's 64-byte body physics (object `8ec21d00`,
19-bit mm fields at bits 267/286/305, minus 250000), thrown ordnance from its area
object (`26452bfc`). `ActivateExplosion` (METEOR, COMET), DETONATOR (radius 3),
STUN FIELD (stuns each victim once: `SR15StunMsg` target = slot, 0x0c for the dummy)
and DOT FIELD (ticks per `grenade_properties.dot`) send `ExplosionPlayerDamage` to all
(victim `f234d509...` with byte 0 xor slot) plus the thrower's hit markers. The
training dummy is only at its one recorded position (43.95, 1.12, -25.42): the dummy
positions in the server's 168-byte area physics are not decoded. COMET explosions use
radius 1 and no marker (its kind id `5427a85c...` is a guess).
Second pass (from the user's test and the native recording):
- Dummies: three, from clustering native's bullet hits: (43.9, 1.1, -25.4),
  (55.7, 2.4, -5.9), (50.3, 4.9, -8.1); they sway, so radius 0.5. DETONATOR never hits
  dummies (native: NothingHit right on one).
- Native DET / STUN / DOT magazine states: armed `1b0b211b...`, released `6388fd28...`
  (both may repeat), detonated/activated `a9509fd4...` (sent twice), server USED `6281fb26...`.
- OVERCHARGE = heal bubble `41e35b49673283b9` activation: the server drains 5 health a
  second (`ability_properties.buff`), 9 `DamageableHealthBroadcast`s from 112 (non-lethal).
  Its gun damage bonus is not applied by native (headshots 18.15 with or without).
- Native answers each `AbilityCooldown` with two copies.
- Walking out of the area: the client sends `LoadoutNumberNotification` x2 + entrant
  state; native frees the 8 gear stations (`SetBits fcffffff` at 0, 15, ... 105). Without
  it the client stayed in combat mode outside the brawler room (stun/block anywhere, tac
  mod kept). `Gear::left_area`.
- Gear events (ordnance spawned/state/used, explosions, OVERCHARGE, area enter/leave)
  are logged at info.
Third pass (`captures/native-det-20261116`, the user's DET / splash / COMET / OVERCHARGE test):
- The server applies damage: `DamageableHealthBroadcast` 64 B = `[owner][u32 0][health][7.0]
  [kind: 4 explosion, 2 ordnance][u32 0][ffffffff until first hurt][hit pos][12 zero][cause]`
  (cause: METEOR marker `8da7eb67...`, item id for DET / DOT). At 0: `PlayerKilled`
  `[owner][cause][0;8]`; client `PlayerRespawn` -> echoed + health 112; `RespawnComplete`
  relayed. Without this the client showed hit markers but took no damage, and after a
  self-DET kept no gun and no recharge. `Gear::hurt`, `lives`.
- METEOR and COMET share explosion kind `dc354e7e...`; the `GameFireGun` with the same
  explosion id (bytes 56..64) says which (byte 0: 0x1e COMET, 0x2d METEOR; f32 at 48 =
  projectile damage = charge). COMET marker `e7dabee848778203`.
- DET: 65 damage (dummy marker), hits dummies; self-hits get no marker.
- Dummy hit codes (marker byte 32): 0x17, 0x0d, 0x16. Second dummy reports ~(55.8, 1.6, -5.25).
- `CooldownRefund` (owner + `58c51a93...`) came 0.5 s after joining team blue; not done.
Re-entering the area (`SetOwner` with 8 zero bytes) resets the tac mod as native does:
`AbilityCooldown`, a full `DamageableHealthBroadcast`, the stored heal bubble params.
Gear list: with EchoCombat on, `mp_equipment` is the native H lobby's
(`data/mp_equipment_echocombat.json`) and `mp_weapon_settings` is the Gauss doc plus the
keys only H has (buff heal, `ability_properties.medigun`, damage_table dot/drone);
both MEDIGUN flags follow `echocombat.ini`. Team switching works in the social lobby
(team 3 = no team). Unit-tested against the recording's bytes; splash not yet tried in game.

### Fifth pass (2026-11-16, 12-player public lobby log)

- **Stutter/lag**: we flushed every peer after each inbound batch, so each client got
  one packet per packet any other player sent (~1,300/s each at 12 players). Native
  sends ~120/s per peer. Now mailbox state + bare acks go out only on server frames
  (`Transport::flush_reliable` between frames), packed into datagrams under
  `MAX_CHUNK` (a fragmented mailbox frame is lost whole).
- **Disc in two places**: touch seq and touch entries are per *object* (actor at
  0x18): the lobby's `b1c75c04` (disc + props) and `d33d44b7` both have entry 0.
  Keyed per slot / entry alone, a second object's requests were dropped ("out of
  order", no reply) and grants for one prop moved the disc's owner. Now
  `touch: (actor, entry)`, `touch_seq: (slot, actor)`; the owner's own request passes.
- **Combat between players**: `DamageableHealthBroadcast` bytes 8..12 are the
  player's *slot* (the 8-byte id `8ec21d00d315993f` is the same for everyone; Gauss
  broadcasts carry one 56-byte record per slot), and the u32 at +12 of a record is
  the body part (2 arm, 3, 4 torso, 5 head). We always wrote slot 0, so everyone's
  damage landed on slot 0. `PlayerKilled` = `[id][cause][victim slot][killer slot]`.
  A `BulletCollision` names gun + shooter, not victim: the server now hurts the
  nearest other in-area body within 1.2 m of the hit point.
- Native lobby also broadcasts server-owned prop states (owner `ffffffff`, 592-664 B
  physics for `b1c75c04`); we don't. Possible remaining prop jitter.

### Sixth pass (2026-11-16, second 12-player log + ReVault `libr15.so`)

ReVault (`EchoCombat/.mcp.json`, project `echovr`; the Quest `libr15.so` has symbols) was
read over HTTP for this: `CR15NetGame::ClientPingResponseCB`, `ClientsPingEstCB`,
`UpdateConnectionStats`. None of this pass is tested in game yet.

- **Stutter**: clients drop a `SR15NetUserPingEstMsg` whose generation (u16 at 2) is not
  the slot's, and we always sent 0, so players in a reused slot had no clock on anyone
  else's client. Now the player's generation. The value (f32 at 4) is native's: the age
  of the server frame the client echoed, in whole frames, averaged over 450 echoes
  (16666 on a LAN), not the mailbox round trip. A response whose client time is not
  newer is dropped, as native does. The scoreboard ping (`SNSLobbyUpdatePings`) is unchanged.
- Reliable resend timer was capped at 60 ms (every frame resent 2-3 times to anyone
  over 60 ms ping): now 1.5 x the measured round trip, 40-500 ms.
- Windows: `timeBeginPeriod(1)` at start (15.6 ms timers ran the 120 Hz frame unevenly
  with few players).
- Native relays player state exactly as we do (same bytes, same key, once per frame,
  0.8-9 ms later: `captures/native-2p-20261115`), so the relay itself was not the cause.
- **Combat area entry** is `SetOwner [owner slot u32][index u32]` with index 0. We matched
  eight zero bytes, i.e. slot 0 only: nobody else was ever "inside", so bullets found no
  victim and leaving freed nothing. Firing a gun also counts as inside (not within 3 s
  of leaving).
- **Stun outside the brawler room**: in the social lobby `SR15StunPlayerMsg` is relayed
  only when the sender is inside the combat area.
- **Lobby props / discs**: touch entries remember whether the owner holds the object
  (exclusive grab). Outside the arena anything not in a hand is granted to whoever asks:
  clients give up what they threw without a message (native's server simulates it then),
  and the stale entry declined everyone who rightly believed nobody owned it. Still not
  done: the server does not simulate or broadcast unowned props (native does, owner
  `ffffffff`), and the public arena disc bugs above are untouched.
- Launcher: `launcher/src/update.rs` gets the newest GitHub release
  (`echo-vrce/Echo-RS`, asset `echo-rust-server-windows.zip` / `-linux.zip`) at start
  and on "Check for updates", swaps the two executables while the server is stopped
  (`*.old` removed next start), records the asset's upload time in `update-state.json`
  (the tag is re-uploaded in place). An automatic check never replaces a newer local
  build; the button does. Checkbox "Update when the launcher starts" (`auto_update`).

### Seventh pass (2026-11-16, third public lobby log; untested in game)

- **Gear stations are per slot**: bit index = `set * 15 + slot` (8 sets; Gauss shows slot 3
  at 3, 18, 33...). `SetOwner` is `[0][index]`, sent only walking in: any `SetOwner` from
  a player not inside is the entry (bodies are logged at info to confirm). Leaving frees
  that slot's eight bits (`SetBits fcffffff`) to **everyone**, also when a player
  disconnects inside. Before, only slot 0's indices went out and only to the leaver:
  others' guns / tac mod stayed floating and their clients stayed in combat mode.
  The leave trigger is still `LoadoutNumberNotification` (native: x2 + entrant state,
  then the SetBits ~0.1 s later); the same pair also precedes `SetOwner` on entry.
- **Death**: the f32 after health in a `DamageableHealthBroadcast` record is 7.0 alive and
  0 dead (native-det at the kill). Always 7.0 left players "critical" at 0 health, and
  with no `PlayerRespawn` from them the server kept them dead (no more damage). Now 0
  when dead, and the server revives a dead player after 10 s if the client never asks.
  `hurt` no longer needs the player's bubble owner id (it is `8ec21d00d315993f` for all).
- **Touch requests name their object in bytes 0..4** (`b1c75c04` lobby props + disc,
  `d33d44b7`), not at 0x18 (ff.. in every lobby request). Entries and sequence numbers are
  now per that object and the reply carries the object's full id (`6daa00a6d33d44b7` for
  `d33d44b7`; we always answered with the level id, so those grants hit the lobby's
  entry of the same number: the disc is entry 0).
- Fury transport: native does not relay clients' `ComponentEvent`s (2-player capture) and
  answers some itself, so the transport is driven by the lobby's scripts. Not done: needs
  a native recording of someone piloting it.
- Other clients still learn a new prop / disc owner only from the owner's physics; in the
  arena the server stamps the reset count into byte 24 of forwarded disc states. The
  lobby prop layout is undecoded, so that is not done there (possible remaining "two discs").
- nakama loadout save: jersey is sent as -1 when unknown (0 overwrote it) and a changed
  loadout is saved once more when the player leaves. Saves were sent for every player in
  the log; why nakama kept only some needs nakama's own log (user lookup by EvrID, or
  symbols missing from its name table).

### Eighth pass (2026-11-16, `captures/native-match-20261116`; untested in game)

`src/engine/props.rs`: the lobby's loose objects (format in its header, facts in the
capture's NOTES.md). The server now sends the objects no client owns (10/s; recorded
resting places, or a departed owner's last record put to sleep), as native does at 45/s.
**Fury** (`props.rs`, header has the detail): while the controller's holder has the
trigger down (ComponentEvents `88678f45e3fb8c0e` on / `4879bc8a220df32f` off) the server
flies entry 2 with native's fitted thrust (`a = 1.40 * nose - 1.31 * v`, R2 0.69) and
sends it 40/s. Steering is **invented** (the ship turns to the controller's facing):
native's ship tumbled and followed neither the controller's orientation nor its
turning, so its steering input is still unknown (a second recording with slow,
deliberate inputs, one axis at a time, would help). No walls: it stops 30 m from its
resting place and slows to a stop when the trigger is released (native coasts and bounces). The combat area's own set
(`26452bfc`, the dummies) is not sent.

### Fourth pass (2026-11-16)
- Health regen: 12.4 s after the last loss, ~73 hp/s, kind-8 broadcasts (ffffffff at 28), then a kind-0 full one.
- Ordnance recharge: `CooldownRefund` (owner + 58c51a933fe33556) at use + `grenade_properties.<item>.cooldown`.
- `GameDepositMagazine` (ordnance put back): relayed to others, magazine forgotten.
- NEUTRON fix: native sends its `PooledActorDestroy` (create bytes 40..56) only for 160-byte creates (projectiles). Tac mod effect creates (NEUTRON is 96 B; its 40..56 = area id + its own actor) are relayed to the others only, never destroyed. Destroying it flashed the stock barrier and froze the player. UMBRAL VEIL (`smoke`) is unrecorded but now gets the same treatment (any non-160 create). OVERCHARGE uses `ActivateHealBubble`, not an actor.
- DET reach: native hit only things within about 0.4 m in both recordings (one NothingHit at 0.49 m from dummy 1). Now `grenade_properties.det.radius` (default 1.5 m, was 3), and one `ExplosionPlayerDamage` per blast (at the nearest victim). One per victim drew several blasts. User said 3 m was "massive".

## Next: Combat

The user's order: **Combat lobby first** (`mpl_lobby_b_combat`, symbol
`0xcb9977f7fc2b4526`), then everything EchoCombat supports plus all native Combat logic.

1. **Combat lobby.** We have its IdsResponse (`ids_response_2645.hex`) but no balance
   settings and no replicated baseline, so a client would wait forever loading
   (`session.rs` logs "no balance settings / no replicated baseline for this level").
   Record a native session there (H server + windowed client
   `-windowed -noovr -level mpl_lobby_b_combat -gametype social_2.0`, the gametype
   is a guess), decode it, add the three captured blobs to `msg.rs::captured()`, and
   check what else differs from `mpl_lobby_b2` (gun range, equipment stations).
2. **Combat matches** (Dyson, Combustion: capture point; Fission, Gauss: payload). The
   native server runs the maps' scripts (capture points, payload, scoring, respawn,
   round flow, damage); this server runs none, so each mode's flow must be
   reimplemented from recordings, as the arena flow was. Record public and private
   matches of each map. Captured data per map (ids, balance, baseline) is missing too.
3. **EchoCombat's server logic** on top: VeilNet's `R` and map switch, pause effects,
   PowerBoost (OVERCHARGE links / damage reduction / cheat death / +damage, MEDIGUN
   heal), DroneFix (drone kills -> `K`). Source: `EchoCombat/plugin/*.cpp`, notes in
   `EchoCombat/agent.md` and `EchoCombat/docs/combat_rules_screen.md`.

## Working rules (from the user)

- **"Work offline" = no game launches at all** (no clients, no native servers) until
  the user says otherwise. That is the current state.
- Never echo or send anywhere the user's config credentials (Discord id, password,
  api_key in the serverdb / login URLs). Redact them in anything you print.
- Don't kill processes by name (`taskkill /IM`): stop only what you started.
- Game installs: `C:\Oculus\Games\Software\Software\ready-at-dawn-echo-arena` (VR /
  windowed client) and `H:\ready-at-dawn-echo-arena` (native server). Both have
  EchoCombat. Their `_local\config.json` point at the local EchoRelay on `:777`
  (`J:\EchoVR-Tools-Launcher\EchoRelay`); backups `config.json.bak-*` exist. Native
  server: `echovr.exe -server -headless` (no `-noovr`: with it the game server never
  initialises).
- Game ports: use 7000+ for the server's UDP range; Echo clients bind near 6792 and a
  server holding those ports breaks "Broadcaster initialization".
- Sources to compare against: `J:\EchoVR-Tools-Launcher\nakama3` (nakama, `server/evr`),
  `J:\EchoVR-Tools-Launcher\EchoRelay`, `J:\EchoVR-Tools-Launcher\nevr-server-rs` (protos).

## Ninth pass (Fury feel, lobby vars, stun)

- Fury: whoever holds the controller (entry 3, HELD bit) steers; the ship turns to the controller's facing
  with or without the trigger. Trigger (ComponentEvent ENGINE_ON/OFF) = thrust (drag 1.31). Engine off it
  coasts (drag 0.12, native's velocity stays flat for seconds) and keeps its spin (decay 0.4/s); sleeps when slow.
- Engine sound: native answers the trigger with `SR15NetReplicatedDeltaUnreliableVars` on the lobby object,
  var `bec4d8edf6acf8f3` = 1 / 0. Also first disc grabs set vars (main `3261fa7226794279`=1, quarter-court
  `1b10bdc962b47fcc`=0 + `a22d90ef70f6fa83`=1, mini-arena `d2797da99ab317a5`=1). `Props::vars`; session sends
  them as a delta on change and every second.
- Stun: native's server (0x140d53da0) relays unless the victim is a spectator (`IsPlayerInPunchState` is really
  that check). The combat-area filter is gone: it blocked every punch in the brawler room (no SetOwner there).

## Tenth pass (native public match via NetRecorder)

- NetRecorder (EchoBits&Bobs `plugin/netrecorder.cpp`) records a native server's sends (CBroadcaster::Send
  0xF89AF0) and, from 1.1, received game messages through a listener tap beside every game listener
  (CBroadcaster::Listen 0xF80ED0, as EchoReplay's wiretap). 1.0's Raise hook saw only connection messages.
  Decode: `echo-rust-server --decode-netrec <dir>`.
- Hoster match 2026-11-16 (11 players, 13 min): disc reset count / grant generation runs 0..=0xe and wraps
  to 0 (no 0xf in 260k disc states). Fixed: `DISC_COUNTS` (was `& 0xf`). Every disc owner change came with
  a grant (headbutts too: grant flags 0x10, disc state byte 25 = 04, then 00 free / 02 held). Contested
  grabs: first request wins, the loser is declined ~80-160 ms later. Grants/declines have 0 at 0x1c (we
  now send 0, not the request's acked). Grant flag bits 5..15 are sometimes noise in native.
- After a goal native pins the disc where it went in (owner ffff, count +1 over the scorer's) for ~19.6 s,
  then moves it to (0, 4536, +-27500) on the conceding side; matches `goal()`.
- StunPlayerMsg is relayed to everyone but its sender, as ours.
- Second hoster match (NetRecorder 1.1, client messages too): touch request = TransferReliable and
  TransferMailbox, same 48 bytes: `+0` object low, `+4` seq, `+6` acked, `+8` sender, `+0xc` new owner,
  `+0x10` believed owner, `+0x14` entry, `+0x20` u32 stamp = the timestamp (bytes 8..12) of the holder's
  disc state the requester last saw, `+0x28` 1 = hit (headbutt/punch; grant bit 0x10, disc state 04).
  Native grants iff believed == current owner (or the owner's own request; owner reset to none on a goal):
  62/63 decisions. Grant flag bits above 0x1f are noise.
- Native forwards only the current grantee's disc states: the old holder's (sent for ~150-200 ms after the
  grant) are dropped. Ours forwarded them with the new reset count. Fixed: `arena.granted` gates observe and
  forward; cleared when the server takes the disc. StunPlayerMsg: relayed unchanged to all but the sender;
  ForcePushPlayer: to all including the sender (both as ours).
- Lobby recording (NetRecorder 1.1, one player: brawler room in/out, gun, ordnance, quarter-court goal):
  the client crossing the combat area's edge sends `ComponentEvent` `c06471ffde095d63` (enter) /
  `c16471ffde095d63` (leave), module `c4b6bab2ee99ea6a`. Native answers enter with the full health
  broadcast + the player's heal bubble params (as `Gear::reset`), and frees the 8 stations 3.3-4.7 s
  after leave. This client sent no `SetOwner` and no `LoadoutNumberNotification` (older captures did), so
  we never saw it leave. `Gear::area_event` + `leaving` (3.5 s). Client `SetBits feffffff` at its station
  indices on entry. Hit markers and the entry health broadcast match native byte for byte (tests).
- `PooledActorCreate` (160 B): native relays to everyone but the sender (we echoed it back); destroy to all.
- Quarter-court goal: native sends `PostScoreUpdate` to all (`[team u16][points u16][type u16][scorer]
  [assister][u16 0][speed][distance][f32]`), nothing else; server-side detection. Disc crossed between
  (-53.2, 1.85, -30.35) and (-53.7, 1.78, -30.7). Not done: needs the goal boxes (more goals recorded).
- Headbutt of the parked disc (our server, player + spectator NetRecorder captures, 2026-11-16): the hit
  request was granted (flags 0x11) and the headbutter's states (count 1, state 04) reached the spectator,
  but our server had already sent its own parked disc with count 1 at the grant, so the spectator kept that
  one and ignored the headbutter's until the next grab moved the count on. Native stops sending its disc at
  the grant (hoster2 127.1 s: last server state count 0, then only the forwarded holder's). Fixed in
  `on_touch`: a grant clears `server_disc`.
- Native goal types seen: 1 slam dunk (speed 0, distance 0.17), 2 inside, 3 long, 5 and 7 (3 points, 12 m and
  11 m: bounce shots?). Ours sends 1 for any goal without a throw (a headbutt goal too), and the HUD then
  shows 0 m/s. Native type for a headbutt goal unknown (no recording).
- Goal: native's first server disc state after the score message is still the flying disc (the scorer's
  last state, owner ffff, the new count, high nibble 0); the pinned one (velocity zeroed) follows ~25 ms
  later. The spectator HUD's DISC SPEED reads it: ours pinned at once and showed 0 m/s (the in-world goal
  panel uses PostScoreUpdate and was right). `arena.holder_state` + `goal_flight` (frame_hz/40 frames).
- Spectator + player on one PC with the SAME account (both OVR-ORG-6032896500068788): when the player joins
  after the spectator, the spectator repeats the player-join sound all match. Not a server message (every
  join message once, same as earlier joins); retest with two accounts before chasing it.
- Frame rate (public 4v4 on a host's `--frame-hz 200`, 2026-11-16): goal pin 33 s, launch 26 s, announcer and
  respawns ~1.7x slow. `server_frame` returned until the next 120 Hz wire frame, so rules stepped <=120/s while
  every timing converted with 200. Now: the wire clock ticks at max(120, frame_hz) (native comp runs 180),
  continuous across rate changes and started on a 240 Hz uptime scale (never lower after a restart); rules
  frames run on real time with catch-up (`run_rules`); range 10..=240. `frame-hz N` on stdin changes the rate
  of a running server (launcher slider sends it); running games rescale their rules tick, schedule and
  possession (`rescale_rules`). Countdown light/fx events held to native's 60/s.
- Goal celebration team: native's first goal event is blue `0xe14549f2f4c40266` / orange `0xf7b1de946009b29e`
  (both hoster matches, every goal); ours always sent blue's.
- TopAwards (native, both matches): count 4, then arrays: ids, stats, values (u64), slots (u64), handles (u32),
  teams (u16), names (32 B), u16 each. Awards seen: MVP (value 655/558, not top points; formula unknown), Goals,
  Passes, Catches, and award 0x3ae29a1016fe966b / stat 0x41d2c4311f22150d (unnamed). Ours sent only the MVP.
  TierRewardMsg `[stat][tier][slot u64][xp u64]` to the player only: 6 per match, gold 150 / silver 100 / bronze
  50 in Goals, Assists, Saves (which runners-up get silver/bronze not derived; ours: top 3 of each).
- Join lock: public match sends LobbySessionEvent Locked at 1:00 left and at match over (Unlocked if a match
  starts again); we never did, so the matchmaker backfilled into ended matches.

## Brawler room punching (2026-11-16)

- The lobby's cease fire is `VAR_HOLD` (`098aa551580b0386`, the same var arena sets during goal celebrations
  and match end), on the lobby object `d09afd15b1c75c04`. Baseline 1 = no punching/blocking. Native drops it
  to 0 for a player ~0.5 s after they enter the combat area (`c064...` event) and raises it with the station
  free ~3.5 s after they leave; in the brawler room (no client message at all) it follows the body position:
  0 at (19.7, 0.5, 10.4) going in, 1 at (22.0, 1.0, 9.7) coming out (lobby1 capture 71.70 / 76.53 s).
  We never changed it, so nobody could punch in the brawler room. Now per player in `replicate`:
  `Gear::may_fight` (combat area, or `in_brawler_room`: within 8.0 m of the dome centre (26.5, -2.17, 14.15)),
  sent on change and every second; logged as "lobby cease fire lifted/on" with the position. The dome
  (`mpl_lobby_b_punch_hex_sphere_a`) was measured from a 1:1 FBX export of the level (cm, same axes as the
  game): a sphere of radius 7.6 m, shell 7.2..7.9. Native's var flipped 7.7..8.3 m out at the doorway, and our
  walk along the inside wall stayed within 7.9 m. An earlier box fit cut the room's sides off.
- Whether native's value is per player or lobby-wide is unknown (one-player captures); we send it per player.
- The `c064` / `c164` events are named `nevt_to_server_holstered_next_target_0/1` in SymbolNames.txt but are
  the combat-area edge in every capture. `nevt_to_all_*` / `nevt_to_peers_*` script events are not relayed by us.

## IPv6 (2026-11-16)

- Game ports bind dual-stack `[::]` (IPV6_V6ONLY off, `pool::bind_udp`) when `bind_ip` is 0.0.0.0, IPv4 only if
  that fails. Received addresses go through `to_canonical()` (IPv4 players stay plain IPv4 everywhere); sends map
  IPv4 back to `::ffff:` on a v6 socket (`engine::send_to`). Checked: raw ping answered on 127.0.0.1 and ::1.
- The game itself is IPv4-only (LobbySessionSuccess and broadcaster connect events carry 4-byte IPs), so a client
  reaches a server over IPv6 only through a client plugin.
- Plugin: EchoBits&Bobs `plugin/ipv6bridge.cpp` -> `Ipv6Bridge.dll` + `Ipv6Bridge.ini` (`hosts=` IPv6 addresses
  or AAAA names, `port=` first game port). Works with any backend (EchoVRCE untouched). It hooks ws2_32
  sendto/recvfrom/closesocket (Detours; the game imports plain sendto/recvfrom for UDP). It asks each host over
  IPv6 `[BRIDGE_QUERY][token]` on the game port; the server answers `[BRIDGE_ANSWER][token][public ipv4]`
  (`engine::set_bridge_ipv4`, from --public-ip or api.ipify.org over IPv4, refreshed every 10 min). Then every
  game packet to that IPv4 (EchoVRCE's server pings included) goes over IPv6 to the same port, and answers are
  injected into the game socket over loopback with a 6-byte [ipv4][port] header the recvfrom hook strips, so
  the game sees them from the IPv4. Tested in Python with the DLL loaded (its sockets hooked): a raw ping to
  203.0.113.7 (server --public-ip) answered over this PC's IPv6, none without the plugin. Not tested: the real
  game, a remote player, the Starlink router passing inbound IPv6.
- The hub listener is dual-stack too (`main::bind_tcp`), unrelated to the plugin.

## Hits, barriers, OVERCHARGE, drone, DOT (2026-11-16)

- `DamageableBulletCollision` (64 B, layout from native `CR15NetDamageableCS::NetBulletCollisionCB`):
  `[gun 8][direction 12][hit point 12][target 8][target space u32][damage f32][body part u8 +3][shooter 8]`.
  The target is what the shooter's client hit: a player is `PLAYER_ID` with byte 0 ^ slot in space `8ec21d00`;
  area objects are pool handles `[index u16][gen u16][kind u32 = 1]` in space `26452bfc` (dummies 0x17/0x0d/0x16,
  the barrier the user shot was index 8, body part 6 = "barrier"). We hurt exactly that target now (the old
  nearest-body-within-1.2 m guess hit the wrong player when two stood close).
- Barriers: native `DealDamage` -> `ProcessDamageServer` then broadcasts `SR15NetDamageableDealDamage`
  `[space 8][handle 8][health f32][shield f32]`; clients set it (`ProcessDamageClient`) and break the barrier
  at 0. Max health, break and respawn now come from the level data: see "Barriers from the level data" below.
- OVERCHARGE (PowerBoost + the user's spec): the user's gun hits +damage_bonus_body (1) / +damage_bonus_head (2)
  for dr_duration; the user and linked teammates take 25% less; a linked teammate cannot die for 8 s from the
  first link (left at 1 health) until the unlink marker or they leave. Links come from the client's PowerBoost as
  `DamageableDamageDealt` with amount -(1e6 use / 2e6 link / +2e6 rez / 6e6 unlink + dr permille*1000 + s).
- Drone (item `b54a80cbdec72de3`): it fell back to the detonator's 0.12 s lifetime, so the server marked it used as
  it was thrown and it blew up. Now `grenade_properties.drone.lifetime` (20 s) and its cooldown. Its shots
  (`SR15NetAutoTargetFireBullet`, sent by the owner's client) are relayed to the others, as native's
  `FireBulletNetwork` does. Shooting a drone down (its 70 health) is not done.
- DOT FIELD: radius back to stock 2.25 m (EchoCombat had 3.5), 11 s on the ground (user; stock 8).
  The host's balance-settings.json can still override the radius.

## Combat matches (2026-11-16, offline; untested in game)

- **Loading, all four maps** (`tools/gen_combat_maps.py`, needs the PC install + the balance JSON):
  - IdsResponse: native answers from the level's net-component table (resource type
    `0xc29715d62e039402`, 32-byte rows, actor id first), handles from 0x1b (the player space
    has 0..0x1a). 13 of Gauss's 352 rows carry another name of the same actor; the client resolves
    any name (`CncaGameSpace::Actor` binary-searches the level's name table), and the rows
    give the captured Gauss and arena responses actor for actor, in order. Gauss uses its captured bytes.
  - Balance: the captured Gauss block with each map's `mpl_combat_<map>_map_settings`.
  - Baseline: Gauss's vars with the map's space id (the client inserts any var a packet names;
    `CReplicatedVariables::SerializeFromPacket`). Vars only another map's scripts use start at defaults.
- `src/engine/combat.rs` (+ `Session::run_combat`): game state, clock and countdown; native's
  match-script events (module `6fa8cc9cd721a563`, the level's last net component = actor
  `9550158657ffada8` on every map); the end sequence and top awards; join lock at the end.
- **Payload, Gauss and Fission**: attackers = orange within 3.5 m (radius guessed), speed
  1.25 x attacker multiplier x section (Gauss s02 0.75 between cp1 and cp2), contested stops, empty 5 s
  then -0.25 m/s back to the last checkpoint. A checkpoint adds +120 s and holds clock and payload
  for the celebration timer. End of track: orange wins; time up (4 s overtime while attackers claim): blue.
  - Tracks from the level (`data/combat_<map>_track.txt`): `CR15TrackPointCR` (0x451830f92ddc5cc6,
    56-byte rows, next at +32, flags u32 at +16: 0x60 checkpoint, 0x10 end) chained; points are the
    first net components (trackpoint = 0x1b + k), the payload the next (Gauss 0xd0, Fission 0x60).
    Positions from the name resource's transform table (48-byte rows quat/pos/scale): the single run of
    `count` (u64 at 0xa8) unit-quaternion rows. Gauss's equal the recorded ones; checkpoints 77.8 / 126.0,
    end 206.8 (recorded 78.1 / 126.0 / 206.8). Fission 56.5 / 103.7, end 181.8; its payload spawns on point 0.
- **Capture point, Dyson and Combustion** (from the decompiled `CR15NetCaptureVolumeCS`, never recorded):
  server sends `SR15NetCaptureVolumeUpdate` `[space 8][pool idx u16 = 0][gen u16 = 0][1 u32][possession u16
  0 blue / 1 orange / 2 neutral][flags u16: 1 contested, 2 locked][converting u16, 0xffff][0 u16]` + per team
  `[claimer bits u16 (bit = slot)][count u16][progress f32]`; `SR15NetCaptureVolumeReset` `[space][idx][gen][1]`
  at round begin. Lock 45 s, a team alone converts in 4 s (progress down when not alone). Shapes from the level
  (`data/combat_<map>_capture.txt`): `CR15NetCaptureVolumeCR` (0x93a1c9eaaaa00b44) lists `[id][kind]` from 0x98
  (9 sphere, 10 OBB); data in resource 0xe8e38d7781a338a6 (sorted id table, second table, data). Dyson: sphere
  r 5.89 at the centre; Combustion: 7 boxes. Points (the maps' scripts natively, unknown): holder 1/s on it,
  0.5/s off it, 0 contested, 75 wins; sent in the arena team score vars (may not show on the combat HUD).
  Round time is the settings' 4000 s; time up goes to the leader.
- Gear in a match: everyone is in combat, the space is the level's id (native's Gauss server uses
  it for explosions, DealDamage, the bitfield), no lobby dummies, respawn 6.65 s after a kill
  (`PlayerRespawn [damageable][slot][0]`).
- Not done: the capture points' native HUD score vars, the server-side level-script events listed under
  "Hand-off" below. A native Dyson/Combustion recording would check the capture guesses.

## Combat pass 2 (2026-11-16, the user's first test of a combat match)

The user's report: joined straight into the map (no war-room hold), the orb had no timer, barriers
didn't respawn, broken barriers stayed visible but let shots through, lobby training dummies show no damage.
Also fixed earlier this pass: queueing for combat landed in the social lobby (`engine::session_info` now
picks a random allowed combat map unless one is given; `lobby.rs` no longer picks a second time).

### New native recordings: a full public Gauss match, both sides

- `captures/netrec-gauss-20261116/` (client, the user playing; NetRecorder 1.1 `role client`) and
  `captures/netrec-gauss-20261116-server/20261116_145847_7488/` (the native server of the same match,
  `role server`). Decoded with `echo-rust-server --decode-netrec <dir>` -> `msgs.csv`. The server side
  shows every client's c2s too. Times differ (client t+~107 s = server t).
- Scratch decoders (not in the repo): replicated-var histories via a CSV pass over
  `SR15NetReplicatedDeltaUnreliableVars` (rows from byte 30: `[hash u64][value u64]`).

### Round flow (native, server times)

| t | state (`VAR_GAME_STATE`) | match-script event (module 6fa8cc9cd721a563, handle 0x17a, filter 5) | countdown |
|---|---|---|---|
| 222.2 | (load) | `0x9ebb9a61909b5440`, `0x5ca9e3d2775375f5` | |
| 223.9 | pre_match `d69cc3ed9f1e31bf` | | 30 (`pre_match_wait_time`) |
| 252.25 | `dad765f3754f6262`, `e1c16321def4cc6c`, `fbc46082c080fbcf` (one frame each) | `0x18c4b28b0f4812c4` | 15 (`team_room_wait_time`) |
| 267.27 | `37756d8877ed5768`, round_start `7b54c48fed3ecd93` | `0x0e5ce3dcce8c0872` | 45 (`pre_launch`) |
| 312.28 | main_loop_enter, playing | `EV_ROUND_BEGIN` | clock 180 |
| 647.74 (time up) | | `0xff20bef70ee3749c`, then `0xe14549f2f4c40266` filter 13 twice; LockEntrants | |
| 648.3 | round_over ... (as before) | round over A/B, top awards; celebration +15, stats +25, lobby +40 | |

- Our flow skipped the team room and its events: Waiting -> pre_match 30 s -> round_start. Now
  `combat.rs` follows the table: `Phase::TeamRoom`, the load / team-room / round-start events, the
  intermediate states one step each (`Combat::states` queue), and the end events by winner (payload home:
  `EV_OVER_A`, `EV_OVER_B`, `EV_OVER_C` filter 13 twice, from the older recording; time up / draw: the
  `EV_TIME_UP_*` pair). `combat::Out::Event` now carries its filter.
- No teleport / orb messages at all in either recording: the war-room hold and the orb's countdown are the
  client's level scripts reading the game state and countdown. The hypothesis (unconfirmed in game) is that
  our missing team-room states/events are why the user walked straight out. The per-wave war-room locks
  (`spawn_timers.orb_a_obj_N_*` in the map settings) showed up as no replicated var: client scripts too.

### Client script events are relayed (new)

- In a combat match clients send `SR15NetComponentEvent`s from their level scripts. Native relays the
  filter-13 ones (bytes 18..20 = 13) to every other player, byte for byte, sender excluded (server recording:
  war room handle 0x11c `2082acf24514897a` during the team room, doors 0x159/0x15a, 0x15e, barriers, etc.).
  Filter-5 ones (`9e9111c27a42c0fb`, `9f9111c27a42c0fb`, `c06471ffde095d63`, `4401a3b4a69a55d5`, ...) go to the
  server only. Before this pass `Session::on_client_event` dropped them all in combat; now it relays filter 13.
- The native server also answers some with its own events (e.g. `bdb8b07964d35935` on 0x159/0x15a twice after a
  client's `786488adc2ff4737`; the checkpoint events on modules `60b5...` / `1a03...` handle 0x15b and the payload
  component 0xd0 events `d319d111f016a32b`, `56607fe3f592c908`, `a15f9368af3295e0`, `ec211f5b7798bc4f`). Those are
  native's server-side scripts and are NOT done: list them from the server recording
  (`dir == s2c`, `name == SR15NetComponentEvent`, no matching c2s just before) and map each to its trigger.

### Barriers from the level data (new)

- `CR15NetDamageableCR` (type `0xbd1868f576836696`): 56-byte rows from 0x40, actor at +0, max health f32 +28,
  respawn seconds +32 (0 = stays down). Pool slots run over the rows in order; an ordnance template
  (bytes 8..12 `ffff0f00`) takes **15** slots (Gauss's first barrier, component 329, is pool 0x69 in the
  recording). The net handle is the actor's in the level's IdsResponse.
- `tools/gen_combat_maps.py` writes `data/combat_<map>_barriers.txt` and `data/lobby_combat_barriers.txt`
  (`pool handle max respawn`). Gauss 9 x (230 hp, 30 s), handles 0x164.. (recorded pool 0x71 -> 0x16c);
  Dyson 13 (6 x 101 hp / 1 s, 3 x 230 / never, 4 x 180 / 30 s); Fission, Combustion none; lobby area 11
  (230 hp, 30 s, pool 2 never) with handles 58.. from `ids_response_2645.hex`.
- Native break (recorded twice): `DealDamage` 0 (the hit), then `ComponentEvent` `0xff278203989fcc2b`
  module `0xc4b6bab2ee99ea6a`, the barrier's net handle, filter 13, `ffffffff`; `DealDamage` 0; the event
  again. Exactly `respawn` s later the same with `DealDamage` max. `gear.rs` now does this (`barriers_of`,
  `hit_barrier`, `respawns` in `due()`); unlisted barriers fall back to 230 hp and no event.
  The break event is what makes the barrier drop visually; without it clients only stopped colliding.

### Dummies

- The lobby training dummies' path is unchanged and matches native byte for byte (only
  `DamageableLocallyTrackedDamage` back to the shooter; tests check it). The user saw no damage numbers in this
  build; not reproduced. Next: ask them to retest in the normal social lobby and compare with v0.1.4.

## Combat pass 3 (2026-11-16 late): native Dyson, Combustion, Fission recordings

Recordings (NetRecorder 1.1, client = the user, server = a native EchoVRCE server), decoded to `msgs.csv`:
`captures/netrec-{dyson,fission,combustion}-20261116/` and `...-server/<run>/`. Server t = client t + 29.0 (Dyson).
Raw balance JSON for every mode: `J:\echoApkTextureDev\Echo VR 4987570\assets\sourcedb\rad15\json\balance`
(the Dyson native block is also decoded in `captures/balance/dyson/`; its map settings = `56f338e6d9c06227.json`).

### Bug fixed: barrier events were byte-reversed

`gear.rs` `BARRIER_EVENT` / `BARRIER_MODULE` were written as the wire bytes read as a big-endian u64, so the
break/respawn event we sent was `2bcc9f98...` / `6aea99ee...`, not native's `ff278203989fcc2b` / `c4b6bab2ee99ea6a`.
Likely why broken barriers stayed visible. Test `barrier_event_is_the_recorded_bytes` pins it.

### Capture point (Dyson, Combustion), now as native

- `SR15NetCaptureVolumeUpdate` bytes 16..24: `[possession u8][0][2 uninitialised bytes][converting u16][0 u16]`
  (no contested/locked flags). Progress is seconds (0..4). Sent every frame while anything changes.
- Nothing is sent while locked: first update exactly 45.0 s after `EV_ROUND_BEGIN`, already naming the converter.
- Conversion: the converter's progress +1/s while its team is alone, -1/s otherwise (contested or empty); the other
  team can only start once it drained to 0. 4 s flips possession, from neutral or straight from the other team.
- Points (checked to the frame on both Dyson rounds): holder alone 1/s, nobody on it 0.5/s, any enemy on it 0.
  75 wins the round. No var carries points (clients' scripts count them from the updates).
- Every frame while contested (after unlock): `ComponentEvent` `eae1d95d61eb7a20`, module `c4b6bab2ee99ea6a`,
  the volume's handle (Dyson 0xae, Combustion 0xa9 = the `CR15NetCaptureVolumeCR` actor at +0x40), filter 5.
- Rounds: both maps played two rounds (one team won both; `round_count` 1 regardless). Code: first to 2
  (`CAPTURE_ROUNDS_TO_WIN`, a guess for 1-1). No `round_start`: team room 15 s opens onto the round.
  Between rounds: win events at +0, `ROUND_OVER_A` + round_over states + LockEntrants at +0.5, at +15.5
  `0x6f884020dc2e8d0c`, UnlockEntrants, `EV_TEAM_ROOM`, all barriers restored (event / DealDamage max / event,
  `Gear::restore_barriers`), team room; `CaptureVolumeReset` 5 s before the round begins.
- Early entrant lock (done): in the round that decides the match, once the leader has 75% of `points_to_win`
  (Combustion 389.26 at 56.27 points; Dyson's fits too). A first round past 75% does not lock.

### All maps: who won

- Orange: `OVER_C` (filter 13) twice. Blue: `TIME_UP_B` twice, after `TIME_UP_A` when a payload's time ran out.
  `OVER_A` / `OVER_B` never appear on a server recording (removed).
- The winner's team score var (`VAR_RED_SCORE` / `VAR_BLUE_SCORE` lo32) goes to 1; round score vars never set.

### Payload (Fission recording)

- `UpdateMiscState` `[handle][state u16][attackers u8][defenders u8][direction i8][0]`; byte 5 is now defenders.
- Defenders alone push it back at once (no 5 s wait). `PosStateUpdate` on every trackpoint change, backwards too;
  `TrackpointReached` only for new ground; the start point is never announced. All three done.
- Multi-attacker speed (x1.2/1.3/1.35) was already in.
- Misc state u16 (done): 1 during a checkpoint's celebration (clock held for `cN_celebration_timer`: Gauss 35 s / 20 s
  exactly), 2 while autodriving, 8 once the match is over (sent with the win events).
- Autodrive (done): `SR15NetPayloadUpdatedAutodriveReliable` `[handle][on u16][speed f32 0.15]`. Only on Fission,
  at checkpoint 1 (tp 58): the payload drives itself FORWARD at 0.15 m/s to the track's 0x20 point (tp 59, 59.31 m;
  `Track::autodrive`), the clock held until it arrives (18.5 s), state 2 from 5 s in, then autodrive off.
  The user thought autodrive was the "payload returning" rollback; the recordings say otherwise: every rollback is
  misc direction -1 with no autodrive message.
- Rollback ("payload returning"), measured on Gauss: 0.25 m/s exactly (0.249 / 0.250 between trackpoints), after
  5 s empty or at once with only defenders on it.
- Overtime (done): when the clock hits 0 with attackers on it, `EV_OVERTIME_A` + `EV_OVERTIME_B`
  (wire `23370169fde82c7d`, `8944e17a6a6fd420`, match module, filter 5; once per overtime). A checkpoint reached in
  overtime sets the clock to 0 + `checkpoint_N` (Gauss: 120.00).
- Early entrant lock (done): at 90% of the track (Fission 163.63 of 181.81 m, exact).
- Not done: Gauss's s01/s03 section multipliers (choke-driven by scripts); an attacker arriving during a rollback
  keeps direction -1 for 0.2-1.2 s natively; Fission's second celebration held 12 s, not its 10 s setting.

### Second Gauss match (`captures/netrec-gauss2-20261116/`, client only so far)

- The server zip (`J:\IDM\20261116_153707_10900.zip`) was not on disk yet; decode it into
  `captures/netrec-gauss2-20261116-server/` when it is.
- Confirms: defenders alone push back at once (`d00000000001ff00` many times), blue time-up events
  (`TIME_UP_A`, `TIME_UP_B` x2, lock, then the usual end), the end state u16 = 8.
- The old `EV_OVER_A` / `EV_OVER_B` are the overtime events (see Payload above): both times the clock hit 0 with
  attackers on, 7.7 s and 12.9 s before the checkpoint that saved it.

### Native-side findings for the user (not our code)

- Detonator double damage (Dyson ~23:15:55): the thrower's client sent "detonated" (`a9509fd4...`) twice, 0.21 s apart;
  native exploded on both (the second after it had already sent USED), so the victim got two
  `ExplosionPlayerDamage`. Ours explodes once (`Mag::ending`).
- Frame drop (~23:17:11): the native server process stalled 1.77 s (no frames to anyone, input queued); it began just
  after a detonator throw. Clients only sent keepalives meanwhile. Not caused by anything we run.

### Hand-off: state at the end of this pass

- 91 tests pass; Windows + Linux built; zips in `dist/` rebuilt from `target/` (the user's server was running from
  `dist/windows`, so its exe was not replaced). Nothing committed since v0.1.4.
- Next: game test of Dyson/Combustion (rounds, contested event, barriers now dropping), Fission/Gauss (defenders
  pushing back, Fission's autodrive, overtime, early lock); the open items above; war room hold; dummies' damage
  numbers.

## Combat pass 4 (2026-11-17): the level scripts' replicated vars

The user's first Dyson test: no game-mode HUD, the point orange instead of locked, no "capturing" voice/sound, it
never turned blue, an orb usable where it should not be, the timer refreshing twice. Cause: native's server runs
the levels' scripts (`bin/win10/scripts/<hash>.dll`, compiled, no names in them) and they drive the HUD and the
level through replicated vars; we sent only state/clock/countdown, over a baseline copied from a Gauss capture
(another level's per-actor keys: 314 of Dyson's 322 vars missing or wrong).

### What native sends (server recordings of all four maps, `peer 0` view)

- One var stream only. The first full `SR15NetReplicatedDeltaUnreliableVars` (~300 rows) is per level: 27 keys are
  common to all four maps, Dyson/Combustion share 43, the rest are per actor. The high 32 bits of int/float values
  are garbage (stack residue: `000001dd`, `00000230`, even a previous key's half); clients read the typed part.
- Match vars (all maps): sub-phase symbol `9de606e4…` (`6f6337a4…` → team room `d5039cdb…` → payload round start
  `541c3aa0…` → round `1509ee27…`); team room flag `16940a2d…` (1 until the round begins; payload maps: also 1 during
  some checkpoint holds, not modelled); round over flag `0eae8218…` (1 with `round_over_update`, 0 4.0 s into the
  next team room; stays 1 after the match); post-match flag `5ef77e9e…` (1 at celebration + 3.04 s, 0 at the return
  to lobby event); `f5bb086c…` celebration; round counter `4eb3e1b1…`.
- Capture maps (keys shared by Dyson and Combustion): owner `e9192a0c…` and "who is on it" `34f3a75d…` as symbols
  (locked `3fe39514…3df69263`, neutral `f558bbb6…`, blue `c8c33e48…`, orange `41d2c82f…`, both `ba11136c…`); per team
  (blue, orange): head count on the point `45000053…`/`ed17977a…`, conversion progress 0..1 `a01bc18b…`/`8ad4206d…`
  (0 again on capture), points fraction `8bd541a7…`/`ba4a488e…` and raw `c3c1c808…`/`5aa67dae…` (created with the
  first point, 0 again in the next team room), scoring now `4f8aa683…`/`df542064…` (owner and no enemy on it); lock
  count `eb40dd00…` (45 → 0); rounds won `72c15861…`/`b546f3ac…` and last round's winner `c5c2fc48…`. The owner stays
  until the volume reset 5 s before the next round (we reset it there now, the points in the team room).
- Level vars from client events: a client's filter-5 `ComponentEvent`s (door triggers `9e9111c2…`/`9f9111c2…`, area
  enter/leave `c06471ff…`/`c16471ff…`, the war room objects' `e2ac5ced…`/`ac06d3b3…`) make native set level vars at
  once, the same value every time. Learned per map (≥ 80% of occurrences, ≥ 2) into
  `data/combat_<map>_reflex.txt`; barrier vars (0 while their barrier is down) into
  `data/combat_<map>_barrier_vars.txt`.

### Done

- `tools/gen_combat_vars.py` writes each map's baseline (native's first full message + the late-created vars the
  server drives, value 0), reflexes and barrier vars. `combat_<map>_rep_s1.hex` removed (native has no stream 1).
- `combat.rs`: `Script` (reflexes, barrier vars), the match and capture vars above (`vars()`, `capture_vars()`),
  `client_event()`, `set_barriers_down()` (from `Gear::barriers_down()`); the end sequence now sends both states of
  `post_match_celebration` and `personal_stats` (enter, then update).
- `session.rs`: combat vars go out when changed, all of them twice a second; client filter-5 events in combat feed
  `client_event`.
- 94 tests pass; Windows + Linux built, `dist/` and zips updated. Nothing committed since v0.1.4.

### Not done / unknown

- Payload maps' HUD vars (Fission, Gauss: `8ba9857e…`/`a8fc5db5…` contested symbol, `70abe536…` payload state
  symbol, `a1570bd3…`/`5faf3b26…`, `d52dae83…` direction, `a989e608…` a counter 1..4, `c38d974a…`, `185651ce…`,
  `e68b8118…` a 20 s count) are not driven yet; their baselines are native's now.
- The "orb" the user could use: not identified. Candidates are the war room objects behind `e2ac5ced`/`ac06d3b3`
  (handles 0x94/0x95/0x97 on Dyson, vars `282ba8fe…`, `3653a99d…`, `6fcbc7fb…`) and the team room flag; both are now
  sent as native does. Needs the user's next test.
- Per-player vars (keys with the slot in their high bits: `f6b7893b…` slot 1, `9bc38757…` 2, `71d285d5…` 3,
  `a5b08065…` 4; floats decaying from ~0.5) are not sent.

## Combat pass 5 (2026-11-17): barriers, war room, round wins, stats and XP
- Barrier break (Dyson/Gauss recordings): `DealDamage` 0 in the hit's frame; +1 frame (10 ms) the `ff27…` event and
  `DealDamage` 0; +1 frame the level script's own event again (logged `c2s -1`: the server's loopback, not a client).
  Respawn: event + full health, the echo event 7 ms later. Ours sent all four in one frame (`gear.rs` now queues them).
- Respawn time: every barrier with the damageable row's +36 flag respawns after 30 s; +32 is not seconds (Dyson's 101-hp
  barriers have 1 there and came back after 30.0 s). `gen_combat_maps.py` fixed, Dyson's table patched; the generator no
  longer writes `rep_s*.hex` (gen_combat_vars.py owns them).
- War room targets are in space `0bef8d109eafa108`: native answers only `LocallyTrackedDamage` in that space to the
  shooter (6 x 1.4 head, from the EchoCombat damage table). Matches now use the combat weapon table, and in a match no
  tracked-damage markers for player hits (native sends none). Shot ordnance takes damage from 9999.
- Not done: explosions damaging barriers (needs barrier positions); Gauss's 415-hp template target `d263e3b2…`.
- Game state actor (module `60b50669f21b4186` on the wire; handle Dyson 0xaf, Combustion 0xaa, Fission 0xee, Gauss
  0x15b), filter 5: round won [blue `6f5d5174…`, orange `b9bbd8fc…`] + another round [`892b8089…`, `6f54f84f…`] or the
  match [`5f7f837e…`, `65663a60…`]; payload end = match won + `6d6e4517…`; checkpoints `b4448190…`, `0af13979…` (Gauss
  cp1 also `bb9a590a…`, `e093402c…` on module `1a03e276b299753e`). Team room cue `7459b7ca…` (module/handle all ones)
  3.07 s and 4.05 s into a between-rounds team room. Unsent: Gauss `8c9fbdd6…` 21.4 s after cp1.
- The end events (round over A/B, celebration, personal stats, return) already matched native.
- Combat stats (`stats::CombatStats`): every 3 s, at a round's win and at the end, `ReliableStatUpdate` per slot with
  running totals (CombatObjectiveDamage, Eliminations, ObjectiveEliminations, ObjectiveTime, Kills, Assists, Deaths,
  Damage) and `ReliableTeamStatUpdate` per team: 8 records `[sym][m i32][e i32][players i32]`, value m*2^e (reals on a
  1/64 grid, whole reals with factors of 2 in e). Objective = in the capture volume / within 3.5 m of the payload while
  playing. Assists: damage in the last 10 s (guess; native Dyson had none).
- Awards: MVP from the winning team (score guessed); tier rewards one per player, gold→silver→bronze over
  Eliminations, ObjectiveEliminations, ObjectiveTime (150/100/50 XP). XP (fits all four recordings): win 1200, loss
  1000 if present for half the match, +100 MVP, + tier XP. `SNSRefreshProfileFromServer` `[platform][account][json]`
  with the stats under combat/daily_/weekly_. Native's "Level" add (21 for the MVP, 3-5 others) is not derivable: not sent.
- Kept: built-in lobby → `Store::apply_stats` ({cnt, op, val}); nakama → native `SNSUserServerProfileUpdateRequest`
  `[xpid][{"matchtype": echo_combat, "sessionid", "update": {"stats": {"combat": {op, val, cnt}}}}]` on its socket (both
  bridges; nakama3 `userServerProfileUpdateRequest` → statistics boards, XP forced to add); EchoRelay → its login service.
- Launcher updates now come from `echo-vrce/Echo-RS`.
- Contested (fixed after the user's test): `SR15NetCaptureVolumeUpdate` byte 18 (after the possession u16) is a flags
  byte, bit 0 = both teams on the point; the client's contested announcer/effects key off it. Its other bits and byte
  19 are uninitialised in native (that is why it looked random). The `eae1d95d…` event every frame was already sent.

## Combat pass 6 (2026-11-17): barrier splash, barrier visuals, war room dummies

- **A broken barrier is hidden by a level var, not by its events.** Native's level script answers its own break event
  (the "c2s -1" loopback) by setting one var per barrier 1 -> 0 a frame later (back on respawn); clients draw what the
  var says. **The var is a CSymbol64 of a name holding the barrier's net handle in hex, 34 characters before its end**:
  two barriers' vars differ by exactly the CRC (linear) of their handles' differing hex digits. So one recorded var
  gives every barrier's (`barrier_var` in `gen_combat_vars.py`, which asserts each result is a baseline var at 1 and
  equals every recorded one). All 13 of Dyson's (0xb8..0xc4) and 9 of Gauss's (0x164..0x16c) are in
  `combat_<map>_barrier_vars.txt`; Fission and Combustion have no level barriers (their damageable tables).
- **Splash on barriers** (native Dyson dets): per barrier reached, `DealDamage` with the new health, then
  `ExplosionPlayerDamage` with victim `ffff…` at the barrier's point nearest the blast; the break follows as for bullets.
  Damage x the damage table's `barrier` (det 2.15: 65 → 140 breaks a 101; rocket_aoe 2; scout_aoe 0). Det reach 2.5 m,
  full to 1 m then down to 40% (native left one 2 m off with 97 taken). Barriers are boxes now:
  `combat_<map>_barriers.txt` carries each actor's place and rotation (level names table) and a box by health
  (`BARRIER_BOXES` in `gen_combat_maps.py`, from recorded bullet hit points; Dyson 101s: half 1.34 x 0.7, z 0..0.26 —
  native's blast points sit on those edges).
- **War room dummies**: the war room level (`0bef8d109eafa108`) has 12 damageables (7 HP), pool slots 0..11, world
  places from its names table (blue and orange war rooms; bullets hit them 1.2–1.4 m above). In a match explosions
  reach them; the marker goes in the war room's space naming `[slot u32][1]`.
- **Body parts** (native war room markers, BLASTER and COMET): 0/1 legs, 2/3 arms, 4 torso, 5 head. We had only 2 = arm,
  so legs and part 3 took torso damage (players too). Gauss's SMG gave 1.0 on 2/3 then: its table had arm 1 at the time.

## Combat pass 7 (2026-11-17): the payload, from the Gauss and Fission server recordings

- **Claiming is a hand on the payload, not being near it.** `SR15NetTouchStateEvent` (mailbox, c2s):
  `[space u32][slot u16][u16][sequence 10 bits][hand 7 bits][hand 7 bits]`, each hand the touch entry it holds
  (none = the level's entry count: Gauss 110, Fission 109). The payload is entry 105 on both maps. Every attacker /
  defender count change in `UpdateMiscState` is 1-2 ms after that player's touch state with a hand going on or off
  105 (a player standing on the cart flipped it 1, 0, 1 without moving). Dead players do not count (Fission 238.52).
  The session keeps `on_payload` per peer; `Combat::set_payload_holders`. The old 3.5 m radius is gone from the claim
  (it is still what the match stats' objective time uses: what native counts there is not recorded).
- **The payload's dynamics are the engine's** (`echovr.exe` server update at 0x140d84ff0, `CR15NetPayloadCS`), with the
  level's `CR15NetPayloadCR` row (type `3850c0283892236e`, 64 bytes after the 0x40 header, the same on Gauss and Fission:
  +0x10 flags 0x20 SlowReverse, +0x1c 1.0, +0x20 6.0, +0x24.. 1/1.2/1.3/1.35, +0x34 1.0, +0x38 0.1, +0x3c 0.2):
  - an inner speed moves toward its target by 1.0 m/s each second (+0x1c);
  - attackers alone: target = max speed x `payload_speed_multi_attacker_N` (N capped at 4), idle timer 0;
  - both teams, paused (a celebration) or autodrive: stopped at once, idle timer 0;
  - nobody: the idle timer runs and it is stopped; past 6.0 s (+0x20), or with a defender's hand on it at once, target =
    -(1.0 + 0.1 x defenders) x max speed (+0x34, +0x38); stopped where it cannot go back (the floor);
  - a negative inner speed moves the cart x 0.2 (+0x3c): 0.25 m/s at 1.25, 0.15 on Gauss's 0.75 stretch (both recorded);
  - the direction byte is the speed's sign; the state word is the component's flags (1 paused, 2 autodrive,
    4 irreversible, 8, 16).
  `ResetMaxMPS` (0x140d3ef90) puts the settings' speed back; a level script's `SetMaxMPS` changes it.
- **Gauss's speed**: 1.25 m/s from the start through checkpoint 1 up to trackpoint 87 (85.49 m), then 0.75 m/s
  (`s02_payload_speed_multiplier`, taken as m/s: 1.25 x 0.75 was not what native ran), slowing at 1 m/s each second.
  `s01_payload_speed_multiplier` (1.7) showed nowhere in section 1. The level script is `e9a57ec0c6890cb1.dll`
  (`section01_first_speed_update`, `section02_bottom_ramp`, `section03_post_tunnel`, `section03_final_push`): its
  triggers are not trackpoints (no script holds a trackpoint's actor id). **Not known**: where section 3's speeds
  (pre tunnel 1.0, post tunnel 1.25, final push 1.0) apply: the recorded match ended at 110 m. Past 85.49 m the server
  stays at 0.75 m/s.
- **State 16**: an empty payload that rolls back onto its last checkpoint reports the trackpoint before it,
  state 16, direction 0 (Gauss 521.04); a hand on it puts it on the checkpoint's trackpoint again at once. Native
  cleared 16 when a later push back ended (561.99; one sample each way).
- Scripts: `bin/win10/scripts/<module>.dll` are the level and mode scripts, compiled node graphs named by their module
  symbol (`6fa8cc9cd721a563` the match, `45e8df51b7524e20` `mp_combat_gamemode_payload`, `adf5c8a958dbf153` the
  payload's lights and sounds). The native servers' own balance blocks are in the recordings
  (`SR15NetBalanceSettingsMassUpdate`): the det has no radius or damage there, only its `barrier` 2.1538462.
- Still not from native: barrier box sizes for the 180 and 230 HP barriers (bullet-hit extents, few hits), a det's
  falloff on barriers (one partial sample: 97 of 140 about 2 m off), `CAPTURE_ROUNDS_TO_WIN` (both native capture
  matches ended 2-0: first to two fits, a 1-1 was never seen; the rule is in the match script).

## Arena disc desync fixes (2026-11-17; untested in game, not committed)

- **Silent or departed holder**: in play, if the holder (`arena.granted`, else `disc_owner`) has left or sent no
  disc state for `DISC_SILENCE` (1 s; native never paused over 0.2 s), `Arena::park_disc` takes the disc: the
  server publishes it at the holder's last position (`arena.parked`, used after `frozen`, before the spawn) with
  the reset count moved on, live and free to grab. Not while paused / resuming. Cleared by goal, launch, new round.
- A disc state from a player the server did not take the disc from is no longer forwarded while the server
  publishes its own (`!server_disc` in the forward path): a stale holder's states carried the server's count.
- **Server disc taken without a grab** (in play): the count now moves past the taker's and `granted` names them,
  as for a no-grab take of a player's disc; in both cases the touch entries now name the taker.
- **Grab with nobody owning the disc**: granted in the arena whatever owner the requester believed.
- **Private free play** (before Start): grabs are granted (`Arena::disc_free`; they were all declined as
  "locked", so a second player's disc was ignored by the others), and the no-grab take and the silent-holder
  park apply there too (not before anyone has touched the disc).
- **Lobby objects**: `Props::expire`: an object its owner has not sent for 2 s is the server's again (asleep
  where last reported), so thrown objects at rest are sent to late joiners.
- The server disc stamp now wraps at 32 bits (it passed 2^32 µs about 70 min into a session; a non-zero high
  word made clients drop the holder's disc). Unknown: whether clients ignore the server's disc for the ~60 s
  in which the server's stamp has wrapped and theirs has not.
- Suspected, not changed. Lobby: a previous owner's records are still relayed after a grant (the arena
  drops them). Pause: nothing holds the disc server-side while paused.

## Relay timing (2026-11-17; untested in game, not committed)

- **Frame clock**: `engine::frame_ticks` — one `frame-clock` thread sleeps to each frame's exact time and wakes
  every game server through a watch channel (a late frame is skipped). The tokio interval it replaces lands on
  whole milliseconds. Measured on this PC at 120 Hz (sleep only, not through tokio): lateness spread (p95-p05)
  0.69 ms against 1.16 ms for whole-ms deadlines, median 0.37 against 0.74 ms. Windows' timer floor is ~0.5 ms;
  closer needs spinning (CPU), not done. `frame-hz N` is picked up by the thread on its next frame.
- **Disc in flight goes out on arrival**: `Transport::mailbox_now` sends the forwarded disc state in a mailbox
  frame of its own to every other player at once (it waited up to a frame). Held discs (state byte 25 = 2) still
  go with the frame, beside the holder's player state. Cost: one extra small packet per flight state per peer.

## Abandoned disc handed to a client (2026-11-17; EXPERIMENT, untested in game, not committed)

No server physics and no level geometry: when the disc's holder leaves or goes silent (`DISC_SILENCE`), the server
first sends another team player's client a `TOUCH_GRANTED` it did not ask for (`Session::delegate_disc`: not held,
next generation, that client's last touch seq, the object / entry / actor of the last real grab), so that client's
own simulation carries the disc on and everyone follows its forwarded states. **Whether a client accepts an
unrequested grant is unknown** (no recording has one). If it sends no disc state within another second the server
parks the disc as before. The log says which: "abandoned disc: handed to..." then either "...took it over and
simulates it" or "...sent nothing". If it never works, remove `delegate_disc`. The same idea would fit lobby props.

## Goals, awards, registration pacing, live server count, Settings layout (2026-11-17; untested in game, not committed)

- **Disc state byte 25** (native 2026-11-11, checked against its touch requests): 2 in a hand, 0 after a throw,
  4 after a hit (a headbutt; the disc cannot be punched: from the grant of a request with the hit bit until the next grab). We read 4 as
  "thrown" and 0 as unknown. **Goal types**: in a hand = slam dunk (1) with speed 0 and distance 0 (the user's rule;
  we sent the hand's speed); hit flight = headbutt 6 / long headbutt 7 (native's recorded type 7: thrown, headbutted
  2.4 s later, distance 8.86 m measured from the throw's release); a flight that turned > ~25 degrees in one step
  at >= 1 m/s = bounce 4 / long bounce 5; else inside 2 / long 3. **Bumper shot 8 / long 9** (the user's
  definition): thrown at an opponent who headbutts it (not a catch) into their own goal: the thrower
  scores (`GoalInfo::scorer`, `DiscTracker::bumper`), distance from the throw's release, and it is not an own goal
  (so it counts with self goals off).
- **Top awards: one per player** (every native award list in `captures/` names each slot once): MVP first, then
  each stat's best among players still without one.
- **Registration pacing** (`upstream::Gate`): one game server registers first; an error answer from the backend
  (nakama `Error`, `ER_REGISTRATION_FAILURE` once the layout question is settled) = nobody tries again until restart;
  no answer within 10 s / connection lost = that one tries again after 10 s, 20 s, 40 s ... (max 5 min); once one is
  registered the rest go five at a time, and one refused then gives up alone. Before: every instance reconnected
  and registered again 0.5 s after any failure (10k requests from 128 servers).
- **Server count while running**: stdin `servers <most at once> <kept running>` -> `Pool::set_limits`; the launcher
  sends it when either number changes. Raising starts servers at once; lowering stops none.
- **Launcher Settings**: Combat pages are per gear (`settings::gear_items`): a gun / tac mod / ordnance shows all
  its entries (its own table, its cone / explosion, its damage table) on one page; the separate Damage tab is gone.
  Names and order are `mp_equipment.json`'s (PROTOSTAR, QUARK, BLAZAR, STARBURST, TRANSLOCATOR, ATTACK DRONE,
  POWER BOOST, POINT BLANK). The stored `balance` changes keep their format.

## Payload HUD and private combat matches (2026-11-17; untested in game, not committed)

- Payload HUD vars (`combat.rs` `payload_vars`), from the Gauss and Fission
  server recordings: claim state (`Unclaimed` / `attackerPossessed` /
  `contested`; a defender alone stays `Unclaimed`) plus each level's copy,
  motion (`still`/`forward`/`backward`; a self-driving payload stays
  `forward`), direction (`backward` only while rolling back), moving (1/0/-1,
  0 while held or self-driving), attacker claim (0/1 and the flag byte),
  defender claim (count), `overtime_progress` 1.0 from the attackers' first
  claim, Gauss's door count (20 s down to 0 from checkpoint 1).
- Level vars set on a trackpoint's first reach (checkpoint counter and symbol,
  load progression, ...): `data/combat_<map>_track_vars.txt` from
  `tools/gen_track_vars.py <map> <server recording msgs.csv>`; applied for
  every trackpoint passed (`reach_track_vars`).
- Private (`MODE_COMBAT_PRIVATE`, `Combat::set_private` from `Session::new`):
  state `pre_match_wait_for_ready` until every team with players pressed its
  terminal's ready (`nevt_blue/red_team_ready`, handled before the combat
  relay in `on_client_event`; `blue/red_team_ready` vars), then straight to
  the team room; Dyson/Combustion then play as public.
- Private payload: two halves. First half ends at time up (after overtime) or
  home: `to_beat` = the payload's furthest point; round over as a capture
  round, then at the team room `Out::SwapTeams` (session `swap_teams`: every
  blue<->orange, `EntrantTeamChanged` each) and the payload back to the start
  (`reset_payload`: `PosStateUpdate` first trackpoint, misc 0, track vars back
  to before). Second half: orange (the old blue) wins on reaching `to_beat`;
  time up short of it, blue (the first attackers) wins. Marker/round vars
  (`payload_furthest_distance_round_1/2`, `payload_team_a/b_distance`,
  `payload_team_a_marker_*`, `payload_progress_marker_*`,
  `net_total_round_count`, `net_rounds_played`, `defending_team_index`) are
  dictionary names, NOT seen in any recording: whether the HUD shows the
  marker from them is unverified. Whether clients accept the payload going
  back (TrackpointReached only rises) is unverified too.

## Arena stats to nakama, `save_loadout` feature (2026-11-17; untested in game, not committed)

- **Arena stats.** At an arena match's end (`awards_due`, with the top awards)
  `Session::send_arena_profile_updates` sends one
  `UserServerProfileUpdateRequest` per joined blue/orange player out through
  `fleet_out`: payload `{"matchtype": symbol("echo_arena"), "sessionid":
  <lobby>, "update": arena_profile_delta(...)}`. `arena_profile_delta` builds
  `{"stats":{"arena":{name:{"op","val","cnt":1}}}}` (the shape atlas's
  `ArenaStatistics` reads): every non-zero `STAT_NAMES` stat as `add`
  (PossessionTime in seconds), `HighestPoints/Saves/Stuns` as `max`, one of
  `ArenaWins/ArenaLosses/ArenaTies` and `GamesPlayed`. Private matches decide
  the result by round wins when they differ, else by goals.
- The bridge forwards it: to nakama as the native SNS frame
  (`upstream.rs` `profile_update_frame`, platform/account u64 + JSON);
  to EchoRelay over a short login-service connection (`echorelay_profile_update`).
- Atlas accepts these from non-native servers (atlas
  `runtime_event_server_profile_update.go`) and **ignores private arena and
  combat matches on purpose**, so only public matches fill the boards. Skill
  ratings are not updated through this path.
- **`save_loadout`.** `upstream.rs` `with_features` appends
  `features=save_loadout` to the backend URL (both the nakama and the
  EchoRelay bridges; a URL that already has `features=` is left alone).
  Atlas then skips the client's lossy `item_equipped` remote log for this
  server (`GameServerFeatureSaveLoadout`), since this server saves the exact
  loadout itself (`save_loadouts` / `loadout_save`). Tests:
  `nakama_url_gets_the_save_loadout_feature`,
  `arena_profile_delta_has_natives_stat_shape`.
