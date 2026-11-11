//! DNS for SIP: which DNS servers pjsua resolves the server with (the
//! configured ones, or the system's for SRV lookups), and keeping pjsua
//! from SRV lookups when they're off.

use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rustone_core::settings::{MAX_DNS_SERVERS, SipSettings};

/// The DNS servers pjsua resolves SIP addresses with: the configured
/// ones, else the system's when SRV lookups are on. None leaves it to the
/// system resolver, which looks up A and AAAA records only.
pub fn nameservers(sip: &SipSettings) -> Vec<String> {
    let servers = sip.dns_servers();
    if !servers.is_empty() {
        servers
    } else if sip.dns_srv {
        system_dns_servers()
    } else {
        Vec::new()
    }
}

/// Whether server and proxy addresses get an explicit port: pjsua looks a
/// host up by SRV only when it has none, and does whenever it has DNS
/// servers, so with configured servers and SRV off they get one.
pub fn pin_ports(sip: &SipSettings) -> bool {
    !sip.dns_srv && !sip.dns_servers().is_empty()
}

/// `host` with SIP's default port added when it has none (and isn't an
/// IP address, which is never looked up).
pub fn with_default_port(host: &str, tls: bool) -> String {
    let port = if tls { 5061 } else { 5060 };
    let has_port = if let Some(rest) = host.strip_prefix('[') {
        rest.contains("]:")
    } else {
        host.contains(':')
    };
    if host.is_empty() || has_port || host.parse::<IpAddr>().is_ok() || host.trim_matches(['[', ']']).parse::<IpAddr>().is_ok() {
        host.to_owned()
    } else {
        format!("{host}:{port}")
    }
}

/// How long the system's DNS servers are trusted before asking again:
/// `nameservers` runs every frame, and a change restarts the engine.
const SYSTEM_DNS_FOR: Duration = Duration::from_secs(30);

/// The system's DNS servers, those of the default network interface (or
/// of every interface when it has none).
pub fn system_dns_servers() -> Vec<String> {
    static CACHE: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, servers)) = cache.as_ref()
        && at.elapsed() < SYSTEM_DNS_FOR
    {
        return servers.clone();
    }
    let mut found = netdev::get_default_interface().map(|i| i.dns_servers).unwrap_or_default();
    if found.is_empty() {
        found = netdev::get_interfaces().into_iter().flat_map(|i| i.dns_servers).collect();
    }
    let servers = usable_servers(found);
    *cache = Some((Instant::now(), servers.clone()));
    servers
}

/// Servers pjsua can use, in order, without repeats: not unspecified, and
/// not IPv6 link-local (which needs an interface pjsua can't be given).
fn usable_servers(found: Vec<IpAddr>) -> Vec<String> {
    let mut servers: Vec<IpAddr> = Vec::new();
    for ip in found {
        let link_local = matches!(ip, IpAddr::V6(v6) if v6.segments()[0] & 0xffc0 == 0xfe80);
        if !ip.is_unspecified() && !link_local && !servers.contains(&ip) {
            servers.push(ip);
        }
    }
    servers.into_iter().take(MAX_DNS_SERVERS).map(|ip| ip.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_servers_win() {
        let mut sip = SipSettings { dns_srv_ns: "192.168.29.1".into(), ..SipSettings::default() };
        assert_eq!(nameservers(&sip), ["192.168.29.1"]);
        assert!(pin_ports(&sip), "SRV off: explicit ports");
        sip.dns_srv = true;
        assert_eq!(nameservers(&sip), ["192.168.29.1"]);
        assert!(!pin_ports(&sip));
        assert!(nameservers(&SipSettings::default()).is_empty(), "the system resolver");
        assert!(!pin_ports(&SipSettings::default()));
    }

    #[test]
    fn default_ports() {
        assert_eq!(with_default_port("sip.example.com", false), "sip.example.com:5060");
        assert_eq!(with_default_port("sip.example.com", true), "sip.example.com:5061");
        assert_eq!(with_default_port("sip.example.com:5080", true), "sip.example.com:5080");
        assert_eq!(with_default_port("192.0.2.1", false), "192.0.2.1");
        assert_eq!(with_default_port("2001:db8::1", false), "2001:db8::1");
        assert_eq!(with_default_port("[2001:db8::1]", false), "[2001:db8::1]");
        assert_eq!(with_default_port("[2001:db8::1]:5062", false), "[2001:db8::1]:5062");
        assert_eq!(with_default_port("", false), "");
    }

    #[test]
    fn usable_servers_skip_link_local_and_repeats() {
        let found = ["fe80::1", "192.168.29.1", "0.0.0.0", "192.168.29.1", "2001:db8::53", "127.0.0.53", "1.1.1.1", "8.8.8.8"];
        let found = found.iter().map(|s| s.parse().unwrap()).collect();
        assert_eq!(usable_servers(found), ["192.168.29.1", "2001:db8::53", "127.0.0.53", "1.1.1.1"]);
    }
}
