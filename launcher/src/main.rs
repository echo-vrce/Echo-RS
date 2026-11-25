//! Echo Server Launcher: a one-page front end for `echo-rust-server`.
//!
//! Paste the EchoRelay or Nakama config (`config.json` with
//! `serverdb_host`), set how many game servers run at once, the ports, the
//! frame rate and which modes and maps they host, then Start.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

mod settings;
mod tray;
mod update;
use serde::{Deserialize, Serialize};

const MODES: &[(&str, &str)] = &[
    ("echo_arena", "Arena (public)"),
    ("echo_arena_private", "Arena (private)"),
    ("echo_arena_public_ai", "Arena vs AI"),
    ("echo_arena_practice", "Arena practice"),
    ("echo_combat", "Combat (public)"),
    ("echo_combat_private", "Combat (private)"),
    ("social_2.0", "Social lobby"),
    ("social_2.0_private", "Social lobby (private)"),
];

const MAPS: &[(&str, &str)] = &[
    ("mpl_arena_a", "Arena"),
    ("mpl_lobby_b2", "Lobby"),
    ("mpl_combat_dyson", "Dyson"),
    ("mpl_combat_combustion", "Combustion"),
    ("mpl_combat_fission", "Fission"),
    ("mpl_combat_gauss", "Gauss"),
];

const LOG_LINES: usize = 3000;
const GREEN: Color32 = Color32::from_rgb(90, 200, 130);
const AMBER: Color32 = Color32::from_rgb(235, 170, 60);
const RED: Color32 = Color32::from_rgb(235, 95, 95);

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
enum ModeChoice {
    All,
    OnlyOne,
    Exclude,
}

/// The server's checks on what clients claim (`--cheat-checks`).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
enum CheatChecks {
    Off,
    Log,
    Enforce,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
struct Setup {
    /// The pasted config.json (or a bare serverdb URL).
    config_json: String,
    /// Most game servers running at once.
    max_games: u32,
    /// Game servers kept started and waiting.
    keep_running: u32,
    /// UDP port of game server 1; game server N uses this + N - 1.
    udp_port: u16,
    frame_hz: u32,
    mode_choice: ModeChoice,
    only_mode: String,
    excluded_modes: Vec<String>,
    excluded_maps: Vec<String>,
    region: String,
    public_ip: String,
    /// Ask the router (UPnP) to forward the game ports.
    upnp: bool,
    cheat_checks: CheatChecks,
    /// Answer EchoCombat (VeilNet). Off: players get stock Combat.
    echocombat: bool,
    /// Settings page: balance values that differ from the stock game's.
    balance: serde_json::Map<String, serde_json::Value>,
    /// Get the newest release from GitHub when the launcher starts.
    auto_update: bool,
    /// Write the server's debug lines (every touch, ping and packet summary).
    /// Off by default: on a busy server they are most of its work and log.
    #[serde(default)]
    debug_log: bool,
    /// Event drive (`--drive event`): other players' movement goes out as it
    /// arrives instead of with the next server frame. Off until it has been
    /// played with full lobbies.
    #[serde(default)]
    event_drive: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            config_json: String::new(),
            max_games: 16,
            keep_running: 2,
            udp_port: 6792,
            frame_hz: 120,
            mode_choice: ModeChoice::All,
            only_mode: "echo_arena".into(),
            excluded_modes: Vec::new(),
            excluded_maps: Vec::new(),
            region: "default".into(),
            public_ip: String::new(),
            upnp: true,
            cheat_checks: CheatChecks::Log,
            echocombat: true,
            balance: serde_json::Map::new(),
            auto_update: true,
            debug_log: false,
            event_drive: false,
        }
    }
}

enum Parsed {
    Empty,
    Bad(String),
    Ok(String),
}

impl Setup {
    fn parse(&self) -> Parsed {
        let t = self.config_json.trim();
        if t.is_empty() {
            return Parsed::Empty;
        }
        if t.starts_with("ws://") || t.starts_with("wss://") {
            return Parsed::Ok(t.to_string());
        }
        match serde_json::from_str::<serde_json::Value>(t) {
            Err(e) => Parsed::Bad(format!("Not valid JSON: {e}")),
            Ok(v) => match v.get("serverdb_host").and_then(|s| s.as_str()).map(str::trim) {
                Some(s) if !s.is_empty() => Parsed::Ok(s.to_string()),
                _ => Parsed::Bad("The config has no \"serverdb_host\".".into()),
            },
        }
    }

    /// The config's `loginservice_host`: where EchoRelay saves players' loadout changes.
    fn login_url(&self) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(self.config_json.trim()).ok()?;
        let s = v.get("loginservice_host")?.as_str()?.trim();
        (!s.is_empty()).then(|| s.to_string())
    }

    fn udp_last(&self) -> u32 {
        u32::from(self.udp_port) + self.max_games.max(1) - 1
    }

    /// `hub` is the local-only TCP port the game servers use to reach the
    /// bridge in this process; nothing outside this PC connects to it.
    fn args(&self, upstream: &str, hub: u16) -> Vec<String> {
        let region = if self.region.trim().is_empty() { "default" } else { self.region.trim() };
        let mut a: Vec<String> = [
            "--backend", "echorelay",
            "--upstream", upstream,
            "--bind", &format!("127.0.0.1:{hub}"),
            "--udp-port", &self.udp_port.to_string(),
            "--max-instances", &self.max_games.max(1).to_string(),
            "--min-idle", &self.keep_running.min(self.max_games).to_string(),
            "--frame-hz", &self.frame_hz.to_string(),
            "--region", region,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if let Some(login) = self.login_url() {
            a.extend(["--login-url".into(), login]);
        }
        if !self.public_ip.trim().is_empty() {
            a.extend(["--public-ip".into(), self.public_ip.trim().into()]);
        }
        match self.mode_choice {
            ModeChoice::All => {}
            ModeChoice::OnlyOne => a.extend(["--only-modes".into(), self.only_mode.clone()]),
            ModeChoice::Exclude if !self.excluded_modes.is_empty() => a.extend(["--exclude-modes".into(), self.excluded_modes.join(",")]),
            ModeChoice::Exclude => {}
        }
        if !self.excluded_maps.is_empty() {
            a.extend(["--exclude-maps".into(), self.excluded_maps.join(",")]);
        }
        if !self.echocombat {
            a.push("--no-veilnet".into());
        }
        if !self.upnp {
            a.push("--no-upnp".into());
        }
        let checks = match self.cheat_checks {
            CheatChecks::Off => "off",
            CheatChecks::Log => "log",
            CheatChecks::Enforce => "enforce",
        };
        a.extend(["--cheat-checks".into(), checks.into()]);
        if self.event_drive {
            a.extend(["--drive".into(), "event".into()]);
        }
        a
    }

    fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        match self.parse() {
            Parsed::Empty => p.push("Paste your config.".into()),
            Parsed::Bad(e) => p.push(e),
            Parsed::Ok(_) => {}
        }
        if self.udp_last() > 65535 {
            p.push("The UDP ports run past 65535.".into());
        }
        if self.mode_choice == ModeChoice::Exclude && self.excluded_modes.len() == MODES.len() {
            p.push("Every game mode is excluded.".into());
        }
        p
    }
}

