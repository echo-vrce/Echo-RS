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
  (`heisthecat31/Echo-RS`, asset `echo-rust-server-windows.zip` / `-linux.zip`) at start
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
  `Gear::may_fight` (combat area, or `in_brawler_room` box x 19.5..42, y -3..5, z 9.7..26), sent on change and
  every second; logged as "lobby cease fire lifted/on" with the position. The box is fitted to one native
  walk: if punching fails in part of the room, the log's position shows where.
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
