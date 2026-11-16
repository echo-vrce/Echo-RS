//! echo-rust-server: a standalone Echo VR server written entirely in Rust.
//!
//! One binary provides
//! * the central services the client talks to (login, config, matching,
//!   transaction, HTTP API), and
//! * a self-scaling pool of game servers running this crate's own engine
//!   (`engine`), so no copy of Echo VR is needed to host matches.
//!
//! The embedded game servers can instead serve an external nakama or EchoRelay
//! backend (`--backend nakama|echorelay --upstream <url>`).

mod decap;
mod engine;
mod lobby;
mod pool;
mod probe;
mod services;
#[allow(dead_code, reason = "full protocol surface; not every field is consumed")]
mod sns;
mod store;
mod upstream;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use dashmap::DashMap;
use tracing::{info, warn};

#[derive(Copy, Clone, Debug, ValueEnum)]
enum BackendKind {
    /// This server's own login/config/matching services.
    Builtin,
    /// Register game servers with a nakama (EchoVRCE) fleet.
    Nakama,
    /// Register game servers with an EchoRelay ServerDB.
    Echorelay,
}

#[derive(Parser, Debug)]
#[command(name = "echo-rust-server", version, about = "Standalone all-Rust Echo VR server (services + game servers)")]
struct Args {
    /// TCP address for the websocket services and HTTP API.
    #[arg(long, env = "ERS_BIND", default_value = "0.0.0.0:7777")]
    bind: SocketAddr,

    /// Directory for accounts and resource overrides.
    #[arg(long, env = "ERS_DATA_DIR", default_value = "server-data")]
    data_dir: PathBuf,

    /// Address clients on the LAN use to reach this machine (auto-detected).
    #[arg(long, env = "ERS_LAN_IP")]
    lan_ip: Option<Ipv4Addr>,

    /// Public address for players on the internet (defaults to the LAN IP).
    #[arg(long, env = "ERS_PUBLIC_IP")]
    public_ip: Option<Ipv4Addr>,

    /// First UDP port for game servers; instance N uses port + N.
    #[arg(long, env = "ERS_UDP_PORT", default_value_t = 6792)]
    udp_port: u16,

    /// Game servers kept idle and ready (Echo needs >= 2: lobby + match).
    #[arg(long, env = "ERS_MIN_IDLE", default_value_t = 2)]
    min_idle: usize,

    /// Upper bound on game server instances (one match or lobby each).
    #[arg(long, env = "ERS_MAX_INSTANCES", default_value_t = 64)]
    max_instances: usize,

    /// Seconds a surplus idle instance lives before it is stopped; 0 (the
    /// default) keeps every started instance registered as an idle server.
    #[arg(long, env = "ERS_IDLE_RETIRE_SECS", default_value_t = 0)]
    idle_retire_secs: u64,

    /// Arena: players needed on each team before the 45 s pre-match countdown
    /// starts. 0 (the default, like native public servers) starts it as soon
    /// as anyone joins.
    #[arg(long, env = "ERS_ARENA_MIN_PER_TEAM", default_value_t = 0)]
    arena_min_per_team: usize,

    /// Don't answer EchoCombat's VeilNet packets: players' clients then treat
    /// this as a stock server and hide the mod's new gear.
    #[arg(long, env = "ERS_NO_VEILNET")]
    no_veilnet: bool,

    /// Balance changes (the launcher's Settings page): JSON `{"<balance file
    /// name>": {partial values}}`, merged into the settings sent to players.
    #[arg(long, env = "ERS_BALANCE")]
    balance: Option<std::path::PathBuf>,

    /// Who the embedded game servers serve.
    #[arg(long, env = "ERS_BACKEND", value_enum, default_value_t = BackendKind::Builtin)]
    backend: BackendKind,

    /// nakama fleet URL or EchoRelay `serverdb_host` (with `?api_key=` if set).
    #[arg(long, env = "ERS_UPSTREAM")]
    upstream: Option<String>,

    /// EchoRelay login service (`loginservice_host`), where players' loadout
    /// changes are saved. Default: the ServerDB host's `/login`.
    #[arg(long, env = "ERS_LOGIN_URL")]
    login_url: Option<String>,

    /// Region symbol announced to EchoRelay.
    #[arg(long, env = "ERS_REGION", default_value = "default")]
    region: String,

