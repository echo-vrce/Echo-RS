//! Headless test client: drives the same websocket flow as the Echo VR
//! client (login → profile → config → documents → matchmaking) against any
//! compatible service (this server, EchoRelay, nakama) and reports the game
//! server it was sent to. Many clients can run at once for load testing.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::sns::{self, Reader, SessionSuccess, Writer, XpId, sym, symbol};

/// A service socket plus messages already received but not yet consumed
/// (servers batch several messages into one frame).
struct Ws {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    pending: std::collections::VecDeque<(u64, Vec<u8>)>,
    /// Every message type received, for diagnostics.
    seen: Vec<u64>,
}

const VERSION_LOCK: u64 = 0xc62f01d78f77910d;
const APP_ID: &str = "1369078409873402";

#[derive(Clone, Debug)]
pub struct ProbeArgs {
    /// Service base, e.g. `ws://127.0.0.1:7777`.
    pub base: String,
    pub clients: usize,
    /// `social`, `arena` or `combat`.
    pub mode: String,
    pub auth: String,
    /// Keep the sockets (and so the lobby seats) open this long afterwards.
    pub hold: Duration,
    /// Join the game server over UDP and play while holding.
    pub udp: bool,
    /// The simulated network the UDP clients play over.
    pub net: NetSim,
}

pub struct Joined {
    pub client: usize,
    pub xpid: XpId,
    pub elapsed: Duration,
    pub success: SessionSuccess,
    pub entrants_reply: bool,
    sockets: Vec<Ws>,
}

pub async fn run(args: ProbeArgs) -> Result<()> {
    let base_account = 9_000_000_000 + u64::from(rand::random::<u32>() % 1_000_000) * 1000;
    let started = Instant::now();
    let tasks: Vec<_> = (0..args.clients)
        .map(|i| {
            let args = args.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(15 * i as u64)).await;
                let r = client(&args, i, XpId { platform: 7, account: base_account + i as u64 }).await;
                (i, r)
            })
        })
        .collect();
    let mut ok = Vec::new();
    let mut failed = 0;
    for t in tasks {
        match t.await? {
            (_, Ok(j)) => {
                println!(
                    "client {:>3} {}  lobby {}  server {}  team {}  {} ms{}",
                    j.client,
                    j.xpid.token(),
                    j.success.lobby_id,
                    j.success.endpoint_string(),
                    j.success.team,
                    j.elapsed.as_millis(),
                    if j.entrants_reply { "" } else { "  (no player-sessions reply)" }
                );
                ok.push(j);
            }
            (i, Err(e)) => {
                failed += 1;
                println!("client {i:>3} FAILED: {e:#}");
            }
        }
    }
    let mut lobbies: Vec<_> = ok.iter().map(|j| (j.success.lobby_id, j.success.port)).collect();
    lobbies.sort();
    lobbies.dedup();
    println!(
        "\n{} of {} clients placed in {} lobbies in {} ms ({} failed)",
        ok.len(),
        args.clients,
        lobbies.len(),
        started.elapsed().as_millis(),
        failed
    );
    for (id, port) in &lobbies {
        let n = ok.iter().filter(|j| j.success.lobby_id == *id).count();
        println!("  lobby {id} on UDP {port}: {n} players");
    }
    if let Some(j) = ok.first() {
        let s = &j.success;
        let h = hex_encode;
        println!(
            "\nUDP join check (arguments for an external test client):\n  --server {}:{} --server-enc-key {} --server-mac-key {} --server-random-key {} \
             --client-enc-key {} --client-mac-key {} --client-random-key {} --server-seq {} --client-seq {} --requested-team {}",
            s.internal_ip,
            s.port,
            h(&s.server_enc),
            h(&s.server_mac),
            h(&s.server_rnd),
            h(&s.client_enc),
            h(&s.client_mac),
            h(&s.client_rnd),
            // The client uses 63-bit sequence ids; issuers may set the top bit.
            s.server_seq & sns::SEQ_MASK,
            s.client_seq & sns::SEQ_MASK,
            s.team.max(0)
        );
    }
    if args.udp && !ok.is_empty() {
        println!("\njoining over UDP and playing for {} s ...", args.hold.as_secs());
        let plays: Vec<_> = ok
            .iter()
            .map(|j| {
                let (client, s, hold, xpid, net) = (j.client, j.success.clone(), args.hold, j.xpid, args.net);
                tokio::spawn(async move { (client, udp_play(&s, xpid, client as u16, hold, net).await) })
            })
            .collect();
        let mut all = UdpReport::default();
        let mut played = 0;
        for p in plays {
            match p.await? {
                (i, Ok(r)) => {
                    println!(
                        "client {i:>3} udp: accepted {}  complete {}  rx {:.0} pkt/s  reliable {}  mailbox {} ({} relayed player states)  resends {}",
                        r.accepted,
                        r.complete,
                        r.packets as f64 / r.secs,
                        r.reliable,
                        r.mailbox,
                        r.relayed,
                        r.resends
                    );
                    if !r.complete {
                        failed += 1;
                    }
                    played += 1;
                    all.secs += r.secs;
                    all.packets += r.packets;
                    all.bytes += r.bytes;
                    all.lost_tx += r.lost_tx;
                    all.lost_rx += r.lost_rx;
                    all.ages.extend(r.ages);
                    all.gaps.extend(r.gaps);
                }
                (i, Err(e)) => {
                    failed += 1;
                    println!("client {i:>3} udp FAILED: {e:#}");
                }
            }
        }
        if played > 0 {
            udp_summary(played, all);
        }
    } else if !args.hold.is_zero() && !ok.is_empty() {
        println!("\nholding {} sockets for {} s ...", ok.len(), args.hold.as_secs());
        tokio::time::sleep(args.hold).await;
    }
    for j in ok.iter_mut() {
        for ws in j.sockets.iter_mut() {
            let _ = ws.ws.close(None).await;
        }
    }
    if failed > 0 {
        bail!("{failed} probe clients failed");
    }
    Ok(())
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

