//! User settings, persisted as `settings.toml` in the config folder.
//!
//! Every field has a default, so a partial or older file loads and missing
//! keys take their defaults. Unknown keys are ignored.

use std::fmt;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::account::{Account, LocalAccount};

pub const FILE_NAME: &str = "settings.toml";

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ui: UiSettings,
    pub sip: SipSettings,
    pub calls: CallSettings,
    pub audio: AudioSettings,
    /// Id of the account in use (one account at a time).
    pub active_account: Option<u32>,
    pub accounts: Vec<Account>,
    pub local_account: LocalAccount,
}

/// Global SIP and media settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SipSettings {
    /// Empty: "RusTone/<version>".
    pub user_agent: String,
    pub rport: bool,
    pub source_port: u16,
    pub rtp_port_min: u16,
    pub rtp_port_max: u16,
    pub enable_stun: bool,
    /// STUN servers, separated by spaces, commas or semicolons.
    pub stun: String,
    pub dns_srv: bool,
    /// DNS servers for SRV lookups, separated like `stun`.
    pub dns_srv_ns: String,
    /// Re-create transports and re-register when the network changes.
    pub network_changes: bool,
    /// Enabled audio codecs in priority order, space-separated.
    pub audio_codecs: String,
    /// Voice activity detection: send silence as comfort noise.
    pub vad: bool,
    /// Echo cancellation (turns stereo codecs mono).
    pub ec: bool,
    pub opus_stereo: bool,
    /// Answer incoming calls with our codec order.
    pub force_codec: bool,
    /// URL of a users directory to keep the contacts in sync with;
    /// empty for none.
    pub users_directory: String,
    /// Write the full SIP log (holds numbers and addresses).
    pub enable_log: bool,
}

impl Default for SipSettings {
    fn default() -> Self {
        Self {
            user_agent: String::new(),
            rport: true,
            source_port: 0,
            rtp_port_min: 0,
            rtp_port_max: 0,
            enable_stun: false,
            stun: String::new(),
            dns_srv: false,
            dns_srv_ns: String::new(),
            network_changes: true,
            audio_codecs: "PCMA/8000/1 PCMU/8000/1".into(),
            vad: false,
            ec: true,
            opus_stereo: false,
            force_codec: false,
            users_directory: String::new(),
            enable_log: false,
        }
    }
}

/// Sound devices and levels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// Device names as pjsua reports them; empty for the system default.
    pub input_device: String,
    pub output_device: String,
    pub ring_device: String,
    /// Levels 0..=100.
    pub volume_output: u8,
    pub volume_input: u8,
    pub volume_ring: u8,
    /// Boost the microphone above the middle of its slider.
    pub mic_amplification: bool,
    /// Set the microphone level in software only.
    pub sw_level_adjustment: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            input_device: String::new(),
            output_device: String::new(),
            ring_device: String::new(),
            volume_output: 100,
            volume_input: 100,
            volume_ring: 100,
            mic_amplification: false,
            sw_level_adjustment: false,
        }
    }
}

/// How DTMF digits are sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DtmfMethod {
    /// RFC 2833, falling back to in-band.
    #[default]
    Auto,
    InBand,
    Rfc2833,
    SipInfo,
}

/// Call behaviour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CallSettings {
    pub dtmf_method: DtmfMethod,
    /// Play DTMF tones and in-call sounds locally.
    pub local_dtmf: bool,
    /// Accept a second call while in a call.
    pub call_waiting: bool,
    /// Most simultaneous calls; 0 for no limit.
    pub max_concurrent_calls: u32,
    /// Bring the window forward for incoming calls.
    pub bring_to_front: bool,
    /// Closing the ringing screen declines instead of ignoring.
    pub no_ignore: bool,
    /// Dialled before a ringing contact's number to pick up its call;
    /// empty turns pickup off.
    pub pickup_code: String,
}