struct Running {
    child: Child,
    /// The server reads commands here (`frame-hz N`).
    stdin: Option<ChildStdin>,
    exited: Option<String>,
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.exited.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// How a Start ran the server, to run it again after an update.
#[derive(Clone)]
struct Launch {
    exe: PathBuf,
    args: Vec<String>,
    log_filter: String,
}

/// What the window and the update steward share. The steward has its own
/// thread: a window hidden to the tray gets no frames, and that is where a
/// launcher hosting for days usually is.
#[derive(Default)]
struct Shared {
    running: Mutex<Option<Running>>,
    launch: Mutex<Option<Launch>>,
    /// Draining for an update since then: the server takes nobody new and
    /// exits once its matches are empty; then the update goes in and the
    /// servers start again.
    drain: Mutex<Option<Instant>>,
    auto_update: AtomicBool,
    /// Hidden to the tray (shared with the log pumps, which then stop
    /// waking the window).
    hidden: Arc<AtomicBool>,
    /// A status line for the window, from the steward.
    notice: Mutex<Option<String>>,
}

impl Shared {
    fn live(&self) -> bool {
        self.running.lock().expect("running").as_ref().is_some_and(|r| r.exited.is_none())
    }

    fn draining_since(&self) -> Option<Instant> {
        *self.drain.lock().expect("drain")
    }

    fn send_line(&self, line: &str) -> bool {
        use std::io::Write;
        let mut running = self.running.lock().expect("running");
        if let Some(r) = running.as_mut()
            && r.exited.is_none()
            && let Some(w) = &mut r.stdin
        {
            if writeln!(w, "{line}").and_then(|_| w.flush()).is_ok() {
                return true;
            }
            r.stdin = None;
        }
        false
    }

    fn spawn(&self, l: &Launch, log: &Log, ctx: &egui::Context) -> std::io::Result<()> {
        let mut cmd = Command::new(&l.exe);
        cmd.args(&l.args)
            .current_dir(l.exe.parent().unwrap_or(Path::new(".")))
            .env("NO_COLOR", "1")
            .env("ERS_LOG", &l.log_filter)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn()?;
        if let Some(o) = child.stdout.take() {
            pump(o, log.clone(), ctx.clone(), self.hidden.clone());
        }
        if let Some(e) = child.stderr.take() {
            pump(e, log.clone(), ctx.clone(), self.hidden.clone());
        }
        let stdin = child.stdin.take();
        *self.running.lock().expect("running") = Some(Running { child, stdin, exited: None });
        *self.launch.lock().expect("launch") = Some(l.clone());
        Ok(())
    }

    /// Ends the server now. `keep_drain`: for "Update now", the update still
    /// goes in and the servers start again; a plain Stop cancels that.
    fn kill(&self, log: &Log, keep_drain: bool) -> bool {
        if !keep_drain {
            *self.drain.lock().expect("drain") = None;
        }
        let mut running = self.running.lock().expect("running");
        let Some(r) = running.as_mut().filter(|r| r.exited.is_none()) else { return false };
        let _ = r.child.kill();
        let _ = r.child.wait();
        r.exited = Some("stopped".into());
        push(log, "— stopped —".into());
        true
    }

    /// The server's exit, once: its message.
    fn reap(&self, log: &Log) -> Option<String> {
        let mut running = self.running.lock().expect("running");
        let r = running.as_mut().filter(|r| r.exited.is_none())?;
        let st = r.child.try_wait().ok()??;
        let msg = st.code().map_or("ended".into(), |c| format!("exited (code {c})"));
        push(log, format!("— {msg} —"));
        r.exited = Some(msg.clone());
        Some(msg)
    }

