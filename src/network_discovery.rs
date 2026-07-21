// SPDX-License-Identifier: GPL-3.0-only
//
// WMDE: LAN device discovery for the `network:///` view (a Windows-like "Network
// neighbourhood"). Two best-effort, time-bounded, dependency-light sources:
//   * mDNS / DNS-SD via the system Avahi daemon (`avahi-browse`), and
//   * WS-Discovery (WSD) via a UDP multicast probe (+ an optional WS-Transfer Get
//     to read a device's friendly name).
// Results are merged and deduplicated by IPv4 address so each physical device shows
// up exactly once (no per-service-type duplication like Windows).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::process::Command;
use std::time::{Duration, Instant};

/// Kind of a discovered device, ordered by display priority (a device that offers
/// several services is shown as the highest-priority kind: a file server that also
/// advertises a web UI stays a `Computer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeviceKind {
    Computer,
    Printer,
    Scanner,
    Other,
}

#[derive(Clone, Debug)]
pub struct Device {
    /// Display label (host or friendly name).
    pub name: String,
    /// IPv4 address, used both as the dedup key and for the `smb://` link.
    pub addr: String,
    pub kind: DeviceKind,
}

const WSD_MCAST: &str = "239.255.255.250:3702";
const MDNS_BUDGET_SECS: &str = "2";
const WSD_TIMEOUT: Duration = Duration::from_millis(1800);
const WSD_RECV_SLICE: Duration = Duration::from_millis(300);
const WSD_GET_BUDGET: Duration = Duration::from_millis(1500);

/// Discover LAN devices via mDNS and WSD concurrently, then merge + dedup by address.
/// Best-effort: any failing source contributes nothing rather than erroring.
pub fn discover_devices() -> Vec<Device> {
    let mdns = std::thread::spawn(mdns_avahi);
    let wsd = std::thread::spawn(wsd_probe);

    let mut by_addr: BTreeMap<String, Device> = BTreeMap::new();
    let mdns_devices = mdns.join().unwrap_or_default();
    let wsd_devices = wsd.join().unwrap_or_default();
    for device in mdns_devices.into_iter().chain(wsd_devices) {
        merge(&mut by_addr, device);
    }

    let mut devices: Vec<Device> = by_addr.into_values().collect();
    // Name any host still labelled by its bare IP (a WSD host whose WS-Transfer Get was
    // firewalled, or an SMB host without an mDNS name) via a NetBIOS node-status query -
    // reliable for Windows/Samba even with SMB1 browsing off. Run in parallel.
    let handles: Vec<_> = devices
        .iter()
        .enumerate()
        .filter(|(_, d)| looks_like_addr(&d.name))
        .map(|(i, d)| {
            let addr = d.addr.clone();
            std::thread::spawn(move || (i, netbios_name(&addr)))
        })
        .collect();
    for handle in handles {
        if let Ok((i, Some(name))) = handle.join() {
            devices[i].name = name;
        }
    }
    devices
}

