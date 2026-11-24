use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;
use tokio::{net::UdpSocket, task::JoinSet, time};

#[derive(Default)]
struct RecoveryMonitor {
    subnets: Option<HashSet<(Ipv4Addr, u32)>>,
    sleep_ms: Option<u64>,
}

impl RecoveryMonitor {
    fn update(&mut self, subnets: Option<HashSet<(Ipv4Addr, u32)>>, sleep_ms: Option<u64>) -> Option<&'static str> {
        let resumed = sleep_ms.is_some_and(|current| {
            self.sleep_ms.replace(current).is_some_and(|last| current.saturating_sub(last) >= 1000)
        });
        let changed = subnets.is_some_and(|current| {
            let changed = self.subnets.as_ref().is_some_and(|last| last != &current);
            self.subnets = Some(current);
            changed
        });
        if resumed { Some("kc-system-resume") }
        else if changed { Some("kc-network-interface-change") }
        else { None }
    }
}

pub(crate) fn monitor_recovery(window: tauri::WebviewWindow) {
    use tauri::Emitter;
    tauri::async_runtime::spawn(async move {
        let mut monitor = RecoveryMonitor { subnets: local_subnets().ok(), sleep_ms: accumulated_sleep_ms() };
        loop {
            time::sleep(Duration::from_secs(4)).await;
            if window.url().is_err() { break; }
            if let Some(event) = monitor.update(local_subnets().ok(), accumulated_sleep_ms()) {
                let _ = window.emit_to(window.label(), event, ());
            }
        }
    });
}

fn accumulated_sleep_ms() -> Option<u64> {
    use windows_sys::Win32::System::{SystemInformation::GetTickCount64, WindowsProgramming::QueryUnbiasedInterruptTime};
    let mut active_100ns = 0;
    unsafe {
        let total_ms = GetTickCount64();
        (QueryUnbiasedInterruptTime(&mut active_100ns) != 0)
            .then(|| total_ms.saturating_sub(active_100ns / 10_000))
    }
}

fn advertised_port(packet: &[u8]) -> Option<u16> {
    if packet.len() > 4096 { return None; }
    let reply: serde_json::Value = serde_json::from_slice(packet).ok()?;
    if reply["service"] != "CHATX_COMMUNITY" { return None; }
    let port = match reply.get("port") {
        None => 45820.0,
        Some(value) => value.as_f64().or_else(|| value.as_str()?.trim().parse().ok())?,
    };
    (port.fract() == 0.0 && (1.0..=65535.0).contains(&port)).then_some(port as u16)
}

async fn healthy(client: reqwest::Client, address: SocketAddrV4, timeout: Duration, primary_only: bool) -> Option<SocketAddrV4> {
    let mut response = client.get(format!("http://{address}/health")).timeout(timeout).send().await.ok()?;
    if response.status() != 200 { return None; }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > 16384 { return None; }
        body.extend_from_slice(&chunk);
    }
    let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
    (value["service"] == "CHATX_COMMUNITY" && (!primary_only || value["temporary"] != true)).then_some(address)
}

pub async fn discover(bind: Ipv4Addr, targets: &[Ipv4Addr], port: u16, timeout: Duration) -> Option<SocketAddrV4> {
    discover_on(bind, targets, port, timeout, false).await
}

async fn discover_on(bind: Ipv4Addr, targets: &[Ipv4Addr], port: u16, timeout: Duration, primary_only: bool) -> Option<SocketAddrV4> {
    let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().ok()?;
    let socket = UdpSocket::bind((bind, 0)).await.ok()?;
    socket.set_broadcast(true).ok()?;
    time::timeout(timeout, async {
        for target in targets {
            let _ = socket.send_to(b"CHATX_COMMUNITY_DISCOVER", (*target, port)).await;
        }
        let mut seen = HashSet::new();
        let mut probes = JoinSet::new();
        let mut buffer = [0; 4097];
        loop {
            tokio::select! {
                packet = socket.recv_from(&mut buffer) => {
                    let (count, SocketAddr::V4(sender)) = packet.ok()? else { continue };
                    if primary_only && !sender.ip().is_private() { continue; }
                    let Some(port) = advertised_port(&buffer[..count]) else { continue };
                    let address = SocketAddrV4::new(*sender.ip(), port);
                    if seen.len() < 32 && seen.insert(address) {
                        probes.spawn(healthy(client.clone(), address, timeout.min(Duration::from_millis(600)), primary_only));
                    }
                }
                result = probes.join_next(), if !probes.is_empty() => {
                    if let Some(Ok(Some(address))) = result { return Some(address); }
                }
            }
        }
    }).await.ok().flatten()
}

