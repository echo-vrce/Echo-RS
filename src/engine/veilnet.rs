//! EchoCombat's VeilNet, server side.
//!
//! EchoCombat clients (VeilNet.dll) send small packets of their own over the
//! game's UDP connection: a hello every 2 s, the new gear's events (UMBRAL VEIL,
//! NEUTRON, TRANSLOCATOR, OVERCHARGE, MEDIGUN) and RULES asks. A client whose
//! hellos go unanswered decides the server has no EchoCombat and hides the new
//! tac mods, so the server must answer, relay the events to every other
//! EchoCombat client and keep the private-match rules.
//!
//! Every packet is 16 bytes: `"ECVL"`, a type byte, 3 pad bytes, a u32 id and
//! `"VEIL"` (little-endian). Ported from EchoCombat's `plugin/veilnet.cpp`.
//!
//! The match itself (`combat.rs`) applies what the rules mean: the RULES page
//! 1 values (sent to the clients here as `R`), POINT RATE, PAYLOAD SPEED and
//! PAUSE; on a payload map a PAUSE waits for the payload to be held.
//!
//! The post-match map buttons (`C` 2000000xh | map, private matches): the
//! ask is handed to the session (`take_map_ask`), which changes the lobby's
//! level and reloads; `announce_map` then tells every client (`L` map), and
//! their VeilNet takes them to the loading screen (see `session.rs`,
//! `request_map`).

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tracing::{debug, info};
use uuid::Uuid;

pub const PACKET_LEN: usize = 16;
const MAGIC: u32 = 0x4c56_4345; // "ECVL"
const MAGIC2: u32 = 0x4c49_4556; // "VEIL"

/// A client counts as present for this long after its last packet.
const CLIENT_LIFE: Duration = Duration::from_secs(30);
const MAX_CLIENTS: usize = 64;
/// UNPAUSE: the announcer counts 5..1 on every client, then the match goes on.
const RESUME: Duration = Duration::from_secs(5);
/// Answer flag: MEDIGUN is enabled on this server.
const ANSWER_MEDIGUN: u32 = 1;

// 'C' (rules ask) ids.
const WANT_CHASSIS: u32 = 0x80;
const WANT_RATE: u32 = 0x40;
const WANT_PAUSE: u32 = 0x20;
const WANT_MAP: u32 = 0x2000_0000;
const LOADOUT_NEW: u32 = 2;
/// The map buttons: Dyson, Combustion, Fission, Gauss (`combat::LEVELS`).
const MAPS: u8 = 4;
/// POINT RATE / PAYLOAD SPEED steps, in quarters (0.25x .. 2.5x).
const RATE_STEPS: std::ops::RangeInclusive<u8> = 1..=10;

const PAUSE_OFF: u8 = 0;
const PAUSE_ON: u8 = 1;
const PAUSE_RESUMING: u8 = 2;

// Modes and maps the rules depend on.
const MODE_COMBAT_PRIVATE: u64 = 0x33bb_f684_2df9_7a3f; // echo_combat_private
const MODE_ARENA_PRIVATE: u64 = 0x0999_0965_f4db_8c03; // echo_arena_private
const LEVEL_FISSION: u64 = 0xdf5c_a7b7_dfa3_83d4; // mpl_combat_fission
const LEVEL_GAUSS: u64 = 0x43e2_da7a_0c62_3a19; // mpl_combat_gauss

pub type Out = Vec<(SocketAddr, [u8; PACKET_LEN])>;

/// The settings file next to the server, as EchoCombat's PowerBoost.ini is
/// next to its DLL on a native server.
pub const SETTINGS_FILE: &str = "echocombat.ini";

/// MEDIGUN is on only when `echocombat.ini` beside the server says
/// `medigun=1` under `[server]`. There is no other switch.
pub fn medigun_from_file() -> bool {
    let Some(path) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(SETTINGS_FILE))) else { return false };
    let on = std::fs::read_to_string(&path).is_ok_and(|t| medigun_in(&t));
    if on {
        info!(file = %path.display(), "EchoCombat MEDIGUN allowed");
    }
    on
}

fn medigun_in(ini: &str) -> bool {
    let mut section = String::new();
    for line in ini.lines().map(str::trim) {
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.trim().to_ascii_lowercase();
        } else if section == "server"
            && let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case("medigun")
        {
            return v.trim() == "1";
        }
    }
    false
}

