//! Automatic port forwarding through the router (UPnP IGD).
//!
//! At startup the server looks for an Internet Gateway Device on the LAN
//! (SSDP), then asks it to forward the hub's TCP port and each game server's
//! UDP port to this machine while they are open. The mappings carry a lease
//! and are renewed; they are removed when an instance retires and on exit.
//!
//! This does nothing behind carrier-grade NAT (Starlink, most mobile, some
//! fibre): the router's own "external" address is then private, and the
//! server says so. A relay or a public address is needed there.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use parking_lot::Mutex;
use tracing::{debug, info, warn};

/// How long each mapping is asked for; renewed well before it runs out.
const LEASE_SECS: u32 = 3600;
const RENEW_EVERY: Duration = Duration::from_secs(20 * 60);
const SEARCH_WAIT: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const DESCRIPTION: &str = "echo-rust-server";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Proto {
    Tcp,
    Udp,
}

impl Proto {
    fn name(self) -> &'static str {
        match self {
            Proto::Tcp => "TCP",
            Proto::Udp => "UDP",
        }
    }
}

/// The router's port mapping service.
struct Gateway {
    /// host:port of the router's HTTP server.
    host: String,
    control_path: String,
    service: String,
    /// This machine's address as the router sees it.
    local_ip: Ipv4Addr,
}

struct Mapper {
    gw: Gateway,
    /// Ports mapped now, and whether the router only takes permanent leases.
    open: Mutex<Vec<(Proto, u16)>>,
    permanent: Mutex<bool>,
}

static MAPPER: OnceLock<Option<Mapper>> = OnceLock::new();
/// Set by `start`: `open`/`close` do nothing without it.
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn enabled() -> bool {
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Turns forwarding on before `start` has found the router, so ports opened
/// meanwhile wait for it.
pub fn enable() {
    ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Finds the router and starts the renewal loop. Blocking (call it from a
/// thread); later `open`/`close` calls queue until it is done.
pub fn start() {
    ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
    let m = match discover() {
        Ok(gw) => {
            info!(router = %gw.host, local_ip = %gw.local_ip, "UPnP router found: forwarding ports automatically");
            Some(Mapper { gw, open: Mutex::new(Vec::new()), permanent: Mutex::new(false) })
        }
        Err(e) => {
            warn!(error = %e, "no UPnP router answered: forward the ports by hand if players outside your network can't join");
            None
        }
    };
    let _ = MAPPER.set(m);
    let Some(Some(m)) = MAPPER.get() else { return };
    match m.external_ip() {
        Ok(ip) if is_shared(ip) => warn!(
            router_ip = %ip,
            "the router's own internet address is private (carrier-grade NAT, as on Starlink or mobile): port forwarding can't work on this connection; players outside it need a relay or a public IP"
        ),
        Ok(ip) => info!(%ip, "router's internet address"),
        Err(e) => debug!(error = %e, "router didn't give its internet address"),
    }
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(RENEW_EVERY);
            let Some(Some(m)) = MAPPER.get() else { return };
            let ports = m.open.lock().clone();
            for (proto, port) in ports {
                if let Err(e) = m.add(proto, port) {
                    warn!(proto = proto.name(), port, error = %e, "UPnP renewal failed");
                }
            }
        }
    });
}

/// Forwards `port` to this machine (in the background).
pub fn open(proto: Proto, port: u16) {
    if !enabled() {
        return;
    }
    std::thread::spawn(move || {
        let Some(m) = wait() else { return };
        match m.add(proto, port) {
            Ok(()) => {
                let mut open = m.open.lock();
                if !open.contains(&(proto, port)) {
                    open.push((proto, port));
                }
                info!(proto = proto.name(), port, "UPnP: port forwarded");
            }
            Err(e) => warn!(proto = proto.name(), port, error = %e, "UPnP: router refused to forward the port"),
        }
    });
}

/// Removes a forward made by `open` (in the background).
pub fn close(proto: Proto, port: u16) {
    if !enabled() {
        return;
    }
    std::thread::spawn(move || {
        let Some(m) = wait() else { return };
        m.open.lock().retain(|p| *p != (proto, port));
        if let Err(e) = m.delete(proto, port) {
            debug!(proto = proto.name(), port, error = %e, "UPnP: removing the forward failed");
        }
    });
}

/// Removes every forward (at exit). Blocking, a few seconds at most.
pub fn close_all() {
    let Some(Some(m)) = MAPPER.get() else { return };
    let ports = std::mem::take(&mut *m.open.lock());
    for (proto, port) in ports {
        let _ = m.delete(proto, port);
    }
    if !m.open.lock().is_empty() {
        debug!("UPnP: forwards added during shutdown left to expire");
    }
}

