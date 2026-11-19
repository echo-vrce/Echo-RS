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

## 2026-11-18 17:47: cease fire build deployed

- `dist/windows/echo-rust-server.exe` is now the build of the uncommitted cease-fire work (brawler room only lifts
  `VAR_HOLD`; stuns under a cease fire dropped server-side; equipment sent to newcomers; one award a player).
  Previous binary kept as `echo-rust-server.pre-ceasefire.exe`. 123 tests pass. Not committed. Not yet checked in game.


## 2026-11-18 18:30: stun/block outside the brawler room (OPEN), native 2-player recording blocked

**Problem (user):** in the lobby, every client after the first can hold stun and block outside the brawler room and
sees the stun charge and the block locally. It is not about stunning others. The first client behaves. Different
accounts show the same thing, so it is not the shared-account test setup.

**Tried, not the fix:**
- `LOBBY_HOLD_VERSION` (msg.rs `with_var`; session.rs `replicate`): the lobby `VAR_HOLD` now carries native's
  `0x6aea99ee` high word, baseline included. Deployed in `dist/windows/echo-rust-server.exe` (previous
  `echo-rust-server.pre-ceasefire.exe`); 124 tests pass; uncommitted. The user tested at 18:02: no change.

**What the evidence says (no nevr references; captures and echovr.exe only):**
- Server log `dist/windows/logs/server-1795024916.log` (debug on): both slots get "lobby cease fire on". Both ack
  `SR15NetReplicatedDeltaAck`. Nothing differs per slot server-side.
- Native addressing matches ours. In `captures/native-2p-20261115`, the replicated-vars mailbox key low nibble =
  4 + recipient slot, and the client's ack carries its slot. `key_replicated` already does that.
- Native never sends `SR15NetPropertyModifiersAdd` in any capture.
- In `captures/native-det-20261116` (one player), the hold is lifted 0.5 s after the CLIENT sends its area-entry
  burst (8 `SetOwner`, then 8 `SetBits feffffff` at indices 0,15,...,105). Native's bit-field base is uniform
  (`ffffffe0` for all 120 entries, data/bitfield_response_2645.hex).
- echovr.exe gates:
  - The punch function 0x140cb3400 checks bit 7 of `[[game+0x2b18]+0xc40]` and a per-player `_punch_disabled`
    property modifier (string 0x141cb4e58; other users 0x140ccc150, 0x140d2b1b1).
  - Script nodes `R15NetPunchEnablePunchingNode` (exec fn 0x140db9ca0) and `R15NetPunchEnableBlockingNode`
    (0x140db86e0) exist, so the lobby level script may toggle punching/blocking per player.
  - Not yet traced: what the client needs from the server for those to apply to players after the first.
- Every native lobby capture has ONE player in the brawler area. A native 2-player lobby recording is the missing
  evidence.

