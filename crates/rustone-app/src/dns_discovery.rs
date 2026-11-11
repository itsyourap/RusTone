//! Finding a SIP server the way RFC 3263 describes, for Settings' "Look
//! up": NAPTR records for the transports the domain offers, SRV records
//! for the servers of the account's transport, then their A and AAAA
//! records, all from the chosen DNS servers. pjsua does the SRV, A and
//! AAAA part itself when it connects, but never NAPTR; this shows what
//! it will find, and what NAPTR advertises.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{Name, RData, RecordType};
use rustone_core::account::Transport;

/// How long each DNS server gets to answer one query.
const TIMEOUT: Duration = Duration::from_secs(3);
/// SRV targets whose addresses are looked up.
const MAX_TARGETS: usize = 4;

/// One query and what it found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// `NAPTR example.com`.
    pub query: String,
    /// One line per record, or why there are none.
    pub answers: Vec<String>,
    pub failed: bool,
}

/// The lookup's report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Discovery {
    /// The DNS servers asked.
    pub servers: Vec<String>,
    pub steps: Vec<Step>,
    /// Where the account would connect, best first (`[2001:db8::1]:5061`).
    pub endpoints: Vec<String>,
    /// Remarks: a NAPTR transport that isn't the account's, a host with
    /// no records.
    pub notes: Vec<String>,
}

/// The SRV service and NAPTR service tag for a transport.
fn service(transport: Transport) -> (&'static str, &'static str) {
    match transport {
        Transport::Udp => ("_sip._udp", "SIP+D2U"),
        Transport::Tcp => ("_sip._tcp", "SIP+D2T"),
        Transport::Tls => ("_sips._tcp", "SIPS+D2T"),
    }
}

fn transport_name(tag: &str) -> &str {
    match tag.to_ascii_uppercase().as_str() {
        "SIP+D2U" => "UDP",
        "SIP+D2T" => "TCP",
        "SIPS+D2T" => "TLS",
        _ => tag,
    }
}

/// The host part of a server address: without `sip:`, port or brackets.
pub fn host_of(server: &str) -> &str {
    let s = server.trim();
    let s = s.strip_prefix("sips:").or_else(|| s.strip_prefix("sip:")).unwrap_or(s);
    let s = s.split(';').next().unwrap_or(s);
    if let Some(rest) = s.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match s.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') && port.parse::<u16>().is_ok() => host,
        _ => s,
    }
}

/// Looks up `host`'s SIP servers for `transport` through DNS servers on
/// port 53.
pub fn discover(host: &str, transport: Transport, servers: &[IpAddr]) -> Discovery {
    let servers: Vec<SocketAddr> = servers.iter().map(|ip| SocketAddr::new(*ip, 53)).collect();
    discover_via(host, transport, &servers)
}

