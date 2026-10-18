//! SIP accounts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Udp,
    Tcp,
    Tls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SrtpMode {
    #[default]
    Disabled,
    Optional,
    Mandatory,
    Dtls,
    DtlsSdes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    /// Stable identifier (also names the stored password).
    pub id: u32,
    /// Name shown in menus; falls back to display name / username.
    pub label: String,
    /// Registrar host[:port]; empty means no registration.
    pub server: String,
    /// Outbound proxies, space-separated host[:port].
    pub proxy: String,
    /// Domain appended to numbers that have no `@`.
    pub domain: String,
    pub username: String,
    /// Kept in the OS credential store, never in settings.toml.
    #[serde(skip)]
    pub password: String,
    /// Authentication user if different from `username`.
    pub auth_id: String,
    pub display_name: String,
    /// Prefix added to dialled numbers.
    pub dialing_prefix: String,
    /// Dial plan rules, `|`-separated (applied in Phase 9).
    pub dial_plan: String,
    /// Send `Privacy: id` with an anonymous From (Phase 9).
    pub hide_cid: bool,
    /// Number dialled by the voicemail button; enables MWI.
    pub voicemail_number: String,
    pub srtp: SrtpMode,
    pub transport: Transport,
    /// Public address to advertise when STUN is off.
    pub public_addr: String,
    /// Registration expiry, seconds.
    pub register_refresh: u32,
    /// Keep-alive interval, seconds.
    pub keep_alive: u32,
    /// Publish presence.
    pub publish: bool,
    pub ice: bool,
    pub allow_rewrite: bool,
    pub disable_session_timer: bool,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            id: 0,
            label: String::new(),
            server: String::new(),
            proxy: String::new(),
            domain: String::new(),
            username: String::new(),
            password: String::new(),
            auth_id: String::new(),
            display_name: String::new(),
            dialing_prefix: String::new(),
            dial_plan: String::new(),
            hide_cid: false,
            voicemail_number: String::new(),
            srtp: SrtpMode::Disabled,
            transport: Transport::Udp,
            public_addr: String::new(),
            register_refresh: 300,
            keep_alive: 15,
            publish: false,
            ice: false,
            allow_rewrite: false,
            disable_session_timer: false,
        }
    }
}

impl Account {
    /// The name to show: label, else display name, else username, else server.
    pub fn title(&self) -> &str {
        [&self.label, &self.display_name, &self.username, &self.server]
            .into_iter()
            .find(|s| !s.trim().is_empty())
            .map_or("Unnamed account", |s| s.as_str())
    }

    /// The key under which the password is stored.
    pub fn secret_key(&self) -> String {
        format!("account-{}", self.id)
    }

    /// Whether enough is filled in to try registering or calling.
    pub fn is_usable(&self) -> bool {
        !self.username.trim().is_empty() && (!self.server.trim().is_empty() || !self.domain.trim().is_empty())
    }

    pub fn proxies(&self) -> Vec<String> {
        self.proxy.split_whitespace().map(str::to_owned).collect()
    }
}

/// The local "call by IP address" account.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalAccount {
    pub enabled: bool,
    pub display_name: String,
    pub username: String,
    pub domain: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_falls_back() {
        let mut a = Account { server: "sip.example.com".into(), ..Account::default() };
        assert_eq!(a.title(), "sip.example.com");
        a.username = "1001".into();
        assert_eq!(a.title(), "1001");
        a.label = "Office".into();
        assert_eq!(a.title(), "Office");
        assert_eq!(Account::default().title(), "Unnamed account");
    }

    #[test]
    fn password_is_never_serialized() {
        let a = Account { password: "secret".into(), ..Account::default() };
        let text = toml::to_string(&a).unwrap();
        assert!(!text.contains("secret"), "{text}");
        let back: Account = toml::from_str(&text).unwrap();
        assert!(back.password.is_empty());
    }

    #[test]
    fn enums_use_kebab_case() {
        let a: Account = toml::from_str("transport = \"tls\"\nsrtp = \"dtls-sdes\"").unwrap();
        assert_eq!(a.transport, Transport::Tls);
        assert_eq!(a.srtp, SrtpMode::DtlsSdes);
    }

    #[test]
    fn usable_needs_user_and_host() {
        assert!(!Account::default().is_usable());
        assert!(Account { username: "1001".into(), domain: "pbx".into(), ..Account::default() }.is_usable());
        assert_eq!(Account { proxy: " a:5060  b ".into(), ..Account::default() }.proxies(), ["a:5060", "b"]);
    }
}
