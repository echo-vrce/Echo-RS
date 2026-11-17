# Handoff: atlas (nakama) work and local test setup

Status on 2026-11-17. Nothing in this file has been committed. **Do not commit this file to atlas** (atlas's
AGENTS.md forbids handoff documents in that repo). It sits in EchoRustServer because the work started here.

Repos involved:

| Repo | Path | State |
| --- | --- | --- |
| EchoRustServer (Rust game server + launcher) | `J:\EchoVR-Tools-Launcher\EchoRustServer` | uncommitted changes |
| atlas (EchoTools nakama fork, Go) | `J:\EchoVR-Tools-Launcher\atlas` | uncommitted changes |
| EVR-Runtime (new client runtime, another session) | `H:\EVR\EVR-Runtime` | owned by the "Echo VR runtime update system" session |
| EchoRelay (reference only) | `J:\EchoVR-Tools-Launcher\EchoRelay` | read only |

## 0. Current state (2026-11-17 21:30) — read this first

**Running**
- Atlas: `HOWTOBUILD/local/atlas.exe` = `atlas.2130.exe` (PID 16176 since 21:22). Older builds kept as `atlas.2059.exe`,
  `atlas.2037.exe`, `atlas.2011.exe`, `atlas.1944.exe`; their logs as `logs/atlas.<time>.stdout.log`.