    /// Give every account only the default cosmetics instead of all of them.
    #[arg(long, env = "ERS_NO_UNLOCK_ALL")]
    no_unlock_all: bool,

    /// Game frames per second (replication, the disc, match rules), 10 to
    /// 200. The client's own 120 Hz frame clock is separate and unchanged.
    #[arg(long, env = "ERS_FRAME_HZ", default_value_t = 20, value_parser = clap::value_parser!(u64).range(10..=200))]
    frame_hz: u64,

    /// Host only these game modes (comma-separated game-type names, e.g.
    /// echo_arena,echo_arena_private). Empty: every mode.
    #[arg(long, env = "ERS_ONLY_MODES", value_delimiter = ',')]
    only_modes: Vec<String>,

    /// Never host these game modes (comma-separated game-type names).
    #[arg(long, env = "ERS_EXCLUDE_MODES", value_delimiter = ',')]
    exclude_modes: Vec<String>,

    /// Never host these maps (comma-separated level names, e.g.
    /// mpl_combat_gauss).
    #[arg(long, env = "ERS_EXCLUDE_MAPS", value_delimiter = ',')]
    exclude_maps: Vec<String>,

    /// Print the known game-mode and map names (for the filters) and exit.
    #[arg(long)]
    list_modes: bool,

    /// Run the headless test client against a service base URL (e.g.
    /// ws://127.0.0.1:7777) instead of starting a server.
    #[arg(long, value_name = "WS_URL")]
    probe: Option<String>,

    /// Decrypt a `tools/record_native.py` capture directory into
    /// `<DIR>/msgs.csv` (one row per message) and exit.
    #[arg(long, value_name = "DIR")]
    decode_capture: Option<std::path::PathBuf>,

    /// Probe: number of concurrent clients.
    #[arg(long, default_value_t = 1)]
    probe_clients: usize,

    /// Probe: social, arena or combat.
    #[arg(long, default_value = "social")]
    probe_mode: String,

    /// Probe: the `auth` login parameter (account password).
    #[arg(long, default_value = "probe")]
    probe_auth: String,

    /// Probe: keep the clients connected this many seconds after joining.
    #[arg(long, default_value_t = 0)]
    probe_hold_secs: u64,

    /// Log filter (tracing EnvFilter syntax).
    #[arg(long, env = "ERS_LOG", default_value = "info")]
    log: String,
}

/// The outward-facing address of this host, found by asking the OS which
/// interface routes to a public address (no packet is sent).
fn detect_lan_ip() -> Option<Ipv4Addr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