async fn connect(base: &str, path: &str) -> Result<Ws> {
    let url = format!("{}/{}", base.trim_end_matches('/'), path);
    let (ws, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(&url))
        .await
        .with_context(|| format!("connecting {url}: timed out"))?
        .with_context(|| format!("connecting {url}"))?;
    Ok(Ws { ws, pending: Default::default(), seen: Vec::new() })
}

async fn send(ws: &mut Ws, w: Writer, symbol: u64) -> Result<()> {
    let mut out = Vec::new();
    w.finish(symbol, &mut out);
    ws.ws.send(Message::Binary(out)).await?;
    Ok(())
}

/// Read until one of `want` arrives; other messages are skipped.
async fn expect(ws: &mut Ws, want: &[u64], secs: u64, what: &str) -> Result<(u64, Vec<u8>)> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(pos) = ws.pending.iter().position(|(s, _)| want.contains(s)) {
            // Messages before the match are ones the caller is not interested in.
            ws.pending.drain(..pos);
            return Ok(ws.pending.pop_front().expect("found"));
        }
        ws.pending.clear();
        let left = deadline.saturating_duration_since(Instant::now());
        let msg = tokio::time::timeout(left, ws.ws.next())
            .await
            .map_err(|_| anyhow::anyhow!("waiting for {what}: timed out; received so far: {}", ws.seen.iter().map(|s| format!("{s:#018x}")).collect::<Vec<_>>().join(" ")))?
            .with_context(|| format!("waiting for {what}: socket closed"))??;
        let data = match msg {
            Message::Binary(b) => b,
            Message::Close(c) => bail!("waiting for {what}: closed by server ({c:?})"),
            _ => continue,
        };
        for (s, p) in sns::split(&data)? {
            ws.seen.push(s);
            ws.pending.push_back((s, p.to_vec()));
        }
    }
}

fn failure_text(p: &[u8], skip: usize) -> String {
    let mut r = Reader::new(p);
    let _ = r.skip(skip);
    r.cstr().unwrap_or_default()
}