/// A VeilNet packet, if `d` is one.
pub fn parse(d: &[u8]) -> Option<(u8, u32)> {
    if d.len() != PACKET_LEN {
        return None;
    }
    let u = |i: usize| u32::from_le_bytes(d[i..i + 4].try_into().expect("4"));
    (u(0) == MAGIC && u(12) == MAGIC2).then(|| (d[4], u(8)))
}

pub fn packet(kind: u8, id: u32) -> [u8; PACKET_LEN] {
    let mut p = [0u8; PACKET_LEN];
    p[..4].copy_from_slice(&MAGIC.to_le_bytes());
    p[4] = kind;
    p[8..12].copy_from_slice(&id.to_le_bytes());
    p[12..].copy_from_slice(&MAGIC2.to_le_bytes());
    p
}

fn what(kind: u8) -> &'static str {
    match kind {
        b'V' => "UMBRAL VEIL",
        b'P' => "NEUTRON",
        b'T' => "TRANSLOCATOR",
        b'O' => "OVERCHARGE",
        b'M' => "MEDIGUN",
        _ => "?",
    }
}

/// One game server's VeilNet: its EchoCombat clients and the match rules.
pub struct VeilNet {
    /// Off: VeilNet packets are dropped unanswered (the mod is off for players).
    enabled: bool,
    medigun: bool,
    clients: Vec<(SocketAddr, Instant)>,
    /// The last event ids seen: each event (sent twice) goes out once.
    seen: VecDeque<u32>,
    session: Option<Uuid>,
    private: bool,
    payload_map: bool,
    loadouts: u32,
    chassis: bool,
    paused: u8,
    resume_at: Option<Instant>,
    rate: u8,
    payload_speed: u8,
    /// PAUSE asked for on a payload map while the payload moves: it begins at
    /// the next checkpoint ('G' bit 7, the podium's QUEUED).
    pause_queued: bool,
    /// The match says a PAUSE may begin now.
    pause_ok: bool,
    /// RULES page 1 as one float's bits, unlock * 60 + capture (`R`).
    total: u32,
    /// The session's level: a map switch keeps the session, not the map.
    level: u64,
    /// A map button pressed (map 0..3: Dyson, Combustion, Fission, Gauss),
    /// for the session to take.
    map_ask: Option<u8>,
}

impl VeilNet {
    pub fn new(enabled: bool, medigun: bool) -> Self {
        Self {
            enabled,
            medigun: enabled && medigun,
            clients: Vec::new(),
            seen: VecDeque::with_capacity(32),
            session: None,
            private: false,
            payload_map: false,
            loadouts: 0,
            chassis: false,
            paused: PAUSE_OFF,
            resume_at: None,
            rate: 4,
            payload_speed: 4,
            pause_queued: false,
            pause_ok: true,
            total: 0,
            level: 0,
            map_ask: None,
        }
    }

    /// Follows the server's session: a new one starts with the default rules.
    pub fn sync_session(&mut self, session: Option<(Uuid, u64, u64)>) {
        let Some((lobby, mode, level)) = session else { return };
        if self.session == Some(lobby) {
            if level != self.level {
                // A map switch: the match rules stay, a pause does not.
                self.level = level;
                self.payload_map = level == LEVEL_FISSION || level == LEVEL_GAUSS;
                self.paused = PAUSE_OFF;
                self.resume_at = None;
                self.pause_queued = false;
                self.pause_ok = true;
                self.total = 0;
                debug!(%lobby, payload = self.payload_map, "VeilNet: same session on a new map");
            }
            return;
        }
        self.level = level;
        self.map_ask = None;
        self.session = Some(lobby);
        self.private = mode == MODE_COMBAT_PRIVATE || mode == MODE_ARENA_PRIVATE;
        self.payload_map = level == LEVEL_FISSION || level == LEVEL_GAUSS;
        self.loadouts = 0;
        self.chassis = false;
        self.paused = PAUSE_OFF;
        self.resume_at = None;
        self.rate = 4;
        self.payload_speed = 4;
        self.pause_queued = false;
        self.pause_ok = true;
        self.total = 0;
        debug!(%lobby, private = self.private, payload = self.payload_map, "VeilNet: new session, rules back to the defaults");
    }