fn discover_via(host: &str, transport: Transport, servers: &[SocketAddr]) -> Discovery {
    let shown = |s: &SocketAddr| if s.port() == 53 { s.ip().to_string() } else { s.to_string() };
    let mut report = Discovery { servers: servers.iter().map(shown).collect(), ..Discovery::default() };
    let host = host.trim().trim_end_matches('.');
    if host.parse::<IpAddr>().is_ok() {
        report.notes.push(format!("{host} is an IP address: nothing to look up."));
        return report;
    }
    if servers.is_empty() {
        report.notes.push("No DNS servers to ask.".into());
        return report;
    }
    let Ok(name) = Name::from_ascii(host) else {
        report.notes.push(format!("{host} isn't a host name."));
        return report;
    };
    let (srv_service, tag) = service(transport);

    // 1. NAPTR: which transports the domain offers, and their SRV names.
    let mut srv_name = None;
    let mut offered = Vec::new();
    let naptr = query(servers, &name, RecordType::NAPTR);
    let mut step = Step { query: format!("NAPTR {host}"), answers: Vec::new(), failed: false };
    match &naptr {
        Ok(records) => {
            let mut records: Vec<_> = records
                .iter()
                .filter_map(|r| match r {
                    RData::NAPTR(n) => Some(n.clone()),
                    _ => None,
                })
                .collect();
            records.sort_by_key(|n| (n.order, n.preference));
            for n in &records {
                let services = String::from_utf8_lossy(&n.services).into_owned();
                let flags = String::from_utf8_lossy(&n.flags).into_owned();
                let replacement = n.replacement.to_ascii();
                step.answers.push(format!("{} {} \"{flags}\" \"{services}\" {replacement}  ({})", n.order, n.preference, transport_name(&services)));
                if flags.eq_ignore_ascii_case("s") {
                    offered.push(transport_name(&services).to_owned());
                    if srv_name.is_none() && services.eq_ignore_ascii_case(tag) {
                        srv_name = Some(n.replacement.clone());
                    }
                }
            }
            if records.is_empty() {
                step.answers.push("No NAPTR records".into());
            }
        }
        Err(e) => {
            step.answers.push(e.clone());
            step.failed = true;
        }
    }
    report.steps.push(step);
    let ours = service_label(transport);
    if !offered.is_empty() && srv_name.is_none() {
        report.notes.push(format!("NAPTR offers {}, not {ours}, this account's transport.", offered.join(", ")));
    }

    // 2. SRV: the servers for the account's transport.
    let srv_name = srv_name.unwrap_or_else(|| Name::from_ascii(format!("{srv_service}.{host}")).unwrap_or_else(|_| name.clone()));
    let srv_text = srv_name.to_ascii();
    let mut step = Step { query: format!("SRV {}", srv_text.trim_end_matches('.')), answers: Vec::new(), failed: false };
    let mut targets: Vec<(u16, u16, u16, Name)> = Vec::new();
    match query(servers, &srv_name, RecordType::SRV) {
        Ok(records) => {
            for r in records {
                if let RData::SRV(s) = r {
                    targets.push((s.priority, s.weight, s.port, s.target.clone()));
                }
            }
            targets.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
            for (priority, weight, port, target) in &targets {
                step.answers.push(format!("{priority} {weight} {port} {}", target.to_ascii()));
            }
            if targets.is_empty() {
                step.answers.push("No SRV records".into());
            }
        }
        Err(e) => {
            step.answers.push(e);
            step.failed = true;
        }
    }
    report.steps.push(step);

    // 3. A and AAAA: of the SRV targets, or of the host itself.
    let default_port = if transport == Transport::Tls { 5061 } else { 5060 };
    let hosts: Vec<(Name, u16)> = if targets.is_empty() {
        vec![(name.clone(), default_port)]
    } else {
        let mut hosts: Vec<(Name, u16)> = Vec::new();
        for (_, _, port, target) in &targets {
            if !hosts.iter().any(|(t, _)| t == target) {
                hosts.push((target.clone(), *port));
            }
        }
        hosts.truncate(MAX_TARGETS);
        hosts
    };
    for (target, port) in hosts {
        let target_text = target.to_ascii();
        let target_text = target_text.trim_end_matches('.');
        for record_type in [RecordType::AAAA, RecordType::A] {
            let mut step = Step { query: format!("{record_type} {target_text}"), answers: Vec::new(), failed: false };
            match query(servers, &target, record_type) {
                Ok(records) => {
                    for r in records {
                        let ip: Option<IpAddr> = match r {
                            RData::A(a) => Some(a.0.into()),
                            RData::AAAA(a) => Some(a.0.into()),
                            _ => None,
                        };
                        if let Some(ip) = ip {
                            step.answers.push(ip.to_string());
                            report.endpoints.push(SocketAddr::new(ip, port).to_string());
                        }
                    }
                    if step.answers.is_empty() {
                        step.answers.push(format!("No {record_type} records"));
                    }
                }
                Err(e) => {
                    step.answers.push(e);
                    step.failed = true;
                }
            }
            report.steps.push(step);
        }
    }
    if report.endpoints.is_empty() {
        report.notes.push(format!("No addresses found for {host} over {ours}."));
    } else if targets.is_empty() {
        report.notes.push(format!("No SRV records: {host}'s own addresses, on port {default_port}."));
    }
    report
}