/// NetBIOS node-status (NBSTAT) query over UDP 137 for a host's computer name. Works for
/// Windows/Samba hosts even when SMB1 browsing is off. Returns the unique workstation name.
fn netbios_name(ip: &str) -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket
        .set_read_timeout(Some(Duration::from_millis(1200)))
        .ok()?;

    // Header: txn id, flags=0, qdcount=1, others 0.
    let mut query: Vec<u8> = vec![
        0x13, 0x37, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    // Question: first-level-encoded wildcard name "*" (+ 15 NUL bytes), then NBSTAT/IN.
    let mut raw = [0u8; 16];
    raw[0] = b'*';
    query.push(0x20); // encoded length
    for b in raw {
        query.push(0x41 + (b >> 4));
        query.push(0x41 + (b & 0x0f));
    }
    query.push(0x00); // name terminator
    query.extend_from_slice(&[0x00, 0x21, 0x00, 0x01]); // qtype NBSTAT, qclass IN

    socket.send_to(&query, (ip, 137)).ok()?;
    let mut buf = [0u8; 2048];
    let (n, _) = socket.recv_from(&mut buf).ok()?;
    let data = &buf[..n];

    // The type/class marker (00 21 00 01) appears in both the echoed question and the
    // answer RR; try each position and parse the one that yields a valid name table.
    let markers: Vec<usize> = (0..data.len().saturating_sub(3))
        .filter(|&i| data[i..i + 4] == [0x00, 0x21, 0x00, 0x01])
        .collect();
    for pos in markers {
        let rd = pos + 4 + 4 + 2; // skip type+class, ttl, rdlength
        let Some(&num) = data.get(rd) else { continue };
        let num = num as usize;
        if num == 0 || num > 100 {
            continue;
        }
        let mut p = rd + 1;
        let mut ok = true;
        let mut name_opt = None;
        for _ in 0..num {
            let (Some(name_bytes), Some(&suffix), Some(flags)) =
                (data.get(p..p + 15), data.get(p + 15), data.get(p + 16..p + 18))
            else {
                ok = false;
                break;
            };
            let is_group = flags[0] & 0x80 != 0;
            // The unique (non-group) name with suffix 0x00 is the computer name.
            if name_opt.is_none() && !is_group && suffix == 0x00 {
                let name = String::from_utf8_lossy(name_bytes).trim().to_string();
                if !name.is_empty() {
                    name_opt = Some(name);
                }
            }
            p += 18;
        }
        if ok {
            if let Some(name) = name_opt {
                return Some(name);
            }
        }
    }
    None
}

fn merge(map: &mut BTreeMap<String, Device>, device: Device) {
    if device.addr.is_empty() {
        return;
    }
    match map.get_mut(&device.addr) {
        Some(existing) => {
            // Keep the highest-priority kind (Computer < Printer < Scanner < Other).
            if device.kind < existing.kind {
                existing.kind = device.kind;
            }
            // Prefer a real name over a bare IP-address label.
            if existing.name.is_empty()
                || (looks_like_addr(&existing.name) && !looks_like_addr(&device.name))
            {
                existing.name = device.name;
            }
        }
        None => {
            map.insert(device.addr.clone(), device);
        }
    }
}

fn looks_like_addr(name: &str) -> bool {
    name.parse::<IpAddr>().is_ok()
}

/// Classify a *raw* DNS-SD service type (avahi-browse is run with `-k`, so field[4]
/// is e.g. `_smb._tcp`, not the human-readable description).
fn kind_for_service(service_type: &str) -> DeviceKind {
    let s = service_type.to_ascii_lowercase();
    if s.contains("smb")
        || s.contains("_ssh.")
        || s.contains("sftp")
        || s.contains("afp")
        || s.contains("nfs")
        || s.contains("workstation")
    {
        DeviceKind::Computer
    } else if s.contains("scanner") || s.contains("uscan") {
        DeviceKind::Scanner
    } else if s.contains("printer") || s.contains("ipp") || s.contains("pdl") {
        DeviceKind::Printer
    } else {
        DeviceKind::Other
    }
}

// ---------------------------------------------------------------------------
// mDNS / DNS-SD via avahi-browse
// ---------------------------------------------------------------------------