    /// LOADOUTS bits 0-1, SERIES-10 ONLY bit 2, PAUSE bits 3-4, payload map
    /// bit 5, PAUSE queued bit 7, POINT RATE bits 8-15, PAYLOAD SPEED bits 16-23.
    fn rules_word(&self) -> u32 {
        self.loadouts
            | u32::from(self.chassis) << 2
            | u32::from(self.paused & 3) << 3
            | u32::from(self.payload_map) << 5
            | u32::from(self.pause_queued) << 7
            | u32::from(self.rate) << 8
            | u32::from(self.payload_speed) << 16
    }

    fn remember(&mut self, from: SocketAddr, now: Instant) {
        if let Some(c) = self.clients.iter_mut().find(|c| c.0 == from) {
            c.1 = now;
            return;
        }
        info!(client = %from, "VeilNet: EchoCombat client");
        self.clients.retain(|c| now.duration_since(c.1) < CLIENT_LIFE);
        if self.clients.len() >= MAX_CLIENTS {
            let oldest = self.clients.iter().enumerate().min_by_key(|(_, c)| c.1).map(|(i, _)| i).expect("full");
            self.clients.swap_remove(oldest);
        }
        self.clients.push((from, now));
    }

    fn first_time(&mut self, id: u32) -> bool {
        if self.seen.contains(&id) {
            return false;
        }
        if self.seen.len() == 32 {
            self.seen.pop_front();
        }
        self.seen.push_back(id);
        true
    }

    /// To every present client but `except`, twice (against loss).
    fn relay(&self, except: Option<SocketAddr>, kind: u8, id: u32, now: Instant, out: &mut Out) -> usize {
        let p = packet(kind, id);
        let mut n = 0;
        for &(addr, last) in &self.clients {
            if Some(addr) != except && now.duration_since(last) < CLIENT_LIFE {
                out.push((addr, p));
                out.push((addr, p));
                n += 1;
            }
        }
        n
    }

    fn tell_rules(&self, now: Instant, out: &mut Out) -> usize {
        self.relay(None, b'G', self.rules_word(), now, out)
    }

    /// A VeilNet packet from `from`; what to send goes to `out`. True for an
    /// event seen for the first time (not a resend).
    pub fn handle(&mut self, from: SocketAddr, kind: u8, id: u32, now: Instant, out: &mut Out) -> bool {
        if !self.enabled {
            return false;
        }
        self.remember(from, now);
        match kind {
            b'H' => {
                out.push((from, packet(b'A', if self.medigun { ANSWER_MEDIGUN } else { 0 })));
                // 0 too: a new session's reset must reach the clients.
                out.push((from, packet(b'R', self.total)));
                out.push((from, packet(b'G', self.rules_word())));
            }
            b'C' => self.rules_ask(from, id, now, out),
            b'M' if self.medigun && self.first_time(id) => {
                self.relay(Some(from), kind, id, now, out);
                return true;
            }
            b'V' | b'P' | b'T' | b'O' if self.first_time(id) => {
                let n = self.relay(Some(from), kind, id, now, out);
                debug!(client = %from, id = format_args!("{id:08x}"), told = n, "VeilNet: {}", what(kind));
                return true;
            }
            _ => {}
        }
        false
    }

    fn rules_ask(&mut self, from: SocketAddr, id: u32, now: Instant, out: &mut Out) {
        if id & 0xF000_0000 == WANT_MAP {
            let k = (id & 0xF) as u8;
            if !self.private || k >= MAPS {
                debug!(client = %from, map = k, "VeilNet: map switch refused (not a private match)");
            } else if self.map_ask.is_none() {
                info!(client = %from, map = k, "VeilNet: map switch asked");
                self.map_ask = Some(k);
            }
            return;
        }
        if !self.private {
            debug!(client = %from, id = format_args!("{id:08x}"), "VeilNet: RULES ask refused (not a private match)");
            return;
        }
        let before = self.rules_word();
        if id & (WANT_CHASSIS | WANT_RATE) == WANT_RATE {
            let asked = (id >> 8) as u8;
            if RATE_STEPS.contains(&asked) {
                *(if self.payload_map { &mut self.payload_speed } else { &mut self.rate }) = asked;
            }
        } else if id & (WANT_CHASSIS | WANT_RATE | WANT_PAUSE) == WANT_PAUSE {
            // PAUSE from running or resuming; UNPAUSE from paused starts the countdown.
            let mut want = if id & 1 != 0 {
                PAUSE_ON
            } else if self.paused == PAUSE_ON {
                PAUSE_RESUMING
            } else {
                self.paused
            };
            // A payload map: only while the payload is held, else at the
            // next checkpoint.
            if want == PAUSE_ON && self.paused == PAUSE_OFF && !self.pause_ok {
                if !self.pause_queued {
                    self.pause_queued = true;
                    info!(client = %from, "VeilNet: PAUSE queued until the payload reaches a checkpoint");
                }
                want = self.paused;
            }
            if want != self.paused {
                self.paused = want;
                self.resume_at = (want == PAUSE_RESUMING).then(|| now + RESUME);
            }
        } else if id & WANT_CHASSIS != 0 {
            self.chassis = id & 1 != 0;
        } else if id <= LOADOUT_NEW {
            self.loadouts = id;
        }
        if self.rules_word() != before {
            let n = self.tell_rules(now, out);
            info!(client = %from, rules = format_args!("{:08x}", self.rules_word()), told = n, "VeilNet: RULES changed");
        }
    }

