//! Embedded game-server pool.
//!
//! Each instance is one `engine::Instance` running inside this process on its
//! own UDP port: one session per instance, like native servers. An idle
//! instance is one UDP socket, one websocket and one parked task; it does no
//! work until a session starts. The pool keeps `min_idle` instances ready, starts more as lobbies fill
//! them, and (only when `idle_retire` is non-zero) stops surplus idle ones after it.
//! By default every started instance stays registered as a blank idle server.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::engine::{self, EngineConfig, Instance};

use crate::lobby::Hub;
use crate::upstream::{Backend, Upstreams};

#[derive(Clone, Debug)]
pub struct PoolConfig {
    pub bind_ip: IpAddr,
    pub first_udp_port: u16,
    pub min_idle: usize,
    pub max_instances: usize,
    pub idle_retire: Duration,
    pub engine: EngineConfig,
    /// Port of this process's HTTP/WS listener (instances connect to it).
    pub hub_port: u16,
}

struct Slot {
    inst: Instance,
    started: Instant,
}

pub struct Pool {
    cfg: PoolConfig,
    hub: Arc<Hub>,
    upstreams: Arc<Upstreams>,
    instances: Mutex<BTreeMap<u32, Slot>>,
    /// (kept idle, most at once): the launcher can raise them while running.
    limits: Mutex<(usize, usize)>,
}

impl Pool {
    pub fn new(cfg: PoolConfig, hub: Arc<Hub>, upstreams: Arc<Upstreams>) -> Arc<Self> {
        let limits = Mutex::new((cfg.min_idle, cfg.max_instances));
        Arc::new(Self { cfg, hub, upstreams, instances: Mutex::new(BTreeMap::new()), limits })
    }

    /// New limits for a running pool: more game servers start on the next
    /// pass. Lowering them stops nothing that already runs (surplus idle
    /// ones go only when `--idle-retire-secs` is set).
    pub fn set_limits(&self, max_instances: usize, min_idle: usize) {
        let max = max_instances.clamp(1, 1024);
        let min = min_idle.min(max);
        *self.limits.lock() = (min, max);
        info!(kept_running = min, most_at_once = max, running = self.count(), "game server limits changed");
    }

    pub fn count(&self) -> usize {
        self.instances.lock().len()
    }