/// Query the system Avahi daemon for all advertised services and fold them into one
/// entry per host. Requires `avahi-browse` (avahi) + a running `avahi-daemon`.
fn mdns_avahi() -> Vec<Device> {
    // `timeout` bounds the run; `-a` all types, `-t` terminate after the cache dump,
    // `-r` resolve to host/address, `-p` machine-parseable, `-k` raw types (no db lookup,
    // so field[4] is the DNS-SD type rather than a localized description).
    let output = match Command::new("timeout")
        .arg(MDNS_BUDGET_SECS)
        .args(["avahi-browse", "-a", "-t", "-r", "-p", "-k"])
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            log::warn!("avahi-browse unavailable, skipping mDNS discovery: {err}");
            return Vec::new();
        }
    };

    // host -> (name, ipv4, best-kind)
    let mut hosts: BTreeMap<String, (String, String, DeviceKind)> = BTreeMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        // Resolved records start with '=': =;iface;proto;name;type;domain;host;addr;port;txt...
        if !line.starts_with('=') {
            continue;
        }
        let fields: Vec<&str> = line.split(';').collect();
        if fields.len() < 9 {
            continue;
        }
        // Only IPv4 - link-local IPv6 addresses are not usable for an smb:// link.
        if fields[2] != "IPv4" {
            continue;
        }
        let name = unescape_avahi(fields[3]);
        let service_type = fields[4];
        let host = fields[6].trim_end_matches('.').to_string();
        let addr = fields[7].to_string();
        if host.is_empty() {
            continue;
        }
        // Reject non-routable addresses (loopback / link-local / unspecified) so a
        // multi-homed self-advertisement can't yield e.g. an smb://127.0.0.1/ link.
        match addr.parse::<Ipv4Addr>() {
            Ok(ip) if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() => {}
            _ => continue,
        }
        let kind = kind_for_service(service_type);
        let entry = hosts
            .entry(host)
            .or_insert_with(|| (name.clone(), addr.clone(), DeviceKind::Other));
        if kind < entry.2 {
            entry.2 = kind;
            // Prefer the name coming from the higher-priority service.
            if !name.is_empty() {
                entry.0 = name.clone();
            }
        }
        if entry.0.is_empty() {
            entry.0 = name;
        }
        if entry.1.is_empty() {
            entry.1 = addr;
        }
    }

    hosts
        .into_values()
        .map(|(name, addr, kind)| Device { name, addr, kind })
        .collect()
}

/// Unescape avahi's `-p` field encoding. Avahi escapes each raw byte >= 0x7f (and a few
/// specials) as `\NNN` decimal, and literal chars as `\<char>`. We must reassemble the
/// original *bytes* and decode UTF-8 once at the end - decoding each `\NNN` as its own
/// `char` would be Latin-1 and mangle Cyrillic/accented/CJK names.
fn unescape_avahi(field: &str) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(field.len());
    let mut buf = [0u8; 4];
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        let d1 = chars.clone().next();
        if d1.is_some_and(|c| c.is_ascii_digit()) {
            let digits: String = chars.clone().take(3).collect();
            if digits.len() == 3 && digits.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(byte) = digits.parse::<u8>() {
                    for _ in 0..3 {
                        chars.next();
                    }
                    out.push(byte);
                    continue;
                }
            }
        }
        if let Some(next) = chars.next() {
            out.extend_from_slice(next.encode_utf8(&mut buf).as_bytes());
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// WS-Discovery (WSD)
// ---------------------------------------------------------------------------

/// Send a WS-Discovery Probe on the LAN and collect responders. For each responder we
/// read its device kind from the ProbeMatch `<Types>` and (best-effort, in parallel)
/// its friendly name via a WS-Transfer Get, else fall back to the IP as the label.
fn wsd_probe() -> Vec<Device> {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(socket) => socket,
        Err(err) => {
            log::warn!("WSD: failed to bind UDP socket: {err}");
            return Vec::new();
        }
    };
    let _ = socket.set_multicast_ttl_v4(2);
    let _ = socket.set_read_timeout(Some(WSD_RECV_SLICE));

    let message_id = new_message_id();
    let probe = wsd_probe_envelope(&message_id);
    if let Err(err) = socket.send_to(probe.as_bytes(), WSD_MCAST) {
        log::warn!("WSD: failed to send probe: {err}");
        return Vec::new();
    }

    // ip -> (metadata transport URL, device kind from <Types>)
    let mut responders: BTreeMap<String, (Option<String>, DeviceKind)> = BTreeMap::new();
    let deadline = Instant::now() + WSD_TIMEOUT;
    let mut buf = vec![0u8; 65536];
    while Instant::now() < deadline {
        match socket.recv_from(&mut buf) {
            Ok((n, src)) => {
                let ip = match src.ip() {
                    IpAddr::V4(v4) => v4.to_string(),
                    // Skip IPv6 - not usable for an smb:// link and usually a dup of v4.
                    IpAddr::V6(_) => continue,
                };
                let body = String::from_utf8_lossy(&buf[..n]);
                let (xaddr, kind) = parse_probe_match(&body);
                let slot = responders
                    .entry(ip)
                    .or_insert((None, DeviceKind::Computer));
                // A later response carrying a metadata URL upgrades an earlier None.
                if slot.0.is_none() {
                    slot.0 = xaddr;
                }
                // Prefer a specific device kind (printer/scanner) over generic Computer.
                if slot.1 == DeviceKind::Computer && kind != DeviceKind::Computer {
                    slot.1 = kind;
                }
            }
            // Read slice timed out: keep looping until the overall deadline.
            Err(_) => {}
        }
    }

    // Resolve friendly names in parallel so N slow/firewalled endpoints cost about one
    // lookup instead of N (each lookup is itself wall-clock bounded).
    let handles: Vec<_> = responders
        .into_iter()
        .map(|(ip, (xaddr, kind))| {
            std::thread::spawn(move || {
                let name = xaddr
                    .as_deref()
                    .and_then(wsd_friendly_name)
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| ip.clone());
                Device { name, addr: ip, kind }
            })
        })
        .collect();
    handles
        .into_iter()
        .filter_map(|handle| handle.join().ok())
        .collect()
}