async fn client(args: &ProbeArgs, i: usize, xpid: XpId) -> Result<Joined> {
    let t0 = Instant::now();
    let name = format!("Probe{i}");
    let mut login = connect(&args.base, &format!("login?auth={}&displayname={name}", args.auth)).await?;

    // Login.
    let mut w = Writer::new();
    w.guid(Uuid::nil()).xpid(xpid).json_nt(
        &serde_json::to_vec(&json!({
            "accountid": xpid.account, "displayname": name, "bypassauth": false, "access_token": "", "nonce": "",
            "buildversion": 631547, "lobbyversion": 0, "appid": APP_ID.parse::<u64>()?, "publisher_lock": "rad15_live",
            "hmdserialnumber": "PROBE", "desiredclientprofileversion": 0,
            "system_info": {"headset_type": "No VR", "driver_version": "", "network_type": "Unknown", "video_card": "",
                            "cpu": "", "num_physical_cores": 0, "num_logical_cores": 0, "memory_total": 0, "memory_used": 0, "dedicated_gpu_memory": 0}
        }))?,
    );
    send(&mut login, w, sym::LOGIN_REQUEST).await?;
    let (s, p) = expect(&mut login, &[sym::LOGIN_SUCCESS, sym::LOGIN_FAILURE], 10, "login reply").await?;
    if s == sym::LOGIN_FAILURE {
        bail!("login rejected: {}", failure_text(&p, 24));
    }
    let mut r = Reader::new(&p);
    let session = r.guid()?;
    expect(&mut login, &[sym::GAME_SETTINGS], 5, "game settings").await?;

    // Profile.
    let mut w = Writer::new();
    w.guid(session).xpid(xpid).json_nt(br#"{"defaultclientprofileid":"default","defaultserverprofileid":"default","unlocksetids":{"all":"rad15_live"},"statgroupids":{"arena":"rad15_live","combat":"rad15_live"}}"#);
    send(&mut login, w, sym::LOGGED_IN_PROFILE_REQUEST).await?;
    let (s, _) = expect(&mut login, &[sym::LOGGED_IN_PROFILE_SUCCESS, sym::LOGGED_IN_PROFILE_FAILURE], 10, "profile").await?;
    if s != sym::LOGGED_IN_PROFILE_SUCCESS {
        bail!("profile request failed");
    }

    // Main menu config, EULA, channels (what the client fetches before the menu).
    let mut config = connect(&args.base, "config").await?;
    let mut w = Writer::new();
    w.u8(0).json_nt(br#"{"type":"main_menu","id":"main_menu"}"#);
    send(&mut config, w, sym::CONFIG_REQUEST).await?;
    let (s, _) = expect(&mut config, &[sym::CONFIG_SUCCESS, sym::CONFIG_FAILURE], 10, "main_menu config").await?;
    if s != sym::CONFIG_SUCCESS {
        bail!("main_menu config failed");
    }
    let mut w = Writer::new();
    w.cstr("en").cstr("eula");
    send(&mut login, w, sym::DOCUMENT_REQUEST).await?;
    expect(&mut login, &[sym::DOCUMENT_SUCCESS], 10, "eula document").await?;
    let mut w = Writer::new();
    w.u8(0);
    send(&mut login, w, sym::CHANNEL_INFO_REQUEST).await?;
    let (_, p) = expect(&mut login, &[sym::CHANNEL_INFO_RESPONSE], 10, "channel info").await?;
    // Like the client, matchmake in the first social channel the server lists.
    let channel = sns::unzlib_json(&mut Reader::new(&p))
        .ok()
        .and_then(|v| v["group"][0]["channeluuid"].as_str().and_then(|s| s.parse::<Uuid>().ok()))
        .unwrap_or_else(Uuid::new_v4);

    // Matchmaking.
    let (mode, level, team_flag, role) = match args.mode.as_str() {
        "arena" => ("echo_arena", "mpl_arena_a", 1u32, -1i8),
        "combat" => ("echo_combat", "mpl_combat_dyson", 1, -1),
        _ => ("social_2.0", "mpl_lobby_b2", 0, -1),
    };
    let mut matching = connect(&args.base, "matching").await?;
    let mut w = Writer::new();
    w.u64(VERSION_LOCK).u64(symbol(mode)).u64(symbol(level)).u64(symbol("DMO")).guid(session).u8(1).u32(team_flag).zeros(3);
    w.guid(Uuid::nil()).guid(channel);
    w.json_nt(&serde_json::to_vec(&json!({"appid": APP_ID, "gametype": symbol(mode) as i64, "level": symbol(level) as i64}))?);
    w.xpid(xpid);
    if team_flag != 0 {
        w.u8(role as u8);
    }
    send(&mut matching, w, sym::FIND_SESSION).await?;
    let (s, p) = loop {
        let (s, p) = expect(
            &mut matching,
            // EchoRelay sends v4 and v5 back to back; the final build reads v5.
            &[sym::SESSION_SUCCESS_V5, sym::SESSION_FAILURE_V4, sym::PING_REQUEST],
            45,
            "lobby session",
        )
        .await?;
        if s != sym::PING_REQUEST {
            break (s, p);
        }
        // EchoRelay asks the client to ping candidate servers first:
        // u16, u16, u32, then [ipv4 BE internal, ipv4 BE external, u16 BE port, 2 pad].
        // Answer u64 count, then [internal, external, u32 ms] per address pair
        // (results carry no port, and EchoRelay keys them by address pair).
        let mut endpoints: Vec<&[u8]> = p.get(8..).unwrap_or_default().chunks_exact(12).map(|e| &e[0..8]).collect();
        endpoints.sort();
        endpoints.dedup();
        let mut w = Writer::new();
        w.u64(endpoints.len() as u64);
        for e in endpoints {
            w.bytes(e).u32(1);
        }
        send(&mut matching, w, sym::PING_RESPONSE).await?;
    };
    match s {
        sym::SESSION_FAILURE_V4 => {
            let mut r = Reader::new(&p);
            r.skip(24)?;
            let code = r.u32()?;
            bail!("matchmaking failed (code {code}): {}", failure_text(&p, 32));
        }
        _ => {}
    }
    let success = sns::decode_session_success_v5(&p)?;
    let elapsed = t0.elapsed();

    // The client asks who is in the lobby right after joining.
    let mut w = Writer::new();
    w.guid(session).xpid(xpid).guid(success.lobby_id).u64(symbol("DMO")).u64(1).xpid(xpid);
    send(&mut matching, w, sym::PLAYER_SESSIONS_REQUEST).await?;
    let entrants_reply = expect(&mut matching, &[sym::ENTRANTS_V3], 5, "entrants").await.is_ok();

    Ok(Joined { client: i, xpid, elapsed, success, entrants_reply, sockets: vec![login, config, matching] })
}

/// Simulated network for `--probe-udp`: packets lost each way, and a delay
/// with random jitter on what the clients send (stands for the whole round
/// trip, so the server's ping shows it).
#[derive(Clone, Copy, Debug, Default)]
pub struct NetSim {
    pub loss_pct: f64,
    pub delay: Duration,
    pub jitter: Duration,
}

/// What one probe client saw while playing over UDP.
#[derive(Default)]
struct UdpReport {
    accepted: bool,
    complete: bool,
    secs: f64,
    packets: u64,
    /// UDP payload bytes received (IP and UDP add 28 per packet).
    bytes: u64,
    reliable: u64,
    mailbox: u64,
    relayed: u64,
    resends: u64,
    /// The simulated network's losses: sent, received.
    lost_tx: u64,
    lost_rx: u64,
    /// How old the other players' states were when they got here (µs since
    /// their client sent them).
    ages: Vec<u32>,
    /// Time between two states from the same player arriving (µs): what
    /// the game smooths their movement over.
    gaps: Vec<u32>,
}

/// Another player's state this long after the last one is a visible hitch:
/// one of their 72 Hz frames (13.9 ms) went missing or came late.
const HITCH_US: u32 = 25_000;

/// One clock for every probe client (they run in this process), so a
/// state's age is exact: client to server to client.
fn stamp_us() -> u64 {
    static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    BASE.get_or_init(Instant::now).elapsed().as_micros() as u64
}

fn pct(v: &[u32], q: usize) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    f64::from(v[(v.len() - 1) * q / 100]) / 1000.0
}

/// Every client's view together: what the players would see.
fn udp_summary(clients: usize, mut all: UdpReport) {
    all.ages.sort_unstable();
    all.gaps.sort_unstable();
    let hitches = all.gaps.iter().filter(|g| **g > HITCH_US).count();
    println!(
        "\nplayers' view ({clients} clients): rx {:.0} pkt/s, {:.1} kbit/s each (with IP and UDP headers), {} states from others",
        all.packets as f64 / all.secs.max(1e-3),
        (all.bytes + 28 * all.packets) as f64 * 8.0 / 1000.0 / all.secs.max(1e-3),
        all.ages.len()
    );
    println!(
        "  state age (sent by a player -> here)  p50 {:.2}  p95 {:.2}  p99 {:.2}  max {:.2} ms",
        pct(&all.ages, 50),
        pct(&all.ages, 95),
        pct(&all.ages, 99),
        pct(&all.ages, 100)
    );
    println!(
        "  gap between one player's states       p50 {:.2}  p95 {:.2}  p99 {:.2}  max {:.2} ms  hitches (> {} ms) {:.2}%",
        pct(&all.gaps, 50),
        pct(&all.gaps, 95),
        pct(&all.gaps, 99),
        pct(&all.gaps, 100),
        HITCH_US / 1000,
        100.0 * hitches as f64 / all.gaps.len().max(1) as f64
    );
    // How unevenly they arrive: the mean distance from the senders' own
    // frame time (13.9 ms at 72 Hz), and from the median gap.
    let mean_off = |c: f64| all.gaps.iter().map(|g| (f64::from(*g) - c).abs()).sum::<f64>() / all.gaps.len().max(1) as f64 / 1000.0;
    println!(
        "  arrival spread: p5 gap {:.2} ms, mean distance from 13.89 ms {:.2} ms",
        pct(&all.gaps, 5),
        mean_off(13_889.0)
    );
    if all.lost_tx + all.lost_rx > 0 {
        println!("  simulated loss: {} sent and {} received packets dropped", all.lost_tx, all.lost_rx);
    }
}

/// Sends through the simulated network: some packets lost, the rest after
/// the delay and jitter.
fn net_send(sock: &std::sync::Arc<tokio::net::UdpSocket>, to: std::net::SocketAddr, pkts: &mut Vec<Vec<u8>>, net: NetSim, r: &mut UdpReport) {
    for p in pkts.drain(..) {
        if net.loss_pct > 0.0 && rand::random::<f64>() * 100.0 < net.loss_pct {
            r.lost_tx += 1;
            continue;
        }
        if net.delay.is_zero() && net.jitter.is_zero() {
            let _ = sock.try_send_to(&p, to);
            continue;
        }
        let d = net.delay + net.jitter.mul_f64(rand::random::<f64>());
        let sock = sock.clone();
        tokio::spawn(async move {
            tokio::time::sleep(d).await;
            let _ = sock.send_to(&p, to).await;
        });
    }
}

/// Joins the game server as the game does (connect, `SNSLobbyJoinRequestv4`)
/// and plays for `hold` (at least 5 s): once accepted, its entrant state
/// twice and an equipment update (which spawn it, so the others get its
/// body), then player state every client frame (72 Hz), acks for whatever
/// the server sends, and a disconnect at the end. Each state carries when
/// it was sent and by whom, so the others can tell how old it is.
async fn udp_play(s: &SessionSuccess, xpid: XpId, id: u16, hold: Duration, net: NetSim) -> Result<UdpReport> {
    use crate::engine::msg::{CONNECT, DISCONNECT, ENTRANT_STATE, EQUIPMENT, JOIN_ACCEPTED, JOIN_COMPLETE, JOIN_REQUEST, PLAYER_STATE, SAVE_LOADOUT_REQUEST};
    let (_, mut t) = crate::decap::transports(&s.to_proto());
    let sock = std::sync::Arc::new(tokio::net::UdpSocket::bind("0.0.0.0:0").await?);
    let server = std::net::SocketAddr::from((s.internal_ip, s.port));
    let mut r = UdpReport::default();
    let mut out = Vec::new();
    t.queue_reliable(CONNECT, &[0; 16]);
    let mut join = [0u8; 64];
    join[17..33].copy_from_slice(Uuid::new_v4().as_bytes());
    join[37..41].copy_from_slice(&(s.team.max(0) as u32).to_le_bytes());
    t.queue_reliable(JOIN_REQUEST, &join);
    t.flush(&mut out);
    net_send(&sock, server, &mut out, net, &mut r);
    let start = Instant::now();
    let mut frame = tokio::time::interval(Duration::from_micros(13_889));
    let mut buf = vec![0u8; 2048];
    let mut seq: u32 = 0;
    let mut last_from: std::collections::HashMap<u16, u64> = std::collections::HashMap::new();
    while start.elapsed() < hold.max(Duration::from_secs(5)) {
        tokio::select! {
            got = sock.recv_from(&mut buf) => {
                let Ok((n, _)) = got else { continue };
                if net.loss_pct > 0.0 && rand::random::<f64>() * 100.0 < net.loss_pct {
                    r.lost_rx += 1;
                    continue;
                }
                r.packets += 1;
                r.bytes += n as u64;
                let Some(rx) = t.receive(&buf[..n]) else { continue };
                let now = stamp_us();
                for m in &rx.reliable {
                    r.reliable += 1;
                    if m.type_ == JOIN_ACCEPTED && !r.accepted {
                        r.accepted = true;
                        let mut entrant = [0u8; 0x80];
                        entrant[..8].copy_from_slice(&xpid.platform.to_le_bytes());
                        entrant[8..16].copy_from_slice(&xpid.account.to_le_bytes());
                        t.queue_reliable(ENTRANT_STATE, &entrant);
                        t.queue_reliable(ENTRANT_STATE, &entrant);
                        t.queue_reliable(EQUIPMENT, &[1; 56]);
                    }
                    r.complete |= m.type_ == JOIN_COMPLETE;
                }
                for m in &rx.mailbox {
                    r.mailbox += 1;
                    if m.type_ != PLAYER_STATE || m.data.len() < 14 {
                        continue;
                    }
                    r.relayed += 1;
                    let from = u16::from_le_bytes([m.data[12], m.data[13]]);
                    let sent = u64::from_le_bytes(m.data[4..12].try_into().expect("8 bytes"));
                    if from == id {
                        continue;
                    }
                    r.ages.push(now.saturating_sub(sent).min(u64::from(u32::MAX)) as u32);
                    if let Some(prev) = last_from.insert(from, now) {
                        r.gaps.push(now.saturating_sub(prev).min(u64::from(u32::MAX)) as u32);
                    }
                }
            }
            _ = frame.tick() => {
                if r.accepted {
                    seq = seq.wrapping_add(1);
                    let mut state = [0u8; 64];
                    state[..4].copy_from_slice(&seq.to_le_bytes());
                    state[4..12].copy_from_slice(&stamp_us().to_le_bytes());
                    state[12..14].copy_from_slice(&id.to_le_bytes());
                    t.queue_mailbox(PLAYER_STATE, u64::from(id) + 1, &state);
                    // A loadout save now and then: the server's save waits
                    // on a timer of its own (its event drive's deadline wake).
                    if seq.is_multiple_of(144) {
                        t.queue_reliable(SAVE_LOADOUT_REQUEST, &[0; 8]);
                    }
                }
                t.flush(&mut out);
                net_send(&sock, server, &mut out, net, &mut r);
            }
        }
    }
    r.secs = start.elapsed().as_secs_f64();
    r.resends = t.stats.resends;
    t.queue_reliable(DISCONNECT, &[]);
    t.flush(&mut out);
    net_send(&sock, server, &mut out, NetSim::default(), &mut r);
    Ok(r)
}