    /// Ask the running server to drain for the update `tag`.
    fn begin_drain(&self, log: &Log, tag: &str) {
        if self.draining_since().is_some() || !self.send_line("drain") {
            return;
        }
        *self.drain.lock().expect("drain") = Some(Instant::now());
        push(log, format!("— {tag} downloaded: running matches take nobody new and finish first; then the update goes in and the servers start again —"));
    }
}

/// How often the steward asks GitHub for a new release (the launcher also
/// asks once as it starts).
const CHECK_EVERY: Duration = Duration::from_secs(30 * 60);

/// Ask the window to redraw, unless it is hidden to the tray. On Windows a
/// redraw asked of a hidden window is never delivered, and eframe then polls
/// its event loop nonstop (a whole core) until the window is shown again.
/// Showing it redraws anyway.
fn redraw(ctx: &egui::Context, hidden: &AtomicBool) {
    if !hidden.load(Ordering::Relaxed) {
        ctx.request_repaint();
    }
}

/// The update steward: checks for releases, drains a running server when one
/// is downloaded, installs it once the server is gone, and starts the servers
/// again (or the new launcher, when the launcher itself changed).
fn steward(shared: Arc<Shared>, updater: update::Updater, log: Log, ctx: egui::Context, bridge: Arc<tray::Bridge>, me: Option<PathBuf>) {
    use update::Status;
    let mut next_check = Instant::now() + CHECK_EVERY;
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if let Some(msg) = shared.reap(&log) {
            *shared.notice.lock().expect("notice") = Some(format!("Server {msg}."));
            redraw(&ctx, &shared.hidden);
        }
        let live = shared.live();
        let auto = shared.auto_update.load(Ordering::Relaxed);
        let status = updater.status();
        if auto && Instant::now() >= next_check {
            next_check = Instant::now() + CHECK_EVERY;
            if !updater.busy() && !matches!(status, Status::Ready(_) | Status::Installed { restart: true, .. }) {
                let (c, h) = (ctx.clone(), shared.hidden.clone());
                updater.check(exe_dir(), false, move || redraw(&c, &h));
            }
        }
        let Status::Ready(tag) = status else { continue };
        if live {
            if auto {
                shared.begin_drain(&log, &tag);
                redraw(&ctx, &shared.hidden);
            }
            continue;
        }
        // The server is gone (drained, stopped or never started): in it goes.
        updater.install_pending(&exe_dir());
        let resume = shared.drain.lock().expect("drain").take().is_some();
        let launch = shared.launch.lock().expect("launch").clone();
        let hidden = shared.hidden.load(Ordering::Relaxed);
        match updater.status() {
            Status::Installed { restart: true, .. } => {
                // The launcher itself changed: start the new one, and close this one.
                let Some(me) = &me else { continue };
                let mut cmd = Command::new(me);
                if resume {
                    cmd.arg("--start");
                }
                if hidden {
                    cmd.arg("--hidden");
                }
                cmd.current_dir(exe_dir());
                match cmd.spawn() {
                    Ok(_) => {
                        push(&log, format!("— updated to {tag}: the new launcher takes over —"));
                        bridge.request_quit();
                    }
                    Err(e) => {
                        push(&log, format!("— updated to {tag}, but the new launcher did not start ({e}): start it yourself —"));
                        *shared.notice.lock().expect("notice") = Some(format!("Updated to {tag}: restart the launcher"));
                    }
                }
            }
            s => {
                if let Status::Failed(e) = &s {
                    push(&log, format!("— the update did not go in ({e}) —"));
                }
                if resume && let Some(l) = launch {
                    let what = if matches!(s, Status::Installed { .. }) { format!("updated to {tag}") } else { "still on the old version".to_string() };
                    match shared.spawn(&l, &log, &ctx) {
                        Ok(()) => {
                            push(&log, format!("— {what}: servers started again —"));
                            *shared.notice.lock().expect("notice") = Some("Running.".into());
                        }
                        Err(e) => *shared.notice.lock().expect("notice") = Some(format!("Could not start again after the update: {e}")),
                    }
                }
            }
        }
        redraw(&ctx, &shared.hidden);
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

type Log = Arc<Mutex<VecDeque<String>>>;

/// This run's log file (`logs/server-<unix time>.log` next to the launcher).
static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
/// Log files kept; older ones are deleted when a server starts.
const LOG_FILES_KEPT: usize = 20;

/// Start a new log file for a server run, deleting the oldest past the limit.
fn open_log_file() -> Option<PathBuf> {
    let dir = exe_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let mut old: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("server-") && n.ends_with(".log")))
        .collect();
    old.sort();
    while old.len() >= LOG_FILES_KEPT {
        let _ = std::fs::remove_file(old.remove(0));
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let path = dir.join(format!("server-{secs}.log"));
    let file = std::fs::File::create(&path).ok()?;
    *LOG_FILE.lock().expect("log file") = Some(file);
    Some(path)
}

fn push(log: &Log, line: String) {
    // Keys and passwords never reach the window or the file.
    let line = line.split(' ').map(redact).collect::<Vec<_>>().join(" ");
    if let Some(f) = LOG_FILE.lock().expect("log file").as_mut() {
        use std::io::Write;
        let _ = writeln!(f, "{line}");
    }
    let mut l = log.lock().expect("log lock");
    if l.len() >= LOG_LINES {
        l.pop_front();
    }
    l.push_back(line);
}

fn pump(stream: impl Read + Send + 'static, log: Log, ctx: egui::Context, hidden: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut r = BufReader::new(stream);
        let mut buf = Vec::new();
        while matches!(r.read_until(b'\n', &mut buf), Ok(n) if n > 0) {
            push(&log, strip_ansi(String::from_utf8_lossy(&buf).trim_end()));
            buf.clear();
            // Redraw for new output, batched. Not while hidden: eframe still
            // runs and paints a hidden window's frames, so a busy log kept the
            // launcher drawing (on the software renderer) in the tray.
            if !hidden.load(Ordering::Relaxed) {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
    });
}

/// Hide passwords and keys in a URL's query.
fn redact(url: &str) -> String {
    let Some((base, q)) = url.split_once('?') else { return url.to_string() };
    let parts: Vec<String> = q
        .split('&')
        .map(|kv| match kv.split_once('=') {
            Some((k, _)) if ["password", "api_key", "token", "key", "auth"].contains(&k.to_ascii_lowercase().as_str()) => format!("{k}=•••"),
            _ => kv.to_string(),
        })
        .collect();
    format!("{base}?{}", parts.join("&"))
}

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."))
}

fn server_exe_name() -> &'static str {
    if cfg!(windows) { "echo-rust-server.exe" } else { "echo-rust-server" }
}

fn find_server() -> Option<PathBuf> {
    let d = exe_dir();
    [d.join(server_exe_name()), d.join("..").join(server_exe_name()), d.join("../../../target/release").join(server_exe_name())]
        .into_iter()
        .find(|p| p.is_file())
}