**Getting that recording (blocked):**
- The user's first try ran `-server -headless` from `H:\EVR
eady-at-dawn-echo-arena` (EVR runtime install). The
  server failed to load `mpl_lobby_b2` (`[NETGAME] Failed to load level`): the runtime applied all its client
  changes there. Removing EDITH.dll (moved to `plugins\disabled\`) did not help.
- Fixed in EVR-Runtime: under `-server` the runtime now installs only crash log + login (see EVR-Runtime HANDOFF).
  Installed 18:25. Not yet tried with a server from that install.
- The user's next try ran the server from the OTHER install `H:
eady-at-dawn-echo-arena` (no EVR loader). Atlas
  closed its serverdb socket right after registration: `close_reason: received malformed payload`
  (atlas.equip2.stdout.log 18:24:44). That install's EchoRelay patch registers with "(region 0x2fd38a995f615b9e,
  version lock 0x134b1272e1c4c0b7)". The likely cause is a registration format Atlas does not parse; unconfirmed.

**Next steps:**
1. Start the native server from `H:\EVR
eady-at-dawn-echo-arena` with the new runtime. Check evr-runtime.log for
   `[EVR] server launch (-server): client changes skipped` and that `mpl_lobby_b2` loads. If it still fails, try
   without `plugins\local` DLLs (dev mode loads ~28 of them into the server).
   - Or find why Atlas calls the other install's registration malformed.
2. Record through `tools/record_native.py` (serverdb_host -> proxy :7781, UDP relay :6800; REC_UPSTREAM to Atlas).
   Script: two players join; player 2 holds stun/block outside the brawler room; walks in, grabs gear, walks out,
   holds again.
3. Diff what native sends player 2 (replicated vars, SetBits, ComponentEvents, anything per slot) against our
   server, then fix and test in game.
4. Still open separately: other clients don't show a player's new cosmetics (chassis/boosters/CDN items). Needs a
   viewing client's log of a different account wearing them.


## 2026-11-18 18:45: mercy instant, time-up disc, late-joiner stun findings (offline, no recording)

**Deployed:** `dist/windows/echo-rust-server.exe` = this build (previous kept as `echo-rust-server.pre-mercy.exe`).
126 tests pass (2 new in `arena_end_tests`). Not committed. Not yet checked in game.

- **Mercy is instant** (`session.rs`, goal handler). A goal that reaches the margin (public 12; private the rule),
  outside overtime, now keeps only this goal's frame-`g` score events and schedules `EndRound` at `g+1`. Before, it
  waited for `celebration_end` (+15 s). `end_round` sets `VAR_MERCY_WIN` (boards and clock), then `end_match`.
  Overtime goals keep the celebration as before.
- **Mercy announcer**: unchanged, "That wraps up the round" then the winner (user confirmed that is native).
- **Time-up disc**: `pin_disc` now bumps the reset count (and clears owner/holder state) when a player's client was
  simulating the disc. Before, the holder kept flying its own copy (no bump, so it never snapped) and the server
  stopped relaying it, so everyone else saw the pin frozen in place. No native capture has a holder at time-up; this
  matches what a goal already does.

**Late joiners stun/block under a cease fire (lobby outside the brawler room, AND arena during a goal): OPEN.**
It is one bug: the first player is fine, everyone who joins later is not. Ruled out (offline):
- echovr.exe punch gate `0x140cb3400`: `[[game+0x2b18]+0xc40]` bit 7 is only "net session active" (tested in ~20
  places), NOT a cease fire. The real gate is a `_punch_disabled` property modifier on `[game+0x2e00]`, which is the
  LOCAL player's own body actor (set in `0x140cf7080` when the local player's component spawns; cleared in
  `0x140d3ba80`). So the client scripts must put `_punch_disabled` on the local body, and on late joiners they don't.
- Scripts that set it: `bin/win10/scripts/08317e89cc3ae611.dll` (= `MOD_MATCH`, arena/lobby mode graph:
  `state_enables_punch`, `EnableAllPlayerPunches`, `EnablePunch`), `992004b15156adc5.dll` (lobby zone: on
  `onlobbymemberjoined`/`actorchanged` and trigger `playerentered`/`exited`), `3be08d996be43882.dll`,
  `6a7cf459db7587f3.dll` (brawler score, `enable_punch_volume`), `d44f62ba114f0cde.dll` (combat local cease fire).
- `captures/native-2p-20261115` is a 2-player ARENA match (EntrantTeamChanged teams 1 and 0, mode 0x09990965f4db8c03),
  not the social lobby. Our join batch matches native's order. IdsResponse is identical for both players. The full
  replicated vars differ by one var only (`79ac0fb75468917e` = 0, sent only to p2). HostState/OwnerState first bytes
  vary randomly across captures (padding). `VAR_HOLD` reaches every slot from our server.
- Native sent 7 `MOD_MATCH`/`0xb9e136910995a482` ComponentEvents to p1 at match start and none to p2; ours does the
  same, so that alone doesn't explain it.

**Next (offline, no native server needed):** instrument the client through EVR-Runtime. Hook the property-modifier
add/remove (callers of `0x140ce4390`, and the script node exec fns `0x140db9ca0` punch / `0x140db86e0` block). Log
each `_punch_disabled` add/remove with its target actor and whether it is `[game+0x2e00]`. Run two clients; the
first log line that is missing on client 2 names the script event its graph never got.

### 2026-11-18 19:10: public overtime (sudden death), from the arena graph itself

Deployed in `dist/windows/echo-rust-server.exe` (previous build still `echo-rust-server.pre-mercy.exe`). 127 tests pass. Not committed, not tried in game.

- Source: `bin/win10/scripts/08317e89cc3ae611.dll` (MOD_MATCH arena graph) and native's `mp_arena_rules`.
  `game_state` values are CSymbol64 of the state names (`main_loop_enter` = 0x3b529c356a118cc6, matches ours).
  The graph has `pre_sudden_death_enter/_update`, `sudden_death_new_round_enter/_update`,
  `post_sudden_death_enter/_update`, `OT_IS_ENABLED_THIS_ROUND`, "ot round: ", and reads the replicated var
  `sudden_death_active` (0xee2e44f738a30c35, in the arena baseline at 0) 14 times. `VAR_OVERTIME` (0x5faf...) is
  combat's and absent from arena.
- round_rules: `enable_sudden_death` true, `sudden_death_round_time` 120, `sudden_death_regulation_over_time` 0,
  `sudden_death_round_over_time` 5, `wait_for_launch_time` 15. Now read into `ArenaPublic`.
- Flow (`Arena::start_sudden_death`): public time up tied → disc to centre (reset bump), `sudden_death_active` 1,
  clock 120, `pre_sudden_death` + `MP_ANNOUNCER_001A_OVERTIME` → `sudden_death_new_round` → `transition_to_launch`
  → `pre_launch` → the usual 15 s launch → play. Next goal: `EndRound` after the goal celebration → match over.
  Scoreless OT: `post_sudden_death` (5 s) → another OT round, repeated as needed. `end_round` clears the flag.
- Private overtime is unchanged except it now also sets `sudden_death_active`.
- Unknown without a native capture: the exact events between the OT states (we send disable-local-disc / restart
  disc only), and whether an OT goal ends the match at once or after the 15 s celebration (we keep the celebration).
- Static graph tracing note: the node logic is a big node-id switch; a symbol map of every DLL string plus our Rust
  constants (`scratchpad names.txt` → hashes) labels ~540 of its constants. Recreate with rad_protocol's Symbol.

### 2026-11-18 19:45: lobby now relays clients' filter-13 events (stun outside brawler: candidate fix)

Deployed (previous `echo-rust-server.pre-lobbyrelay.exe`). 128 tests pass. Not tried in game.
- Our social path consumed every client `SR15NetComponentEvent` (gear area triggers, props) and relayed none.
  Combat already relays filter-13 events byte for byte, as native does (netrec captures: client c2s on module
  0x6aea99eeb2bab6c4, the server sends the same out). The lobby's level scripts each client runs (punch zone
  `992004b15156adc5`, cease fire `d44f62ba114f0cde`) would sync through them, so later players were never told.
  Unconfirmed: no native 2-player lobby capture. Native arena (native-2p) did NOT relay p1's filter-13 event to p2,
  so arena is unchanged.
- Every lobby client event is now logged at info ("lobby client event": sym, module, handle, filter). If the stun
  bug remains, compare player 1's and player 2's lines around walking in/out of the brawler room.

### 2026-11-18 20:15: native server+client recordings, lobby-wide var sequence, lobby combat-room vars

Deployed `dist/windows/echo-rust-server.exe` (previous `echo-rust-server.pre-lobbyseq.exe`). 129 tests pass. Not committed, not tried in game.

New captures: `captures/netrec-native-srv-1201/` (native SERVER recording, user alone in the lobby: brawler, combat
room, gun, stun field, 3 barriers broken), `captures/netrec-lobby-native-20261118/` (client, native 4-player lobby,
user joined at slot 4, could not stun), `captures/netrec-lobby-ours-1959/` (client side of the 1201 session).

- **Var sequence** (`replicate`): every replicated-vars message is now stamped with the lobby frame, not a per-player
  counter. Native: seq = frame - constant, the same clock for every player (srv-1201 diff 3713, slot-4 client 788077),
  mailbox bucket = frame % 90, low nibble 4 + slot (ours already). Candidate fix for late joiners; unconfirmed.
- **Native cease fire (srv-1201)**: `VAR_HOLD` (low word flag, high `6aea99ee`) in the join snapshot at 1, then sent
  ONLY on change: 0 at 52.0 s (walked into the brawler room, no client message before it: the server tracks position),
  1 at 59.2, 0 at 66.1 (combat room entry, 0.5 s after the client's SetBits/area burst). Ours also resends it every
  second; harmless as far as known.
- **Client gate (echovr.exe)**: `R15NetPunchEnablePunchingNode` sets bit 0x80 of the player's punch-component flags
  (`[entry+0x608]`, entries 0x628 bytes, setter 0x140cd39b0); blocking is bit 0x20 (0x140cd39f0). Read at 0x140d2bec3
  (punch) and 0x140d67c97 (block). `_punch_disabled` is the STUN result (modifier per stunner), not the cease fire.
- The slot-4 native lobby snapshot has a different var set (278 vars, only game_state/disc owner/one more shared
  with ours) and no `VAR_HOLD` at all. Unexplained (other server build/variant?). Not used.
- **Lobby combat room (props.rs `LOBBY_REFLEXES`, `LOBBY_BARRIER_VARS`)**, from srv-1201:
  - barriers pool 6/7/8 = handles 64/65/66 -> vars `7452cb2080662e3a` / `6a2aca435523d9d2` / `2dcaa546f5510a9c`:
    0 at the break (same frame as the break event), 1 at the 30 s respawn. Health 230, matches
    `lobby_combat_barriers.txt`. The other 8 barriers' vars are unknown (never broken in a capture).
  - client server-only events -> vars (low word; the high word is a per-level version and stays the baseline's):
    `fbc0427ac211919e/f` on 0xa8 -> `ec48aa3ff8748bae` 0.0/1.0; same on 0xb7 -> `ab544d4b21130c68`;
    `ea7a6e23ed5cace2`/`8a4a9239b3d306ac` on 0xc6 -> `01d5d534863ecb0e` 0/1 (ability station: used, ready 40 s later);
    `635d09deff7164c0` on 0xb7 -> `cb3e710e2dd8e5c2` 0.0 (the 1.0 pair `...c1` inferred).
- Combat messages in srv-1201 (FireGun, BulletCollision, LocallyTrackedDamage, DealDamage, ExplosionPlayerDamage,
  StunMsg, Cooldown*, DestroyMagazine) are all types gear.rs already handles; values not diffed one by one yet.

### 2026-11-18 20:45: lobby entrant data (SNSLobbyEntrantDataChanged), late-joiner candidate

Deployed (previous `echo-rust-server.pre-entrantdata.exe`). 130 tests pass. Not tried in game.
- Our server never sent or relayed `SNSLobbyEntrantDataChanged` (0x8f05b6735e916658), and clients never sent us any
  (server log 1795033195: none). Native: right after JoinAccepted the server sends the newcomer an 80-byte entry for
  its own slot (key 0xb10f1a6e5ef124cc + 8 key hashes, identical in every capture); the client then publishes its own
  entries (`[slot][key][data][u16]`), which the server gives to everyone, the sender too, as `[slot][key][data]`.
  A late joiner also gets its own empty `0xa0191e7a56fe32c3` entry and each earlier entrant's (value = its slot).
  Native sent one for the joiner on entering the brawler room (24.3 s). The first player has nobody else's to miss,
  which fits "only later players can stun". Implemented: `on_entrant_data`, `send_entrant_data_on_join`.
- Also found: the punch component's reset (echovr 0x140d44cb0, via vtable 0x141caf7d8) sets punching ON by default
  (`flags | 0x180`, block bit from component data), so the client's scripts must turn it off after; only the script
  nodes call the enable/disable setters.
- Not done: `SNSLobbyOwnerDataChanged` (owner's data to joiners; s2c form `[byte][key][data][u16]` not understood).

## 2026-11-18 21:00 — late joiner spawn gate (stun/block outside brawler)
- Native late joiner (netrec-lobby-native-20261118): no other player's equipment, physics,
  player state or touch reaches the newcomer until ~50-150 ms after its OWN first
  SR15NetPlayerEquipmentUpdate (24.28 s -> 24.34/24.43 s). Ours relayed them from the join,
  before the newcomer's body existed. Now gated per recipient (`Peer::spawned`: equipment seen,
  or 45 s fallback). Every capture: clients send equipment before any own body state.
- Native's server (peer -1 in netrec-native-srv-1201) sets entrant data itself: `afk`
  (0xc8e8d0b1a891e4e1, 16-byte value) right after the join header; `spawned`
  (0x2fc49f9e497a528e, empty) and `mm|status` (0x191cf30c481f067d, c4bedaed35d9f32d) on the
  first equipment update. Now sent by ours to everyone.
- Ruled out: per-slot var hashes (all slots of a session get the same set). Var hashes are
  XOR-masked per session (19:52 session differs from all others by mostly one constant);
  not relevant to ours (one baseline for all).
- Not yet done: SNSLobbyOwnerDataChanged, `bodytype` (0x34e2900e3de69463) entrant data.
- dist: echo-rust-server.exe = this build; pre-spawngate.exe = previous; old-running.exe =
  the copy that was running when deployed (locked, renamed).

## 2026-11-18 21:20 — spawn gate did not fix it
- Test server-1795035470.log (new build confirmed: "spawned" lines). Late joiner sent NO
  SR15StunPlayerMsg at all: the stun is client-local, its client never applied the cease fire.
- Var hashes: one XOR constant K=0xa068d94367b13cfc maps 263/270 lobby var hashes between the
  user's sessions (user slot 0, identical across days) and 19:52 (user slot 4, late joiner).
  CRC-linear => a session/receiver-specific suffix in every var name. Searched strings, u64s,
  uuids, slot numbers in both sessions' join messages: no match. Open question: is the lobby
  var set per receiver (only arena multi-slot server captures exist; those are shared)?
- Native owner data (SNSLobbyOwnerDataChanged, also appended to OwnerState) is constant across
  the user's sessions and differs at 19:52, like K. Ours sends none. Not proven related.
- Next data needed: NetRecorder on the late-joiner client on our server.

## 2026-11-18 21:35 — native 2-player reference (client 1, client 2, server)
- captures/native-2p-c1-20261118_2105, native-2p-c2-20261118_2106, native-2p-server-20261118_1305.
  All NATIVE (echovr.exe server at 10.0.92.127).
- Native gives both players the SAME var hashes (270/271, one time-based extra) and VAR_HOLD
  0x6aea99ee00000001 in the full snapshot; no hold changes after. Late joiner gets no
  ComponentEvents; first player alone gets nevt_round_start/nevt_enable_scoring (module
  0x557807c73b06750f) at session start. Native server re-sets owner data key 0x8aeba5e6a2b14847
  per join (value changes) — server-generated (peer -1).
- Native sends a replicated-vars message EVERY frame (24-byte header heartbeat). Ours now does
  too (social lobby); previously silent, and the 1 Hz refresh always hit the same mailbox key.
- Still missing: a recording of a late joiner on OUR server to diff against native-2p-c2.

## 2026-11-18 21:45 — ROOT CAUSE (likely): lobby cease-fire var depends on game mode
- Native sessions our lobby baseline came from: mode 0x305d6e37c1589c45, cease fire VAR_HOLD
  0x098aa551580b0386. netrec-lobby-native-20261118 (19:52): mode 0x042d9cf9cfddcf76, no VAR_HOLD;
  0x9f65696aa4ce186c goes 1->0 entering the brawler room, ->1 leaving.
- Our lobby sessions run mode 0x042d9cf9cfddcf76 (EchoRelay/nakama) but sent VAR_HOLD: clients had
  no such var -> no cease fire for anyone (late joiner just had someone to stun).
- Fix: msg::lobby_hold_var(mode); with_var_added appends the var to the snapshot.
- Also mode-dependent: IdsResponse 045c/b744/2645 bodies, most var hashes (XOR K). Ours sends the
  0x305d set to 0x042d sessions; the other vars are still unchecked (barriers, props).
- Arena goal stun: not addressed; native arena captures show VAR_HOLD only in the snapshot.

## 2026-11-18 ~22:00 — late-joiner lobby stun: STILL BROKEN (handoff)

Status: the user reports late joiners (slot >= 1) can still stun/block outside the brawler room on our
server, after every change below. The first player is fine. Arena goals show the same bug.
User requirement: fix SERVER-SIDE only; "track all player positions, enable stun only for whoever is
in the brawler room". Don't ask the user for more recordings.

### Deployed now (dist/windows/echo-rust-server.exe; backups pre-spawngate/-heartbeat/-modehold/-trace)
- Spawn gate: body-state types (physics, player state, touch) are not relayed to a newcomer until its
  first equipment update (or 45 s). Matches native. Not the fix.
- Entrant data: afk, spawned, mm|status entries like native.
- Social replicate sends a 24-byte header-only heartbeat every frame like native.
- Cease fire sent under BOTH hashes: msg::lobby_hold_var(mode) (0x9f65696aa4ce186c for mode
  0x042d9cf9cfddcf76) and VAR_HOLD 0x098aa551580b0386 (mode 0x305d). Snapshot + every delta.
- Trace logging: "replicated ack", "stun message", per-2s "client sent" (64) / "server sent" (96), 90 s
  after join.

### Proven
- Late joiner's client ACKS our var stream (DeltaAck ~240/s), so it applies the vars.
- Clients send no SR15StunPlayerMsg. SR15StunMsg (0x1feb495968559bc3) is the stun GRENADE only
  (gear.rs). The punch stun therefore isn't a message: it rides on body state (PlayerState / physics /
  touch) or the victim's local hit detection. The attacker's client decides it.
- Native 19:52 (mode 0x042d, late joiner slot 4): 0x9f65 var goes 1->0 at brawler-room entry and 0->1
  at exit. The nevt_enable_scoring ComponentEvents (a90ab82c..., module 0x557807c73b06750f) at 37.59
  and 52.61 line up with VAR_GAME_STATE b059068ef8158682 -> ST_PLAYING. That's a lobby GOAL/score
  reset, NOT brawler. Dead lead for stun.
- Native 2p 21:05/21:06 (mode 0x305d): both peers get VAR_HOLD = 0x6aea99ee00000001.

### Not yet checked (best leads)
1. The other vars in our snapshot: we send the 0x305d hash set to 0x042d sessions. Most map by XOR
   K=0xa068d94367b13cfc but haven't been translated. The first player may work only by luck/ordering.
   Diff the native 0x042d full snapshot (netrec-lobby-native-20261118) against ours, var by var, and
   build a proper per-mode snapshot. IdsResponse 045c/b744/2645 bodies differ by mode as well.
2. Server-side enforcement per the user's instruction: find which bits of the attacker's relayed
   PlayerState/physics carry punch/block-active (compare a native brawler punch vs no punch). Clear
   them in relay when gear.may_fight(sender) is false. gear.may_fight = position in brawler room;
   cease_fire_for() already computes it per player.
3. Var 0x000889cde4391ca2 went 1->0 at 27.18 in native 19:52 (before brawler entry). Unknown, check it.
4. HostState first byte: native 0x38, ours 0x00. OwnerState lacks the owner-data blob.

### Tools
- Decode: echo-rust-server.exe --decode-netrec <dir> -> msgs.csv (dir,t,channel,type,name,mailbox_key,len,hex,peer)
- Var parse: replicated vars at [28..30] count, then (hash u64, value u64) x 16 bytes from offset 30.
- Scratchpad scripts (sym.py hash, vars_lib.py, xref.py) live in the session scratchpad and may be gone.
- 131 tests pass (cargo test).

## 2026-11-18 22:50 — ROOT CAUSE FOUND: cease fire is a per-ENTRANT var (fix deployed, untested in game)
- The 21:45 "per-mode var" claim was WRONG. 0x9f65696aa4ce186c is not mode 0x042d's hold var. It is
  entrant (gen 2, slot 4)'s hold var, which was the recording user's own entrant in netrec-lobby-native-20261118.
- Every entrant (slot, generation from JoinAccepted) has its own cease-fire var, and a client obeys
  only its own entrant's. VAR_HOLD 0x098aa551580b0386 is entrant (0,0)'s. That is why the first
  player was always fine and every late joiner could stun and block anywhere, in the lobby and
  through arena goals.
- Native sends ALL entrants' vars to EVERY client: in the snapshot, and as a delta on join or change.
  Proof: native-2p-c1 receives 0x79ac0fb75468917e = 1 (entrant 0/1) when p2 joins at 54.56 s, and
  native-2p-c2's snapshot has it.
- Hash: the names differ only in the generation and slot digits (`<g>?<s>` followed by 15 more chars,
  full name unknown). For names of the same length the Symbol hash is affine, so
  var(g,s) = VAR_HOLD ^ Symbol::calculate(bytes [g,0,s,0*15], 0). See msg::entrant_hold_var; the tests
  pin (0,1), (0,2), (0,9) and (2,4). It works for both modes, 0x305d and 0x042d.
- Fix (msg.rs, session.rs replicate):
  - The social snapshot and the deltas carry every joined entrant's var, each with that entrant's own
    may_fight (LOBBY_HOLD_VERSION | !fight).
  - Deltas are resent when any entrant's value changes (Peer.holds_sent).
  - The arena goal hold (a.flags VAR_HOLD) is mirrored to every entrant's var in tracked and in the snapshot.
  - msg::lobby_hold_var was removed.
- GAP: two-digit slots and generations lengthen the name, which is unknown, so there is no var for them.
  - Generations now wrap mod 10.
  - Slots 10-15 have no cease fire. They are only reached with 11+ players. A debug log line
    "no cease fire var name" fires for them.
- Deployed to dist/windows/echo-rust-server.exe. Backup: echo-rust-server.pre-entranthold.exe. 131 tests pass.
  To verify in game: a late joiner (slot >= 1) cannot stun or block outside the brawler room, and
  can inside it. The log shows "lobby cease fire lifted/on" with slot and genidx.

## 2026-11-18 23:05: lobby quarter-court goal (deployed, untested in game)
- Source data:
  - captures/netrec-info-20261118: a NATIVE SERVER-side NetRecorder (from the user's info.zip), about
    10 min with up to 32 peers. It is the same session as the pasted log with the goal at 14:40:19
    (t = 146.66 s). msgs.csv is 1.4 GB; small.csv drops the per-frame traffic; discs.csv holds the
    decoded d33d44b7 disc records.
  - The goal mesh is mpl_arena_goal_di_i705 in C:/Users/lucas/Documents/lobby.fbx (the user named it):
    - Centre (-53.540, 1.482, -30.910) m, rotated 60 deg about +Y, so its front normal is (0.866, 0, 0.5).
    - Ring radius 1.555 m.
    - The FBX is in the same frame as the game's disc positions.
    - i740 and i746 are the mini-arena goals. The user said to ignore them.
- What native does:
  - Goal test: the quarter-court disc (set d33d44b7, entry 0) crosses the ring plane front->back.
    - All 6 goals did this, 0.09-1.02 m from the centre.
    - Every other front->back pass was >= 1.42 m out.
    - One back->front pass at 0.34 m did nothing.
    - The disc owner keeps simulating; there is no freeze or reset.
  - VAR_GAME_STATE goes ST_PLAYING -> 0xb059068ef8158682 for 3 s, then back. At that moment native
    sends ComponentEvent EV_ENABLE_SCORING on module 0x557807c73b06750f, handle 0x2e (OnRoundStartCB).
    Our 045c IdsResponse is byte-identical to that session's, so the handle numbering matches.
  - PostScoreUpdate (28 bytes, same layout as the arena's) goes to everyone:
    team 1, 2 points, type 2 (INSIDE SHOT), the owner's handle, assist ffffffff, speed, release->ring
    distance, and the lobby clock in seconds.
  - Native sent it for only 3 of the 6 goals. The reason is unknown; release side offset might matter.
    OURS SCORES EVERY GOAL.
  - Var 1b10bdc962b47fcc is the quarter-court disc's current owner handle (ffffffff = server). Ours
    used to set it to 0 on the first grab only; it now follows every grant and goes to ffffffff when
    the holder leaves.
  - Also confirmed: 207b92ea865d8b86 (entrant 0/7's hold var) flips at brawler-room entry and exit,
    which backs up the per-entrant cease-fire fix.
- Code:
  - props.rs: QC_*, through_the_ring, qc_track/qc_step, take_goal/take_round_start, and on_grant(to, handle, ..).
  - session.rs: lobby_goal().
  - msg.rs: MOD_LOBBY_SCORING, LOBBY_SCORING_HANDLE.
  - 134 tests pass.
- Deployed: dist/windows/echo-rust-server.exe. Backup: echo-rust-server.pre-qcgoal.exe (= the entranthold build).
  The log line to look for is "quarter court goal: team 1, 2 points".
- Not done: mini-arena goals. Native sends PostScoreUpdate (team 0 near i746, team 1 near i740, types 4/0x0a)
  but no game-state change.

## 2026-11-18 23:13: quarter court 2s and 3s (deployed, untested in game)
- User report: goals and the stun fix both work, but every goal showed 2 points. The bubble just
  outside the goal decides 2 vs 3.
- The bubble is mpl_arena_barrier_3pt_bubble_blue_i4364 in lobby.fbx (material mpl_score_volume_blue).
  Its vertices fit a sphere cap of radius 7.97 m (all within 0.2 m), centred at world
  (-54.106, 1.466, -31.237), 0.7 m behind the ring.
- Rule:
  - 3 points (goal type 3, LONG SHOT) when the disc's first free-flight position after leaving a hand
    is more than 7.97 + 0.3 m from that centre. Otherwise 2 points (type 2, INSIDE SHOT).
  - The 0.3 m slack is there because native's six recorded goals were all 2-pointers, and their first
    free positions were 6.2-8.2 m from the centre, sampled at about 15/s.
  - The arena's rule (release->goal distance >= 7 m) does NOT apply here: native gave 2 points at
    7.41 m and 8.08 m.
- Code: props.rs QC_BUBBLE and qc_points; QcGoal has points and goal_type; session.rs lobby_goal sends them.
- 136 tests pass. Deployed. Backup: echo-rust-server.pre-qcthree.exe (= the first quarter-court build).
- If 3s trigger too easily or too rarely in game, tune QC_BUBBLE_SLACK.

## 2026-11-18 23:40: tablet stats and Early Quits

- Stats were being saved (our server sends `UserServerProfileUpdateRequest`, and atlas queued and wrote them with no warnings). The tablet showed 0 because atlas builds the profile only at login and never sent it again after a match.
- Atlas fix (`server/evr_statistics_queue.go`): after the statistics worker writes entries from the profile-update or remote-log writers, it calls `RefreshPlayerProfile` for each user it wrote. This is server side, so it works for every client, with or without EVR-Runtime. Built to `atlas/HOWTOBUILD/local/atlas.statsrefresh.exe`. NOT deployed: stop atlas, back up `atlas.exe` as `atlas.pre-statsrefresh.exe`, then swap the new build in.
- EVR-Runtime `menu/wording.h`: renames the tablet's "Arena Ties" row to "Early Quits" (atlas counts early quits in ArenaTies). All 106 tests pass. The DLL was rebuilt in `runtime/build` but not deployed.
- Party tablets: atlas has no switch for them; it always handles them. EVR-Runtime enables party only when built with `-DEVR_PARTY=ON`, which CI does for DevTest/beta only. Main/stable builds ship with the PARTY tab disabled.

## 2026-11-18 23:55: all lobby combat-room barriers vanish when broken

- The user reported that barriers broke but didn't vanish. `LOBBY_BARRIER_VARS` had only 3 of the 11 barriers (64/65/66, the ones broken in srv-1201), so the other 8 never set their var to 0.
- The other 8 come from `tools/gen_combat_vars.py` `barrier_var` with the handle read in decimal. Checked: it reproduces 64→65/66, and all 11 vars are in native srv-1201's lobby baseline at 1 (the hex reading misses 7 of them). Test `every_lobby_barrier_has_its_var`; 137 tests pass.
- Deployed `dist/windows/echo-rust-server.exe` (previous `echo-rust-server.pre-barriervars.exe`). Not seen in game yet.

## 2026-11-19 00:20: arena overtime announced and relaunched; LAN servers get a location (built, NOT deployed)

**Overtime** (user report: "announcer doesnt mention overtime and it instantly starts").
- Cause: `start_sudden_death` waited `sudden_death_regulation_over_time` (0 s) before the new round. The overtime call went out 2 frames before the walk, and nobody was sent back.
- Now: play holds (gates shut, `VAR_HOLD` 1, a countdown), "it's a draw" plays at 1 s and "overtime" at 3.5 s, and the pause lasts at least `SD_CALL_SECS` = 6 s. Then the goal flow's restart (`EV_ROUND_RESTART`, `EV_ROUND_START`, `EV_REALLY_RESTART`) sends everyone back to the launchers, followed by `transition_to_launch`, the `HOLD_SECS` hold, `pre_launch` and the usual launch. A scoreless overtime round repeats the same calls.
- No capture has a native overtime (`sudden_death_active` stays at its baseline in all three that carry it), so the timings come from the user's description and our goal flow.
- Test `tied_time_up_goes_to_repeating_sudden_death` now checks the order (call < restart < launch) and the hold. 137 tests pass.
- Built as `dist/windows/echo-rust-server.overtime.exe`. Not deployed because the server was running (PID 22476). To deploy: stop the server, copy the build over `echo-rust-server.exe`, and start it again. The backup is already there: `echo-rust-server.pre-overtime.exe` is identical to the running exe.

**Atlas: game server location** (user: "atlas has no clue where game servers are located").
- Servers on the LAN register from `192.168.1.157`. `IPInfoCache.Get` answers private addresses with an empty stub, so they got no country, region or city, and the Discord pickers showed "--". The same applies to native and Rust servers.
- `server/evr_ip_info_location.go`: for a reserved address (private, loopback, link-local, CGNAT), `buildRegionCodes` looks up `EVR_LOCATION_IP` if it is set, otherwise Atlas's own public IP, asked once from ip-api.com (`?fields=status,query`) and retried after 10 min on failure. Only the lookup changes, not the endpoint. Such servers also gain the ipinfo region codes.
- Test `TestLocationLookupIPKeepsPublicAndReplacesPrivate`. Built as `atlas/HOWTOBUILD/local/atlas.location.exe` (from the full working tree, so it includes the stats refresh). Not deployed: atlas is running (PID 5548) as a background task of the session that started it.

## Arena overtime: native sequence (2026-11-19, captures from C:\...\NetRecorder\20261119_005556_20308, decoded tail in H:\EVR\captures\netrec-pub-ot-20261119)
- `start_sudden_death` now follows the public capture exactly: round_over_enter + two match events (0x086e…, 0x46dd…) →
  pre_sudden_death (sudden_death_active 1, hold, event 0xe283…, `MP_ANNOUNCER_0100{A-F}_DRAW`, event 0x0ae9…) →
  sudden_death_new_round 5 s → round restart + round start, transition_to_launch (team-room cue twice, 3 s apart), 4.5 s →
  pre_launch with `MP_ANNOUNCER_OT_ROUND_<n>` → 1 s → disc restart, round_start, 15 s launch. Public OT clock 2:00 (seen 120.00).
  `001A_OVERTIME` is not used by native here.
- Overtime goal: after the 15 s celebration native goes goal[2], goal[3], EV_RESTART_B, post_sudden_death (active 0),
  then match over (`leave_sudden_death` from `end_round`). Scoreless OT round repeat is not in a capture (we restart the
  same sequence with OT_ROUND_n+1).
- Private overtime uses the same sequence with no clock (unverified: no private capture). Deployed; backup `echo-rust-server.pre-native-ot.exe`.
- **Open (user, 2026-11-19): overtime round counter.** A second overtime (scoreless first OT round) starts another
  overtime, but the scoreboard keeps reading "OVERTIME / SUDDEN DEATH / ROUND: 0"; native counts the OT rounds there. The
  end result is the same (next goal wins), but the counter needs the right var. Our `start_sudden_death` only bumps
  `ot_rounds` (used for the `MP_ANNOUNCER_OT_ROUND_<n>` call); find the var the HUD's ROUND reads in sudden death (the
  capture netrec-pub-ot-20261119 has one OT round only: diff its replicated vars at 3591.4 for one that goes 0 -> 1, or
  get a capture with two OT rounds) and set it per OT round.

## Disc at time up (2026-11-19)
- At time up the disc now stays free (`Arena::disc_loose`, `end_disc`): the holder keeps simulating it, grabs are granted,
  states are forwarded. Only a goal-ended round/match pins it (`frozen` set by the goal). The next round's walk
  (`Act::NewRound`) takes it back for the server; sudden death's `take_disc` clears it too. (Was: `pin_disc` at every end,
  which froze a thrown disc in mid-air.)

## 2026-11-19: cease fire for slots 10-15 (v0.1.6)
- Source: the user's native server recording `J:\IDM\part_003.zip`. It is the rest of `captures/netrec-info-20261118`
  (same session, parts 1-12, up to 108 peers); only parts 0-1 are decoded, which was enough. J: has ~9 GB free,
  so decode the rest on H: if it's needed.
- The server's own entrant data (peer -1) shows slots 10 and 11 joining at 213.7 s and 215.5 s. Vars that first
  appear in a delta at those moments: (0,10) 0x1ed136b5f1c0fc6a, (0,11) 0x6ef79c53fda36e92, and later (1,10)
  0x81379f7209860106, (1,11) 0xf111359405e593fe (right after (1,9) at 447.0 s).
- Their XORs show the generation and the ones digit at the same distances from the end as in the 18-character tail.
  So H(g, 10+s) = VAR_HOLD_SLOT10 ^ fold0([g,0,s,0*15]), whatever the full name is (`msg::entrant_hold_var`, pinned
  by tests). '?'->'1' (zero-padded tens) and a '1' prefix on the generation were tried and do not fit.
- Two-digit generations are still unknown (they wrap mod 10).

## 2026-11-19: native announcer calls for saves, interceptions, steals, goals

- `src/engine/announcer.rs`: the stock `mp_announcer_server_state` (e28785a6428603f9) pick tables, read by emulating
  each `expression_<id>_name` from the script DLL (unicorn; DLL+PDB from the StreamingScriptWin10 container, symbols
  via dbghelp). Every send picks one line uniformly at random from its table.
- Calls are made in `apply_credits` where we count the stat: save at once; interception 0.75 s later from the
  interceptor's team's table; steal = `steal_sound` at once and the call 0.75 s later. Goal calls now use native's
  random 2/3-point tables. Checked against netrec-pub-ot-20261119 (cue times vs player and team stat increments).
- Not done: clears, possession changes, generic interception, post hit, punch, rampage. The engine raises the clear,
  steal, interception and possession events from one routine around 0x140330000 in FunBuild echovr.exe (refs to the
  `OlPrEfIxnevt_*` records at 0x142b96d70+); its clear and possession conditions are not decoded.
- `docs/announcer_lines.md/.csv`: every file in J:\EchoAudio\vo_announcer_arena, now renamed to its cue name
  (`_rename_map.csv` in that folder maps back to the `<event>_<media>.wav` names).