fn wsd_probe_envelope(message_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://www.w3.org/2003/05/soap-envelope" xmlns:wsa="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:wsd="http://schemas.xmlsoap.org/ws/2005/04/discovery">
<soap:Header>
<wsa:To>urn:schemas-xmlsoap-org:ws:2005:04:discovery</wsa:To>
<wsa:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</wsa:Action>
<wsa:MessageID>{message_id}</wsa:MessageID>
</soap:Header>
<soap:Body><wsd:Probe/></soap:Body>
</soap:Envelope>"#
    )
}

/// Parse a WSD ProbeMatch/ResolveMatch for its metadata transport URL and device kind
/// (from `<Types>`: a printer advertises PrintDeviceType, a scanner ScanDeviceType, a
/// PC pub:Computer). Parses the document once.
fn parse_probe_match(body: &str) -> (Option<String>, DeviceKind) {
    let Ok(doc) = roxmltree::Document::parse(body) else {
        return (None, DeviceKind::Computer);
    };
    let xaddr = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "XAddrs")
        .and_then(|n| n.text())
        .and_then(pick_http_xaddr);
    let types = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Types")
        .and_then(|n| n.text())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    // Check print before scan so a print+scan MFP resolves to Printer.
    let kind = if types.contains("print") {
        DeviceKind::Printer
    } else if types.contains("scan") {
        DeviceKind::Scanner
    } else {
        DeviceKind::Computer
    };
    (xaddr, kind)
}

/// Pick a usable http(s) transport URL out of a (space-separated) XAddrs list.
fn pick_http_xaddr(text: &str) -> Option<String> {
    let text = text.trim();
    text.split_whitespace()
        .find(|u| u.starts_with("http") && !u.contains('['))
        .map(str::to_string)
        .or_else(|| text.split_whitespace().next().map(str::to_string))
}

