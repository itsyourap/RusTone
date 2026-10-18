//! Spike configuration, a subset of the account and settings keys.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub account: Account,
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Account {
    pub server: String,
    pub proxy: String,
    pub domain: String,
    pub username: String,
    pub password: String,
    pub auth_id: String,
    pub display_name: String,
    pub transport: String,
    pub srtp: String,
    pub register_refresh: u32,
    pub keep_alive: u32,
    pub publish: bool,
    pub ice: bool,
    pub allow_rewrite: bool,
    pub disable_session_timer: bool,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            server: String::new(),
            proxy: String::new(),
            domain: String::new(),
            username: String::new(),
            password: String::new(),
            auth_id: String::new(),
            display_name: String::new(),
            transport: "udp".into(),
            srtp: String::new(),
            register_refresh: 300,
            keep_alive: 15,
            publish: false,
            ice: false,
            allow_rewrite: false,
            disable_session_timer: false,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub user_agent: String,
    pub rport: bool,
    pub source_port: u32,
    pub rtp_port_min: u32,
    pub rtp_port_max: u32,
    pub audio_codecs: String,
    pub vad: bool,
    pub ec: bool,
    pub log_file: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            user_agent: String::new(),
            rport: true,
            source_port: 0,
            rtp_port_min: 0,
            rtp_port_max: 0,
            audio_codecs: "PCMA/8000/1 PCMU/8000/1".into(),
            vad: false,
            ec: true,
            log_file: "rustone-cli.log".into(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

impl Account {
    /// `;transport=tcp|tls` suffix.
    pub fn transport_suffix(&self) -> &'static str {
        match self.transport.as_str() {
            "tcp" => ";transport=tcp",
            "tls" => ";transport=tls",
            _ => "",
        }
    }

    /// `<sip:user@domain;transport=..>`:
    /// the domain is only appended when `user` has no `@`.
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
        self.proxy
            .split_whitespace()
            .map(|p| format!("sip:{p}{};lr", self.transport_suffix()))
            .collect()
    }

    pub fn auth_user(&self) -> &str {
        if self.auth_id.is_empty() { &self.username } else { &self.auth_id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jf_account() -> Account {
        Account {
            server: "sip.example.com".into(),
            proxy: "192.168.29.1:5068".into(),
            domain: "192.168.29.1:5068".into(),
            username: "+910000000000@sip.example.com".into(),
            transport: "tls".into(),
            ..Account::default()
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
    }
}
