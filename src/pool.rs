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
}

impl Pool {
    pub fn new(cfg: PoolConfig, hub: Arc<Hub>, upstreams: Arc<Upstreams>) -> Arc<Self> {
        Arc::new(Self { cfg, hub, upstreams, instances: Mutex::new(BTreeMap::new()) })
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
        if idle < self.cfg.min_idle && total < self.cfg.max_instances {
            let want = (self.cfg.min_idle - idle).min(self.cfg.max_instances - total);
            for _ in 0..want {
                self.spawn_next().await?;
            }
        } else if idle > self.cfg.min_idle && !self.cfg.idle_retire.is_zero() {
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
        for idx in 0..(self.cfg.max_instances as u32 * 4) {
            if used.contains(&idx) {
                continue;
            }
            let Some(port) = self.cfg.first_udp_port.checked_add(idx as u16) else { break };
            let addr = SocketAddr::new(self.cfg.bind_ip, port);
            let socket = match tokio::net::UdpSocket::bind(addr).await {
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
        inst.inst.stop().await;
    }

    pub async fn shutdown(&self) {
        let all: Vec<Slot> = std::mem::take(&mut *self.instances.lock()).into_values().collect();
        futures_util::future::join_all(all.into_iter().map(|s| s.inst.stop())).await;
    }
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