pub async fn discover_primary(port: u16) -> std::io::Result<Option<SocketAddrV4>> {
    let adapters = local_subnets()?;
    let mut probes = JoinSet::new();
    for (address, mask) in adapters {
        let broadcast = Ipv4Addr::from((u32::from(address) & mask) | !mask);
        let targets = [broadcast, Ipv4Addr::BROADCAST];
        probes.spawn(async move { discover_on(address, &targets, port, Duration::from_millis(1600), true).await });
    }
    while let Some(result) = probes.join_next().await {
        if let Ok(Some(address)) = result { return Ok(Some(address)); }
    }
    Ok(None)
}

pub async fn probe_primary(origin: &str) -> Option<SocketAddrV4> {
    let address = origin.strip_prefix("http://")?.parse::<SocketAddrV4>().ok()?;
    if !address.ip().is_private() || address.port() == 0 { return None; }
    let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().ok()?;
    healthy(client, address, Duration::from_millis(1300), true).await
}

fn local_subnets() -> std::io::Result<HashSet<(Ipv4Addr, u32)>> {
    use std::{io, mem::{offset_of, size_of}};
    use windows_sys::Win32::{Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR}, NetworkManagement::IpHelper::{
        GetIpAddrTable, MIB_IPADDRROW_XP, MIB_IPADDRTABLE, MIB_IPADDR_DELETED, MIB_IPADDR_DISCONNECTED,
    }};
    let mut bytes = 0;
    let status = unsafe { GetIpAddrTable(std::ptr::null_mut(), &mut bytes, 0) };
    if status != ERROR_INSUFFICIENT_BUFFER { return Err(io::Error::from_raw_os_error(status as i32)); }
    for _ in 0..3 {
        if bytes as usize > 1024 * 1024 || (bytes as usize) < size_of::<u32>() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Ukuran tabel IPv4 tidak valid"));
        }
        let mut buffer = vec![0u32; (bytes as usize).div_ceil(size_of::<u32>())];
        let table = buffer.as_mut_ptr().cast::<MIB_IPADDRTABLE>();
        let status = unsafe { GetIpAddrTable(table, &mut bytes, 0) };
        if status == ERROR_INSUFFICIENT_BUFFER { continue; }
        if status != NO_ERROR { return Err(io::Error::from_raw_os_error(status as i32)); }
        let count = buffer[0] as usize;
        let offset = offset_of!(MIB_IPADDRTABLE, table);
        let max = (buffer.len() * size_of::<u32>() - offset) / size_of::<MIB_IPADDRROW_XP>();
        if count > max { return Err(io::Error::new(io::ErrorKind::InvalidData, "Tabel IPv4 tidak lengkap")); }
        // GetIpAddrTable writes aligned MIB_IPADDRROW_XP entries after the count.
        let rows = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>().add(offset).cast::<MIB_IPADDRROW_XP>(), count) };
        return Ok(rows.iter().filter_map(|row| {
            let address = Ipv4Addr::from(row.dwAddr.to_ne_bytes());
            let mask = u32::from_be_bytes(row.dwMask.to_ne_bytes());
            let disconnected = u32::from(row.wType) & (MIB_IPADDR_DELETED | MIB_IPADDR_DISCONNECTED) != 0;
            (address.is_private() && !disconnected && mask != 0 && mask != u32::MAX).then_some((address, mask))
        }).collect());
    }
    Err(io::Error::new(io::ErrorKind::Interrupted, "Adaptor IPv4 berubah saat pencarian HOST"))
}

pub fn primary_lan_address() -> std::io::Result<Ipv4Addr> {
    use std::io::{Error, ErrorKind};
    let addresses: HashSet<_> = local_subnets()?.into_iter().map(|(address, _)| address).collect();
    if addresses.len() == 1 {
        return Ok(*addresses.iter().next().expect("one private adapter"));
    }
    if addresses.is_empty() {
        return Err(Error::new(ErrorKind::AddrNotAvailable, "Tidak ada adaptor IPv4 privat yang aktif"));
    }
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    // Connecting a UDP socket selects the Windows route without sending a packet.
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
    match socket.local_addr()? {
        SocketAddr::V4(address) if addresses.contains(address.ip()) => Ok(*address.ip()),
        _ => Err(Error::new(ErrorKind::AddrNotAvailable,
            "Adaptor LAN belum dapat dipilih. Jalankan HOST dengan --lan-host dan IPv4 kantor.")),
    }
}