    /// Whether `addr` is an EchoCombat client (heard from lately).
    pub fn is_client(&self, addr: SocketAddr, now: Instant) -> bool {
        self.clients.iter().any(|&(a, last)| a == addr && now.duration_since(last) < CLIENT_LIFE)
    }

    /// A map button press, once.
    pub fn take_map_ask(&mut self) -> Option<u8> {
        self.map_ask.take()
    }

    /// Tells every EchoCombat client the server is switching to map `k`
    /// (`L`): they leave the game once the lobby shows the new level.
    pub fn announce_map(&self, k: u8, now: Instant, out: &mut Out) -> usize {
        if !self.enabled {
            return 0;
        }
        let n = self.relay(None, b'L', u32::from(k), now, out);
        info!(map = k, told = n, "VeilNet: map switch announced");
        n
    }

    /// What the match should apply.
    pub fn rules(&self) -> crate::engine::combat::EchoRules {
        crate::engine::combat::EchoRules {
            rate: self.rate,
            payload_speed: self.payload_speed,
            paused: self.enabled && self.private && self.paused != PAUSE_OFF,
        }
    }

    /// The match's side: whether a PAUSE may begin now, and RULES page 1
    /// (`R`, told to every client when it changes). A queued PAUSE begins
    /// once it may.
    pub fn set_match(&mut self, pause_ok: bool, total: u32, now: Instant, out: &mut Out) {
        if !self.enabled {
            return;
        }
        self.pause_ok = pause_ok;
        if total != self.total {
            self.total = total;
            let n = self.relay(None, b'R', total, now, out);
            debug!(total = f32::from_bits(total), told = n, "VeilNet: RULES unlock / capture time");
        }
        if self.pause_queued && pause_ok && self.paused == PAUSE_OFF {
            self.pause_queued = false;
            self.paused = PAUSE_ON;
            let n = self.tell_rules(now, out);
            info!(told = n, "VeilNet: queued PAUSE on (the payload is held)");
        }
    }