fn service_label(transport: Transport) -> &'static str {
    match transport {
        Transport::Udp => "UDP",
        Transport::Tcp => "TCP",
        Transport::Tls => "TLS",
    }
}

/// The answers to one question, from the first server that replies
/// (an error reply counts: NXDOMAIN is an answer). Answers of other types
/// (a CNAME on the way) are left out.
fn query(servers: &[SocketAddr], name: &Name, record_type: RecordType) -> Result<Vec<RData>, String> {
    let mut last_error = String::from("no DNS servers");
    for server in servers {
        match exchange(*server, name, record_type) {
            Ok(reply) => {
                return match reply.metadata.response_code {
                    ResponseCode::NoError => Ok(reply.answers.into_iter().filter(|r| r.record_type() == record_type).map(|r| r.data).collect()),
                    ResponseCode::NXDomain => Err(format!("{} doesn't exist (NXDOMAIN)", name.to_ascii().trim_end_matches('.'))),
                    code => Err(format!("{server} answered {code}")),
                };
            }
            Err(e) => last_error = format!("{server}: {e}"),
        }
    }
    Err(last_error)
}

/// Asks `server` over UDP, and again over TCP when the answer was cut short.
fn exchange(server: SocketAddr, name: &Name, record_type: RecordType) -> Result<Message, String> {
    let mut request = Message::new(rand_id(), MessageType::Query, OpCode::Query);
    request.metadata.recursion_desired = true;
    request.add_query(Query::query(name.clone(), record_type));
    let bytes = request.to_vec().map_err(|e| e.to_string())?;

    let local: SocketAddr = if server.is_ipv6() { "[::]:0".parse().expect("valid") } else { "0.0.0.0:0".parse().expect("valid") };
    let socket = UdpSocket::bind(local).map_err(|e| e.to_string())?;
    socket.connect(server).map_err(|e| e.to_string())?;
    socket.send(&bytes).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + TIMEOUT;
    let mut buffer = [0u8; 4096];
    let reply = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("no answer".into());
        }
        socket.set_read_timeout(Some(left)).map_err(|e| e.to_string())?;
        let n = socket.recv(&mut buffer).map_err(|e| match e.kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => "no answer".to_owned(),
            _ => e.to_string(),
        })?;
        // Anything that isn't the reply to this question is ignored.
        if let Ok(reply) = Message::from_vec(&buffer[..n])
            && reply.metadata.id == request.metadata.id
        {
            break reply;
        }
    };
    if !reply.metadata.truncation {
        return Ok(reply);
    }
    // Over TCP: each message is prefixed with its length.
    let mut stream = TcpStream::connect_timeout(&server, TIMEOUT).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    let length = u16::try_from(bytes.len()).map_err(|_| "query too long".to_owned())?;
    stream.write_all(&[length.to_be_bytes().as_slice(), &bytes].concat()).map_err(|e| e.to_string())?;
    let mut prefix = [0u8; 2];
    stream.read_exact(&mut prefix).map_err(|e| e.to_string())?;
    let mut message = vec![0u8; usize::from(u16::from_be_bytes(prefix))];
    stream.read_exact(&mut message).map_err(|e| e.to_string())?;
    Message::from_vec(&message).map_err(|e| e.to_string())
}

