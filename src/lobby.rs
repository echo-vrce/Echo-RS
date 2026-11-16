//! Lobby registry and matchmaker.
//!
//! Game servers (embedded nevr instances, or external nevr-server processes
//! pointed at `/fleet`) register here. Client lobby requests are matched to an
//! existing lobby or allocated a free game server. All state sits behind one
//! short-held lock that is never held across an await, so matching is a few
//! microseconds of work per request.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nevr_proto::rtapi::v1 as pb;
use nevr_proto::rtapi::v1::envelope::Message as Em;
use parking_lot::Mutex;
use tokio::sync::{Notify, mpsc};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::sns::{self, LobbyRequest, LobbyRequestKind, SessionSuccess, XpId, fail, symbol};
use crate::store::Store;

pub const TEAM_ANY: i16 = -1;
pub const TEAM_BLUE: i16 = 0;
pub const TEAM_ORANGE: i16 = 1;
pub const TEAM_SPECTATOR: i16 = 2;
pub const TEAM_SOCIAL: i16 = 3;
pub const TEAM_MODERATOR: i16 = 4;

pub const APP_ID: &str = "1369078409873402";
/// Final PCVR build (34.4.631547).
pub const VERSION_LOCK: u64 = 0xc62f01d78f77910d;

/// How long an empty lobby is kept before its server is released.
const EMPTY_LOBBY_GRACE: Duration = Duration::from_secs(30);
/// How long a lobby request waits for a free game server to appear.
const ALLOCATION_WAIT: Duration = Duration::from_secs(15);

pub type ReplyTx = mpsc::UnboundedSender<Vec<u8>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKind {
    Social,
    Arena,
    Combat,
}

pub struct Modes {
    pub social: u64,
    pub social_private: u64,
    pub arena: u64,
    pub arena_private: u64,
    pub arena_ai: u64,
    pub arena_practice: u64,
    pub combat: u64,
    pub combat_private: u64,
    pub level_social: u64,
    pub level_arena: u64,
    pub combat_levels: [u64; 4],
}

impl Modes {
    fn new() -> Self {
        Self {
            social: symbol("social_2.0"),
            social_private: symbol("social_2.0_private"),
            arena: symbol("echo_arena"),
            arena_private: symbol("echo_arena_private"),
            arena_ai: symbol("echo_arena_public_ai"),
            arena_practice: symbol("echo_arena_practice"),
            combat: symbol("echo_combat"),
            combat_private: symbol("echo_combat_private"),
            level_social: symbol("mpl_lobby_b2"),
            level_arena: symbol("mpl_arena_a"),
            combat_levels: [
                symbol("mpl_combat_dyson"),
                symbol("mpl_combat_combustion"),
                symbol("mpl_combat_fission"),
                symbol("mpl_combat_gauss"),
            ],
        }
    }
    pub fn kind(&self, mode: u64) -> ModeKind {
        if mode == self.social || mode == self.social_private || mode == symbol("social_2.0_npe") {
            ModeKind::Social
        } else if mode == self.combat || mode == self.combat_private || mode == symbol("echo_combat_tournament") {
            ModeKind::Combat
        } else {
            ModeKind::Arena
        }
    }
    pub fn is_private(&self, mode: u64) -> bool {
        mode == self.social_private || mode == self.arena_private || mode == self.combat_private
    }
    fn default_level(&self, mode: u64) -> u64 {
        match self.kind(mode) {
            ModeKind::Social => self.level_social,
            ModeKind::Arena => self.level_arena,
            ModeKind::Combat => {
                // A random combat map this server hosts (any, if none is).
                let ok: Vec<u64> = self.combat_levels.iter().copied().filter(|&l| crate::engine::filter::allows(mode, l)).collect();
                let from = if ok.is_empty() { self.combat_levels.to_vec() } else { ok };
                from[rand::random::<usize>() % from.len()]
            }
        }
    }
    pub fn name(&self, mode: u64) -> &'static str {
        match mode {
            m if m == self.social => "social",
            m if m == self.social_private => "social_private",
            m if m == self.arena => "arena",
            m if m == self.arena_private => "arena_private",
            m if m == self.combat => "combat",
            m if m == self.combat_private => "combat_private",
            m if m == self.arena_ai => "arena_ai",
            m if m == self.arena_practice => "arena_practice",
            _ => "other",
        }
    }
}