/// Best-effort WS-Transfer Get to read a device's friendly name from its metadata URL.
fn wsd_friendly_name(xaddr: &str) -> Option<String> {
    let (authority, path) = split_http_url(xaddr)?;
    // Resolve once (some XAddrs use a hostname); prefer IPv4.
    let addrs: Vec<SocketAddr> = authority.to_socket_addrs().ok()?.collect();
    let socket_addr = addrs
        .iter()
        .find(|a| a.is_ipv4())
        .copied()
        .or_else(|| addrs.first().copied())?;

    let mut stream = TcpStream::connect_timeout(&socket_addr, Duration::from_millis(700)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(1000)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_millis(700)))
        .ok()?;

    let envelope = wsd_get_envelope();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/soap+xml; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{envelope}",
        envelope.len()
    );
    stream.write_all(request.as_bytes()).ok()?;

    // Bounded by an overall deadline (not just the per-read timeout) so a slow/dribbling
    // host cannot spin here forever.
    let mut response = Vec::new();
    let mut chunk = [0u8; 4096];
    let deadline = Instant::now() + WSD_GET_BUDGET;
    loop {
        if Instant::now() >= deadline {
            break;
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                response.extend_from_slice(&chunk[..n]);
                if response.len() > 256 * 1024 {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let text = String::from_utf8_lossy(&response);
    let (headers, raw_body) = text.split_once("\r\n\r\n")?;
    let chunked = headers.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        line.starts_with("transfer-encoding:") && line.contains("chunked")
    });
    let decoded;
    let body = if chunked {
        decoded = dechunk(raw_body);
        decoded.as_str()
    } else {
        raw_body
    };
    extract_friendly_name(body)
}

fn wsd_get_envelope() -> String {
    let message_id = new_message_id();
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://www.w3.org/2003/05/soap-envelope" xmlns:wsa="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:wst="http://schemas.xmlsoap.org/ws/2004/09/transfer">
<soap:Header>
<wsa:Action>http://schemas.xmlsoap.org/ws/2004/09/transfer/Get</wsa:Action>
<wsa:MessageID>{message_id}</wsa:MessageID>
</soap:Header>
<soap:Body/>
</soap:Envelope>"#
    )
}

/// Read `<FriendlyName>` from a WSD device metadata (ThisDevice) response.
fn extract_friendly_name(body: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(body).ok()?;
    doc.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "FriendlyName")
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Decode an HTTP/1.1 `Transfer-Encoding: chunked` body.
fn dechunk(mut s: &str) -> String {
    let mut out = String::new();
    loop {
        let Some((size_line, rest)) = s.split_once("\r\n") else {
            break;
        };
        let hex = size_line.split(';').next().unwrap_or("").trim();
        let Ok(n) = usize::from_str_radix(hex, 16) else {
            break;
        };
        if n == 0 || rest.len() < n {
            break;
        }
        out.push_str(&rest[..n]);
        s = rest.get(n..).and_then(|r| r.strip_prefix("\r\n")).unwrap_or("");
    }
    out
}

/// Split `http://host[:port]/path...` into (`host:port`, `/path...`), defaulting the
/// port to the scheme's default when absent (`to_socket_addrs` needs `host:port`).
fn split_http_url(url: &str) -> Option<(String, String)> {
    let (rest, default_port) = if let Some(rest) = url.strip_prefix("http://") {
        (rest, "80")
    } else if let Some(rest) = url.strip_prefix("https://") {
        (rest, "443")
    } else {
        return None;
    };
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority.to_string(), format!("/{path}")),
        None => (rest.to_string(), "/".to_string()),
    };
    let authority = if authority.contains(':') {
        authority
    } else {
        format!("{authority}:{default_port}")
    };
    Some((authority, path))
}

/// A unique `urn:uuid:` MessageID for a WSD message. WSD only requires uniqueness, not a
/// canonical UUID, so this is derived from time + pid + a counter (no uuid crate needed).
fn new_message_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = u64::from(std::process::id());
    let a = (nanos >> 32) as u32;
    let b = nanos as u16;
    let c = 0x4000 | (n & 0x0fff) as u16; // version-4 nibble
    let d = 0x8000 | (pid & 0x3fff) as u16; // variant bits
    let e = ((pid << 20) ^ nanos ^ (n << 8)) & 0xffff_ffff_ffff;
    format!("urn:uuid:{a:08x}-{b:04x}-{c:04x}-{d:04x}-{e:012x}")
}