fn setup_path() -> PathBuf {
    exe_dir().join("launcher-setup.json")
}

struct App {
    setup: Setup,
    shared: Arc<Shared>,
    /// From `--start` / `--hidden` (the launcher restarting after an update):
    /// done on the first frame.
    autostart: bool,
    autohide: bool,
    log: Log,
    status: String,
    ctx: egui::Context,
    bridge: std::sync::Arc<tray::Bridge>,
    tray: Option<tray::Tray>,
    hidden: bool,
    page: Page,
    settings: settings::State,
    updater: update::Updater,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Server,
    Settings,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        style(&cc.egui_ctx);
        let setup = std::fs::read_to_string(setup_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let bridge = tray::Bridge::new(cc.egui_ctx.clone());
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = cc.window_handle()
                && let RawWindowHandle::Win32(w) = h.as_raw()
            {
                bridge.set_hwnd(w.hwnd.get());
            }
        }
        let setup: Setup = setup;
        // This executable's path, taken before an update can rename it aside.
        let me = std::env::current_exe().ok();
        update::clean_old(&exe_dir());
        let updater = update::Updater::default();
        let shared = Arc::new(Shared::default());
        if setup.auto_update {
            let (ctx, hidden) = (cc.egui_ctx.clone(), shared.hidden.clone());
            updater.check(exe_dir(), false, move || redraw(&ctx, &hidden));
        }
        shared.auto_update.store(setup.auto_update, Ordering::Relaxed);
        let log: Log = Arc::default();
        {
            let (shared, updater, log, ctx, bridge) = (shared.clone(), updater.clone(), log.clone(), cc.egui_ctx.clone(), bridge.clone());
            std::thread::Builder::new().name("update-steward".into()).spawn(move || steward(shared, updater, log, ctx, bridge, me)).ok();
        }
        let args: Vec<String> = std::env::args().skip(1).collect();
        Self {
            setup,
            shared,
            autostart: args.iter().any(|a| a == "--start"),
            autohide: args.iter().any(|a| a == "--hidden"),
            log,
            status: String::new(),
            ctx: cc.egui_ctx.clone(),
            bridge,
            tray: None,
            hidden: false,
            page: Page::Server,
            settings: settings::State::default(),
            updater,
        }
    }

    /// The update line under the header: what the updater is doing, and its controls.
    fn update_row(&mut self, ui: &mut egui::Ui, live: bool) {
        use update::Status;
        let draining = self.shared.draining_since();
        // (short text, colour, detail on hover)
        let (text, color, detail) = match self.updater.status() {
            Status::Ready(t) if live && draining.is_some() => {
                let mins = draining.map_or(0, |d| d.elapsed().as_secs() / 60);
                (
                    format!("Updating to {t}: waiting for the running matches to empty ({mins} min)"),
                    AMBER,
                    Some("Running matches take nobody new and play on until their players leave; idle game servers have stopped. Then the update goes in and the servers start again on their own.".to_string()),
                )
            }
            Status::Ready(t) if live && self.setup.auto_update => (format!("{t} ready: updating once the running matches are over"), AMBER, None),
            Status::Ready(t) if live => (format!("{t} ready"), AMBER, Some("Update when empty: running matches finish first, then the servers restart on the new version.".to_string())),
            Status::Ready(t) => (format!("Installing {t}…"), AMBER, None),
            Status::Idle => ("Not checked yet".to_string(), MUTED, None),
            Status::Checking => ("Checking for updates…".to_string(), MUTED, None),
            Status::UpToDate(t) => ("Up to date".to_string(), GREEN, Some(format!("Newest release on GitHub: {t}"))),
            Status::Downloading(t) => (format!("Downloading {t}…"), AMBER, None),
            Status::Installed { tag, restart: true } => (format!("Updated to {tag}: restarting the launcher…"), GREEN, None),
            Status::Installed { tag, restart: false } => (format!("Updated to {tag}"), GREEN, None),
            Status::Failed(e) => ("Update check failed".to_string(), RED, Some(e)),
        };
        let busy = self.updater.busy();
        ui.add_space(10.0);
        egui::Frame::default()
            .fill(CARD)
            .stroke(egui::Stroke::new(1.0_f32, BORDER))
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(12, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    // Version badge.
                    egui::Frame::default().fill(RAISED).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
                        ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).size(12.5).strong().color(TEXT));
                    });
                    ui.add_space(4.0);
                    if busy {
                        ui.add(egui::Spinner::new().size(12.0).color(MUTED));
                    } else {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, color);
                    }
                    let l = ui.label(RichText::new(text).size(13.0).color(if color == MUTED { MUTED } else { TEXT }));
                    if let Some(d) = detail {
                        l.on_hover_text(d);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let small = |t: &str| egui::Button::new(RichText::new(t).size(12.5)).corner_radius(6).min_size(egui::vec2(0.0, 24.0));
                        if let Status::Ready(tag) = self.updater.status()
                            && live
                        {
                            if draining.is_some() {
                                if ui.add(small("Update now")).on_hover_text("Stop the server now, players and all, and update").clicked() {
                                    self.shared.kill(&self.log, true);
                                    self.status = "Updating…".into();
                                }
                            } else if !self.setup.auto_update
                                && ui.add(small("Update when empty")).on_hover_text("Running matches take nobody new and finish first; then the update goes in and the servers start again").clicked()
                            {
                                self.shared.begin_drain(&self.log, &tag);
                            }
                            ui.add_space(6.0);
                        }
                        let check = small("Check now");
                        if ui.add_enabled(!busy, check).on_hover_text("Look for a newer release on GitHub now").clicked() {
                            let ctx = self.ctx.clone();
                            self.updater.check(exe_dir(), true, move || ctx.request_repaint());
                        }
                        ui.add_space(6.0);
                        let auto = ui
                            .checkbox(&mut self.setup.auto_update, RichText::new("Auto-update").size(12.5).color(MUTED))
                            .on_hover_text("Check GitHub as the launcher starts and every 30 minutes. A new release goes in once the running matches are over: they take nobody new, idle servers stop, and the servers start again on the new version.");
                        if auto.changed() {
                            self.shared.auto_update.store(self.setup.auto_update, Ordering::Relaxed);
                            self.save();
                        }
                    });
                });
            });
    }

    fn live(&self) -> bool {
        self.shared.live()
    }

    fn save(&mut self) {
        if let Err(e) = std::fs::write(setup_path(), serde_json::to_string_pretty(&self.setup).unwrap_or_default()) {
            self.status = format!("Could not save settings: {e}");
        }
    }

    fn start(&mut self) {
        let Parsed::Ok(upstream) = self.setup.parse() else { return };
        let Some(exe) = find_server() else {
            self.status = format!("Can't find {} next to the launcher.", server_exe_name());
            return;
        };
        self.save();
        // Any free local port: only the game servers inside the server program use it.
        let hub = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map_or(7777, |a| a.port());
        let mut args = self.setup.args(&upstream, hub);
        if !self.setup.balance.is_empty() {
            let path = exe_dir().join("balance-settings.json");
            if let Err(e) = std::fs::write(&path, serde_json::to_string_pretty(&self.setup.balance).unwrap_or_default()) {
                self.status = format!("Could not write {}: {e}", path.display());
                return;
            }
            args.extend(["--balance".into(), path.display().to_string()]);
        }
        self.log.lock().expect("log").clear();
        let log_path = open_log_file();
        if let Some(p) = &log_path {
            push(&self.log, format!("(saving this log to {})", p.display()));
        }
        let shown: Vec<String> = args.iter().map(|a| redact(a)).collect();
        push(&self.log, format!("> {} {}", exe.display(), shown.join(" ")));
        // Debug lines (disc and touch decisions, pings, packet summaries)
        // only when asked for: they cost the server more than anything else.
        let log_filter = std::env::var("ERS_LOG").unwrap_or_else(|_| if self.setup.debug_log { "info,echo_rust_server::engine::session=debug".into() } else { "info".into() });
        match self.shared.spawn(&Launch { exe, args, log_filter }, &self.log, &self.ctx) {
            Ok(()) => self.status = "Running.".into(),
            Err(e) => self.status = format!("Could not start: {e}"),
        }
    }

    /// Tells a running server the new frame rate (its games switch at once).
    fn send_frame_hz(&mut self, hz: u32) {
        self.send_line(&format!("frame-hz {hz}"));
    }

    /// A command line for the running server's stdin.
    fn send_line(&mut self, line: &str) {
        self.shared.send_line(line);
    }

    fn stop(&mut self) {
        if self.shared.kill(&self.log, false) {
            self.status = "Stopped.".into();
        }
    }

    fn reap(&mut self) {
        if let Some(msg) = self.shared.reap(&self.log) {
            self.status = format!("Server {msg}.");
        }
        if let Some(n) = self.shared.notice.lock().expect("notice").take() {
            self.status = n;
        }
    }
}