fn client_config(host: Ipv4Addr, port: u16) -> serde_json::Value {
    let base = format!("{host}:{port}");
    serde_json::json!({
        "apiservice_host": format!("http://{base}/api"),
        "configservice_host": format!("ws://{base}/config"),
        "loginservice_host": format!("ws://{base}/login?auth=CHANGE_ME&displayname=CHANGE_ME"),
        "matchingservice_host": format!("ws://{base}/matching"),
        "transactionservice_host": format!("ws://{base}/transaction"),
        "publisher_lock": "rad15_live"
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    // Windows timers tick every 15.6 ms unless asked otherwise: the 120 Hz
    // server frame then ran in uneven bursts whenever few packets came in.
    #[cfg(windows)]
    {
        #[link(name = "winmm")]
        unsafe extern "system" {
            fn timeBeginPeriod(period: u32) -> u32;
        }
        // SAFETY: plain Win32 call, no pointers; the period lasts for the process.
        unsafe { timeBeginPeriod(1) };
    }
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&args.log)))
        .with_target(false)
        .init();
    engine::balance::set_base(engine::balance::Base { echocombat: !args.no_veilnet, medigun: engine::veilnet::medigun_from_file() });
    if let Some(path) = &args.balance
        && let Err(e) = engine::balance::load(path)
    {
        tracing::warn!(error = %e, "balance changes not loaded; using the stock settings");
    }

    if args.list_modes {
        println!("modes:");
        for (n, label) in engine::filter::MODES {
            println!("  {n:<24} {label}");
        }
        println!("maps:");
        for (n, label) in engine::filter::MAPS {
            println!("  {n:<24} {label}");
        }
        return Ok(());
    }
    engine::session::set_frame_hz(args.frame_hz);
    engine::filter::set(&args.only_modes, &args.exclude_modes, &args.exclude_maps).context("mode/map filter")?;

    if let Some(dir) = &args.decode_capture {
        return decap::run(dir, &dir.join("msgs.csv"));
    }

    if let Some(base) = args.probe.clone() {
        return probe::run(probe::ProbeArgs {
            base,
            clients: args.probe_clients.max(1),
            mode: args.probe_mode.clone(),
            auth: args.probe_auth.clone(),
            hold: Duration::from_secs(args.probe_hold_secs),
        })
        .await;
    }

    let backend = match (args.backend, args.upstream.clone()) {
        (BackendKind::Builtin, _) => upstream::Backend::Builtin,
        (BackendKind::Nakama, Some(url)) => upstream::Backend::Nakama { url },
        (BackendKind::Echorelay, Some(url)) => upstream::Backend::EchoRelay { url },
        (_, None) => bail!("--backend nakama/echorelay needs --upstream <url>"),
    };

    let lan_ip = args.lan_ip.or_else(detect_lan_ip).unwrap_or(Ipv4Addr::LOCALHOST);
    let public_ip = args.public_ip.unwrap_or(lan_ip);

    let store = store::Store::open(&args.data_dir, !args.no_unlock_all).context("opening data directory")?;
    let hub = lobby::Hub::new(store, lobby::Advertise { lan_ip, public_ip });
    let upstreams = upstream::Upstreams::new(backend.clone(), lan_ip, &args.region, args.login_url.clone());
    let login_url = upstreams.login_url.clone();
    let app = services::App { hub: hub.clone(), sessions: Arc::new(DashMap::new()), upstreams: upstreams.clone() };

    let listener = tokio::net::TcpListener::bind(args.bind).await.with_context(|| format!("binding {}", args.bind))?;
    let hub_port = listener.local_addr()?.port();
    let router = services::router(app).into_make_service_with_connect_info::<SocketAddr>();
    let http = tokio::spawn(async move { axum::serve(listener, router).await });

    let pool = pool::Pool::new(
        pool::PoolConfig {
            bind_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            first_udp_port: args.udp_port,
            min_idle: args.min_idle,
            max_instances: args.max_instances.max(args.min_idle),
            idle_retire: Duration::from_secs(args.idle_retire_secs),
            engine: engine::EngineConfig { arena_min_per_team: args.arena_min_per_team, veilnet: !args.no_veilnet, medigun: engine::veilnet::medigun_from_file() },
            hub_port,
        },
        hub.clone(),
        upstreams,
    );
    let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
    *hub.scale_tx.lock() = Some(wake_tx.clone());
    let _ = wake_tx.send(());
    let pool_task = tokio::spawn(pool.clone().run(wake_rx));

    let cfg = client_config(public_ip, hub_port);
    let cfg_text = serde_json::to_string_pretty(&cfg)?;
    let cfg_path = args.data_dir.join("client-config.json");
    if let Err(e) = std::fs::write(&cfg_path, &cfg_text) {
        warn!(error = %e, "could not write client config");
    }
    info!(
        bind = %args.bind, lan_ip = %lan_ip, public_ip = %public_ip, udp_ports = %format!("{}+", args.udp_port),
        backend = ?args.backend, frame_hz = args.frame_hz,
        only_modes = ?args.only_modes, exclude_modes = ?args.exclude_modes, exclude_maps = ?args.exclude_maps,
        "echo-rust-server running"
    );
    match &backend {
        upstream::Backend::Builtin => info!(
            "client config (save as ready-at-dawn-echo-arena\\_local\\config.json; also written to {}):\n{cfg_text}",
            cfg_path.display()
        ),
        upstream::Backend::EchoRelay { url } | upstream::Backend::Nakama { url } => {
            // The query holds credentials; log where, not how.
            let host = url.split('?').next().unwrap_or(url);
            info!(serverdb = host, login = login_url.as_deref().map(|u| u.split('?').next().unwrap_or(u)), "game servers will register with the external backend");
        }
    }

    tokio::select! {
        r = http => { r??; }
        _ = pool_task => {}
        _ = tokio::signal::ctrl_c() => info!("shutting down"),
    }
    pool.shutdown().await;
    Ok(())
}