fn legacy_ipv4(value: &str) -> Option<Ipv4Addr> {
    let mut octets = [0; 4];
    let mut parts = value.split('.');
    for octet in &mut octets {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        *octet = part.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(Ipv4Addr::from(octets))
}

pub fn directed_broadcast(address: &str, netmask: &str) -> Option<Ipv4Addr> {
    let ip = legacy_ipv4(address)?;
    let mask = legacy_ipv4(netmask)?;
    if address.starts_with("169.254.") {
        return None;
    }
    Some(Ipv4Addr::from((u32::from(ip) & u32::from(mask)) | !u32::from(mask)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener};

    #[test]
    fn recovery_ignores_initial_samples_and_transient_errors() {
        let mut monitor = RecoveryMonitor::default();
        let lan = HashSet::from([(Ipv4Addr::new(192, 168, 1, 25), 0xffffff00)]);
        assert_eq!(monitor.update(Some(lan.clone()), Some(12)), None);
        assert_eq!(monitor.update(None, None), None);
        assert_eq!(monitor.update(Some(lan), Some(20)), None);
    }

    #[test]
    fn recovery_coalesces_resume_and_address_change() {
        let mut monitor = RecoveryMonitor::default();
        let lan = |address| HashSet::from([(Ipv4Addr::new(192, 168, 1, address), 0xffffff00)]);
        assert_eq!(monitor.update(Some(lan(25)), Some(0)), None);
        assert_eq!(monitor.update(Some(lan(26)), Some(20)), Some("kc-network-interface-change"));
        assert_eq!(monitor.update(Some(lan(27)), Some(5020)), Some("kc-system-resume"));
        assert_eq!(monitor.update(Some(lan(27)), Some(5025)), None);
        assert_eq!(monitor.update(Some(HashSet::new()), Some(5025)), Some("kc-network-interface-change"));
    }

    async fn health_endpoint(service: &'static str) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let count = stream.read(&mut request).await.unwrap();
            assert!(request[..count].starts_with(b"GET /health HTTP/1.1"));
            let body = serde_json::json!({ "service": service }).to_string();
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(reply.as_bytes()).await.unwrap();
        });
        (port, task)
    }

    #[tokio::test]
    async fn rejects_unhealthy_reply_and_uses_udp_sender_for_valid_host() {
        let loopback = Ipv4Addr::LOCALHOST;
        let udp = UdpSocket::bind((loopback, 0)).await.unwrap();
        let discovery_port = udp.local_addr().unwrap().port();
        let (bad_port, bad_probe) = health_endpoint("OTHER_SERVICE").await;
        let (good_port, good_probe) = health_endpoint("CHATX_COMMUNITY").await;
        let responder = tokio::spawn(async move {
            let mut query = [0; 32];
            let (length, sender) = udp.recv_from(&mut query).await.unwrap();
            assert_eq!(&query[..length], b"CHATX_COMMUNITY_DISCOVER");
            udp.send_to(b"invalid json", sender).await.unwrap();
            let packet = |port| {
                serde_json::json!({
                    "service": "CHATX_COMMUNITY", "ip": "192.0.2.7", "port": port,
                }).to_string()
            };
            udp.send_to(packet(bad_port).as_bytes(), sender).await.unwrap();
            bad_probe.await.unwrap();
            udp.send_to(packet(good_port).as_bytes(), sender).await.unwrap();
        });
        let address = discover(loopback, &[loopback], discovery_port, Duration::from_secs(2)).await;
        assert_eq!(address, Some(SocketAddrV4::new(loopback, good_port)));
        time::timeout(Duration::from_secs(2), async {
            responder.await.unwrap();
            good_probe.await.unwrap();
        }).await.unwrap();
    }

    #[tokio::test]
    async fn times_out_when_discovery_has_no_reply() {
        let loopback = Ipv4Addr::LOCALHOST;
        let silent = UdpSocket::bind((loopback, 0)).await.unwrap();
        let port = silent.local_addr().unwrap().port();
        let result = time::timeout(Duration::from_secs(2),
            discover(loopback, &[loopback], port, Duration::from_millis(100))).await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn preserves_broadcast_rules_used_by_legacy_discovery() {
        for (ip, mask, expected) in [
            ("192.168.1.25", "255.255.255.0", Some("192.168.1.255")),
            ("10.4.3.2", "255.255.252.0", Some("10.4.3.255")),
            ("010.4.3.2", "255.255.255.255", Some("10.4.3.2")),
            ("169.254.1.2", "255.255.0.0", None),
            ("192.168.256.1", "255.255.255.0", None),
            ("192.168.1.2", "255.255.0", None),
        ] {
            assert_eq!(directed_broadcast(ip, mask).map(|ip| ip.to_string()).as_deref(), expected);
        }
    }
}