// ---- Look ---------------------------------------------------------------

const BG: Color32 = Color32::from_rgb(14, 16, 22);
const CARD: Color32 = Color32::from_rgb(23, 26, 35);
const BORDER: Color32 = Color32::from_rgb(38, 43, 56);
const FIELD: Color32 = Color32::from_rgb(11, 13, 18);
const RAISED: Color32 = Color32::from_rgb(34, 38, 50);
const TEXT: Color32 = Color32::from_rgb(232, 235, 242);
const MUTED: Color32 = Color32::from_rgb(132, 140, 160);
const ACCENT: Color32 = Color32::from_rgb(88, 140, 255);

fn style(ctx: &egui::Context) {
    use egui::{CornerRadius, Stroke};
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.hyperlink_color = ACCENT;
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.slider_trailing_fill = true;
    let r = CornerRadius::same(7);
    for (w, fill, stroke) in [
        (&mut v.widgets.noninteractive, CARD, BORDER),
        (&mut v.widgets.inactive, RAISED, BORDER),
        (&mut v.widgets.hovered, Color32::from_rgb(44, 50, 66), Color32::from_rgb(70, 80, 104)),
        (&mut v.widgets.active, ACCENT, ACCENT),
        (&mut v.widgets.open, RAISED, ACCENT),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0_f32, stroke);
        w.corner_radius = r;
        w.expansion = 0.0;
        w.fg_stroke = Stroke::new(1.0_f32, TEXT);
    }
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 10.0);
        s.spacing.button_padding = egui::vec2(14.0, 7.0);
        s.spacing.interact_size = egui::vec2(72.0, 30.0);
        s.spacing.slider_width = 300.0;
        for (t, size) in [
            (egui::TextStyle::Body, 15.0),
            (egui::TextStyle::Button, 15.0),
            (egui::TextStyle::Heading, 21.0),
            (egui::TextStyle::Small, 12.5),
            (egui::TextStyle::Monospace, 13.0),
        ] {
            if let Some(f) = s.text_styles.get_mut(&t) {
                f.size = size;
            }
        }
    });
}

/// A rounded panel with a small caption.
fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::default()
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title.to_uppercase()).size(12.0).strong().extra_letter_spacing(1.2).color(MUTED));
            ui.add_space(4.0);
            add(ui)
        })
        .inner
}

fn hint(ui: &mut egui::Ui, s: &str) {
    ui.label(RichText::new(s).size(12.5).color(MUTED));
}

/// A row of options that act as one control.
fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, opts: &[(T, &str)]) {
    egui::Frame::default().fill(FIELD).corner_radius(9).inner_margin(egui::Margin::same(3)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for (v, l) in opts {
                let on = *value == *v;
                let b = egui::Button::new(RichText::new(*l).color(if on { Color32::WHITE } else { MUTED }).strong())
                    .fill(if on { ACCENT } else { Color32::TRANSPARENT })
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(7)
                    .min_size(egui::vec2(104.0, 30.0));
                if ui.add(b).clicked() {
                    *value = *v;
                }
            }
        });
    });
}