impl Default for CallSettings {
    fn default() -> Self {
        Self {
            dtmf_method: DtmfMethod::Auto,
            local_dtmf: true,
            call_waiting: true,
            max_concurrent_calls: 0,
            bring_to_front: true,
            no_ignore: false,
            pickup_code: "**".into(),
        }
    }
}

/// Splits a server list (`;`, `,` or whitespace separated).
pub fn split_list(list: &str) -> Vec<String> {
    list.split(|c: char| c == ';' || c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub theme: ThemePreference,
    /// Draw RusTone's own title bar instead of the system frame.
    pub custom_titlebar: bool,
    /// Closing the window keeps RusTone running in the tray.
    pub close_to_tray: bool,
    /// Start in the tray without opening the window.
    pub start_minimized: bool,
    pub window: WindowGeometry,
    /// egui zoom factor (Ctrl +/-).
    pub zoom: f32,
    /// The default window height already applied to a saved size (see
    /// [`Settings::grow_window_to_default`]); 0 in files from before it.
    #[serde(default)]
    pub applied_default_height: f32,
}

/// The main window's default size, logical pixels: tall enough for the
/// call screen without scrolling.
pub const DEFAULT_WINDOW_SIZE: [f32; 2] = [380.0, 780.0];

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            custom_titlebar: true,
            close_to_tray: true,
            start_minimized: false,
            window: WindowGeometry::default(),
            zoom: 1.0,
            applied_default_height: DEFAULT_WINDOW_SIZE[1],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    /// Follow the operating system's light/dark appearance.
    #[default]
    System,
    Light,
    Dark,
}

/// The main window's last size and place, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowGeometry {
    pub size: Option<[f32; 2]>,
    pub position: Option<[f32; 2]>,
    pub maximized: bool,
}