/// A query id that differs from one query to the next.
fn rand_id() -> u16 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(Instant::now().elapsed().as_nanos());
    hasher.finish() as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::rdata::{AAAA, NAPTR, SRV};
    use hickory_proto::rr::Record;

    #[test]
    fn hosts_lose_scheme_port_and_brackets() {
        assert_eq!(host_of("sip.example.com"), "sip.example.com");
        assert_eq!(host_of(" sip:sip.example.com:5061;transport=tls "), "sip.example.com");
        assert_eq!(host_of("[2001:db8::1]:5061"), "2001:db8::1");
        assert_eq!(host_of("2001:db8::1"), "2001:db8::1");
        assert_eq!(host_of("192.0.2.1:5060"), "192.0.2.1");
    }

    #[test]
    fn ip_addresses_and_no_servers_need_no_lookup() {
        let ns: Vec<IpAddr> = vec!["192.0.2.53".parse().unwrap()];
        let d = discover("192.0.2.1", Transport::Udp, &ns);
        assert!(d.steps.is_empty() && d.notes[0].contains("IP address"));
        let d = discover("sip.example.com", Transport::Udp, &[]);
        assert!(d.steps.is_empty() && d.notes[0].contains("No DNS servers"));
    }

    /// A DNS server on localhost with the records of an IMS-style domain
    /// that has no addresses of its own: NAPTR to TLS, SRV to a P-CSCF,
    /// AAAA for that.
    #[test]
    fn follows_naptr_srv_and_aaaa() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            while let Ok((n, from)) = socket.recv_from(&mut buffer) {
                let request = Message::from_vec(&buffer[..n]).unwrap();
                let q = request.queries[0].clone();
                let mut reply = Message::new(request.metadata.id, MessageType::Response, OpCode::Query);
                reply.add_query(q.clone());
                let domain = Name::from_ascii("ims.example.").unwrap();
                let srv = Name::from_ascii("_sips._tcp.ims.example.").unwrap();
                let pcscf = Name::from_ascii("pcscf.ims.example.").unwrap();
                let data = match (q.query_type(), q.name()) {
                    (RecordType::NAPTR, n) if *n == domain => vec![
                        RData::NAPTR(NAPTR::new(10, 50, b"s".to_vec().into(), b"SIPS+D2T".to_vec().into(), Vec::new().into(), srv.clone())),
                        RData::NAPTR(NAPTR::new(20, 50, b"s".to_vec().into(), b"SIP+D2U".to_vec().into(), Vec::new().into(), Name::from_ascii("_sip._udp.ims.example.").unwrap())),
                    ],
                    (RecordType::SRV, n) if *n == srv => vec![RData::SRV(SRV::new(0, 0, 5061, pcscf.clone()))],
                    (RecordType::AAAA, n) if *n == pcscf => vec![RData::AAAA(AAAA("2001:db8::1e".parse().unwrap()))],
                    _ => Vec::new(),
                };
                for d in data {
                    reply.add_answer(Record::from_rdata(q.name().clone(), 60, d));
                }
                socket.send_to(&reply.to_vec().unwrap(), from).unwrap();
            }
        });
        let servers = [SocketAddr::from(([127, 0, 0, 1], port))];
        let d = discover_via("ims.example", Transport::Tls, &servers);
        let queries: Vec<&str> = d.steps.iter().map(|s| s.query.as_str()).collect();
        assert_eq!(queries, ["NAPTR ims.example", "SRV _sips._tcp.ims.example", "AAAA pcscf.ims.example", "A pcscf.ims.example"]);
        assert_eq!(d.steps[1].answers, ["0 0 5061 pcscf.ims.example."]);
        assert_eq!(d.endpoints, ["[2001:db8::1e]:5061"]);
        assert!(d.notes.is_empty(), "{:?}", d.notes);
        // TCP isn't offered; its SRV name is guessed and has no records,
        // and the domain has no addresses of its own.
        let d = discover_via("ims.example", Transport::Tcp, &servers);
        assert_eq!(d.steps[1].query, "SRV _sip._tcp.ims.example");
        assert!(d.endpoints.is_empty());
        assert!(d.notes.iter().any(|n| n.contains("NAPTR offers TLS, UDP, not TCP")), "{:?}", d.notes);
    }
}