/// Pills for excluding names from a list: an excluded one is red and struck through.
fn chips(ui: &mut egui::Ui, list: &[(&str, &str)], off: &mut Vec<String>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for (n, l) in list {
            let excluded = off.iter().any(|m| m == n);
            let text = if excluded {
                RichText::new(format!("🗙  {l}")).color(Color32::from_rgb(255, 170, 170)).strikethrough()
            } else {
                RichText::new(format!("✔  {l}")).color(TEXT)
            };
            let b = egui::Button::new(text)
                .fill(if excluded { Color32::from_rgb(70, 30, 36) } else { RAISED })
                .stroke(egui::Stroke::new(1.0_f32, if excluded { Color32::from_rgb(120, 50, 58) } else { BORDER }))
                .corner_radius(20);
            if ui.add(b).on_hover_text(if excluded { "Excluded: click to allow" } else { "Allowed: click to exclude" }).clicked() {
                off.retain(|m| m != n);
                if !excluded {
                    off.push(n.to_string());
                }
            }
        }
    });
}

/// A label with a muted explanation under it.
fn field_label(ui: &mut egui::Ui, label: &str, help: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.label(RichText::new(label).color(TEXT));
        if !help.is_empty() {
            ui.label(RichText::new(help).size(12.0).color(MUTED));
        }
    });
}

fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::default().fill(color.gamma_multiply(0.18)).corner_radius(20).inner_margin(egui::Margin::symmetric(12, 5)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 4.5, color);
            ui.label(RichText::new(text).color(color).strong());
        });
    });
}

fn big_button(text: &str, fill: Color32) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).size(17.0).strong().color(Color32::WHITE))
        .fill(fill)
        .stroke(egui::Stroke::NONE)
        .corner_radius(10)
        .min_size(egui::vec2(140.0, 42.0))
}