/// Seats per team for player teams; spectators/moderators share what remains
/// of the 16 entrant slots.
fn team_cap(kind: ModeKind, team: i16) -> usize {
    match (kind, team) {
        (ModeKind::Social, TEAM_SOCIAL) => 12,
        (ModeKind::Arena, TEAM_BLUE | TEAM_ORANGE) => 4,
        (ModeKind::Combat, TEAM_BLUE | TEAM_ORANGE) => 5,
        (_, TEAM_SPECTATOR | TEAM_MODERATOR) => 6,
        _ => 0,
    }
}
const MAX_ENTRANTS: usize = 16;

/// What the hub sends to a registered game server's fleet connection.
pub type ServerTx = mpsc::UnboundedSender<pb::Envelope>;

pub struct GameServer {
    pub id: u64,
    pub tx: ServerTx,
    pub internal_ip: Ipv4Addr,
    pub external_ip: Ipv4Addr,
    pub port: u16,
    pub lobby: Option<Uuid>,
    pub embedded: bool,
    pub idle_since: Instant,
}

struct Player {
    entrant_id: Uuid,
    team: i16,
    connected: bool,
}

struct Lobby {
    id: Uuid,
    server: u64,
    mode: u64,
    level: u64,
    kind: ModeKind,
    private: bool,
    group_id: Uuid,
    players: HashMap<XpId, Player>,
    empty_since: Option<Instant>,
    created: Instant,
}

impl Lobby {
    fn count(&self, team: i16) -> usize {
        self.players.values().filter(|p| p.team == team).count()
    }
    fn pick_team(&self, wanted: i16) -> Option<i16> {
        if self.players.len() >= MAX_ENTRANTS {
            return None;
        }
        let fits = |t: i16| self.count(t) < team_cap(self.kind, t);
        match self.kind {
            ModeKind::Social => {
                if wanted == TEAM_MODERATOR && fits(TEAM_MODERATOR) {
                    Some(TEAM_MODERATOR)
                } else {
                    fits(TEAM_SOCIAL).then_some(TEAM_SOCIAL)
                }
            }
            _ => {
                if matches!(wanted, TEAM_BLUE | TEAM_ORANGE | TEAM_SPECTATOR | TEAM_MODERATOR) && fits(wanted) {
                    return Some(wanted);
                }
                if matches!(wanted, TEAM_SPECTATOR | TEAM_MODERATOR) {
                    return None;
                }
                let (b, o) = (self.count(TEAM_BLUE), self.count(TEAM_ORANGE));
                let first = if b <= o { TEAM_BLUE } else { TEAM_ORANGE };
                let second = if first == TEAM_BLUE { TEAM_ORANGE } else { TEAM_BLUE };
                [first, second].into_iter().find(|t| fits(*t))
            }
        }
    }
}