/// The mapper once discovery finished (None: off or no router).
fn wait() -> Option<&'static Mapper> {
    let until = Instant::now() + SEARCH_WAIT * 4;
    loop {
        if let Some(m) = MAPPER.get() {
            return m.as_ref();
        }
        if Instant::now() > until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// An address the internet can't reach directly: private, CGNAT's 100.64/10,
/// link-local or unspecified.
pub fn is_shared(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private() || ip.is_link_local() || ip.is_unspecified() || ip.is_loopback() || (o[0] == 100 && (o[1] & 0xc0) == 64)
}

impl Mapper {
    fn add(&self, proto: Proto, port: u16) -> Result<()> {
        let lease = if *self.permanent.lock() { 0 } else { LEASE_SECS };
        let args = |lease: u32| {
            format!(
                "<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>{}</NewProtocol>\
                 <NewInternalPort>{port}</NewInternalPort><NewInternalClient>{}</NewInternalClient><NewEnabled>1</NewEnabled>\
                 <NewPortMappingDescription>{DESCRIPTION}</NewPortMappingDescription><NewLeaseDuration>{lease}</NewLeaseDuration>",
                proto.name(),
                self.gw.local_ip
            )
        };
        match self.gw.soap("AddPortMapping", &args(lease)) {
            Ok(_) => Ok(()),
            // 725 OnlyPermanentLeasesSupported: older routers take only 0.
            Err(e) if lease != 0 && e.to_string().contains("725") => {
                *self.permanent.lock() = true;
                self.gw.soap("AddPortMapping", &args(0)).map(|_| ())
            }
            Err(e) => Err(e),
        }
    }

    fn delete(&self, proto: Proto, port: u16) -> Result<()> {
        let args = format!("<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>{}</NewProtocol>", proto.name());
        self.gw.soap("DeletePortMapping", &args).map(|_| ())
    }

    fn external_ip(&self) -> Result<Ipv4Addr> {
        let r = self.gw.soap("GetExternalIPAddress", "")?;
        tag(&r, "NewExternalIPAddress").context("no address in the reply")?.trim().parse().context("bad address")
    }
}

impl Gateway {
    fn soap(&self, action: &str, args: &str) -> Result<String> {
        let body = format!(
            "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
             s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">{args}</u:{action}></s:Body></s:Envelope>\r\n",
            self.service
        );
        let head = format!(
            "POST {} HTTP/1.0\r\nHost: {}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: \"{}#{action}\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.control_path,
            self.host,
            self.service,
            body.len()
        );
        let (status, reply, _) = http(&self.host, &(head + &body))?;
        if status != 200 {
            let code = tag(&reply, "errorCode").unwrap_or_default();
            let desc = tag(&reply, "errorDescription").unwrap_or_default();
            bail!("{action}: HTTP {status} UPnP error {code} {desc}");
        }
        Ok(reply)
    }
}

/// Searches the LAN for an Internet Gateway Device and reads where its
/// port mapping service is.
fn discover() -> Result<Gateway> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_read_timeout(Some(Duration::from_millis(300)))?;
    for st in [
        "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
        "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
        "urn:schemas-upnp-org:service:WANIPConnection:1",
        "urn:schemas-upnp-org:service:WANPPPConnection:1",
    ] {
        let msg = format!("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {st}\r\n\r\n");
        let _ = sock.send_to(msg.as_bytes(), "239.255.255.250:1900");
    }
    let until = Instant::now() + SEARCH_WAIT;
    let mut tried = Vec::new();
    let mut last_err = anyhow!("no answer to the search");
    let mut buf = [0u8; 2048];
    while Instant::now() < until {
        let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
        let text = String::from_utf8_lossy(&buf[..n]);
        let Some(loc) = text.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("location")).map(|(_, v)| v.trim().to_string())) else {
            continue;
        };
        if tried.contains(&loc) {
            continue;
        }
        tried.push(loc.clone());
        match gateway_at(&loc) {
            Ok(gw) => return Ok(gw),
            Err(e) => {
                debug!(location = %loc, error = %e, "UPnP device has no port mapping service");
                last_err = e;
            }
        }
    }
    Err(last_err)
}

