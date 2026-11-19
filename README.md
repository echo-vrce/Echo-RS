# echo-rust-server

A standalone Echo VR server written entirely in Rust. You do not need a copy of Echo VR or `echovr.exe -server` to host matches.

One process runs:

- **Central services**: login, config, matching, transaction and the HTTP API, all on one port. The client connects to these.
- **A self-scaling pool of game servers** running this project's own engine (`src/engine`). Each instance owns one UDP port and hosts one lobby or match. An idle instance does no work at all: no timer runs until a session starts.

The pool can also serve an external **EchoRelay** or **nakama (EchoVRCE)** backend instead of the built-in services.

> **Disclaimer.** This is an unofficial community project. It is not affiliated with or endorsed by Ready At Dawn or Meta. It contains no game code or assets; you need your own legally obtained copy of the Echo VR client to connect.

## Build

### Requirements

- Rust 1.85 or newer (edition 2024)
- A checkout of [nevr-server-rs](https://github.com/EchoTools/nevr-server-rs) **next to this repo**. Only its wire primitives are used, unmodified: `nevr-crypto` (AES/HMAC/Keccak), `rad-protocol` (message codec, header bits, symbol hashes) and `nevr-proto` (the fleet protobuf schema). The game server itself is not nevr-server.
- `protox`, a pure-Rust `protoc`, so you don't need to install protoc.

```
some-folder/
├── nevr-server-rs/     # git clone https://github.com/EchoTools/nevr-server-rs
└── echo-rust-server/   # this repo
```

### Steps

```
git clone https://github.com/EchoTools/nevr-server-rs
git clone <this repo> echo-rust-server
cd echo-rust-server
cargo install protox --features bin --root tools
cargo build --release
```

`.cargo/config.toml` points `PROTOC` at `tools/bin/protox.exe`. On Linux or macOS, set it yourself: `PROTOC=tools/bin/protox cargo build --release` (an existing `PROTOC` environment variable takes precedence over the config).

Run the tests with `cargo test`.

## Run (built-in services)

```
target/release/echo-rust-server
```

On startup the server:

- prints the client config and writes it to `server-data/client-config.json`
- listens on TCP `7777` for the websocket services and the API
- uses UDP `6792+` for game servers

To connect a client, put the printed config in `ready-at-dawn-echo-arena/_local/config.json`. Change `auth=` (your account password) and `displayname=` in `loginservice_host`.

Firewall: open TCP 7777 and UDP 6792 to 6792+max-instances.

| Flag / env | Default | Meaning |
|---|---|---|
| `--bind` / `ERS_BIND` | `0.0.0.0:7777` | services + API |
| `--data-dir` / `ERS_DATA_DIR` | `server-data` | accounts, resource overrides |
| `--lan-ip`, `--public-ip` | auto / LAN IP | addresses advertised to clients |
| `--udp-port` | `6792` | first game-server port (instance N uses port+N) |
| `--min-idle` | `2` | game servers kept ready |
| `--max-instances` | `64` | upper bound on concurrent lobbies/matches |
| `--idle-retire-secs` | `0` | `0` keeps every started server registered as a blank idle server; a positive value stops spare idle ones after that many seconds |
| `--arena-min-per-team` | `0` | arena: players needed on each team before the 45 s pre-match countdown starts; `0` starts it on the first join, as native public servers do |
| `--no-unlock-all` | off | only default cosmetics instead of all |
| `--log` / `ERS_LOG` | `info` | tracing filter (`info,echo_rust_server::engine=debug` for engine detail) |

### Accounts and resources

- **Accounts.** An account is created on first login and saved to `server-data/accounts/<XPID>.json`. The first `auth=` value used locks the account to that password.
- **Resources.** To override any embedded resource, drop a JSON file in `server-data/resources/`. The names are `game_settings`, `channel_info`, `eula`, `config_main_menu`, `config_active_battle_pass_season`, `config_active_store_entry` and `config_active_store_featured_entry`.

### Admin

The admin endpoints are loopback-only:

- `GET /status`: servers, lobbies and players.
- `GET /admin/kick?xpid=OVR_ORG-123`
- `GET /admin/ban?xpid=OVR_ORG-123&minutes=60` (`minutes=0` unbans).

### Matchmaking

- **Find.** A find request fills the fullest public lobby of the same mode in the same social channel. Party members follow their leader.
- **Join and create.** Join-by-id, `-moderateuser` and private create are supported.
- **Lobby sizes.** Social lobbies hold 12. Arena is 4v4 and combat is 5v5, plus spectators and moderators.
- **Empty lobbies.** A lobby that stays empty for 30 s is ended, and its game server returns to the pool.

## Run against EchoRelay

The game servers register with an EchoRelay ServerDB, and EchoRelay does login and matchmaking:

```
echo-rust-server --backend echorelay --upstream ws://127.0.0.1:777/serverdb
```

Append `?api_key=...` to the upstream URL if EchoRelay requires one. Clients keep using EchoRelay's config. Pick a different `--bind` port if 7777 is taken. This mode uses the port only for the local bridge.

## Run against nakama

```
echo-rust-server --backend nakama --upstream "ws://<nakama-host>:7350/ws/nevr?token=<session token>"
```

nakama serves nevr game servers at `/ws/nevr`, which takes a session `token`. The bridge accepts both bare and legacy-wrapped protobuf framing, and it rewrites a loopback registration address to the LAN IP.

## Test client (`--probe`)

The binary includes a headless client. It runs the real client's websocket flow (login, profile, config, EULA, channels, matchmaking, player sessions) against any compatible service.

```
echo-rust-server --probe ws://127.0.0.1:7777 --probe-clients 60 --probe-mode social
echo-rust-server --probe ws://127.0.0.1:777  # through EchoRelay
```

It prints where each client was placed. It also prints a `nevr-test-client` command line (an external test tool), which runs the encrypted UDP handshake and join request with the issued keys. Use `--probe-hold-secs` to keep the seats while that runs.

## Game-server engine (`src/engine`)

- `transport.rs`: packet crypto (MAC, sequence, AES-CBC, Keccak IV stream with reorder tolerance), reliable channel (in-order, windowed, retransmit at about 2× round-trip time, capped at 60 ms), mailbox channel (latest wins, never stalls on loss), fragmentation, and the compression switch-over.
- `session.rs`: the join sequence (`SNSLobbyJoinSetup`, entrant states, owner/host state, `SNSLobbyJoinAcceptedv2`, then `SNSLobbyJoinComplete` and server info), message relay, replicated game state, disc ownership arbitration, and the arena rules.
- **Relay.** Player and physics traffic is forwarded the moment it is decoded, not on the next frame.
- **Ping.** Ping is measured, never a constant. The transport times each outbound mailbox frame until the client's mailack for it comes back. A client only acks on its next outgoing frame, so one client send interval (the median gap between its frames, about 13.9 ms at 72 Hz) is taken off: that is the client's render cadence, not network distance. The reported value is the minimum over the last 32 round trips, so a local server shows about 0 ms.
- **Server clock like native's.** The frame word counts 120 Hz ticks and `server_us` is exactly frame × 8333 µs, both measured from machine uptime (native's frame is about 4 million). They stay small, and they never go backwards when this process restarts. The client drops any frame not above the last one it saw, even one from a previous server, and its own match timers (the tube countdown and doors) run on this clock. Frames are published 20 times a second, as native does.
- **Arena flow.** Timed to a recorded native game: a full public arena match with two goals, both directions decrypted (see *Recording a native game*). The 45 s `pre_match` countdown starts as soon as each team has `--arena-min-per-team` players (by default, on the first join).
  1. A 45 s countdown runs in `pre_match`. The disc is the server's: it stays parked at centre and touch-ownership requests are declined until play starts.
  2. **At 0:**
     - A pre-match-done state (`0xdad765f3754f6262`) with `disable_local_disc` and `round_start`.
     - Next frame: `round_start_enter`, and the replicated **tunnel-doors flag** (`0xe4056c13b804cba2`) opens.
     - Then `round_start_update` (plus flag `0x1ec783ef243da0c4`), then two more round-start states, the second with a **4.5 s count**. Native runs this count too.
  3. `pre_launch_enter` with a disc reset, then `pre_launch_update`: a 15 s count in the tubes. The disc reset and its FX are restated every frame.
  4. **At 0:** `enable_scoring`, the **launch-gates flag** (`0x72652e2ce806d370`) opens, then `new_round_started`, `main_loop_enter` (the not-playing flag clears) and `playing`, with a 300 s clock.

  Doors and gates are replicated flag variables, not events. The client's match graph opens them when their low byte goes from 0 to 1.

  **Goals** are detected from the disc transform the owning client reports. As native does:
  - Two goal events (filter 13, sent twice), `game_score` and `point_score`, and the launch gates close.
  - Then a 15 s celebration with the hold flag and the announcer's goal call.
  - Then the same round-start walk and 4.5 s count as a fresh round, then relaunch. The relaunch disc FX is orange after a blue goal; after an orange goal it is white, because the blue colours are unrecorded.

  **The disc after a goal.** As in the native recording, the server takes the disc at the goal:
  - It pins the disc on the goal plane (depth ±36000) where it crossed.
  - It adds 3 to the reset count in the low nibble of the disc state word. The client snaps to the server's disc whenever that count changes.
  - It ignores clients' disc updates until play resumes.
  - At the 4.5 s count it parks the disc on the conceding team's side (depth ±27500).
  - During the round restart it sets the disc-spawn var (`0x7d0a66581bb45536`) to the conceding side: 1 is orange, 0 is blue (assumed). It goes back to 2 (centre) at match end. Every `restart_disc` respawns the disc wherever that var says, so without it the disc snaps back to centre.

  **Touch grants** set `0x10` for an exclusive grab and `0x20` when the requester already held the disc. Without `0x20` the client plays the steal sound on every re-grab.

  **Stats.** Every 3 s of play, each player gets an `SR15NetReliableStatUpdate` (`[slot u32]` plus JSON `add` deltas), and each team an `SR15NetReliableTeamStatUpdate`. The team update holds 12 records of `[hash][mantissa][exponent][count]`, with possession in 2^-6 s. Points, goals, shots on goal and possession are tracked.

  **Awards.** At match over, `SR15NetTopAwardsMsg` names the MVP (the player with the most points) for the end-of-match board. Each player gets at most one award.

  **Mercy.** A goal that reaches the mercy margin (12 in public, the private rule otherwise) ends the match at once, outside overtime.

  **Overtime (sudden death).** Public time up with the score tied follows native's sequence (from a recorded public overtime): `round_over`, then `pre_sudden_death` (`sudden_death_active` 1, hold, the draw call), then `sudden_death_new_round`. Everyone is sent back to the launchers, the overtime round is announced in `pre_launch` (`MP_ANNOUNCER_OT_ROUND_<n>`), and play relaunches on a 2:00 clock. The next goal ends the match after its celebration. A scoreless overtime round starts another. The timings come from the round rules (`enable_sudden_death`, `sudden_death_round_time`, `sudden_death_round_over_time`).

  **Disc at time up.** The disc stays live when the clock runs out: the holder keeps simulating it and grabs are still granted. Only a goal pins it. The next round takes it back for the server.

  **The match ends** with the seven-state post-match tail, plus native's end-doors, celebration and round-score flags. If a team empties before launch, the match goes back to waiting.