    /// Ends the UNPAUSE countdown when it is due.
    pub fn tick(&mut self, now: Instant, out: &mut Out) {
        if self.paused == PAUSE_RESUMING && self.resume_at.is_some_and(|t| now >= t) {
            self.paused = PAUSE_OFF;
            self.resume_at = None;
            let n = self.tell_rules(now, out);
            info!(told = n, "VeilNet: PAUSE off (countdown over)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sns::symbol;

    fn a(port: u16) -> SocketAddr {
        SocketAddr::from(([10, 0, 0, 1], port))
    }

    #[test]
    fn symbols_match_the_names() {
        assert_eq!(MODE_COMBAT_PRIVATE, symbol("echo_combat_private"));
        assert_eq!(MODE_ARENA_PRIVATE, symbol("echo_arena_private"));
        assert_eq!(LEVEL_FISSION, symbol("mpl_combat_fission"));
        assert_eq!(crate::engine::msg::LEVEL_FISSION_CLIMAX, symbol("mpl_combat_fission_climax"));
        assert_eq!(LEVEL_GAUSS, symbol("mpl_combat_gauss"));
    }

    #[test]
    fn packets_round_trip_and_match_veilnet_cpp() {
        let p = packet(b'V', 0x1234_5678);
        assert_eq!(&p[..4], b"ECVL");
        assert_eq!(&p[12..], b"VEIL");
        assert_eq!(parse(&p), Some((b'V', 0x1234_5678)));
        assert_eq!(parse(&p[..15]), None);
        let mut bad = p;
        bad[13] = 0;
        assert_eq!(parse(&bad), None);
    }

    #[test]
    fn medigun_only_from_the_server_section() {
        assert!(medigun_in("[server]
medigun=1
"));
        assert!(medigun_in("; EchoCombat
[Server]
 medigun = 1
"));
        assert!(!medigun_in("medigun=1"));
        assert!(!medigun_in("[client]
medigun=1"));
        assert!(!medigun_in("[server]
medigun=0"));
        assert!(!medigun_in(""));
    }

    #[test]
    fn off_means_no_answers() {
        let mut v = VeilNet::new(false, true);
        let mut out = Out::new();
        v.handle(a(1), b'H', 0, Instant::now(), &mut out);
        v.handle(a(2), b'H', 0, Instant::now(), &mut out);
        v.handle(a(1), b'V', 1, Instant::now(), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn hello_is_answered_with_the_rules() {
        let mut v = VeilNet::new(true, true);
        let mut out = Out::new();
        v.handle(a(1), b'H', 0, Instant::now(), &mut out);
        assert_eq!(out.len(), 3);
        assert_eq!(parse(&out[0].1), Some((b'A', ANSWER_MEDIGUN)));
        assert_eq!(parse(&out[1].1), Some((b'R', 0)));
        assert_eq!(parse(&out[2].1), Some((b'G', 4 << 8 | 4 << 16)));
        assert!(out.iter().all(|(to, _)| *to == a(1)));
    }

    #[test]
    fn events_go_to_everyone_else_once() {
        let mut v = VeilNet::new(true, false);
        let now = Instant::now();
        let mut out = Out::new();
        for p in 1..=3 {
            v.handle(a(p), b'H', 0, now, &mut out);
        }
        out.clear();
        v.handle(a(1), b'V', 7, now, &mut out);
        v.handle(a(1), b'V', 7, now, &mut out); // the resend
        assert_eq!(out.len(), 4, "two others, twice each");
        assert!(out.iter().all(|(to, d)| *to != a(1) && parse(d) == Some((b'V', 7))));
        out.clear();
        v.handle(a(2), b'M', 9, now, &mut out);
        assert!(out.is_empty(), "MEDIGUN is off on this server");
    }

    #[test]
    fn rules_only_change_in_a_private_match() {
        let mut v = VeilNet::new(true, false);
        let now = Instant::now();
        let mut out = Out::new();
        v.handle(a(2), b'H', 0, now, &mut out);
        v.sync_session(Some((Uuid::new_v4(), symbol("echo_combat"), symbol("mpl_combat_dyson"))));
        out.clear();
        v.handle(a(1), b'C', 1, now, &mut out);
        assert_eq!(v.loadouts, 0);
        v.sync_session(Some((Uuid::new_v4(), MODE_COMBAT_PRIVATE, symbol("mpl_combat_dyson"))));
        v.handle(a(1), b'C', 1, now, &mut out); // LOADOUTS CLASSIC
        v.handle(a(1), b'C', WANT_CHASSIS | 1, now, &mut out);
        v.handle(a(1), b'C', WANT_RATE | 8 << 8, now, &mut out);
        assert_eq!(v.rules_word(), 1 | 1 << 2 | 8 << 8 | 4 << 16);
        assert!(out.iter().any(|(to, d)| *to == a(2) && parse(d) == Some((b'G', v.rules_word()))));
        // A new session puts them back.
        v.sync_session(Some((Uuid::new_v4(), MODE_COMBAT_PRIVATE, LEVEL_GAUSS)));
        assert_eq!(v.rules_word(), 1 << 5 | 4 << 8 | 4 << 16);
        // On a payload map the rate row is PAYLOAD SPEED.
        v.handle(a(1), b'C', WANT_RATE | 2 << 8, now, &mut out);
        assert_eq!(v.payload_speed, 2);
        assert_eq!(v.rate, 4);
    }

    #[test]
    fn a_payload_pause_waits_for_a_checkpoint() {
        let mut v = VeilNet::new(true, false);
        let now = Instant::now();
        let mut out = Out::new();
        v.handle(a(2), b'H', 0, now, &mut out);
        v.sync_session(Some((Uuid::new_v4(), MODE_COMBAT_PRIVATE, LEVEL_GAUSS)));
        v.set_match(false, 0, now, &mut out);
        v.handle(a(1), b'C', WANT_PAUSE | 1, now, &mut out);
        assert_eq!(v.paused, PAUSE_OFF);
        assert!(v.rules_word() & 1 << 7 != 0, "queued");
        assert!(!v.rules().paused);
        out.clear();
        v.set_match(true, 0, now, &mut out);
        assert_eq!(v.paused, PAUSE_ON);
        assert!(v.rules().paused);
        assert!(v.rules_word() & 1 << 7 == 0);
        assert!(out.iter().any(|(to, d)| *to == a(2) && parse(d) == Some((b'G', v.rules_word()))));
    }

    #[test]
    fn a_map_button_is_handed_to_the_session() {
        let mut v = VeilNet::new(true, false);
        let now = Instant::now();
        let mut out = Out::new();
        v.handle(a(1), b'H', 0, now, &mut out);
        v.handle(a(2), b'H', 0, now, &mut out);
        v.sync_session(Some((Uuid::new_v4(), 0, LEVEL_FISSION)));
        v.handle(a(1), b'C', WANT_MAP | 3, now, &mut out);
        assert_eq!(v.take_map_ask(), None, "public: refused");
        let lobby = Uuid::new_v4();
        v.sync_session(Some((lobby, MODE_COMBAT_PRIVATE, LEVEL_FISSION)));
        assert!(v.payload_map);
        v.handle(a(1), b'C', WANT_MAP | 3, now, &mut out);
        v.handle(a(1), b'C', WANT_MAP | 3, now, &mut out);
        v.handle(a(1), b'C', WANT_MAP | 9, now, &mut out);
        assert_eq!(v.take_map_ask(), Some(3));
        assert_eq!(v.take_map_ask(), None, "once");
        assert!(v.is_client(a(2), now) && !v.is_client(a(3), now));
        out.clear();
        assert_eq!(v.announce_map(3, now, &mut out), 2);
        assert_eq!(out.iter().filter(|(_, d)| parse(d) == Some((b'L', 3))).count(), 4, "each twice");
        // Same session, new map: Dyson has no payload.
        v.paused = PAUSE_ON;
        v.sync_session(Some((lobby, MODE_COMBAT_PRIVATE, 0x43e2_da79_1464_2604)));
        assert!(!v.payload_map);
        assert_eq!(v.paused, PAUSE_OFF);
        assert!(v.private);
    }

    #[test]
    fn rules_total_goes_out_as_r() {
        let mut v = VeilNet::new(true, false);
        let now = Instant::now();
        let mut out = Out::new();
        v.handle(a(1), b'H', 0, now, &mut out);
        assert!(out.iter().any(|(_, d)| parse(d) == Some((b'R', 0))));
        out.clear();
        let t = (30.0f32 * 60.0 + 2.0).to_bits();
        v.set_match(true, t, now, &mut out);
        assert_eq!(out.iter().filter(|(_, d)| parse(d) == Some((b'R', t))).count(), 2, "twice, against loss");
        out.clear();
        v.set_match(true, t, now, &mut out);
        assert!(out.is_empty(), "only when it changes");
    }

    #[test]
    fn unpause_counts_down_then_resumes() {
        let mut v = VeilNet::new(true, false);
        let t0 = Instant::now();
        let mut out = Out::new();
        v.sync_session(Some((Uuid::new_v4(), MODE_COMBAT_PRIVATE, symbol("mpl_combat_dyson"))));
        v.handle(a(1), b'C', WANT_PAUSE | 1, t0, &mut out);
        assert_eq!(v.paused, PAUSE_ON);
        v.handle(a(1), b'C', WANT_PAUSE, t0, &mut out);
        assert_eq!(v.paused, PAUSE_RESUMING);
        v.tick(t0 + Duration::from_secs(4), &mut out);
        assert_eq!(v.paused, PAUSE_RESUMING);
        out.clear();
        v.tick(t0 + Duration::from_secs(5), &mut out);
        assert_eq!(v.paused, PAUSE_OFF);
        assert!(out.iter().any(|(_, d)| parse(d) == Some((b'G', v.rules_word()))));
    }
}
