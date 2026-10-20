//! Engine and account configuration: the settings that
//! reach pjsua, and the rules for turning them into SIP URIs.

use std::path::PathBuf;

/// Global settings, applied when pjsua starts.
/// Changing any of them restarts the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    /// `User-Agent` header.
    pub user_agent: String,
    /// Add `rport` to Via.
    pub rport: bool,
    /// Local UDP port, 0 for any.
    pub source_port: u16,
    /// RTP port range, 0 for pjsua's default.
    pub rtp_port_min: u16,
    pub rtp_port_max: u16,
    /// STUN servers; empty disables STUN.
    pub stun_servers: Vec<String>,
    /// DNS servers for SRV lookups; empty uses the system resolver.
    pub nameservers: Vec<String>,
    /// Enabled audio codecs in priority order, e.g. `AMR-WB/16000/1`.
    pub audio_codecs: Vec<String>,
    /// Voice activity detection.
    pub vad: bool,
    /// Echo cancellation.
    pub ec: bool,
    /// Opus in stereo; disables EC.
    pub opus_stereo: bool,
    /// Listen on 5060/5061 for the local "call by IP" account.
    pub local_account: bool,
    /// Full pjsua log including SIP messages.
    pub log_file: Option<PathBuf>,
    /// Use no sound device (tests, headless use).
    pub null_audio: bool,
    pub video: crate::video::VideoConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            user_agent: String::new(),
            rport: true,
            source_port: 0,
            rtp_port_min: 0,
            rtp_port_max: 0,
            stun_servers: Vec::new(),
            nameservers: Vec::new(),
            audio_codecs: vec!["PCMA/8000/1".into(), "PCMU/8000/1".into()],
            vad: false,
            ec: true,
            opus_stereo: false,
            local_account: false,
            log_file: None,
            null_audio: false,
            video: crate::video::VideoConfig::default(),
        }
    }
}

impl EngineConfig {
    /// Media clock rate: the highest codec clock rate (Opus
    /// counts as 24000), at least 8000.
    pub fn clock_rate(&self) -> u32 {
        self.audio_codecs
            .iter()
            .map(|c| {
                let mut parts = c.split('/');
                let name = parts.next().unwrap_or_default();
                if name.eq_ignore_ascii_case("opus") {
                    24000
                } else {
                    parts.next().and_then(|r| r.parse().ok()).unwrap_or(8000)
                }
            })
            .fold(8000, u32::max)
    }

    /// Channel count: 2 for Opus stereo, or for a `/2` codec
    /// when echo cancellation is off; otherwise 1.
    pub fn channel_count(&self) -> u32 {
        if self.opus_stereo {
            return 2;
        }
        let stereo_codec = self.audio_codecs.iter().any(|c| {
            let mut parts = c.split('/');
            let is_opus = parts.next().is_some_and(|n| n.eq_ignore_ascii_case("opus"));
            !is_opus && parts.nth(1) == Some("2")
        });
        if !self.ec && stereo_codec { 2 } else { 1 }
    }

    /// Echo canceller tail in ms: 20 when EC is on (and Opus isn't stereo).
    pub fn ec_tail_ms(&self) -> u32 {
        if self.ec && !self.opus_stereo { 20 } else { 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    #[default]
    Udp,
    Tcp,
    Tls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Srtp {
    #[default]
    Disabled,
    Optional,
    Mandatory,
    /// DTLS-SRTP only.
    Dtls,
    /// DTLS-SRTP, falling back to SDES.
    DtlsSdes,
}

/// A SIP account.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountConfig {
    /// Registrar; empty means no registration.
    pub server: String,
    /// Outbound proxies.
    pub proxies: Vec<String>,
    /// Appended to user parts without `@`.
    pub domain: String,
    pub username: String,
    pub password: String,
    /// Authentication user; empty uses `username`.
    pub auth_id: String,
    pub display_name: String,
    pub transport: Transport,
    pub srtp: Srtp,
    /// Public address for SDP/Contact when STUN is off.
    pub public_addr: String,
    /// Registration expiry in seconds.
    pub register_refresh: u32,
    /// Keep-alive interval in seconds.
    pub keep_alive: u32,
    /// Publish presence.
    pub publish: bool,
    pub ice: bool,
    /// Rewrite Via/Contact/SDP with the address the server sees.
    pub allow_rewrite: bool,
    pub disable_session_timer: bool,
    /// Subscribe to message-waiting indications (on
    /// when a voicemail number is set).
    pub mwi: bool,
}

impl Default for AccountConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            proxies: Vec::new(),
            domain: String::new(),
            username: String::new(),
            password: String::new(),
            auth_id: String::new(),
            display_name: String::new(),
            transport: Transport::Udp,
            srtp: Srtp::Disabled,
            public_addr: String::new(),
            register_refresh: 300,
            keep_alive: 15,
            publish: false,
            ice: false,
            allow_rewrite: false,
            disable_session_timer: false,
            mwi: false,
        }
    }
}