- **Social lobby.**
  - **Cease fire per entrant.** Every entrant (slot, generation) has its own cease-fire replicated var, and a client obeys only its own. The server sends every joined entrant's var to everyone, in the snapshot and whenever one changes (`msg::entrant_hold_var`). The cease fire is lifted only for a player whose tracked position is inside the brawler room. The combat area (gun range) keeps it on, and the server drops stuns from a player under a cease fire. The var name for two-digit slots or generations is unknown, so slots 10 to 15 have no cease fire yet, and generations wrap mod 10.
  - **Quarter court.** A disc passing through the goal ring from the front scores 2 points, or 3 when it was thrown from outside the bubble in front of the goal. The server sends native's game-state change, scoring event and `PostScoreUpdate`.
  - **Combat room.** All 11 barriers hide when broken and respawn after 30 s. The ability station and the room's other switches are replicated vars set from the clients' events.
  - **Joining.** Others' body state reaches a newcomer only after the newcomer's first equipment update, which is when native starts sending it too. After it, the newcomer gets everyone's latest equipment. The server publishes each entrant's lobby data (`afk`, `spawned`, `mm|status`) and relays the clients' level-script events. Replicated vars carry the lobby frame as their sequence and go out every frame.
- **Combat explosions** follow native's query and falloff. A victim not on blue or orange is never hit, and the owner's teammates are skipped, but the owner is hit by their own blast. Damage is scored per body part (fixed offsets below the tracked head) with the damage table, and the best part counts.
- **Component events are addressed to real handles.** The arena IdsResponse we send (native's own) assigns the match graph handle `0xa02d` and the disc-FX handle `0xa01b`, so round-start, scoring and announcer events go to those handles, as native does. The handles come from whatever IdsResponse the server sends: a different native session used `0x002f`/`0x001b`.
- **Arena baseline.** The replicated-variable baseline is native's own pre-match set from a fresh public match. An older baseline taken mid-match carried flag values that made the client run pre-match like a private match, with the tunnels open.
- **Session end.** A session ends 3 s after its last player leaves (90 s if nobody joined), and the server is reported free to its backend (EchoRelay gets `EndSession`). EchoRelay itself marks a server Locked when its player count reaches 0, so ending quickly keeps that window short. Servers then stay registered, blank and idle.
- **Captured data.** The level object ids, balance settings and replicated-variable baselines (`src/engine/data`) are captured protocol data the client needs byte for byte. They exist for `mpl_lobby_b2` (social), `mpl_arena_a` (arena) and the four combat maps (Combustion, Dyson, Fission, Gauss), with each combat map's barriers, reflexes and payload track vars.

## Recording a native game

`tools/record_native.py <out-dir>` records a native `echovr.exe -server -headless` session end to end. Point the native server's `serverdb_host` at `ws://127.0.0.1:7781/serverdb`. The recorder proxies that socket to EchoRelay on `:777` and logs every message, including the SessionSuccess v5 that carries the packet keys. It also rewrites the server's registration to advertise UDP `6800`, where it relays and logs every datagram to and from the server's real port.

Then:
- `echo-rust-server --decode-capture <out-dir>` decrypts both directions with our own transport into `<out-dir>/msgs.csv`, one row per message with its name.
- `python tools/timeline.py <out-dir>/msgs.csv src/engine/msg.rs` prints state, flag and score changes, component events and reliable messages over time.

The recorder needs no libpcap: loopback traffic never reaches a capture driver on Windows.

## Verified

- **Codec.** Every encoder and decoder matches byte for byte the vectors from nakama's `server/evr` Go implementation (`cargo test`).
- **Built-in mode, own engine.** Probe login and placement, then the test client's raw ping, encrypted handshake, key claim, `SNSLobbyJoinRequestv4`, backend accept and join batch, all against `src/engine`.
- **Game clients on the new engine.** Social lobbies, public and private arena matches and combat matches have been played with real clients, multiplayer included.
- **Load (previous engine).** 60 concurrent clients were packed into 5 full lobbies of 12. With 64 game servers up, the process used **47 MB RAM and 0.4 s of CPU in total**.
- **EchoRelay mode (previous engine).** Tested against a live EchoRelay through every step:
  1. Registration with the ServerDB
  2. Matchmaking and EchoRelay's ping round
  3. Session start
  4. UDP join
  5. PlayersAccepted
- **nakama mode** runs against a live nakama (Atlas): registration, matchmaking, profile and stats updates.

## Notes

- **63-bit sequence IDs.** Packet sequence IDs are 63-bit on the wire, and the server masks them. This also covers EchoRelay, which issues full 64-bit random values.
- **EchoRelay ping results.** EchoRelay keys ping results by (internal IP, external IP) and throws on duplicate keys. Several game servers on one IP share that key, so the probe sends one result per address pair.
- **Windows UDP resets.** On Windows, the game-server UDP sockets ignore ICMP "port unreachable", so a departed client can't cause recv errors.

## Not in this repo

- `server-data/` is created at runtime and holds accounts (with their passwords) and your generated client config, so it is git-ignored.
- `captures/` holds native-game recordings (pcaps, decrypted traffic, session keys) and is git-ignored. Make your own with `tools/record_native.py`.

## License

MIT. The `nevr-server-rs` crates it depends on are licensed separately (Apache-2.0).
