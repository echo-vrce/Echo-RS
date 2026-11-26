# Changelog

All notable changes to echo-rust-server and its launcher. The server and the
launcher share one version number.

## 0.3.4 (2026-11-26)

### Changes
- Respawn fix and native combat announcer lines
- Respawn: no capturing or damage before the body reaches the spawn
- Combat announcer: native's payload lines play, extra lines removed

## 0.3.3 (2026-11-25)

### Changes
- Launcher no longer uses CPU while hidden to the tray
- Launcher: no CPU use while hidden to the tray
  - Hidden to the tray, the launcher kept one CPU core busy until shown again (worst on PCs using Windows' software renderer): the redraw that hiding asks for never reached the hidden window, so the window's event loop kept polling for it. While hidden, the launcher now delivers that redraw itself and goes idle

## 0.3.2 (2026-11-25)

### Changes
- Guns and overheat show for everyone, OVERCHARGE works in full lobbies
  - Every player's gun heat is sent from the moment they join, in matches and in the lobby, as native does; it was only sent after a player's first shot and never in the lobby, so other players' guns stayed hidden or stuck on PULSAR and no overheat showed
  - OVERCHARGE: each player gets their own heal bubble settings back on entering the combat area; the server kept only the last sender's, so in a full lobby OVERCHARGE never started and its tac mod stayed stuck
  - DETONATOR blasts are logged with where they went off, how fresh that position was and how far the nearest dummy was
  - Heal bubbles the server does not know are logged

## 0.3.1 (2026-11-25)

### Changes
- Security hardening, faster player updates (experimental), even server frames
  - Accounts: salted PBKDF2 password hashes (old SHA-256 ones upgraded at login), lockout after 5 wrong passwords (1 min, doubling to 1 h), 20 new accounts an hour per address, --require-password
  - Admin endpoints: POST only, from this machine, Host localhost and an X-ERS-Admin header; /status hides private lobbies from other machines
  - /fleet: remote game servers need --fleet-token; a game server only changes its own lobbies and its own players' stats
  - Moderator team and -moderateuser into private or locked lobbies only for --moderators
  - Limits: 32 service connections per address, 8 MiB of unread replies, resource names, stat sizes, fragment buffers; VeilNet answers strangers once
  - Profile updates can no longer abort the process with a non-object value
  - Per-link packet rates and relay waits in the 30 s link log
  - Event drive: no 10 ms service pass; timed work wakes at its own deadline, state checks ride on the server frames
  - Relayed player state goes out --relay-hold-us (500) after it arrives, paced to one early packet per client frame
  - No server frames while nobody is connected
  - Launcher: "Faster player updates (experimental)", off by default
  - Ticks land just after each wire frame boundary: drifting against the wire clock, up to 11% of frames were skipped then sent two at once
  - Pings and the link log run on wall time, so a skipped frame no longer drops them
  - 30 s frame cadence log (gaps, repeats, bursts)
  - --probe-udp joins over UDP, plays and reports state age, arrival gaps, hitches and bandwidth
  - --probe-delay-ms, --probe-jitter-ms, --probe-loss-pct simulate a bad network

## 0.3.0 (2026-11-24)

### Changes
- EchoCombat-only gear hidden from players without the mod
  - Players without EchoCombat no longer see UMBRAL VEIL, OVERCHARGE, NEUTRON or DRONE in the loadout terminal
- EchoCombat players get their own gear tables first
  - A game that asks for its gear tables before VeilNet has recognised it waits up to 2.5 s; EchoCombat games then get EchoCombat's tables first instead of the stock ones (which left their EchoCombat guns and gear missing)
  - Games without EchoCombat still get the stock tables, at most 2.5 s later while loading

## 0.2.9 (2026-11-24)

### Changes
- THREAT SCANNER pings reach teammates
  - Each THREAT SCANNER ping goes to its user's teammates (the server used to drop them)
- Every network message named
  - docs/network-messages.md lists all 146 message types in the native recordings, with names, sizes, and whether this server handles and sends them
- Release notes keep each change on its own line
  - A commit message with no blank line after its title is split into its bullets
  - The 0.2.8 changelog entry is rewritten as separate changes

## 0.2.8 (2026-11-24)

### Fixed
- Fission: a payload rolling back after checkpoint 1 stops at the bottom of
  the lift. It used to roll back up the lift to the checkpoint.

### Added
- Gun heat: each player's heat meter is sent as native's server sends it, up
  with each shot, down after the gun's cooldown, empty after an overheat
  (an approximation of native's; player slots 0-9).
- Automatic builds and releases on GitHub: raising the version on main builds
  Windows and Linux, runs the tests and publishes the release. Release notes
  come from this file, or from the commit messages since the last release.

## 0.2.7 (2026-11-24)

### Fixed
- **STUN FIELD** works as native: the bubble rides on its grenade for its
  whole 6 seconds and stuns whoever it reaches. It no longer jumps to the
  thrower's next ordnance or another object and stuns people far from it.
- Dyson and Combustion: the server sends the match area's off-map object set
  (three objects under the map) every other frame, as native does. It was
  never sent on these maps.
- A player leaving a combat match has their gear bits cleared for everyone,
  as native does.

### Changed
- **OVERCHARGE:** linked teammates can't die for 6.5 seconds (was 5), and its
  user's hits deal +1.5 on the body and +2.5 on the head (were +1 and +2).
- Players without EchoCombat no longer see an ENERGY BARRIER where someone
  used UMBRAL VEIL or NEUTRON (and their shots no longer stop on it).
  NEUTRON's stun still hits them. For UMBRAL
  VEIL they get the stock DETONATOR smoke at the veil instead (a puff every
  2 seconds for the veil's 8), since only EchoCombat can draw the veil itself.
- UMBRAL VEIL no longer counts as a barrier on the server: it stops no blasts
  and takes no damage, as in EchoCombat.

### Added
- A log line for each stun-field hit: thrower, victim, distance from the
  field, its age, and whether it was following its grenade.
- A recharge safety net: a tac mod still in use well past its duration
  (its duration plus 30 seconds) gets its cooldown from the server, and a
  thrown ordnance not given back 5 seconds after its lifetime and cooldown
  gets its refund. Each is logged as a warning.
- **Rejoins:** the server holds a player's stats for 90 seconds after they
  leave a match. If they come back in that time, they get their stats back
  on the scoreboard, in the team totals and for the end-of-match awards and
  XP, and their old team if it has room.
- **Lag compensation for blasts:** DETONATOR, METEOR and COMET blasts also
  hit a player where the thrower's game showed them (up to 250 ms back, by
  the thrower's ping), not only where they are now. Bullet hits were already
  measured this way.
- **Even teams in public matches:** a joiner asking for the bigger team goes
  on the smaller one, and nobody can switch onto a team that would outnumber
  the other. Private matches are unchanged.

### Known issues
- Other players' guns don't show in their hands. Not fixed yet.
- Props and the disc held by others look less smooth than on native. The
  relay measures the same as native; native runs at 120 Hz, so try the
  frame rate slider at 120.

## 0.2.6 (2026-11-23)

### Changed
- The default frame rate is now 120 Hz, for the server (`--frame-hz`) and the
  launcher. Player movement and the held disc used to go out only 20 times a
  second, which made bodies, grabs and saves feel late. A saved launcher
  setting keeps its own value.
- Atlas is told which maps a server hosts when some are excluded
  (`--exclude-maps`), so the matchmaker skips this server for those maps
  instead of the server refusing the match after it was assigned.

### Added
- Connection-quality logging: each player's ping, jitter, packet loss, late
  packets and resends every 30 seconds and when they leave.
- Contested-grab logging in arena: when a grab loses to someone who got the
  disc less than 200 ms earlier, both players' pings and the gap are logged.

### Fixed
- **INSTANT REPAIR** now heals when it is thrown, not only when it is let go
  of, and heals around the grenade itself instead of where an older grenade
  was.
- **STUN FIELD** follows its grenade as it rolls. It used to stay for its whole
  6 seconds where the grenade was thrown, often the thrower's hand, so it
  stunned people nowhere near the visible field.
- **Countdown:** on capture maps the announcer counts all the way from ten to
  one. It stopped at six.
- **Payload checkpoints** count when the payload is let go of just short of
  them (within 0.35 m). Players saw it reach the checkpoint, but the server's
  payload had stopped slightly behind, so it only counted on the next push.
- **Payload dive bombing:** the payload always stays level. It pitched
  nose-down at the edge of Fission's lift.
- Launcher: no longer redraws while hidden in the tray.

### Known issues
- Other players' guns sometimes don't show in their hands. Not fixed yet.

## 0.2.5 (2026-11-22)

### Changed
- The server's network code is now its own (packet encryption, message
  format, name hashes, backend messages). It replaces the nevr crates and was
  checked byte for byte against them. No `protoc` needed to build.

### Fixed
- A STUN FIELD never stuns its own thrower.
- Players joining a match that has already finished are let in and sent back
  to the lobby. A match left with only spectators sends them back too.
- The log warns when a player has no cease-fire value.

## 0.2.4 (2026-11-21)

### Added
- Fission's checkpoint and track events: the lift, doors, gates and the end.
- Arena sends a goal log to Atlas at each goal, so its match score is right.
- Logs for kills and INSTANT REPAIR.

### Fixed
- REPAIR MATRIX heals everyone within 8 m, without needing line of sight.
- Health updates carry the direction a shot came from.
- Cease fire follows each player's reported generation, and unknown loadout
  requests go to the owner.
- Joins are refused once a public match is over, and no award is given twice.
- The payload stays level on Fission's vertical drop.

## 0.2.3 (2026-11-21)

### Added
- Launcher updates drain the servers first (lock running matches, stop idle
  ones), install, restart them, then restart the launcher.
- Kicks from nakama and Atlas take effect at once.
- Players without EchoCombat get the stock gear tables.
- Fission's end-game pillars rise after 50 damage and come back down with full
  health 25 seconds later. They never break.

### Fixed
- Combat matches send every player's cease-fire value, as native does: guns
  unfold, ordnance stays, tac mods recharge.
- STUN FIELD, ARC MINE and NEUTRON stun in matches. ARC MINE uses its own
  blast.
- DETONATOR damage to barriers fades out at 3 m, not 2.5 m.
- Dyson and Surge barrier health and damage checked against native.

## 0.2.1 (2026-11-20)

### Added
- Team-only voice: off in public matches, on by default in private ones, until
  the match is over.
- Host defaults for every private arena rule.
- About 140 more arena announcer lines: dunks, bounce shots, hat tricks,
  assists, punches, rampages, clears, posts, near misses, jousts and loose-disc
  possession.
- Capture-point entry sounds per player, and Gauss's door event after
  checkpoint 1.

## 0.2.0 (2026-11-20)

### Added
- UPnP port forwarding for game servers and the hub (`--no-upnp` turns it off).
- Server-side cheat checks (`--cheat-checks off|log|enforce`): damage, fire
  rate, hits far from the victim or through walls, speed and throws.

### Fixed
- Players who rejoin can be hurt again.
- COMET hits use COMET's barrier multiplier.
- REPAIR MATRIX no longer repeats its hit sound, and OVERCHARGE's cheat death
  happens once per use.

## 0.1.7 (2026-11-19)

### Added
- Arena match rules split into public, private and both in the settings page.
  Private matches start from the private values, and the RULES screen still
  changes them.
- Goalie ping advantage, on or off separately for public and private matches.
  A defender's grab near their own goal wins over an opponent's grab that came
  in less than the defender's round trip earlier.
- Arena award Bruiser (stuns). Combat sends MVP, Guard, Defender, Specialist
  and Cannoneer, like native.
- Private combat: the ready button toggles, restart requests from every team
  restart the match, and the end screen offers a rematch or the lobby.
- Arena announcer calls for saves, interceptions, steals and goals, picked from
  the game's own tables.

### Changed
- The payload moves at one speed along the whole track, faster only with more
  attackers.

### Fixed
- A throw's release no longer counts as a bounce, so most goals are no longer
  called bounce shots.
- Cease fire for lobby slots 10 to 15.

## 0.1.6 (2026-11-19)

### Added
- Lobby: quarter-court goal worth 2 or 3 points, all 11 combat-room barriers,
  and the level scripts' events relayed between players.
- Arena: native sudden-death sequence. The disc stays live at time up.
- Combat: native explosion targeting and whole-body damage.

### Fixed
- Lobby: every player gets their own cease fire, and only the brawler room
  lifts it. Late joiners could stun and block anywhere.
- Arena: mercy ends the match instantly.

## 0.1.5 (2026-11-18)

- Version bump.

## 0.1.4 (2026-11-16 to 2026-11-17)

### Added
- Combat matches on all four maps (Dyson, Combustion, Fission, Gauss), rebuilt
  from native recordings: round flow, capture points, the payload, barriers,
  war rooms, stats and XP.
- End-of-match awards (MVP, goals, passes, catches) and tier XP for goals,
  assists and saves.
- Frame rate up to 240 Hz, changeable while games run.
- Public matches lock to new players in the last minute and once over.
- Punching and blocking in the lobby's brawler room.
- IPv6 support, so players can reach hosts behind CGNAT.
- Launcher updates come from echo-vrce/Echo-RS.

### Fixed
- Match timing runs on real time, so high frame rates no longer slow games
  down.
- Orange goals play orange's celebration.

## 0.1.3 (2026-11-16)

### Fixed
- A headbutted disc is shown to everyone.
- The spectator HUD's disc speed after a goal no longer reads 0 m/s.
- Gear stations are freed 3.5 seconds after leaving the lobby's combat area.

## 0.1.2 (2026-11-16)

### Fixed
- A caught or headbutted disc no longer snaps back: only the current holder's
  disc is forwarded.

### Added
- `--decode-netrec` turns a NetRecorder folder into a message list.

## 0.1.1 (2026-11-16)

### Added
- Lobby props, discs and the Fury transport.
- Bullet damage, kills and automatic respawn.
- Launcher with updates from GitHub releases, a debug logging setting and a
  frame rate slider.

## 0.1.0 (2026-11-12)

- First release: a standalone, all-Rust Echo VR server with its own login,
  config and matchmaking services and its own game-server engine.
