//! Exporting and importing RusTone's configuration as a JSON file: every
//! setting (codec order, speed dial, hooks, …) and the accounts chosen,
//! with their passwords in plain text.
//!
//! ```json
//! {
//!   "format": "rustone-config",
//!   "version": 1,
//!   "exported_by": "RusTone 1.0.0",
//!   "settings": { "ui": { … }, "sip": { "audio_codecs": "…" }, … },
//!   "accounts": [ { "label": "Office", "server": "…", "password": "…", "active": true } ]
//! }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::account::Account;
use crate::settings::Settings;

/// What the file says it is.
pub const FORMAT: &str = "rustone-config";
/// The newest version this build reads and the one it writes.
pub const VERSION: u32 = 1;

/// The file's contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigFile {
    pub format: String,
    pub version: u32,
    /// The app and version that wrote it, for people reading the file.
    #[serde(default)]
    pub exported_by: String,
    /// The settings as written in the file: an import changes only the
    /// ones it has. Its accounts are in `accounts` instead.
    #[serde(default)]
    pub settings: Value,
    #[serde(default)]
    pub accounts: Vec<ExportedAccount>,
}

/// An account with its password.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportedAccount {
    #[serde(flatten)]
    pub account: Account,
    /// In plain text: the file is for moving accounts to another computer.
    #[serde(default)]
    pub password: String,
    /// The account in use when it was exported.
    #[serde(default)]
    pub active: bool,
    /// The account as written in the file, so updating an existing account
    /// changes only the fields it has.
    #[serde(skip)]
    pub raw: Value,
}

/// `settings` and the accounts with `account_ids` (their passwords held in
/// memory) as the file's JSON.
pub fn export(settings: &Settings, account_ids: &[u32], exported_by: &str) -> String {
    let accounts = settings
        .accounts
        .iter()
        .filter(|a| account_ids.contains(&a.id))
        .map(|a| ExportedAccount {
            account: Account { password: String::new(), ..a.clone() },
            password: a.password.clone(),
            active: settings.active_account == Some(a.id),
            raw: Value::Null,
        })
        .collect();
    let mut values = serde_json::to_value(settings).expect("settings serialize");
    strip(&mut values, &[&["accounts"], &["active_account"]]);
    let file = ConfigFile { format: FORMAT.into(), version: VERSION, exported_by: exported_by.into(), settings: values, accounts };
    serde_json::to_string_pretty(&file).expect("settings serialize")
}

/// Reads a file's JSON, checking it is a RusTone configuration this build
/// understands.
pub fn parse(text: &str) -> Result<ConfigFile, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("not a JSON file: {e}"))?;
    match value.get("format").and_then(|f| f.as_str()) {
        Some(FORMAT) => {}
        _ => return Err("not a RusTone configuration file".into()),
    }
    let version = value.get("version").and_then(serde_json::Value::as_u64).unwrap_or(0);
    if version == 0 || version > u64::from(VERSION) {
        return Err(format!("made by a newer RusTone (format version {version})"));
    }
    let mut file: ConfigFile = serde_json::from_value(value.clone()).map_err(|e| format!("can't read it: {e}"))?;
    if file.settings.is_null() {
        // No settings: an import of accounts only.
        file.settings = Value::Object(serde_json::Map::new());
    }
    if !file.settings.is_object() {
        return Err("can't read it: \"settings\" isn't an object".into());
    }
    // The settings it has must be of the right kinds.
    let mut check = serde_json::to_value(Settings::default()).expect("settings serialize");
    merge(&mut check, &file.settings);
    serde_json::from_value::<Settings>(check).map_err(|e| format!("can't read its settings: {e}"))?;
    for (exported, raw) in file.accounts.iter_mut().zip(value["accounts"].as_array().into_iter().flatten()) {
        exported.raw = raw.clone();
    }
    Ok(file)
}