    /// Keep capacity in line with demand; runs until the process exits.
    pub async fn run(self: Arc<Self>, mut wake: mpsc::UnboundedReceiver<()>) {
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                Some(()) = wake.recv() => { while wake.try_recv().is_ok() {} }
            }
            self.reap_dead();
            self.hub.reap();
            if let Err(e) = self.rebalance().await {
                warn!(error = %e, "pool rebalance failed");
            }
        }
    }

    fn reap_dead(&self) {
        self.instances.lock().retain(|idx, i| {
            if i.inst.task.is_finished() {
                warn!(instance = idx, port = i.inst.port, "game server instance exited");
                crate::upnp::close(crate::upnp::Proto::Udp, i.inst.port);
                false
            } else {
                true
            }
        });
    }

    /// (idle-ready count, idle candidates for retirement)
    fn occupancy(&self) -> (usize, Vec<u32>) {
        let instances = self.instances.lock();
        match self.upstreams.backend {
            Backend::Builtin => {
                let snap = self.hub.embedded_snapshot();
                let mut idle = 0;
                let mut retire = Vec::new();
                for (idx, inst) in instances.iter() {
                    match snap.iter().find(|(p, _)| *p == inst.inst.port) {
                        Some((_, Some(idle_for))) => {
                            idle += 1;
                            if *idle_for >= self.cfg.idle_retire {
                                retire.push(*idx);
                            }
                        }
                        Some((_, None)) => {}
                        // Not registered yet: counts as ready if just started.
                        None if inst.started.elapsed() < Duration::from_secs(15) => idle += 1,
                        None => {}
                    }
                }
                (idle, retire)
            }
            _ => {
                let busy = self.upstreams.busy_count();
                let idle = instances.len().saturating_sub(busy);
                let retire = self.upstreams.idle_instances(self.cfg.idle_retire).into_iter().filter(|i| instances.contains_key(i)).collect();
                (idle, retire)
            }
        }
    }

    async fn rebalance(self: &Arc<Self>) -> Result<()> {
        let (idle, retire) = self.occupancy();
        let total = self.count();
        let (min_idle, max_instances) = *self.limits.lock();
        if idle < min_idle && total < max_instances {
            let want = (min_idle - idle).min(max_instances - total);
            for _ in 0..want {
                self.spawn_next().await?;
            }
        } else if idle > min_idle && !self.cfg.idle_retire.is_zero() {
            // Stop one surplus idle instance per pass.
            if let Some(idx) = retire.first().copied() {
                self.retire(idx).await;
            }
        }
        Ok(())
    }

    async fn spawn_next(self: &Arc<Self>) -> Result<()> {
        // Lowest free index whose UDP port binds.
        let used: Vec<u32> = self.instances.lock().keys().copied().collect();
        let most = self.limits.lock().1;
        for idx in 0..(most as u32 * 4) {
            if used.contains(&idx) {
                continue;
            }
            let Some(port) = self.cfg.first_udp_port.checked_add(idx as u16) else { break };
            let addr = SocketAddr::new(self.cfg.bind_ip, port);
            let socket = match bind_udp(addr) {
                Ok(s) => {
                    disable_udp_connreset(&s);
                    s
                }
                Err(e) => {
                    debug!(%addr, error = %e, "UDP port busy; trying next");
                    continue;
                }
            };
            let fleet_url = match self.upstreams.backend {
                Backend::Builtin => format!("ws://127.0.0.1:{}/fleet", self.cfg.hub_port),
                _ => format!("ws://127.0.0.1:{}/upstream/{idx}", self.cfg.hub_port),
            };
            let inst = engine::start(idx, socket, fleet_url, self.cfg.engine)?;
            info!(instance = idx, port, total = self.count() + 1, "game server instance started");
            crate::upnp::open(crate::upnp::Proto::Udp, port);
            self.instances.lock().insert(idx, Slot { inst, started: Instant::now() });
            return Ok(());
        }
        anyhow::bail!("no free UDP port for another game server")
    }

    async fn retire(&self, idx: u32) {
        let port = match self.instances.lock().get(&idx) {
            Some(i) => i.inst.port,
            None => return,
        };
        if matches!(self.upstreams.backend, Backend::Builtin) && !self.hub.retire_port(port) {
            return; // got allocated meanwhile
        }
        let Some(inst) = self.instances.lock().remove(&idx) else { return };
        info!(instance = idx, port, "retiring idle game server instance");
        crate::upnp::close(crate::upnp::Proto::Udp, port);
        inst.inst.stop().await;
    }

    pub async fn shutdown(&self) {
        let all: Vec<Slot> = std::mem::take(&mut *self.instances.lock()).into_values().collect();
        futures_util::future::join_all(all.into_iter().map(|s| s.inst.stop())).await;
    }
}

/// Binds a game port. On the unspecified IPv4 address (the default) it is a
/// dual-stack `[::]` socket, so players can reach it over IPv6 too (hosts
/// behind CGNAT have no public IPv4); IPv4 alone where the system has no IPv6.
fn bind_udp(addr: SocketAddr) -> std::io::Result<tokio::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let open = |domain: Domain, at: SocketAddr, dual: bool| -> std::io::Result<tokio::net::UdpSocket> {
        let s = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
        if dual {
            s.set_only_v6(false)?;
        }
        s.set_nonblocking(true)?;
        s.bind(&at.into())?;
        tokio::net::UdpSocket::from_std(s.into())
    };
    if addr.ip() == std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED) {
        let any6 = SocketAddr::new(std::net::Ipv6Addr::UNSPECIFIED.into(), addr.port());
        match open(Domain::IPV6, any6, true) {
            Ok(s) => return Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => return Err(e),
            Err(e) => debug!(%any6, error = %e, "no dual-stack socket; IPv4 only"),
        }
    }
    open(Domain::for_address(addr), addr, false)
}

/// Windows turns an ICMP "port unreachable" (a player whose client already
/// closed) into a recv error on the whole socket; ignore those like every
/// other OS does.
#[cfg(windows)]
fn disable_udp_connreset(socket: &tokio::net::UdpSocket) {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{SIO_UDP_CONNRESET, SOCKET, WSAIoctl};
    let off: u32 = 0;
    let mut returned = 0u32;
    // SAFETY: valid socket handle, 4-byte input buffer, no output buffer.
    let rc = unsafe {
        WSAIoctl(
            socket.as_raw_socket() as SOCKET,
            SIO_UDP_CONNRESET,
            (&off as *const u32).cast(),
            4,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
            None,
        )
    };
    if rc != 0 {
        debug!("SIO_UDP_CONNRESET could not be disabled");
    }
}

#[cfg(not(windows))]
fn disable_udp_connreset(_socket: &tokio::net::UdpSocket) {}