/// The port mapping service in the device description at `location`.
fn gateway_at(location: &str) -> Result<Gateway> {
    let rest = location.strip_prefix("http://").context("not an http location")?;
    let (host, path) = rest.split_once('/').map_or((rest, "/".to_string()), |(h, p)| (h, format!("/{p}")));
    let host = host.to_string();
    let req = format!("GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    let (status, xml, local_ip) = http(&host, &req)?;
    if status != 200 {
        bail!("description: HTTP {status}");
    }
    let (service, control) = mapping_service(&xml).context("no WANIPConnection or WANPPPConnection service")?;
    let control_path = if let Some(full) = control.strip_prefix("http://") {
        full.split_once('/').map_or("/".to_string(), |(_, p)| format!("/{p}"))
    } else if control.starts_with('/') {
        control
    } else {
        format!("/{control}")
    };
    Ok(Gateway { host, control_path, service, local_ip })
}

/// The first WANIPConnection (else WANPPPConnection) service and its
/// control URL in a device description.
fn mapping_service(xml: &str) -> Option<(String, String)> {
    for kind in ["WANIPConnection", "WANPPPConnection"] {
        for block in xml.split("<service>").skip(1) {
            let block = block.split("</service>").next().unwrap_or(block);
            let Some(ty) = tag(block, "serviceType") else { continue };
            if ty.contains(&format!(":service:{kind}:")) {
                let control = tag(block, "controlURL")?;
                return Some((ty.trim().to_string(), control.trim().to_string()));
            }
        }
    }
    None
}

/// One HTTP/1.0 exchange: (status, body, our local IPv4).
fn http(host: &str, request: &str) -> Result<(u16, String, Ipv4Addr)> {
    let addr: SocketAddr = host.to_socket_addrs()?.find(SocketAddr::is_ipv4).context("router address")?;
    let mut s = TcpStream::connect_timeout(&addr, IO_TIMEOUT)?;
    s.set_read_timeout(Some(IO_TIMEOUT))?;
    s.set_write_timeout(Some(IO_TIMEOUT))?;
    let local_ip = match s.local_addr()?.ip() {
        std::net::IpAddr::V4(ip) => ip,
        std::net::IpAddr::V6(_) => bail!("router reached over IPv6"),
    };
    s.write_all(request.as_bytes())?;
    let mut raw = Vec::new();
    // A router that keeps the connection open: whatever came counts.
    let _ = s.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).context("no HTTP status")?;
    Ok((status, body.to_string(), local_ip))
}

/// The text of the first `<name>` element (namespace prefixes allowed).
fn tag(xml: &str, name: &str) -> Option<String> {
    let mut at = 0;
    while let Some(i) = xml[at..].find('<') {
        let start = at + i + 1;
        let end = start + xml[start..].find('>')?;
        let open = &xml[start..end];
        let local = open.split_whitespace().next().unwrap_or("").rsplit(':').next().unwrap_or("");
        if local == name && !open.starts_with('/') {
            let close = xml[end + 1..].find("</")?;
            return Some(xml[end + 1..end + 1 + close].to_string());
        }
        at = end + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_mapping_service() {
        let xml = r#"<root><device><serviceList>
            <service><serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType><controlURL>/l3f</controlURL></service>
            </serviceList><deviceList><device><serviceList>
            <service><serviceType>urn:schemas-upnp-org:service:WANPPPConnection:1</serviceType><controlURL>/ppp</controlURL></service>
            <service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>/ctl/IPConn</controlURL></service>
            </serviceList></device></deviceList></device></root>"#;
        assert_eq!(mapping_service(xml), Some(("urn:schemas-upnp-org:service:WANIPConnection:1".into(), "/ctl/IPConn".into())));
    }

    #[test]
    fn reads_soap_replies() {
        let r = r#"<s:Envelope><s:Body><u:GetExternalIPAddressResponse xmlns:u="x"><NewExternalIPAddress>100.70.1.2</NewExternalIPAddress></u:GetExternalIPAddressResponse></s:Body></s:Envelope>"#;
        assert_eq!(tag(r, "NewExternalIPAddress").as_deref(), Some("100.70.1.2"));
        let err = r#"<s:Fault><detail><UPnPError><errorCode>725</errorCode><errorDescription>OnlyPermanentLeasesSupported</errorDescription></UPnPError></detail></s:Fault>"#;
        assert_eq!(tag(err, "errorCode").as_deref(), Some("725"));
    }

    /// Read-only check against the real router: `cargo test live_router -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_router() {
        match discover() {
            Ok(gw) => {
                println!("ROUTER service {} local {}", gw.service, gw.local_ip);
                let m = Mapper { gw, open: Mutex::new(Vec::new()), permanent: Mutex::new(false) };
                match m.external_ip() {
                    Ok(ip) => println!("ROUTER external {} shared {}", if is_shared(ip) { "private/CGNAT" } else { "public" }, is_shared(ip)),
                    Err(e) => println!("ROUTER external error {e}"),
                }
            }
            Err(e) => println!("ROUTER none: {e}"),
        }
    }

    #[test]
    fn carrier_grade_nat_is_shared() {
        assert!(is_shared("100.64.0.1".parse().unwrap()));
        assert!(is_shared("100.127.255.254".parse().unwrap()));
        assert!(is_shared("192.168.1.1".parse().unwrap()));
        assert!(!is_shared("100.128.0.1".parse().unwrap()));
        assert!(!is_shared("81.2.69.160".parse().unwrap()));
    }
}