- Rust game servers: `dist/windows/echo-rust-server.exe` (20:07 build, `server_mute`), 8 of 8 registered.
- Game: Echo VR PC with EVR-Runtime `plugins\versions\0.8.0\evr-runtime.dll` = a **party-free** build from 21:26
  (made from a copy of the runtime tree in this session's scratchpad with `src/party` left out). Previous DLLs kept as
  `evr-runtime.2120.dll`, `.2115`, `.2055`, `.2034`, `.pre-earlyquit`. **Do not install `runtime/build/evr-runtime.dll`**:
  it contains the party code, which has never run in the game.
- The user is a Global Operator in the local Atlas DB (system group "Global Operators").

**Working (seen in game)**
- `/lockout` (Discord, member autocomplete or username/EVR ID/Discord ID/account ID): LOCKOUT NOTICE with working OK,
  countdown bar on tablet and main menu, and the countdown now also arrives at login after a rejoin.
- Atlas refuses public Arena/Combat finds during a moderator lockout ("You are locked out of matchmaking for …"; the
  client shows KICKED FROM LOBBY GROUP).
- EARLY QUIT INFO loyalty values, server `/mute-player`, "Echo Friends" wording (built in; party tab not re-checked).

**Open bugs (EVR-Runtime, early-quit area)**
1. **FIND MATCH bar after a lockout.** Once a lockout has been shown in a lobby visit (cleared or run out), Find Match
   enables bar actors #32/#31/#30 (20207787fbccf88d / 9ada32d27410ed7b / f4411ba1efc9b76d) and ~35 ms later, right
   after `evt_ar_tablet_enable_started` (bbe776aa91c346e0, sender #16, key b9aee8d7a6daf176), they are disabled. A good
   find gets the bar's own `delegate_onactorenabled` (0d70b6bd637bb91c) first. Not caused by feature bit 1 (the two
   bit-1 nodes in 17d77f27d465760b are already stock in memory). The actor log's caller field (`by <lib>+0x…`) came out
   empty in the 21:26 run: the stack walk finds no frame inside a script library, so a different way to name the
   disabler is needed (e.g. log the return address into echovr.exe and map it, or hook the script's actor-disable
   node). Repro: `/lockout` yourself, clear it, press Find Match in the same lobby.
2. **Main menu EARLY QUIT card during a /lockout.** The user wants it hidden for moderator lockouts. The patch at
   c18bafab7b6faa62 RVA 0x6958 (condition of `earlyquit_quit_notice_page`) is applied in memory (read back from the
   running game at 21:28) but the card still shows, so that node is not what draws this card. Next: find the card's
   own actor/condition (likely another node reading lockoutcountdownactive), or hide it from Atlas's side.

**Status at 21:45, per the user: BOTH early-quit bugs are still open; the next agent should fix them.**
- **EARLY QUIT card still shows on the main menu during a /lockout** (screenshot 21:27). The installed build's gate on
  `earlyquit_quit_notice_page` (c18bafab7b6faa62 RVA 0x6958) is applied ("EARLY QUIT page node found" at 21:33:40, bytes
  read back from memory), so this card is drawn by something else. Find the card's actor and what enables it while
  lockoutcountdownactive is true; hide it unless Atlas reported an early quit (`g_atlasReportedQuit` in lockout_pc.cpp).
- **Lockout/FIND MATCH bar still doesn't show when queueing after a lockout.** The 21:40 workaround (re-raise the
  penalty event when bar actor #32 bounces) fired at 21:35:53.349 but the bar did not come back, so it does not work;
  remove or replace it. New evidence from the caller log: the bar actors are enabled AND disabled by engine code,
  not by a script library: `enable` by echovr.exe+0x4edece and `disable` by echovr.exe+0x4ed17e (the Enable/Disable
  bodies), both called from **echovr.exe+0x1d3742** (VA 0x1401d3742), which is called from +0x51931c < +0x517984 <
  +0x50be59 (the game loop). Next: disassemble around 0x1401d3742 (likely the AR tablet / tablet-enable component's
  per-child enable pass that reapplies a stored visibility state), find the state it reads for #30–#32 and what
  the lockout path left in it, and reset that state when the lockout ends.

**Parties (moved back into this session at the user's request)**
- Design: 10-player parties; the leader queues and all matchmake and join together; in 4v4 public Arena 8 play and 2
  spectate. Client: our own Social stand-in so the stock PARTY tab works (code in EVR-Runtime `src/party/*`,
  `tests/party/*`, `social AccountId()`, party lines in CMake and `runtime_main.cpp`; builds, never run).
- Handed back at ~21:35 (details in `H:\EVR\EVR-Runtime\HANDOFF.md`, "Parties on PC"). Written and built, NOT deployed:
  - EVR-Runtime: `runtime/build/evr-runtime.dll` is a party build of the current tree (80 tests pass, 11 canvas skips).
  - Atlas: `HOWTOBUILD/local/atlas.party.exe`: SNS and RPC party size 10, `party/names` RPC, a public Arena/Combat
    party bigger than 4 skips the matchmaker and gets its own lobby (`lobbyCreateForParty`), seated by `partyRolePlan`
    (leader's team, other team, then spectators; `PartyOverflow` reservations join as spectators). New tests pass;
    the full `go test ./server/` was rerun by the main session (see below).
  - Never run in game: whether the PARTY tab leaves "disabled", the callback ABIs, the dispatcher thread, a null
    SendToPeer buffer, and 10 members following the leader. First test needs two accounts and the party DLL + atlas.

**Not deployed / not done**
- Nothing is committed anywhere. Commit only when the user asks; never commit `HOWTOBUILD/` or this file to atlas.
- Early-quit ladder settings (`enable_early_quit_penalty`) stay off locally by the user's choice; `/lockout` works
  without them.

## 1. Done

### EchoRustServer

- **Payload HUD and private combat** (`src/engine/combat.rs`, `msg.rs`, `session.rs`; track data in
  `src/engine/data/combat_{fission,gauss}_track_vars.txt` from `tools/gen_track_vars.py`).
  - Private combat waits for both teams to ready up.
  - Payload maps play two halves: the teams swap, and the first attack's distance is the target for the second.
  - Untested in game. agent.md has a section on it.
- **Arena stats to nakama.** At match end `session.rs` sends a `UserServerProfileUpdateRequest` for every
  blue/orange player (`send_arena_profile_updates`, `arena_profile_delta`).
- **Loadout feature flag.** `src/upstream.rs` appends `features=save_loadout` to the backend URL
  (`with_features`), so atlas knows this server saves exact loadouts.
- 117 tests pass. agent.md now has a section on the arena stats and `save_loadout` ("Arena stats to nakama, `save_loadout` feature").

### atlas

- **Cosmetics not fully saving.**
  - Atlas now skips the client's lossy `item_equipped` remote log for servers that registered `save_loadout`
    (`evr_runtime_event_remotelogset.go`, constant in `evr_pipeline_gameserver_loadout.go`).
  - `LoadoutEquipItem` no longer resets `decal_body` on every equip (`evr_profile_cache.go`).
  - Items tagged restricted or blocked are still stripped from players who don't own them. That is atlas's own
    rule.
- **Stats not working.**
  - Atlas accepts arena stats from non-native servers through `UserServerProfileUpdateRequest`
    (`evr_pipeline_login.go`, `runtime_event_server_profile_update.go`).
  - Private arena and combat matches return quietly instead of "unknown mode".
  - Skill ratings for arena are not updated through this path.
- **EVR-Runtime alongside nevr-runtime and the stock game** (`server/evr/login_request.go`,
  `evr_session_parameters.go`).
  - New `evr_runtime` login object: version, build, commit, platform, channel, social, plugins.
  - The `nevr_*` fields are still read; if both are present, `evr_runtime` wins.
  - The stock game (neither) works as before.
  - `RuntimeKind`, `RuntimeVersion`, `RuntimeBuild` and `RuntimePlugins` serve all three. The "Login client" log
    line gains `runtime` and `runtime_platform`.
  - The game-server protocol (`/ws/nevr`, nevr-api protobuf) and the "nevr" database names are unchanged on
    purpose.
- **Tests.**
  - New: `server/evr_echors_profile_test.go`, `server/evr/login_request_runtime_test.go`.
  - `go build` and `go vet` are clean.
  - The full suite has three failures that are not from these changes:
    - `TestMatchStart_SuccessfulStartCommitsTheLabel` and
      `TestSyncJournalAndProfileWithRetry_KeepsLocalEditOfSharedRecord` fail on repeat runs even without the
      changes.
    - `TestKC1_PollFollowPartyLeader_RefusesPrivateSocialLobby` waits on a timer and passes on its own.

### Coordination with the EVR-Runtime session

- It was told the login contract:
  - the `evr_runtime` object;
  - `social: 1` only once it handles the SNS party, friend and recently-met messages (tablet parties need
    every member at level 1 or higher);
  - refresh the auth token with `refresh_token` and `expires_in`;
  - use its own HMD serial, not nevr's `nEVR-Wine`.
- It chose the existing SNS party path, so atlas gets no `evr_party_*` RPCs for now.
- It has a stock PC client logging in to the local atlas, and its friends panel shows in the main menu.

### Local atlas test stack (built and running)

All files are under `J:\EchoVR-Tools-Launcher\atlas\HOWTOBUILD\local\`. `HOWTOBUILD/local/` is in atlas's
`.git/info/exclude`.

| Piece | Where | Notes |
| --- | --- | --- |
| atlas binary | `atlas.exe` | `go build` of the working tree |
| config | `data/nakama.yml` | generated throwaway keys plus the throwaway bot token; Mongo off |
| Postgres 18 + Redis 8 | WSL Ubuntu at `H:\WSL\Ubuntu` | reinstalled; reached from Windows on 127.0.0.1:5432 and :6379 |
| siteproxy | `siteproxy.exe` on :4280 (admin 127.0.0.1:4281) | `data/siteproxy.yaml`; maps `/login /config /matching /transaction /serverdb` onto atlas `/ws?format=evr` |
| host account | `data/host.json` | Discord 800101527305846855, guild 1094260352700653769, host role "Server Hoster" 1572295578082222141, generated password |
| game-server config | `data/gameserver-config.json` | launcher-style config.json for the local atlas |
| logs | `logs/atlas.log`, `logs/siteproxy.log`, `logs/rust-server.log` | |

- **Discord.** The throwaway bot is "Atlas" (app 1572294220494938273) in "he_is_the_cat's server", with the
  Server Members intent on. Reset its token when local testing is done, since it was pasted into chat.
- **Group setting.** The guild group's `roles.server_host` is set to the Server Hoster role.
- **Status endpoint.** The stock client's service-status request is `GET /status/services,news` on 7350
  directly. The siteproxy doesn't serve it, so `apiservice_host` points at `http://192.168.1.157:7350/`.
- **Spark secret.** The Spark app's client secret (app 1094259991013228704) was pasted into chat and needs
  resetting in the Discord portal.

### atlas: EVR-Runtime REST token (added 2026-11-17, late evening)

- New `server/evr_runtime_token_http.go` (+ `_test.go`, 3 tests pass) and one registration line in
  `server/evr_runtime.go` (next to the service-status route).
- `POST /evr/runtime/token {"discord_id","password"}` checks the password exactly like `session_ws.go`'s
  discordid+password login (bcrypt, disabled accounts refused, password cut to 32), is limited to 6 tries a minute per
  Discord ID, and returns the device-auth token pair (`mintDeviceAuthTokenPair`). No server key or HTTP key needed.
- EVR-Runtime uses it to read the friend list (`/v2/friend`) and, later, party RPCs with a Bearer token. Long term this
  should become the device-auth flow with a stored refresh token.
- Running since 2026-11-17 17:59 (swapped in together with the party/cosmetics build; see below).
- Seen while reading: `RegisterBeforeAuthenticateCustom` is **not** restricted in `evr_runtime_api_perms.go` (commented
  out), so anyone with the socket server key can get a session for any Discord ID through
  `/v2/account/authenticate/custom`. Worth checking for production.
- Party: atlas's `party/*` RPCs are the website's parties; there is no in-game invite. The "Atlas: working parties +
  cosmetics fixes" session owns that; EVR-Runtime calls `POST /v2/rpc/party/invite?unwrap {"user_id"}`.

### atlas: parties for stock clients, and cosmetics slots (2026-11-17, night)

Uncommitted, in the atlas working tree. `go build ./...` and the scoped `go vet` are clean. **Running locally**
since 17:59 (PID 18144 at the time; previous build kept as `HOWTOBUILD/local/atlas.prev.exe`); all 128 game
servers re-registered. Atlas now logs to `logs/atlas.stdout.log` (`logs/atlas.log` is the old run's).

- **Parties.** The gate was not in the SNS handlers (create/join/invite/kick/leave already serve stock clients). It was
  that atlas sent `SNSPartyDataNotify` only to clients with social level >= 1, and then refused the tablet party at
  matchmaking (`tabletMembersFollow`, `evr_lobby_tablet.go`) because a member that gets no party data is never
  moved to the leader's lobby by the game's FollowParty.
  - New rule: a client that **sends any SNS party request** reads party data from then on
    (`SessionParameters.ReadsPartyData`, `markSNSPartyClient` hooked in `ProcessRequest` next to
    `applyTabletPartyGroup`). Declared social level still counts as before, so nevr-runtime and EVR-Runtime
    with `social >= 1` are unchanged.
  - Both gates use it: the data relay (`partyMembers`, `evr_pipeline_party_data.go`) and `tabletMembersFollow`.
    A party of stock clients, or stock + runtime, now matchmakes as a tablet party.
  - Logged once per session at Info: "Client sent an SNS party request without a declared social level".
  - Tests: `server/evr_lobby_tablet_stock_test.go` (7). The DB-backed party handler tests skip without `TEST_DB_URL`.
- **In-game invite over REST.** New RPC `party/invite` (`server/evr_runtime_rpc_party_invite.go`, + 2 tests),
  asked for by the EVR-Runtime friends-panel session: `POST /v2/rpc/party/invite?unwrap {"user_id"}` with the
  player's Bearer token sends the same `SNSPartyInviteNotify` as the tablet's invite (shared `snsPartyInvite` in
  `evr_pipeline_party.go`), from the caller's game session's SNS party. Refuses with FailedPrecondition
  `not_in_game`, `not_in_party` or `target_offline`. The storage-backed website `party/*` RPCs are separate and
  unchanged.
- **Cosmetics.** Three lossy paths found and fixed:
  1. **The skip for Rust/nevr servers never fired for split-connection clients.** The remote-log equip skip looked
     up the match by the session that sent the remote log, which is the client's *login* connection. Clients play
     on a separate lobby connection, so no match was found and the lossy `item_equipped` equip was applied on
     top of the server's exact save. Now it checks the login session's lobby session too
     (`remoteLogEquipSavedByServer`, `evr_runtime_event_remotelogset.go`).
  2. **`decalborder` and `decalback` were dropped from every GameServerSaveLoadout** (Rust legacy JSON and nevr
     protobuf). Their symbols are not in atlas's symbol table, so the slot came back as hex and was skipped. Slots
     are now resolved against the loadout's own slot names (`loadoutSlotName`, `evr_pipeline_gameserver_loadout.go`).
     Unowned items in those slots are still stripped (to empty, their default).
  3. **Stock servers: a decal border equip failed** as "unknown category". `LoadoutEquipItem` now has `decalborder`.
  - Tests: `server/evr_loadout_slots_test.go` (8), including all 22 slots surviving a save, a one-slot save and the
    login-time sanitize on both server paths. The slot test was checked to fail against the old lookup.
  - **Not fixable on stock servers without slot data:** the client's `item_equipped` remote log names the item,
    never the slot. So an emote fills both `emote` and `secondemote`, a decal both `decal` and `decal_body`, a
    pattern both pattern slots, a tint `tint` and `tint_body` (alignment tints also their alignment slot). Two
    different emotes, or different heraldry and body decals, cannot be kept on a stock server. Rust and nevr
    servers save the exact slots.

### /mute-player: server voice mute (2026-11-17, late night)

Uncommitted. **Not in the running atlas** (that build is from 17:59) and not in the running launcher. The Rust
release binary was rebuilt at 20:07 (`target/release/echo-rust-server.exe`); atlas has not been rebuilt.

- **What it does.** A guild enforcer (or global operator) runs `/mute-player user duration [reason]
  [allow_private_matches]`. The mute is per guild, like other enforcement, and is enforced by the game server,
  which stops relaying that player's voice to everyone. Only servers that register the `server_mute` feature
  (the Rust server, after this change) can do it. A stock server is never sent the message, so a mute does
  nothing there. The bot's reply says which case applied.
  - **Limit:** the duration is required, at most **30 days** (`ServerMuteMaxDuration`). `0` lifts the mute.
  - **allow_private_matches:** the player can still speak in private matches (`LobbyType == PrivateLobby`).
  - The command is logged to the guild's audit channel, as `/kick-player` is.
- **Saved to the database.** Storage collection `ServerMutes`, key `journal`, one object per user, private (no
  client read or write). It keeps the last 50 records: id, guild, issuer, reason, created, expires,
  allow_private, lifted at/by. A new mute in a guild replaces the active one there.
- **How it reaches the server.** A new atlas message `ERGameServerMuteEntrants`, symbol `0x7777777777770C00`,
  null-terminated JSON `{"entrant_ids":["<uuid>"],"muted":true}`. It is sent:
  - live, to the match the player is in when the mute is issued or lifted (this guild's matches only, so guild
    isolation holds);
  - on every join, after the accept (`lobbyEntrantConnected` → `muteEntrantsOnJoin`), because entrant ids are new
    for each join.
- **atlas files:** `server/evr/gameserver_mute_entrants.go` (registered in `core_packet.go`,
  `core_packet_types.go`, `core_hash_lookup.go`); `server/evr_server_mute.go` (journal, apply);
  `server/evr_discord_appbot_mute.go` (command logic); the command definition and handler in
  `evr_discord_appbot.go`; the permission case in `evr_discord_appbot_handlers.go`; the join hook in
  `evr_pipeline_lobby.go`.
  - Tests: `server/evr_server_mute_test.go` (9 tests), passing. They cover the journal, the duration limit, the
    private option, the wire format, the live mute and lift, stock servers and other guilds being skipped, the
    join-time mute, and the RPC below.
- **Red muted icon in game.** A server can't draw it for a stock client. Instead, a new RPC `mute/match`
  (`server/evr_runtime_rpc_server_mute.go`) returns the server-muted players in the caller's match:
  `{"match_id","enforced":bool,"muted":[{"user_id","evr_id","display_name","expires_at","allow_private"}]}`.
  EVR-Runtime would call it and draw the base muted icon in red by those names. **The EVR-Runtime session has
  not been told yet.**
- **EchoRustServer (compiled and tested 2026-11-17 20:07; 120 tests pass):**
  - `src/upstream.rs`: `FEATURES` is now `"save_loadout,server_mute"`. Both bridges (nakama and ServerDB) pass
    `0x7777_7777_7777_0C00` on to the engine as a text frame (`mute_text` / `mute_texts`).
  - `src/engine/mod.rs`: the engine reads that text frame (`on_mute`, `MuteEntrants`).
  - `src/engine/session.rs`: a `muted` entrant set (`set_muted`), and `on_mailbox` drops VOIP from a muted peer.
  - Tests: `nakama_url_gets_the_save_loadout_feature` expects `save_loadout,server_mute`; new
    `server_mute_tests` in `session.rs` (a muted peer's VOIP reaches nobody, they still hear others, a lift relays
    again, a mute sent before the join applies on join) and `server_mute_frames_reach_the_engine_as_json` in
    `upstream.rs`.
- **Known limits.** A mute that expires mid-match stays in force on that server until the match ends (or a
  moderator runs `/mute-player … 0`). The Discord command list has to be re-registered with Discord for
  `/mute-player` to appear; atlas does that at bot start.

## 2. Started, not finished

- **More than 10 game servers can't register (in production).** Locally it doesn't happen: 128 of 128 register
  (section 3). The cause is something only production has; see Next steps, item 1.
- **/mute-player.** Atlas and Rust sides done and tested, neither deployed. EVR-Runtime red icon not started, and
  that session has not been told the `mute/match` contract (no other session was running at 20:07).
- **Party tablets.** Atlas side done (see "parties for stock clients" above). Not yet checked in game, and atlas
  cannot see whether a pure stock client sends SNS party requests at all (stock party code went through the Oculus
  platform, pnsovr). If it sends none, nothing changes for it; see Next steps, item 3.

## 3. Result of the local registration test (17:11–17:15)

- After setting the password again, **all 16 game servers registered**, then **all 40** on a second run. There
  were no warnings or errors in `atlas.log` or `rust-server.log`, and no bridge closed.
- Atlas's code has no per-host or per-server cap (searched), and neither does the cat-tools siteproxy.
- **Conclusion:** the 10-server limit is not in atlas or the Rust bridge. It comes from something production has
  and this setup doesn't. Candidates, most likely first:
  1. **Per-IP limit in production's front proxy.** A connection or rate limit per client IP (nginx `limit_conn`,
     Cloudflare, etc.) on the `serverdb` sockets. Each game server holds one socket open, so the 11th would be
     refused at the upgrade.
  2. **UDP validation over the internet.** Atlas pings the external IP 3 times with a 500 ms timeout. Locally
     that's the LAN.
  3. **Discord REST limiter.** It's saturated only when error DMs are flowing, which needs failures first.
- **Confirmed with the real launcher** (`dist/windows/echo-rust-server.exe` with 128 servers, log
  `dist/windows/logs/server-1794936006.log`): **128 of 128 registered**, 0 bridges closed. The launcher config
  that works is `atlas/HOWTOBUILD/local/data/gameserver-config.json`.
- **Gotcha seen on the way:** a game-*client* config, with `serverdb_host = ws://…:7350/ws?format=evr&token=<key>`
  and no `discordid`/`password`, makes every game server fail with "unauthenticated session, rejecting". Game
  servers need the host's `discordid` and `password` in `serverdb_host`.

## Next steps

1. **Find the production cause of the 10 limit.**
   - Run the launcher against production with 12 or more servers and read the 11th server's line in
     `dist/windows/logs/server-*.log`. An HTTP status or close at connect means production's front proxy limits
     sockets per IP; a "not accepted" after connecting means atlas.
   - Ask whoever runs production for their proxy config (nginx `limit_conn` / `limit_req`, Cloudflare rules) and
     atlas's log around the refusal.
   - Optional: to check that atlas itself behaves under internet-like probing, register from another machine
     against this PC's LAN address (192.168.1.157:4280).
2. **Test the cosmetics and stats fixes end to end on the local stack.** The running atlas (17:59) has the token
   endpoint, the party/cosmetics changes and `party/invite`. Logs: `HOWTOBUILD/local/logs/atlas.stdout.log`.
   - On a **Rust server**: set every slot to a non-default item, including `decal_body`, decal border, both emotes
     and both tints, then relog. All should come back. `atlas.log` should show no `item_equipped` equip for that
     player (the skip now fires through the lobby session).
   - On a **stock server** (no `save_loadout`): same, and expect the couplings listed above (emote/secondemote etc.).
     Decal border should now save.
   - Try equipping an item the account does not own (a VRML tag): it must come back as the default after relog.
   - Finish an arena public match and check the arena leaderboards fill.
   - Note: the Rust server sends arena stats for every match; **atlas** ignores private arena/combat matches on
     purpose, so only public matches fill the boards.
3. **Party tablets, in game with two clients.** Needs two game clients (two accounts) on the local atlas.
   - Stock + stock, stock + EVR-Runtime, EVR-Runtime + EVR-Runtime: create a party from the tablet, invite, accept,
     kick, leave.
   - Leader finds a social lobby: the member should follow. Leader queues arena: both should land in the same
     match, the member without pressing anything.
   - Watch `atlas.log` for "without a declared social level" (the client spoke SNS party) and "Tablet party is the
     lobby party". If a stock client never logs the first line, it sends no SNS party messages and stock
     parties need client-side work instead.
   - EVR-Runtime's friends-panel INVITE: try it once the runtime calls `party/invite` (below). The caller's game
     client must already be in an SNS party, or the RPC answers `not_in_party`.
4. **Finish /mute-player.**
   - ~~EchoRustServer: tests, features test, VOIP-drop test~~ Done 2026-11-17 20:07 (120 pass).
   - Send EVR-Runtime the `mute/match` contract so it can draw the red muted icon.
   - Rebuild atlas (`HOWTOBUILD/local/atlas.new.exe`). The Rust release binary is built; the launcher's
     `dist/windows/echo-rust-server.exe` is not replaced. Restart only when the user asks.
   - In game: mute a player on a Rust server and check that nobody hears them. Check that `0` lifts it, that
     `allow_private_matches` works in a private match, and that the mute comes back after a rejoin.
   - ~~Write agent.md notes~~ Done 2026-11-17.
5. **Commit when the user asks.**
   - EchoRustServer: payload/private combat, arena stats, `save_loadout`.
   - atlas: cosmetics, stats, EVR-Runtime login, EVR-Runtime token endpoint, stock-client parties, loadout slots,
     /mute-player, tests.
   - EchoRustServer also: server mute (`server_mute`).
   - Never commit `HOWTOBUILD/` or this file to atlas.
6. **Clean up when done.**
   - Reset the throwaway bot's token and the Spark app's client secret in the Discord portal.
   - Stop atlas, siteproxy and the WSL keep-alive.
   - Optionally `wsl --unregister Ubuntu` (it lives on H:).

## 3b. State at 2026-11-17 evening

- Stack is running: atlas.exe, siteproxy.exe, WSL (Postgres/Redis), and the user's launcher (echo-rust-server.exe).
- The test client (H:\EVReady-at-dawn-echo-arena, EVR-Runtime loaded) logs in to the local atlas as
  `he_is_the_cat` (DMO-3296391650685316405). Its `_local\config.json` now carries the user's own
  `discordid`/`password` in the login and serverdb URLs, which fixed "account requires password authentication".
  Note: atlas.log records that query string in clear; treat the log as sensitive.
- That login reports `runtime: ""`: EVR-Runtime does not send the `evr_runtime` login object yet.
- EVR-Runtime's friends panel (rows, clicks, INVITE, avatars) moved to its own session; see H:\EVR\EVR-Runtime\HANDOFF.md.
- Re-checked: Rust `cargo test --release` 117 pass; atlas `go build ./...` clean; `server/evr` and the
  profile/loadout tests in `server` pass.
- Next atlas session's scope (user, 2026-11-17): **working parties** and **cosmetic fixes for both the Rust game
  servers and stock (native) game servers**.

## 3f. Early-quit lockout (2026-11-17 20:25)

- Atlas already has the whole server side (penalty ladder, `/lockout`, `SNSEarlyQuitUpdateNotification` with the
  expiry at +0x20, feature flags 0x23). Early quits are charged only in public arena, from clients' post-match logs,
  so Rust servers need nothing new.
- The missing piece was the client: remade in EVR-Runtime (`runtime/src/earlyquit/`); see H:\EVR\EVR-Runtime\HANDOFF.md.
  Deployed into the game's 0.8.0 folder; hooks install.
- The user's account (he_is_the_cat, c41027c9-a251-4f27-aa70-c00663cdea0b) was added to the "Global Operators" system
  group in the local database (group 96b71419-1bf9-4acd-8e25-9615d4522755, member state 2), so `/lockout` works.
- Still off locally: `matchmaking.enable_early_quit_penalty` (service settings, storage collection "Global") and the
  guild's `enforce_early_quit_penalty` group metadata. Without them the lockout is not re-sent at login and leaving
  a match early charges nothing.

- **Loyalty fix in atlas (20:34, built as `atlas.new.exe`, NOT running):** `SteadyPlayerLevel` (the loyalty tier) was
  only re-derived by the moderator reset RPC, so it never rose with completed matches. Now `applyLadderPenalty`
  re-derives it on every early quit, and both completion paths (`evr_match.go` MatchLoop, post-match remote log)
  call `resolveSteadyLevel` and push an `SNSEarlyQuitUpdateNotification` (`sendLoyaltyUpdate`, skipped in silent
  mode) so the tablet updates without a relog. Tests: `server/evr_earlyquit_loyalty_test.go` (2); the 47 early-quit
  tests pass; `go vet` clean.
- **`/lockout` player picker (20:36, in `atlas.new.exe`, NOT running):** the `player` option is now text with
  autocomplete listing guild members ("Nick (@username)", value = Discord ID), like `/kick-player`'s picker, and still
  takes a typed username, EVR ID (`DMO-…`, any case), Discord ID or mention, or account UUID
  (`server/evr_discord_appbot_lockout.go`, tests in `_test.go`). The option's shape changed, so Discord's client
  may need a reload (Ctrl+R) after atlas re-registers commands.
- `/mute-player` was registered (global commands, 42 incl. mute-player); Discord's client cache hid it until reload.

## 3g. Restart at 2026-11-17 20:37

- Atlas restarted on the build with the loyalty fix and the `/lockout` member autocomplete (PID 21356; previous
  binary kept as `atlas.2011.exe`, its log as `logs/atlas.2011.stdout.log`). Startup clean, 42 commands registered.
- The game was restarted with the EVR-Runtime DLL that has the notice fix; all hooks installed and the tablet
  library's notice node was found (`[EVR.EARLYQUIT] tablet library at ...: notice node found`).
- The launcher's game servers had been stopped (log ends 20:30:59 "— stopped —") before the restart; press Start in
  the launcher. Seen in that run: `server mute ... muted=true changed=1` at 20:29 (the mute reached the Rust server).

## 3e. Restart at 2026-11-17 20:11

- Atlas restarted from the new build (PID 18724, `atlas.exe` = the 20:09 `atlas.new.exe`; the other session's 19:44
  binary is kept as `atlas.1944.exe`). Startup clean, `mute/match` registered, 42 slash commands re-registered with
  Discord. Logs to `logs/atlas.stdout.log` (this overwrote the 17:59–18:40 run's log).
- `dist/windows/echo-rust-server.exe` is now the 20:07 build with `server_mute`; the 19:14 one is kept as
  `echo-rust-server.1914.exe`. Started from the launcher at 20:12: 8 of 8 game servers registered, and atlas logged
  `server_mute` for all 8. Ready for the in-game mute test (Next steps item 4).

## 3d. State at 2026-11-17 20:10

- **Atlas is not running**, and neither is the launcher. Only siteproxy (since 17:03) and WSL (Postgres/Redis) are up.
  Another session ran debug atlas builds after 18:40 (`logs/atlas.debug{,2,3}.stdout.log`, the last ending 20:01)
  and replaced `HOWTOBUILD/local/atlas.exe` at 19:44. That binary's contents were not checked.
- `HOWTOBUILD/local/atlas.new.exe` (20:09) is a fresh `go build` of the whole atlas working tree, including
  /mute-player. The mute, loadout-slot, stock-tablet, party-invite and runtime-token tests in `./server` pass.
- `target/release/echo-rust-server.exe` (20:07) has `server_mute`. To use it, copy it over the launcher's
  `dist/windows/echo-rust-server.exe` while the launcher is stopped.
- To test /mute-player: start atlas from `atlas.new.exe`, start the launcher with the new Rust binary, and run
  through Next steps item 4.

## 3c. State at 2026-11-17 night: EVR-Runtime private matches through atlas invites

All details are in H:\EVR\EVR-Runtime\HANDOFF.md. Nothing is committed.

- EVR-Runtime now shows CREATE GAME and JOIN VIA CODE on the main menu. It turns the wizard's clicks into atlas RPCs
  through `/v2/rpc/<id>?unwrap` with the runtime token:
  - `invite/create {experience:"social", region:"default", expires_s}` -> `{code, expires_at, match_id}`;
  - `invite/resolve {code}` -> `{match_id, mode, level}`.
- Proven in game: codes were issued (MAK6M, DQMSF, NPN4B), and JOIN put the player into the match. atlas logged
  `LobbyJoinSessionRequest` and "Player joining the match".
- atlas refuses `invite/create` with `no_history` until the account has latency history. Only matchmaking ping
  replies write it, so a fresh account must press PLAY once first. Consider seeding history at login, or a softer
  check for `experience:"social"`.
- Not tried yet: resolving a typed code (`invite/resolve`), and `invite/revoke` (REGENERATE).
- The wizard's region ("US WEST") and password are not sent. Mapping the region needs the label -> atlas region code
  list (`invite/regions`).
- From the same test run: loadout saving works, but **notification clearing does not**. That was forwarded to the
  atlas parties + cosmetics session.
- `party/invite` errors (`not_in_game`, `not_in_party`, `target_offline`) are not yet shown in EVR-Runtime's profile
  pop-up.

## 4. How we got here (the earlier failure)

1. **The Rust server runs 16 embedded game servers against the local atlas**, through the proxy.
   - Command: `run_rust.py` in the session scratchpad. Its arguments are the launcher's: `--backend echorelay`,
     `--upstream <serverdb_host>`, `--login-url <loginservice_host>`, `--bind 127.0.0.1:7790`,
     `--udp-port 7100`, `--max-instances 16`, `--min-idle 16`, `--lan-ip 192.168.1.157`.
   - No game launch is needed.
2. **Every registration failed with "Invalid credentials".** `/reset-password` was run from the host's Discord
   account at 17:05:04, which clears the account's password and email (`evr_discord_appbot.go:1857`). The
   generated password in `data/host.json` was set at about 17:02.
3. **Next:**
   - Set the password again (bcrypt of `host.json`'s password plus a placeholder email on the user row).
   - Restart the Rust server.
   - Watch how many of the 16 register, then read `atlas.log` around the 11th registration for the limit.
4. **Don't use `/reset-password`** on the host account while testing. Use a separate Discord account for game
   clients if a password login is needed there.

## Background processes started from the session

- **WSL keep-alive:** `wsl -d Ubuntu -u root -- ... sleep infinity`. Without it WSL idles out and Postgres goes
  down.
- **Running:** `atlas.exe --config data/nakama.yml` and `siteproxy.exe -config data/siteproxy.yaml`.
- **Stopped:** the session's own echo-rust-server test run. The user's launcher now runs the game servers.
- **To restart the stack after a reboot:**
  1. Start WSL with Postgres and Redis.
  2. Start `atlas.exe --config data/nakama.yml` in `atlas/HOWTOBUILD/local`.
  3. Start `siteproxy.exe -config data/siteproxy.yaml` in the same folder.
  4. Start the launcher with `gameserver-config.json`.
- **If registrations fail with "Invalid credentials",** someone ran `/reset-password` on the host account. Set
  the password again from `data/host.json`: bcrypt it into `users.password`, and set `users.email` to
  `<user id>@echonakama.invalid`.

### 3h. 21:00: matchmaking during a lockout, Echo wording, parties (built, NOT deployed)

- Blank FIND MATCH bar (no time, no cancel): the user had run `/lockout` again at 20:43:59 (120 s) and pressed Find
  Match at 20:44:06 while still locked out. The client showed the lockout bar with matchmaking state underneath, and
  Atlas still matched them: its lockout refusal only covered the ladder (`enable_early_quit_penalty`, off locally) and
  one test user. Fix in atlas `server/evr_lobby_find.go`: a running moderator lockout (`ModeratorPenaltyUntil`) refuses
  Arena/Combat public finds with "You are locked out of matchmaking for <time>", whatever the settings
  (`moderatorLockoutLeft` in `evr_earlyquit.go`, test in `evr_earlyquit_loyalty_test.go`). Built as
  `HOWTOBUILD/local/atlas.2100.exe`; swap it in for atlas.exe on the next restart.
- "You have no Oculus Friends." -> "You have no Echo Friends.": EVR-Runtime `src/menu/wording.h` / `wording_pc.cpp`
  (an `OnUiTextSet` filter). Built, 58 tests pass, not deployed (with the bit-46 fix).
- "Parties are currently disabled.": the stock PARTY tab needs the platform Social object that the Oculus plugin
  supplied on PC. Atlas already speaks the SNS party protocol (`evr_pipeline_party.go`, "tablet party"); the client
  side is missing. The user wants 10-player parties: the leader queues, all 10 matchmake and join together, 8 play
  4v4 and 2 spectate. Not started.

### 3i. Restart at 2026-11-17 20:59

- Atlas is now the 3h build (`atlas.exe` = `atlas.2100.exe`, PID 22160; previous kept as `atlas.2037.exe`, its log as
  `logs/atlas.2037.stdout.log`). Startup clean, 42 commands; the Rust server's 8 game servers reconnected by themselves.
- `plugins\versions\0.8.0\evr-runtime.dll` is the 20:55 build (early-quit warning fix + Echo wording); previous kept
  as `evr-runtime.2034.dll`. The game was not running, so it picks this up when next launched.

### 3j. 21:20: lockout after a rejoin, the bar after a lockout

- A player who rejoined during a `/lockout` saw no countdown and got KICKED FROM LOBBY GROUP on Find Match: Atlas sent
  the early-quit messages at login only with `enable_early_quit_penalty` on. Now `loginSendsEarlyQuit` (evr_earlyquit.go)
  also sends them while a moderator lockout runs (test in `evr_earlyquit_loyalty_test.go`). Built as
  `HOWTOBUILD/local/atlas.2130.exe`, not running yet.
- The FIND MATCH bar after any lockout (cleared or run out), in the same lobby visit: Find Match enables bar actors
  20207787fbccf88d/9ada32d27410ed7b/f4411ba1efc9b76d (#32/#31/#30), then `evt_ar_tablet_enable_started` fires and they
  are disabled ~35 ms later. A find with no lockout seen keeps them. Cause not found yet; the runtime's actor log now
  names the script library and offset that enables/disables each actor (`... by <lib>.dll+0x...`), installed in
  0.8.0 (previous kept as `evr-runtime.2115.dll`). Next: reproduce (lockout, clear, Find Match) and read the disabler.
- 21:22: Atlas restarted on `atlas.2130.exe` (PID 16176; previous kept as `atlas.2059.exe`, log as
  `logs/atlas.2059.stdout.log`). Clean startup, 42 commands, 8 game servers reconnected.
- 21:40: full `go test ./server/` with the party changes: one failure, `TestMatchStart_SuccessfulStartCommitsTheLabel`
  ("Started() false"), fails on every run and is not party code (MatchStart sets StartTime = now and `Started()` needs
  `time.Now().After(StartTime)`; Windows clock resolution makes them equal). All else passes.
- Also `TestSyncJournalAndProfileWithRetry_KeepsLocalEditOfSharedRecord` (evr_suspension_profile_occ_test.go:203) is
  flaky: it failed once and passed once when run on its own, and failed in the party session's full run. That's
  suspension/profile code, not party code.
