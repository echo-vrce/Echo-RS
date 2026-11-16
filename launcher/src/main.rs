//! Echo Server Launcher: a one-page front end for `echo-rust-server`.
//!
//! Paste the EchoRelay or Nakama config (`config.json` with
//! `serverdb_host`), set how many game servers run at once, the ports, the
//! frame rate and which modes and maps they host, then Start.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            config_json: String::new(),
            max_games: 16,
            keep_running: 2,
            udp_port: 6792,
            frame_hz: 20,
            mode_choice: ModeChoice::All,
            only_mode: "echo_arena".into(),
            excluded_modes: Vec::new(),
            excluded_maps: Vec::new(),
            region: "default".into(),
            public_ip: String::new(),
            echocombat: true,
            balance: serde_json::Map::new(),
            auto_update: true,
            debug_log: false,
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

fn pump(stream: impl Read + Send + 'static, log: Log, ctx: egui::Context) {
    std::thread::spawn(move || {
        let mut r = BufReader::new(stream);
        let mut buf = Vec::new();
        while matches!(r.read_until(b'\n', &mut buf), Ok(n) if n > 0) {
            push(&log, strip_ansi(String::from_utf8_lossy(&buf).trim_end()));
            buf.clear();
            // Redraw for new output, batched; nothing is drawn while hidden.
            ctx.request_repaint_after(Duration::from_millis(200));
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
    running: Option<Running>,
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
        update::clean_old(&exe_dir());
        let updater = update::Updater::default();
        if setup.auto_update {
            let ctx = cc.egui_ctx.clone();
            updater.check(exe_dir(), false, move || ctx.request_repaint());
        }
        Self { setup, running: None, log: Arc::default(), status: String::new(), ctx: cc.egui_ctx.clone(), bridge, tray: None, hidden: false, page: Page::Server, settings: settings::State::default(), updater }
    }

    /// The update line under the header: what the updater is doing, and its controls.
    fn update_row(&mut self, ui: &mut egui::Ui, live: bool) {
        use update::Status;
        // A downloaded release goes in as soon as the server is not running.
        if !live && matches!(self.updater.status(), Status::Ready(_)) {
            self.updater.install_pending(&exe_dir());
        }
        // (short text, colour, detail on hover)
        let (text, color, detail) = match self.updater.status() {
            Status::Idle => ("Not checked yet".to_string(), MUTED, None),
            Status::Checking => ("Checking for updates…".to_string(), MUTED, None),
            Status::UpToDate(t) => ("Up to date".to_string(), GREEN, Some(format!("Newest release on GitHub: {t}"))),
            Status::Downloading(t) => (format!("Downloading {t}…"), AMBER, None),
            Status::Ready(t) => (format!("{t} ready: installs when the server stops"), AMBER, None),
            Status::Installed { tag, restart: true } => (format!("Updated to {tag}: restart the launcher"), GREEN, None),
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
                        let check = egui::Button::new(RichText::new("Check now").size(12.5)).corner_radius(6).min_size(egui::vec2(0.0, 24.0));
                        if ui.add_enabled(!busy, check).on_hover_text("Look for a newer release on GitHub now").clicked() {
                            let ctx = self.ctx.clone();
                            self.updater.check(exe_dir(), true, move || ctx.request_repaint());
                        }
                        ui.add_space(6.0);
                        let auto = ui
                            .checkbox(&mut self.setup.auto_update, RichText::new("Auto-update").size(12.5).color(MUTED))
                            .on_hover_text("Check GitHub and update each time the launcher starts");
                        if auto.changed() {
                            self.save();
                        }
                    });
                });
            });
    }

    fn live(&self) -> bool {
        self.running.as_ref().is_some_and(|r| r.exited.is_none())
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
        let mut cmd = Command::new(&exe);
        cmd.args(&args)
            .current_dir(exe.parent().unwrap_or(Path::new(".")))
            .env("NO_COLOR", "1")
            // Debug lines (disc and touch decisions, pings, packet summaries)
            // only when asked for: they cost the server more than anything else.
            .env("ERS_LOG", std::env::var("ERS_LOG").unwrap_or_else(|_| if self.setup.debug_log { "info,echo_rust_server::engine::session=debug".into() } else { "info".into() }))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        match cmd.spawn() {
            Ok(mut child) => {
                if let Some(o) = child.stdout.take() {
                    pump(o, self.log.clone(), self.ctx.clone());
                }
                if let Some(e) = child.stderr.take() {
                    pump(e, self.log.clone(), self.ctx.clone());
                }
                self.running = Some(Running { child, exited: None });
                self.status = "Running.".into();
            }
            Err(e) => self.status = format!("Could not start: {e}"),
        }
    }

    fn stop(&mut self) {
        if let Some(r) = &mut self.running
            && r.exited.is_none()
        {
            let _ = r.child.kill();
            let _ = r.child.wait();
            r.exited = Some("stopped".into());
            push(&self.log, "— stopped —".into());
            self.status = "Stopped.".into();
        }
    }

    fn reap(&mut self) {
        if let Some(r) = &mut self.running
            && r.exited.is_none()
            && let Ok(Some(st)) = r.child.try_wait()
        {
            let msg = st.code().map_or("ended".into(), |c| format!("exited (code {c})"));
            push(&self.log, format!("— {msg} —"));
            self.status = format!("Server {msg}.");
            r.exited = Some(msg);
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
            });
        });
    }

    fn games(&mut self, ui: &mut egui::Ui) {
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
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Frame rate").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{} Hz", s.frame_hz)).size(20.0).strong().color(ACCENT));
                });
            });
            ui.add(egui::Slider::new(&mut s.frame_hz, 10..=200).show_value(false));
            hint(ui, "How often each game updates the disc, players and rules. 20 Hz is the default; higher is smoother but costs more CPU and bandwidth.");
            ui.add_space(8.0);
            ui.checkbox(&mut s.echocombat, RichText::new("EchoCombat").strong());
            hint(ui, "Answers the EchoCombat mod. Off, players with the mod get stock Combat: its new gear is hidden.");
            ui.add_space(8.0);
            ui.checkbox(&mut s.debug_log, RichText::new("Debug logging").strong());
            hint(ui, "Logs every grab, ping and packet summary. Only for tracking down a problem: it is the heaviest work the server does and fills the log fast. Takes effect at the next start.");
        });
        ui.add_space(12.0);
        card(ui, "Game modes", |ui| {
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
        });
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
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.reap();
        let live = self.live();
        let problems = self.setup.problems();

        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::default().fill(CARD).inner_margin(egui::Margin::symmetric(24, 16)).stroke(egui::Stroke::new(1.0_f32, BORDER)))
            .show(ctx, |ui| self.header(ui, live, &problems));

        if self.running.is_some() {
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
                ui.add_enabled_ui(!live, |ui| {
                    if self.page == Page::Settings {
                        settings::page(ui, &mut self.settings, &mut self.setup.balance);
                    } else if ui.available_width() >= 860.0 {
                        ui.columns(2, |c| {
                            self.connection(&mut c[0]);
                            self.games(&mut c[1]);
                        });
                    } else {
                        self.connection(ui);
                        ui.add_space(12.0);
                        self.games(ui);
                    }
                });
            });
        });

        // New output wakes the window; this only notices the server exiting.
        if live && !self.hidden {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop();
        self.save();
    }
}

fn options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Echo Server Launcher").with_inner_size([1060.0, 780.0]).with_min_inner_size([560.0, 480.0]),
        ..Default::default()
    }
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