impl App {
    fn connection(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.setup;
        card(ui, "Backend config", |ui| {
            hint(ui, "Paste your EchoRelay or Nakama config.json. Game servers register with its serverdb_host.");
            ui.add(
                egui::TextEdit::multiline(&mut s.config_json)
                    .code_editor()
                    .desired_rows(7)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Margin::same(10))
                    .hint_text("{\n  \"serverdb_host\": \"ws://…/serverdb?…\",\n  …\n}"),
            );
            match s.parse() {
                Parsed::Empty => {}
                Parsed::Bad(e) => {
                    ui.label(RichText::new(format!("🗙  {e}")).color(RED));
                }
                Parsed::Ok(u) => {
                    ui.label(RichText::new(format!("✔  {}", redact(&u))).color(GREEN).size(13.0));
                }
            }
        });
        ui.add_space(12.0);
        card(ui, "Network", |ui| {
            egui::Grid::new("net").num_columns(2).spacing([24.0, 12.0]).min_col_width(170.0).show(ui, |ui| {
                field_label(ui, "First UDP port", &format!("Uses UDP {}–{}", s.udp_port, s.udp_last()));
                ui.add(egui::DragValue::new(&mut s.udp_port).range(1..=65535));
                ui.end_row();
                field_label(ui, "Region", "");
                ui.add(egui::TextEdit::singleline(&mut s.region).desired_width(150.0));
                ui.end_row();
                field_label(ui, "Public IP", "Leave empty to detect");
                ui.add(egui::TextEdit::singleline(&mut s.public_ip).desired_width(150.0).hint_text("auto"));
                ui.end_row();
                field_label(ui, "Open ports automatically", "Asks the router (UPnP)");
                ui.checkbox(&mut s.upnp, "");
                ui.end_row();
            });
            hint(ui, "Needs UPnP on in the router. It can't work behind carrier-grade NAT (Starlink, most mobile): the server log says when that's the case.");
        });
    }

    fn games(&mut self, ui: &mut egui::Ui, live: bool) {
        let hz_before = self.setup.frame_hz;
        let servers_before = (self.setup.max_games, self.setup.keep_running);
        let s = &mut self.setup;
        card(ui, "Game servers", |ui| {
            egui::Grid::new("games").num_columns(2).spacing([24.0, 12.0]).min_col_width(170.0).show(ui, |ui| {
                field_label(ui, "Max at once", "Most game servers running together");
                ui.add(egui::DragValue::new(&mut s.max_games).range(1..=128));
                ui.end_row();
                field_label(ui, "Kept running", "Waiting for players, even when empty");
                let max = s.max_games;
                ui.add(egui::DragValue::new(&mut s.keep_running).range(0..=max));
                ui.end_row();
            });
            if live {
                hint(ui, "Raising these starts more game servers at once. Lowering them stops none that already run; that takes a restart.");
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Frame rate").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{} Hz", s.frame_hz)).size(20.0).strong().color(ACCENT));
                });
            });
            ui.add(egui::Slider::new(&mut s.frame_hz, 10..=240).show_value(false));
            hint(ui, "How often each game updates the disc, players and rules. 20 Hz is the default; native public servers run 120, comp servers 180. Higher is smoother but costs more CPU and bandwidth. Changes apply to running games at once.");
            ui.add_space(8.0);
            ui.add_enabled_ui(!live, |ui| {
                ui.checkbox(&mut s.echocombat, RichText::new("EchoCombat").strong());
                hint(ui, "Answers the EchoCombat mod. Off, players with the mod get stock Combat: its new gear is hidden.");
                ui.add_space(8.0);
                ui.checkbox(&mut s.debug_log, RichText::new("Debug logging").strong());
                hint(ui, "Logs every grab, ping and packet summary. Only for tracking down a problem: it is the heaviest work the server does and fills the log fast. Takes effect at the next start.");
                ui.add_space(8.0);
                ui.checkbox(&mut s.event_drive, RichText::new("Faster player updates (experimental)").strong());
                hint(ui, "Sends other players' movement as it arrives instead of waiting for the next game frame: about half the delay (2.4 ms instead of 4.5 ms on this server), and it arrives more evenly. Each player gets about 40% more packets. Not yet tried with full lobbies of real players. Takes effect at the next start.");
            });
        });
        ui.add_space(12.0);
        ui.add_enabled_ui(!live, |ui| card(ui, "Cheat checks", |ui| {
            segmented(ui, &mut s.cheat_checks, &[(CheatChecks::Off, "Off"), (CheatChecks::Log, "Log"), (CheatChecks::Enforce, "Enforce")]);
            hint(ui, match s.cheat_checks {
                CheatChecks::Off => "Clients' shots, hits and movement aren't checked.",
                CheatChecks::Log => "Impossible damage, fire rate, hits far from their target, hits through walls, speed over 100 m/s and disc throws over 18.95 m/s are written to the server log. Nothing is changed.",
                CheatChecks::Enforce => "As Log, and impossible damage is capped, shots over the fire rate are dropped and hits far from their target do nothing. Hits through walls, speed and throws are only logged.",
            });
        }));
        ui.add_space(12.0);
        ui.add_enabled_ui(!live, |ui| card(ui, "Game modes", |ui| {
            segmented(ui, &mut s.mode_choice, &[(ModeChoice::All, "All"), (ModeChoice::OnlyOne, "Only one"), (ModeChoice::Exclude, "Exclude")]);
            match s.mode_choice {
                ModeChoice::All => hint(ui, "Every mode can be hosted."),
                ModeChoice::OnlyOne => {
                    let label = MODES.iter().find(|(n, _)| *n == s.only_mode).map_or("Pick a mode", |(_, l)| *l);
                    egui::ComboBox::from_id_salt("only").selected_text(label).width(240.0).show_ui(ui, |ui| {
                        for (n, l) in MODES {
                            ui.selectable_value(&mut s.only_mode, n.to_string(), *l);
                        }
                    });
                }
                ModeChoice::Exclude => chips(ui, MODES, &mut s.excluded_modes),
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Maps").strong());
            chips(ui, MAPS, &mut s.excluded_maps);
            hint(ui, "Click a map to exclude it.");
        }));
        // A running server switches its games to the new rate at once.
        if self.setup.frame_hz != hz_before {
            self.send_frame_hz(self.setup.frame_hz);
        }
        // And starts more game servers when the numbers go up.
        self.setup.keep_running = self.setup.keep_running.min(self.setup.max_games);
        if (self.setup.max_games, self.setup.keep_running) != servers_before {
            self.send_line(&format!("servers {} {}", self.setup.max_games.max(1), self.setup.keep_running));
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, live: bool, problems: &[String]) {
        let found = find_server().is_some();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new("Echo Server Launcher").size(22.0).strong());
                ui.label(RichText::new("Host Echo VR game servers for EchoRelay or Nakama").color(MUTED));
            });
            ui.add_space(16.0);
            let n = settings::changed_count(&self.setup.balance);
            let label = if n > 0 { format!("Settings ({n})") } else { "Settings".into() };
            segmented(ui, &mut self.page, &[(Page::Server, "Server"), (Page::Settings, label.as_str())]);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if live {
                    if ui.add(big_button("⏹  Stop", Color32::from_rgb(200, 70, 75))).clicked() {
                        self.stop();
                    }
                    pill(ui, "Running", GREEN);
                    if ui.button("Hide to tray").on_hover_text("The servers keep running; click the tray icon to come back.").clicked() {
                        self.hide_to_tray(ui.ctx());
                    }
                } else {
                    let ready = problems.is_empty() && found;
                    if ui.add_enabled(ready, big_button("▶  Start", if ready { ACCENT } else { RAISED })).clicked() {
                        self.start();
                    }
                    pill(ui, "Stopped", MUTED);
                }
            });
        });
        let mut notes: Vec<String> = problems.to_vec();
        if !found {
            notes.push(format!("{} not found next to the launcher.", server_exe_name()));
        }
        if !live && !notes.is_empty() {
            ui.add_space(6.0);
            for n in notes {
                ui.label(RichText::new(format!("⚠  {n}")).color(AMBER));
            }
        } else if !self.status.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(&self.status).color(MUTED));
        }
        self.update_row(ui, live);
    }
}