/// Lays `overlay` over `base`: objects key by key, at every depth;
/// anything else (numbers, strings, lists) replaces what was there.
fn merge(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                merge(base.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

/// Removes the keys at `paths` (e.g. `["ui", "window"]`) from `value`.
fn strip(value: &mut Value, paths: &[&[&str]]) {
    for path in paths {
        strip_path(value, path);
    }
}

fn strip_path(value: &mut Value, path: &[&str]) {
    match path {
        [] => {}
        [last] => {
            if let Some(object) = value.as_object_mut() {
                object.remove(*last);
            }
        }
        [first, rest @ ..] => {
            if let Some(next) = value.get_mut(*first) {
                strip_path(next, rest);
            }
        }
    }
}

/// Whether two accounts are the same one: same user on the same server and
/// domain (ignoring case and spaces).
pub fn same_account(a: &Account, b: &Account) -> bool {
    let eq = |x: &str, y: &str| x.trim().eq_ignore_ascii_case(y.trim());
    eq(&a.username, &b.username) && eq(&a.server, &b.server) && eq(&a.domain, &b.domain)
}

/// What importing `file` with the accounts at `selected` (indexes into
/// `file.accounts`) does to `current`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportPlan {
    /// The new settings, with `current`'s accounts and window still in.
    pub settings: Settings,
    /// Accounts to save, with their passwords: `id` 0 for new ones, else
    /// the id of the existing account they replace.
    pub accounts: Vec<Account>,
    /// Index into `accounts` of the one to make active, when the file's
    /// active account is among them or none is active yet.
    pub activate: Option<usize>,
}

/// Plans an import: the settings the file has replace `current`'s and the
/// ones it hasn't stay, as do the accounts (the chosen ones are added, or
/// update the matching one, again only in the fields the file has) and
/// the window's place on this screen.
pub fn plan_import(current: &Settings, file: &ConfigFile, selected: &[usize]) -> ImportPlan {
    let mut overlay = file.settings.clone();
    strip(&mut overlay, &[&["accounts"], &["active_account"], &["ui", "window"]]);
    let mut values = serde_json::to_value(current).expect("settings serialize");
    merge(&mut values, &overlay);
    let mut settings: Settings = serde_json::from_value(values).unwrap_or_else(|_| current.clone());
    // Passwords aren't serialized: the accounts come back as they were.
    settings.accounts = current.accounts.clone();
    settings.active_account = current.active_account;

    let mut accounts = Vec::new();
    let mut activate = None;
    for (_, exported) in file.accounts.iter().enumerate().filter(|(i, _)| selected.contains(i)) {
        let account = match current.accounts.iter().find(|a| same_account(a, &exported.account)) {
            Some(mine) => {
                let mut raw = exported.raw.clone();
                strip(&mut raw, &[&["id"], &["password"], &["active"]]);
                let mut values = serde_json::to_value(mine).expect("account serializes");
                merge(&mut values, &raw);
                let merged: Account = serde_json::from_value(values).unwrap_or_else(|_| exported.account.clone());
                // A file without the password keeps the one stored here.
                let password = if exported.raw.get("password").is_some() { exported.password.clone() } else { mine.password.clone() };
                Account { id: mine.id, password, ..merged }
            }
            None => Account { id: 0, password: exported.password.clone(), ..exported.account.clone() },
        };
        if exported.active || (activate.is_none() && current.active_account.is_none()) {
            activate = Some(accounts.len());
        }
        accounts.push(account);
    }
    ImportPlan { settings, accounts, activate }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Transport;

    fn sample() -> Settings {
        let mut s = Settings::default();
        s.sip.audio_codecs = "opus/48000/2 AMR-WB/16000/1 PCMA/8000/1".into();
        s.ui.window.size = Some([400.0, 700.0]);
        s.add_account(Account { label: "Office".into(), server: "sip.example.com".into(), username: "1001".into(), password: "secret".into(), transport: Transport::Tls, ..Account::default() });
        s.add_account(Account { label: "Home".into(), server: "pbx.example.net".into(), username: "2002".into(), password: "hunter2".into(), ..Account::default() });
        s.active_account = Some(2);
        s
    }

    #[test]
    fn exports_settings_and_the_chosen_accounts() {
        let s = sample();
        let text = export(&s, &[2], "RusTone test");
        let file = parse(&text).unwrap();
        assert_eq!(file.settings["sip"]["audio_codecs"], "opus/48000/2 AMR-WB/16000/1 PCMA/8000/1", "codec order");
        assert!(file.settings.get("accounts").is_none() && file.settings.get("active_account").is_none());
        assert_eq!(file.accounts.len(), 1);
        assert_eq!(file.accounts[0].account.label, "Home");
        assert_eq!(file.accounts[0].password, "hunter2", "plain-text password");
        assert!(file.accounts[0].active);
        assert!(text.contains("\"password\": \"hunter2\""));
        assert!(!text.contains("secret"), "unchosen accounts stay out");
        // Settings only.
        assert!(parse(&export(&s, &[], "RusTone test")).unwrap().accounts.is_empty());
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse("not json").unwrap_err().contains("JSON"));
        assert!(parse("{\"format\": \"other\"}").unwrap_err().contains("not a RusTone"));
        assert!(parse("{\"format\": \"rustone-config\", \"version\": 99, \"settings\": {}}").unwrap_err().contains("newer"));
        assert!(parse("{\"format\": \"rustone-config\", \"version\": 1, \"settings\": {\"sip\": {\"rport\": \"yes\"}}}").unwrap_err().contains("settings"), "wrong kinds");
        let file = parse("{\"format\": \"rustone-config\", \"version\": 1}").unwrap();
        assert!(file.accounts.is_empty());
    }

    #[test]
    fn settings_the_file_lacks_stay() {
        let mut current = sample();
        current.ui.theme = crate::settings::ThemePreference::Dark;
        current.sip.dns_srv = true;
        current.calls.auto_answer = true;
        let text = r#"{"format": "rustone-config", "version": 1,
            "settings": {"sip": {"audio_codecs": "AMR-WB/16000/1"}, "calls": {"dnd": true}}}"#;
        let plan = plan_import(&current, &parse(text).unwrap(), &[]);
        assert_eq!(plan.settings.sip.audio_codecs, "AMR-WB/16000/1", "present: imported");
        assert!(plan.settings.calls.dnd);
        assert!(plan.settings.sip.dns_srv, "absent in a present section: kept");
        assert!(plan.settings.calls.auto_answer);
        assert_eq!(plan.settings.ui.theme, crate::settings::ThemePreference::Dark, "absent section: kept");
        assert_eq!(plan.settings.accounts, current.accounts, "passwords included");
    }

    #[test]
    fn account_fields_the_file_lacks_stay() {
        let current = sample();
        // Office, with only its label and transport in the file, no password.
        let text = r#"{"format": "rustone-config", "version": 1, "settings": {},
            "accounts": [{"server": "sip.example.com", "username": "1001", "label": "Work", "transport": "udp"}]}"#;
        let plan = plan_import(&current, &parse(text).unwrap(), &[0]);
        let office = &plan.accounts[0];
        assert_eq!((office.id, office.label.as_str()), (1, "Work"));
        assert_eq!(office.transport, Transport::Udp);
        assert_eq!(office.password, "secret", "no password in the file: kept");
        assert_eq!(office.register_refresh, current.accounts[0].register_refresh);
    }

    #[test]
    fn imports_update_matching_accounts_and_keep_the_rest() {
        let source = sample();
        let file = parse(&export(&source, &[1, 2], "RusTone test")).unwrap();
        // This computer already has the Office account (another password),
        // its own window place and different codecs.
        let mut current = Settings::default();
        current.sip.audio_codecs = "PCMU/8000/1".into();
        current.ui.window.size = Some([500.0, 900.0]);
        current.add_account(Account { label: "Old office".into(), server: " SIP.example.com ".into(), username: "1001".into(), password: "old".into(), ..Account::default() });
        current.add_account(Account { label: "Other".into(), server: "other.example".into(), username: "9".into(), ..Account::default() });

        let plan = plan_import(&current, &file, &[0, 1]);
        assert_eq!(plan.settings.sip.audio_codecs, "opus/48000/2 AMR-WB/16000/1 PCMA/8000/1");
        assert_eq!(plan.settings.ui.window.size, Some([500.0, 900.0]), "the window stays where it is");
        assert_eq!(plan.settings.accounts.len(), 2, "existing accounts stay");
        assert_eq!(plan.accounts.len(), 2);
        assert_eq!((plan.accounts[0].id, plan.accounts[0].label.as_str(), plan.accounts[0].password.as_str()), (1, "Office", "secret"), "updates the match");
        assert_eq!((plan.accounts[1].id, plan.accounts[1].password.as_str()), (0, "hunter2"), "adds the new one");
        assert_eq!(plan.activate, Some(1), "the file's active account");

        let plan = plan_import(&current, &file, &[]);
        assert!(plan.accounts.is_empty() && plan.activate.is_none(), "settings only");
    }
}