impl AccountConfig {
    /// `;transport=tcp|tls`.
    pub fn transport_suffix(&self) -> &'static str {
        match self.transport {
            Transport::Udp => "",
            Transport::Tcp => ";transport=tcp",
            Transport::Tls => ";transport=tls",
        }
    }

    /// `<sip:user@domain;transport=..>`: the
    /// domain is only appended when `user` has no `@`.
    pub fn sip_uri(&self, user: &str) -> String {
        let mut uri = if user.to_ascii_lowercase().contains("sip:") {
            user.to_owned()
        } else {
            format!("sip:{user}")
        };
        if !uri.contains('@') && !self.domain.is_empty() {
            uri.push('@');
            uri.push_str(&self.domain);
        }
        uri.push_str(self.transport_suffix());
        format!("<{uri}>")
    }

    /// Account identity: `"Display" <sip:..>`.
    pub fn id_uri(&self) -> String {
        let uri = self.sip_uri(&self.username);
        if self.display_name.is_empty() {
            uri
        } else {
            format!("\"{}\" {uri}", self.display_name)
        }
    }

    pub fn reg_uri(&self) -> Option<String> {
        (!self.server.is_empty()).then(|| format!("sip:{}{}", self.server, self.transport_suffix()))
    }

    /// Outbound proxies as loose-routing URIs.
    pub fn proxy_uris(&self) -> Vec<String> {
        self.proxies.iter().map(|p| format!("sip:{p}{};lr", self.transport_suffix())).collect()
    }

    pub fn auth_user(&self) -> &str {
        if self.auth_id.is_empty() { &self.username } else { &self.auth_id }
    }
}

/// The local "call by IP" account.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LocalAccountConfig {
    pub display_name: String,
    /// Optional user part; empty gives `<sip:host>`.
    pub username: String,
    /// Host for the identity; empty uses the local address.
    pub domain: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jf_account() -> AccountConfig {
        AccountConfig {
            server: "sip.example.com".into(),
            proxies: vec!["192.168.29.1:5068".into()],
            domain: "192.168.29.1:5068".into(),
            username: "+910000000000@sip.example.com".into(),
            transport: Transport::Tls,
            ..AccountConfig::default()
        }
    }

    /// Expected values come from a working client's log for the same account.
    #[test]
    fn uris_match_the_account() {
        let acc = jf_account();
        assert_eq!(acc.id_uri(), "<sip:+910000000000@sip.example.com;transport=tls>");
        assert_eq!(acc.reg_uri().unwrap(), "sip:sip.example.com;transport=tls");
        assert_eq!(acc.proxy_uris(), ["sip:192.168.29.1:5068;transport=tls;lr"]);
        assert_eq!(acc.sip_uri("1234"), "<sip:1234@192.168.29.1:5068;transport=tls>");
        assert_eq!(acc.auth_user(), acc.username);
    }

    #[test]
    fn display_name_and_udp() {
        let acc = AccountConfig {
            username: "alice".into(),
            domain: "example.com".into(),
            display_name: "Alice".into(),
            ..AccountConfig::default()
        };
        assert_eq!(acc.id_uri(), "\"Alice\" <sip:alice@example.com>");
        assert_eq!(acc.reg_uri(), None);
    }

    #[test]
    fn media_follows_the_codecs() {
        let mut c = EngineConfig { audio_codecs: vec!["AMR-WB/16000/1".into(), "AMR/8000/1".into()], ..EngineConfig::default() };
        assert_eq!(c.clock_rate(), 16000);
        assert_eq!(c.channel_count(), 1);
        c.audio_codecs = vec!["opus/48000/2".into(), "PCMA/8000/1".into()];
        assert_eq!(c.clock_rate(), 24000);
        assert_eq!(c.channel_count(), 1);
        c.audio_codecs = vec!["L16/44100/2".into()];
        assert_eq!(c.channel_count(), 1, "EC on keeps mono");
        c.ec = false;
        assert_eq!(c.channel_count(), 2);
        c.opus_stereo = true;
        c.ec = true;
        assert_eq!(c.ec_tail_ms(), 0);
        assert_eq!(c.channel_count(), 2);
        assert_eq!(EngineConfig { audio_codecs: vec![], ..EngineConfig::default() }.clock_rate(), 8000);
    }
}