impl App {
    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        if self.tray.is_none() {
            match tray::create(self.bridge.clone()) {
                Ok(t) => self.tray = Some(t),
                Err(e) => {
                    self.status = format!("No system tray available ({e}); minimised instead.");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    return;
                }
            }
        }
        self.hidden = true;
        self.shared.hidden.store(true, Ordering::Relaxed);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        // Where a window can't be hidden (Wayland), at least minimise it.
        #[cfg(target_os = "linux")]
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }

    fn log_view(&mut self, ui: &mut egui::Ui) {
        // Only the rows on screen are laid out.
        let log = self.log.lock().expect("log");
        let row = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show_rows(ui, row, log.len(), |ui, range| {
            for l in log.range(range) {
                let c = if l.contains("ERROR") { RED } else if l.contains("WARN") { AMBER } else { Color32::from_rgb(170, 176, 192) };
                ui.label(RichText::new(l).monospace().color(c));
            }
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.bridge.take_quit() {
            self.stop();
            self.save();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.bridge.take_show() {
            self.hidden = false;
            self.shared.hidden.store(false, Ordering::Relaxed);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        // Restarted by an update: start the servers again, back to the tray.
        if std::mem::take(&mut self.autostart) && self.setup.problems().is_empty() {
            self.start();
        }
        if std::mem::take(&mut self.autohide) {
            self.hide_to_tray(ctx);
        }
        self.reap();
        // Brought back some other way than the tray (Wayland only minimises,
        // and the taskbar restores it): it is shown again, and its log
        // redraws again.
        if self.hidden && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true)))) {
            self.hidden = false;
            self.shared.hidden.store(false, Ordering::Relaxed);
        }
        let live = self.live();
        let problems = self.setup.problems();

        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::default().fill(CARD).inner_margin(egui::Margin::symmetric(24, 16)).stroke(egui::Stroke::new(1.0_f32, BORDER)))
            .show(ctx, |ui| self.header(ui, live, &problems));

        if self.shared.running.lock().expect("running").is_some() {
            egui::TopBottomPanel::bottom("log")
                .resizable(true)
                .default_height(220.0)
                .min_height(80.0)
                .frame(egui::Frame::default().fill(FIELD).inner_margin(egui::Margin::symmetric(24, 12)).stroke(egui::Stroke::new(1.0_f32, BORDER)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("SERVER LOG").size(12.0).strong().extra_letter_spacing(1.2).color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Clear").clicked() {
                                self.log.lock().expect("log").clear();
                            }
                        });
                    });
                    self.log_view(ui);
                });
        }

        egui::CentralPanel::default().frame(egui::Frame::default().fill(BG).inner_margin(egui::Margin::same(20))).show(ctx, |ui| {
            // Each page keeps its own scroll position.
            let scroll = if self.page == Page::Settings { "settings-scroll" } else { "server-scroll" };
            egui::ScrollArea::vertical().id_salt(scroll).auto_shrink([false, false]).show(ui, |ui| {
                // Locked while the server runs, except the frame rate (it
                // changes running games at once).
                if self.page == Page::Settings {
                    ui.add_enabled_ui(!live, |ui| settings::page(ui, &mut self.settings, &mut self.setup.balance));
                } else if ui.available_width() >= 860.0 {
                    ui.columns(2, |c| {
                        c[0].add_enabled_ui(!live, |ui| self.connection(ui));
                        self.games(&mut c[1], live);
                    });
                } else {
                    ui.add_enabled_ui(!live, |ui| self.connection(ui));
                    ui.add_space(12.0);
                    self.games(ui, live);
                }
            });
        });

        // New output wakes the window; this only notices the server exiting.
        if live && !self.hidden {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    /// The mouse's raw motion (`MouseMoved`, from anywhere on the desktop
    /// while the window has focus) is dropped: every one asked for a redraw,
    /// so moving the mouse on any screen redrew the launcher nonstop (44% CPU
    /// on the software renderer). Pointer moves over the window still come.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        raw_input.events.retain(|e| !matches!(e, egui::Event::MouseMoved(_)));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop();
        self.save();
    }
}

fn options() -> eframe::NativeOptions {
    #[allow(unused_mut)]
    let mut viewport = egui::ViewportBuilder::default().with_title("Echo Server Launcher").with_inner_size([1060.0, 780.0]).with_min_inner_size([560.0, 480.0]);
    // Debug builds only: where the window opens (`x,y`), for testing.
    #[cfg(debug_assertions)]
    if let Some((x, y)) = std::env::var("ERS_LAUNCHER_POS").ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?))
    }) {
        viewport = viewport.with_position([x, y]);
    }
    eframe::NativeOptions { viewport, ..Default::default() }
}

fn run(options: eframe::NativeOptions) -> eframe::Result<()> {
    eframe::run_native("Echo Server Launcher", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

/// The launcher has no console on Windows, so failures are written next to
/// the exe and shown in a message box instead of vanishing.
fn report(msg: &str) {
    let path = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("launcher-error.log")));
    if let Some(p) = &path {
        let _ = std::fs::write(p, msg);
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (text, title) = (w(msg), w("Echo Server Launcher"));
        // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call.
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR) };
    }
    #[cfg(not(windows))]
    eprintln!("{msg}");
}

/// DirectX 12 with any adapter that can draw the window: a real GPU first,
/// else WARP (Windows' software renderer) or `only_software`.
#[cfg(windows)]
fn wgpu_options(only_software: bool) -> eframe::NativeOptions {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
    use eframe::wgpu;
    let mut setup = WgpuSetupCreateNew::default();
    setup.instance_descriptor.backends = wgpu::Backends::DX12;
    setup.native_adapter_selector = Some(std::sync::Arc::new(move |adapters: &[wgpu::Adapter], surface: Option<&wgpu::Surface<'_>>| {
        let usable = |a: &&wgpu::Adapter| surface.is_none_or(|s| a.is_surface_supported(s));
        let software = |a: &&wgpu::Adapter| a.get_info().device_type == wgpu::DeviceType::Cpu;
        let names = || adapters.iter().map(|a| a.get_info().name).collect::<Vec<_>>().join(", ");
        adapters
            .iter()
            .filter(usable)
            .filter(|a| !only_software || software(a))
            .min_by_key(|a| software(a))
            .cloned()
            .ok_or_else(|| format!("no DirectX 12 adapter can draw the window (found: [{}])", names()))
    }));
    eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: WgpuConfiguration { wgpu_setup: WgpuSetup::CreateNew(setup), ..Default::default() },
        ..options()
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| report(&format!("The launcher crashed:

{info}"))));
    // ERS_LAUNCHER_RENDERER=directx|software skips the earlier tries (for testing).
    let forced = std::env::var("ERS_LAUNCHER_RENDERER").unwrap_or_default();
    let mut errors = Vec::new();
    if forced.is_empty() {
        match run(eframe::NativeOptions { renderer: eframe::Renderer::Glow, ..options() }) {
            Ok(()) => return,
            Err(e) => errors.push(format!("OpenGL: {e}")),
        }
    }
    #[cfg(windows)]
    match run(wgpu_options(forced == "software")) {
        Ok(()) => return,
        Err(e) => errors.push(format!("DirectX 12: {e}")),
    }
    report(&format!("The launcher window could not start.

{}

Updating the graphics driver usually fixes this.", errors.join("
")));
}