#[derive(Debug)]
pub enum LoadError {
    Io(io::Error),
    Parse(toml::de::Error),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "cannot read settings: {e}"),
            Self::Parse(e) => write!(f, "settings file is invalid: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl Settings {
    pub fn active(&self) -> Option<&Account> {
        let id = self.active_account?;
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn account(&self, id: u32) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    /// Adds `account` with a fresh id and returns the id. The first account
    /// becomes the active one.
    pub fn add_account(&mut self, mut account: Account) -> u32 {
        let id = self.accounts.iter().map(|a| a.id).max().map_or(1, |m| m + 1);
        account.id = id;
        self.accounts.push(account);
        if self.active_account.is_none() {
            self.active_account = Some(id);
        }
        id
    }

    /// Replaces the account with the same id. Returns false if there is none.
    pub fn update_account(&mut self, account: Account) -> bool {
        match self.accounts.iter_mut().find(|a| a.id == account.id) {
            Some(slot) => {
                *slot = account;
                true
            }
            None => false,
        }
    }

    /// Removes an account; if it was active, the first remaining one takes over.
    pub fn remove_account(&mut self, id: u32) -> Option<Account> {
        let index = self.accounts.iter().position(|a| a.id == id)?;
        let removed = self.accounts.remove(index);
        if self.active_account == Some(id) {
            self.active_account = self.accounts.first().map(|a| a.id);
        }
        Some(removed)
    }

    /// Once per new default height: a saved window shorter than it grows to
    /// it (a window made smaller afterwards stays so). Returns whether the
    /// settings changed.
    pub fn grow_window_to_default(&mut self) -> bool {
        let default = DEFAULT_WINDOW_SIZE[1];
        if self.ui.applied_default_height >= default {
            return false;
        }
        self.ui.applied_default_height = default;
        if let Some(size) = &mut self.ui.window.size
            && size[1] < default
        {
            size[1] = default;
        }
        true
    }

    /// Reads `path`. A missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(LoadError::Io(e)),
        }
    }

    pub fn from_toml(text: &str) -> Result<Self, LoadError> {
        toml::from_str(text).map_err(LoadError::Parse)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("settings always serialize")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut s = Settings::default();
        s.ui.theme = ThemePreference::Dark;
        s.ui.window = WindowGeometry { size: Some([400.0, 700.0]), position: Some([10.0, 20.0]), maximized: false };
        s.ui.zoom = 1.25;
        assert_eq!(Settings::from_toml(&s.to_toml()).unwrap(), s);
    }

    #[test]
    fn partial_file_takes_defaults() {
        let s = Settings::from_toml("[ui]\ntheme = \"light\"\nsomething_new = 1\n").unwrap();
        assert_eq!(s.ui.theme, ThemePreference::Light);
        assert!(s.ui.close_to_tray);
        assert_eq!(s.ui.zoom, 1.0);
    }

    #[test]
    fn missing_file_is_default() {
        let s = Settings::load(Path::new("definitely/not/here/settings.toml")).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn accounts_add_update_remove() {
        let mut s = Settings::default();
        let a = s.add_account(Account { username: "a".into(), ..Account::default() });
        let b = s.add_account(Account { username: "b".into(), ..Account::default() });
        assert_eq!((a, b), (1, 2));
        assert_eq!(s.active().unwrap().username, "a");
        let mut edited = s.account(b).unwrap().clone();
        edited.label = "B".into();
        assert!(s.update_account(edited));
        s.remove_account(a);
        assert_eq!(s.active().unwrap().label, "B");
        s.remove_account(b);
        assert_eq!(s.active_account, None);
        // Ids aren't reused while accounts exist, so stored passwords don't mix.
        let c = s.add_account(Account::default());
        assert_eq!(c, 1);
    }

    #[test]
    fn accounts_round_trip_without_passwords() {
        let mut s = Settings::default();
        s.add_account(Account { username: "1001".into(), password: "pw".into(), ..Account::default() });
        let text = s.to_toml();
        assert!(!text.contains("pw\""), "{text}");
        let back = Settings::from_toml(&text).unwrap();
        assert_eq!(back.active().unwrap().username, "1001");
        assert!(back.active().unwrap().password.is_empty());
    }

    #[test]
    fn server_lists_split_on_separators() {
        assert_eq!(split_list("stun.a.com:3478; stun.b.com,stun.c.com  "), ["stun.a.com:3478", "stun.b.com", "stun.c.com"]);
        assert!(split_list(" ").is_empty());
    }

    #[test]
    fn call_settings_have_defaults() {
        let c = Settings::from_toml("[calls]
dtmf_method = \"sip-info\"").unwrap().calls;
        assert_eq!(c.dtmf_method, DtmfMethod::SipInfo);
        assert!(c.local_dtmf && c.call_waiting && c.bring_to_front && !c.no_ignore);
        assert_eq!(c.max_concurrent_calls, 0);
    }

    #[test]
    fn audio_devices_default_to_the_system() {
        let a = Settings::from_toml("[audio]
output_device = \"Speakers (Realtek)\"").unwrap().audio;
        assert_eq!(a.output_device, "Speakers (Realtek)");
        assert!(a.input_device.is_empty() && a.ring_device.is_empty());
        assert_eq!((a.volume_output, a.volume_input, a.volume_ring), (100, 100, 100));
    }

    #[test]
    fn saved_windows_grow_once_to_a_taller_default() {
        let mut old = Settings::from_toml("[ui.window]
size = [340.0, 709.6]
").unwrap();
        assert!(old.grow_window_to_default());
        assert_eq!(old.ui.window.size, Some([340.0, DEFAULT_WINDOW_SIZE[1]]));
        old.ui.window.size = Some([340.0, 600.0]);
        assert!(!old.grow_window_to_default(), "a later choice stays");
        let mut fresh = Settings::default();
        assert!(!fresh.grow_window_to_default());
        let mut tall = Settings::from_toml("[ui.window]
size = [400.0, 900.0]
").unwrap();
        tall.grow_window_to_default();
        assert_eq!(tall.ui.window.size, Some([400.0, 900.0]));
    }

    #[test]
    fn invalid_file_is_an_error() {
        assert!(matches!(Settings::from_toml("[ui]\ntheme = 5"), Err(LoadError::Parse(_))));
    }
}