#[derive(Default)]
struct State {
    servers: HashMap<u64, GameServer>,
    lobbies: HashMap<Uuid, Lobby>,
    /// Where each player currently is.
    player_lobby: HashMap<XpId, Uuid>,
    /// Party members steered to the leader's lobby (expires).
    follow: HashMap<XpId, (Uuid, Instant)>,
    next_server_id: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Advertise {
    pub lan_ip: Ipv4Addr,
    pub public_ip: Ipv4Addr,
}

pub struct Hub {
    state: Mutex<State>,
    pub modes: Modes,
    pub store: Arc<Store>,
    pub advertise: Advertise,
    /// Signalled whenever a server registers or becomes free.
    server_freed: Notify,
    /// Asks the embedded pool to start another instance.
    pub scale_tx: Mutex<Option<mpsc::UnboundedSender<()>>>,
}

#[derive(serde::Serialize)]
pub struct Stats {
    pub servers: usize,
    pub servers_idle: usize,
    pub lobbies: usize,
    pub players: usize,
    pub accounts: usize,
    pub lobby_list: Vec<LobbyInfo>,
}

#[derive(serde::Serialize)]
pub struct LobbyInfo {
    pub id: Uuid,
    pub mode: &'static str,
    pub level: String,
    pub private: bool,
    pub players: usize,
    pub endpoint: String,
    pub age_secs: u64,
}

impl Hub {
    pub fn new(store: Arc<Store>, advertise: Advertise) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State { next_server_id: 1, ..Default::default() }),
            modes: Modes::new(),
            store,
            advertise,
            server_freed: Notify::new(),
            scale_tx: Mutex::new(None),
        })
    }

    pub fn stats(&self) -> Stats {
        let s = self.state.lock();
        Stats {
            servers: s.servers.len(),
            servers_idle: s.servers.values().filter(|g| g.lobby.is_none()).count(),
            lobbies: s.lobbies.len(),
            players: s.player_lobby.len(),
            accounts: self.store.account_count(),
            lobby_list: s
                .lobbies
                .values()
                .map(|l| LobbyInfo {
                    id: l.id,
                    mode: self.modes.name(l.mode),
                    level: format!("{:#018x}", l.level),
                    private: l.private,
                    players: l.players.len(),
                    endpoint: s.servers.get(&l.server).map(|g| format!("{}:{}", g.external_ip, g.port)).unwrap_or_default(),
                    age_secs: l.created.elapsed().as_secs(),
                })
                .collect(),
        }
    }

    /// Embedded (same-host) servers: UDP port and how long each has been idle.
    pub fn embedded_snapshot(&self) -> Vec<(u16, Option<Duration>)> {
        let s = self.state.lock();
        s.servers.values().filter(|g| g.embedded).map(|g| (g.port, g.lobby.is_none().then(|| g.idle_since.elapsed()))).collect()
    }

    /// Take an idle embedded server out of rotation so it can be stopped.
    pub fn retire_port(&self, port: u16) -> bool {
        let mut s = self.state.lock();
        let Some(id) = s.servers.values().find(|g| g.embedded && g.port == port && g.lobby.is_none()).map(|g| g.id) else {
            return false;
        };
        s.servers.remove(&id);
        true
    }

    // ── Game server side ────────────────────────────────────────────────────

    /// A game server registered. `peer_ip` is the address its fleet socket
    /// came from; loopback means it runs on this host and is advertised with
    /// this host's addresses.
    pub fn register_server(&self, tx: ServerTx, reg: &pb::GameServerRegistrationMessage, peer_ip: Ipv4Addr) -> u64 {
        let local = peer_ip.is_loopback();
        let internal_ip = if local {
            self.advertise.lan_ip
        } else {
            reg.internal_ip_address.parse().unwrap_or(peer_ip)
        };
        let external_ip = if local { self.advertise.public_ip } else { peer_ip };
        let mut s = self.state.lock();
        let id = s.next_server_id;
        s.next_server_id += 1;
        let ack = pb::Envelope {
            cid: Uuid::new_v4().to_string(),
            message: Some(Em::GameServerRegistrationSuccess(pb::GameServerRegistrationSuccessMessage {
                server_id: id,
                external_ip_address: external_ip.to_string(),
            })),
        };
        drop(tx.send(ack));
        s.servers.insert(
            id,
            GameServer { id, tx, internal_ip, external_ip, port: reg.port as u16, lobby: None, embedded: local, idle_since: Instant::now() },
        );
        drop(s);
        info!(server = id, endpoint = %format!("{internal_ip}:{external_ip}:{}", reg.port), "game server registered");
        self.server_freed.notify_waiters();
        id
    }

    pub fn unregister_server(&self, id: u64) {
        let mut s = self.state.lock();
        if let Some(g) = s.servers.remove(&id) {
            if let Some(lid) = g.lobby {
                close_lobby(&mut s, lid);
            }
            info!(server = id, "game server unregistered");
        }
    }

    /// Handle one envelope from a game server.
    pub fn on_server_message(&self, server: u64, env: pb::Envelope) {
        match env.message {
            Some(Em::LobbySessionEvent(ev)) => {
                let Ok(lid) = ev.lobby_session_id.parse::<Uuid>() else { return };
                if ev.code == pb::lobby_session_event_message::Code::Ended as i32 {
                    let mut s = self.state.lock();
                    close_lobby(&mut s, lid);
                    if let Some(g) = s.servers.get_mut(&server) {
                        if g.lobby == Some(lid) {
                            g.lobby = None;
                            g.idle_since = Instant::now();
                        }
                    }
                    drop(s);
                    debug!(server, lobby = %lid, "session ended; server free");
                    self.server_freed.notify_waiters();
                }
            }
            Some(Em::LobbyEntrantConnected(m)) => {
                let lid = m.lobby_session_id.parse::<Uuid>().ok();
                let mut accept = Vec::new();
                let mut reject = Vec::new();
                {
                    let mut s = self.state.lock();
                    let mut lobby = lid.and_then(|l| s.lobbies.get_mut(&l));
                    for eid in m.entrant_ids {
                        let Ok(e) = eid.parse::<Uuid>() else {
                            reject.push(eid);
                            continue;
                        };
                        match lobby.as_deref_mut() {
                            Some(l) => {
                                if let Some(p) = l.players.values_mut().find(|p| p.entrant_id == e) {
                                    p.connected = true;
                                    accept.push(eid);
                                } else {
                                    // The peer already proved itself with its per-entrant
                                    // keys; the id is only a label. Bind it to an
                                    // unconnected reservation.
                                    match l.players.values_mut().find(|p| !p.connected) {
                                        Some(p) => {
                                            p.entrant_id = e;
                                            p.connected = true;
                                            accept.push(eid);
                                        }
                                        None => reject.push(eid),
                                    }
                                }
                            }
                            // A session the hub did not allocate: trust the server.
                            None => accept.push(eid),
                        }
                    }
                    if let Some(g) = s.servers.get(&server) {
                        if !accept.is_empty() {
                            drop(g.tx.send(env_msg(Em::LobbyEntrantsAccept(pb::LobbyEntrantsAcceptMessage { entrant_ids: accept }))));
                        }
                        if !reject.is_empty() {
                            warn!(server, ?reject, "rejecting unknown entrants");
                            drop(g.tx.send(env_msg(Em::LobbyEntrantReject(pb::LobbyEntrantsRejectMessage {
                                entrant_ids: reject,
                                code: pb::lobby_entrants_reject_message::Code::BadRequest as i32,
                            }))));
                        }
                    }
                }
            }
            Some(Em::LobbyEntrantRemoved(m)) => {
                let (Ok(lid), Ok(eid)) = (m.lobby_session_id.parse::<Uuid>(), m.entrant_id.parse::<Uuid>()) else { return };
                let mut s = self.state.lock();
                let mut gone = None;
                if let Some(l) = s.lobbies.get_mut(&lid) {
                    if let Some(x) = l.players.iter().find(|(_, p)| p.entrant_id == eid).map(|(x, _)| *x) {
                        l.players.remove(&x);
                        gone = Some(x);
                    }
                    if l.players.is_empty() {
                        l.empty_since = Some(Instant::now());
                    }
                }
                if let Some(x) = gone {
                    if s.player_lobby.get(&x) == Some(&lid) {
                        s.player_lobby.remove(&x);
                    }
                    debug!(lobby = %lid, player = %x, "entrant left");
                }
            }
            Some(Em::GameServerSaveLoadout(m)) => {
                let Ok(lid) = m.lobby_session_id.parse::<Uuid>() else { return };
                let Ok(eid) = m.entrant_id.parse::<Uuid>() else { return };
                let who = {
                    let s = self.state.lock();
                    s.lobbies.get(&lid).and_then(|l| l.players.iter().find(|(_, p)| p.entrant_id == eid).map(|(x, _)| *x))
                };
                if let (Some(x), Some(entry)) = (who, m.loadout_entry) {
                    if let Some(l) = entry.loadout {
                        let slots = [
                            ("banner", l.banner),
                            ("booster", l.booster),
                            ("bracer", l.bracer),
                            ("chassis", l.chassis),
                            ("decal", l.decal),
                            ("decal_body", l.decal_body),
                            ("emissive", l.emissive),
                            ("emote", l.emote),
                            ("secondemote", l.secondemote),
                            ("goal_fx", l.goal_fx),
                            ("medal", l.medal),
                            ("pattern", l.pattern),
                            ("pattern_body", l.pattern_body),
                            ("pip", l.pip),
                            ("tag", l.tag),
                            ("tint", l.tint),
                            ("tint_alignment_a", l.tint_alignment_a),
                            ("tint_alignment_b", l.tint_alignment_b),
                            ("tint_body", l.tint_body),
                            ("title", l.title),
                        ];
                        self.store.save_loadout(x, &slots);
                    }
                }
            }
            _ => {}
        }
    }

    /// Periodic housekeeping: release servers whose lobby stayed empty.
    pub fn reap(&self) {
        let mut s = self.state.lock();
        let stale: Vec<(Uuid, u64)> = s
            .lobbies
            .values()
            .filter(|l| l.empty_since.is_some_and(|t| t.elapsed() > EMPTY_LOBBY_GRACE))
            .map(|l| (l.id, l.server))
            .collect();
        for (lid, server) in stale {
            close_lobby(&mut s, lid);
            if let Some(g) = s.servers.get_mut(&server) {
                drop(g.tx.send(env_msg(Em::LobbySessionEvent(pb::LobbySessionEventMessage {
                    lobby_session_id: lid.to_string(),
                    code: pb::lobby_session_event_message::Code::Ended as i32,
                }))));
                g.lobby = None;
                g.idle_since = Instant::now();
            }
            info!(lobby = %lid, server, "empty lobby released");
        }
        s.follow.retain(|_, (_, t)| t.elapsed() < Duration::from_secs(60));
    }

    // ── Client side ─────────────────────────────────────────────────────────

    pub async fn lobby_request(self: &Arc<Self>, req: LobbyRequest, xpid: XpId, reply: ReplyTx) {
        let mode = if req.mode == 0 || req.mode == u64::MAX { req.settings.mode as u64 } else { req.mode };
        let group = req.group_id;
        let fail_with = |code: u32, msg: &str| {
            let mut out = Vec::new();
            sns::session_failure_v4(&mut out, mode, group, code, msg);
            drop(reply.send(out));
        };
        if req.version_lock != 0 && req.version_lock != VERSION_LOCK {
            debug!(version_lock = format_args!("{:#x}", req.version_lock), "client version lock differs from final build");
        }
        let wanted = req.entrants.iter().find(|e| e.xpid == xpid).map(|e| i16::from(e.role)).unwrap_or(TEAM_ANY);
        let party: Vec<XpId> = req.entrants.iter().map(|e| e.xpid).filter(|x| *x != xpid).collect();

        let deadline = Instant::now() + ALLOCATION_WAIT;
        loop {
            let notified = self.server_freed.notified();
            let outcome = {
                let mut s = self.state.lock();
                self.place(&mut s, &req, mode, xpid, wanted)
            };
            match outcome {
                Placement::Joined(success, server_tx) => {
                    for p in &party {
                        self.state.lock().follow.insert(*p, (success.lobby_id, Instant::now()));
                    }
                    // Server first so it holds the keys before the client connects.
                    drop(server_tx.send(env_msg(Em::LobbySessionSuccessV5(success.to_proto()))));
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    let mut out = Vec::new();
                    success.encode_v5(&mut out);
                    drop(reply.send(out));
                    info!(player = %xpid, lobby = %success.lobby_id, team = success.team, mode = self.modes.name(mode), "player matched");
                    return;
                }
                Placement::Fail(code, msg) => return fail_with(code, msg),
                Placement::NeedServer => {
                    if let Some(tx) = self.scale_tx.lock().as_ref() {
                        let _ = tx.send(());
                    }
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() || tokio::time::timeout(left, notified).await.is_err() {
                        return fail_with(fail::SERVER_FIND_FAILED, "No game server available");
                    }
                }
            }
        }
    }

    fn place(&self, s: &mut State, req: &LobbyRequest, mode: u64, xpid: XpId, wanted: i16) -> Placement {
        let kind = self.modes.kind(mode);
        let target: Option<Uuid> = match &req.kind {
            LobbyRequestKind::Join { lobby_id, moderate } => {
                let id = match moderate {
                    Some(target) => match s.player_lobby.get(target) {
                        Some(id) => *id,
                        None => return Placement::Fail(fail::SERVER_DOES_NOT_EXIST, "User is not in a lobby"),
                    },
                    None => *lobby_id,
                };
                if !s.lobbies.contains_key(&id) {
                    return Placement::Fail(fail::SERVER_DOES_NOT_EXIST, "Lobby does not exist");
                }
                Some(id)
            }
            LobbyRequestKind::Create { .. } => None,
            LobbyRequestKind::Find { current_lobby } => {
                let followed = s.follow.get(&xpid).map(|(id, _)| *id).filter(|id| s.lobbies.get(id).is_some_and(|l| l.mode == mode));
                followed.or_else(|| {
                    // Fill the fullest public lobby of this mode that has a seat.
                    s.lobbies
                        .values()
                        .filter(|l| l.mode == mode && !l.private && l.id != *current_lobby && l.pick_team(wanted).is_some())
                        .filter(|l| req.group_id.is_nil() || l.group_id.is_nil() || l.group_id == req.group_id)
                        .max_by_key(|l| l.players.len())
                        .map(|l| l.id)
                })
            }
        };

        let lobby_id = match target {
            Some(id) => id,
            None => {
                let level_wanted = if req.level == 0 || req.level == u64::MAX {
                    if req.settings.level > 0 { req.settings.level as u64 } else { self.modes.default_level(mode) }
                } else {
                    req.level
                };
                if !crate::engine::filter::allows(mode, level_wanted) {
                    return Placement::Fail(fail::SERVER_FIND_FAILED, "This server does not host that mode or map");
                }
                // Allocate a free server (embedded instances first: lowest latency).
                let Some(server_id) = s.servers.values().filter(|g| g.lobby.is_none()).max_by_key(|g| g.embedded).map(|g| g.id) else {
                    return Placement::NeedServer;
                };
                let level = if req.level == 0 || req.level == u64::MAX {
                    if req.settings.level > 0 { req.settings.level as u64 } else { self.modes.default_level(mode) }
                } else {
                    req.level
                };
                let private = match req.kind {
                    LobbyRequestKind::Create { lobby_type, .. } => lobby_type == 1 || self.modes.is_private(mode),
                    _ => self.modes.is_private(mode),
                };
                let id = Uuid::new_v4();
                let settings = serde_json::json!({
                    "appid": if req.settings.appid.is_empty() { APP_ID } else { req.settings.appid.as_str() },
                    "gametype": mode as i64,
                    "level": level as i64,
                });
                let g = s.servers.get_mut(&server_id).expect("server chosen above");
                g.lobby = Some(id);
                drop(g.tx.send(env_msg(Em::LobbySessionCreate(pb::LobbySessionCreateMessage {
                    lobby_session_id: id.to_string(),
                    lobby_type: i32::from(private),
                    group_id: req.group_id.to_string(),
                    max_entrants: MAX_ENTRANTS as i32,
                    settings_json: settings.to_string(),
                    features: req.settings.features.clone(),
                    time_step_usecs: 0,
                }))));
                s.lobbies.insert(
                    id,
                    Lobby {
                        id,
                        server: server_id,
                        mode,
                        level,
                        kind,
                        private,
                        group_id: req.group_id,
                        players: HashMap::new(),
                        empty_since: Some(Instant::now()),
                        created: Instant::now(),
                    },
                );
                info!(lobby = %id, server = server_id, mode = self.modes.name(mode), private, "lobby created");
                id
            }
        };

        // Leave any previous lobby record (the old server reports the UDP drop itself).
        if let Some(prev) = s.player_lobby.get(&xpid).copied() {
            if prev != lobby_id {
                if let Some(l) = s.lobbies.get_mut(&prev) {
                    l.players.remove(&xpid);
                    if l.players.is_empty() {
                        l.empty_since = Some(Instant::now());
                    }
                }
            }
        }

        let lobby = s.lobbies.get_mut(&lobby_id).expect("lobby exists");
        let team = match lobby.players.get(&xpid) {
            Some(p) => p.team,
            None => match lobby.pick_team(wanted) {
                Some(t) => t,
                None => return Placement::Fail(fail::SERVER_IS_FULL, "Lobby is full"),
            },
        };
        let Some(g) = s.servers.get(&lobby.server) else {
            return Placement::Fail(fail::INTERNAL_ERROR, "Game server went away");
        };
        let success = SessionSuccess::generate(lobby.mode, lobby_id, lobby.group_id, g.internal_ip, g.external_ip, g.port, team);
        let entrant_id = Uuid::new_v4();
        lobby.players.insert(xpid, Player { entrant_id, team, connected: false });
        lobby.empty_since = None;
        let tx = g.tx.clone();
        s.player_lobby.insert(xpid, lobby_id);
        Placement::Joined(success, tx)
    }

    /// `SNSLobbyPlayerSessionsRequest`: the client asks for its entrant id.
    pub fn player_sessions(&self, xpid: XpId, lobby_id: Uuid) -> Option<(Uuid, i16)> {
        let s = self.state.lock();
        let l = s.lobbies.get(&lobby_id)?;
        l.players.get(&xpid).map(|p| (p.entrant_id, p.team))
    }

    /// A client went away entirely (login socket closed).
    pub fn client_gone(&self, xpid: XpId) {
        let mut s = self.state.lock();
        s.follow.remove(&xpid);
        // The game server reports the UDP disconnect; only unconnected
        // reservations are dropped here.
        if let Some(lid) = s.player_lobby.get(&xpid).copied() {
            let mut drop_it = false;
            if let Some(l) = s.lobbies.get_mut(&lid) {
                if l.players.get(&xpid).is_some_and(|p| !p.connected) {
                    l.players.remove(&xpid);
                    drop_it = true;
                    if l.players.is_empty() {
                        l.empty_since = Some(Instant::now());
                    }
                }
            }
            if drop_it {
                s.player_lobby.remove(&xpid);
            }
        }
    }

    /// Kick a player: the server drops them from its session.
    pub fn kick(&self, xpid: XpId) -> bool {
        let s = self.state.lock();
        let Some(lid) = s.player_lobby.get(&xpid) else { return false };
        let Some(l) = s.lobbies.get(lid) else { return false };
        let Some(p) = l.players.get(&xpid) else { return false };
        if let Some(g) = s.servers.get(&l.server) {
            drop(g.tx.send(env_msg(Em::LobbyEntrantRemoved(pb::LobbyEntrantRemovedMessage {
                lobby_session_id: lid.to_string(),
                entrant_id: p.entrant_id.to_string(),
                code: pb::lobby_entrant_removed_message::Code::Kicked as i32,
            }))));
            return true;
        }
        false
    }
}

enum Placement {
    Joined(SessionSuccess, ServerTx),
    NeedServer,
    Fail(u32, &'static str),
}

fn close_lobby(s: &mut State, lid: Uuid) {
    if let Some(l) = s.lobbies.remove(&lid) {
        for x in l.players.keys() {
            if s.player_lobby.get(x) == Some(&lid) {
                s.player_lobby.remove(x);
            }
        }
    }
}

pub fn env_msg(m: Em) -> pb::Envelope {
    pb::Envelope { cid: Uuid::new_v4().to_string(), message: Some(m) }
}
