//! Opening `sip:`, `tel:` and `callto:` links with RusTone, per user:
//!
//! * Windows: a `RusTone.Url` class that runs `rustone.exe "%1"`, the
//!   schemes pointing at it, and the capabilities that list RusTone in
//!   Windows' Default apps (where Windows lets the user choose when another
//!   app has claimed a scheme).
//! * Linux: a hidden desktop entry that runs this copy of RusTone with the
//!   link, made the schemes' default in `mimeapps.list` (XDG).
//! * macOS: the bundle declares the schemes (Info.plist); LaunchServices
//!   makes RusTone their default. Links arrive as Apple Events
//!   (`rustone_mac::on_open_url`).

/// The link schemes RusTone handles.
pub const SCHEMES: [&str; 3] = ["sip", "tel", "callto"];

#[cfg(windows)]
mod imp {
    use std::io;

    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    use super::SCHEMES;

    const PROG_ID: &str = r"Software\Classes\RusTone.Url";
    const CAPABILITIES: &str = r"Software\itsyourap\RusTone\Capabilities";

    /// The name RusTone gives a scheme key it creates.
    fn scheme_name(scheme: &str) -> String {
        format!("URL:{} Protocol", scheme.to_ascii_uppercase())
    }

    fn command() -> io::Result<String> {
        let exe = std::env::current_exe()?;
        Ok(format!("\"{}\" \"%1\"", exe.display()))
    }

    /// Whether every scheme opens this copy of RusTone.
    pub fn registered() -> bool {
        let Ok(ours) = command() else { return false };
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        SCHEMES.iter().all(|scheme| {
            hkcu.open_subkey(format!(r"Software\Classes\{scheme}\shell\open\command"))
                .and_then(|k| k.get_value::<String, _>(""))
                .is_ok_and(|v| v.eq_ignore_ascii_case(&ours))
        })
    }

    pub fn set(on: bool) -> io::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let ours = command()?;
        if on {
            let (prog, _) = hkcu.create_subkey(PROG_ID)?;
            prog.set_value("", &"RusTone link")?;
            prog.set_value("URL Protocol", &"")?;
            let (cmd, _) = hkcu.create_subkey(format!(r"{PROG_ID}\shell\open\command"))?;
            cmd.set_value("", &ours)?;
            let (caps, _) = hkcu.create_subkey(CAPABILITIES)?;
            caps.set_value("ApplicationName", &"RusTone")?;
            caps.set_value("ApplicationDescription", &"RusTone softphone")?;
            let (urls, _) = hkcu.create_subkey(format!(r"{CAPABILITIES}\URLAssociations"))?;
            let (apps, _) = hkcu.create_subkey(r"Software\RegisteredApplications")?;
            apps.set_value("RusTone", &CAPABILITIES)?;
            for scheme in SCHEMES {
                urls.set_value(scheme, &"RusTone.Url")?;
                let (key, created) = hkcu.create_subkey(format!(r"Software\Classes\{scheme}"))?;
                // A scheme key another app made keeps its own name.
                if created == winreg::enums::RegDisposition::REG_CREATED_NEW_KEY {
                    key.set_value("", &scheme_name(scheme))?;
                }
                key.set_value("URL Protocol", &"")?;
                let (cmd, _) = hkcu.create_subkey(format!(r"Software\Classes\{scheme}\shell\open\command"))?;
                cmd.set_value("", &ours)?;
            }
        } else {
            // Only what points at us goes; another app's handler stays.
            for scheme in SCHEMES {
                let points_here = hkcu
                    .open_subkey(format!(r"Software\Classes\{scheme}\shell\open\command"))
                    .and_then(|k| k.get_value::<String, _>(""))
                    .is_ok_and(|v| v.eq_ignore_ascii_case(&ours));
                // Just our command: the scheme key may predate us. One we
                // made goes once nothing else is in it.
                if points_here {
                    let path = format!(r"Software\Classes\{scheme}");
                    hkcu.delete_subkey_all(format!(r"{path}\shell"))?;
                    let ours_alone = hkcu.open_subkey(&path).is_ok_and(|k| {
                        k.enum_keys().next().is_none() && k.get_value::<String, _>("").is_ok_and(|name| name == scheme_name(scheme))
                    });
                    if ours_alone {
                        hkcu.delete_subkey_all(&path)?;
                    }
                }
            }
            let _ = hkcu.delete_subkey_all(PROG_ID);
            let _ = hkcu.delete_subkey_all(r"Software\itsyourap\RusTone\Capabilities");
            if let Ok(apps) = hkcu.open_subkey_with_flags(r"Software\RegisteredApplications", winreg::enums::KEY_SET_VALUE) {
                let _ = apps.delete_value("RusTone");
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::io;
    use std::path::PathBuf;

    use super::{SCHEMES, mimeapps};

    /// The desktop entry for links, apart from the launcher's own (which a
    /// package installs and an AppImage doesn't have).
    const ENTRY: &str = "dev.itsyourap.rustone.url.desktop";

    fn home_dir(var: &str, fallback: &str) -> io::Result<PathBuf> {
        std::env::var_os(var)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
            .ok_or_else(|| io::Error::other("no home folder"))
    }

    fn entry_path() -> io::Result<PathBuf> {
        Ok(home_dir("XDG_DATA_HOME", ".local/share")?.join("applications").join(ENTRY))
    }

    fn mimeapps_path() -> io::Result<PathBuf> {
        Ok(home_dir("XDG_CONFIG_HOME", ".config")?.join("mimeapps.list"))
    }

    /// The program links should start: the AppImage itself when running
    /// from one (its mount point changes every run), else this executable.
    fn program() -> io::Result<String> {
        let exe = match std::env::var_os("APPIMAGE") {
            Some(appimage) => PathBuf::from(appimage),
            None => std::env::current_exe()?,
        };
        Ok(exe.display().to_string())
    }

    fn entry(program: &str) -> String {
        let mime: String = SCHEMES.iter().map(|s| format!("x-scheme-handler/{s};")).collect();
        let quoted = program.replace('\\', r"\\").replace('"', r#"\""#);
        format!(
            "[Desktop Entry]\nType=Application\nName=RusTone\nComment=Call sip:, tel: and callto: links\n\
             Exec=\"{quoted}\" %u\nIcon=rustone\nNoDisplay=true\nMimeType={mime}\n"
        )
    }

    /// Whether every scheme opens this copy of RusTone.
    pub fn registered() -> bool {
        let (Ok(program), Ok(entry_path), Ok(list_path)) = (program(), entry_path(), mimeapps_path()) else { return false };
        let ours = std::fs::read_to_string(entry_path).is_ok_and(|e| e == entry(&program));
        let list = std::fs::read_to_string(list_path).unwrap_or_default();
        ours && SCHEMES.iter().all(|s| mimeapps::default_for(&list, &format!("x-scheme-handler/{s}")) == Some(ENTRY))
    }

    pub fn set(on: bool) -> io::Result<()> {
        let (entry_path, list_path) = (entry_path()?, mimeapps_path()?);
        let mut list = std::fs::read_to_string(&list_path).unwrap_or_default();
        for scheme in SCHEMES {
            list = mimeapps::set_default(&list, &format!("x-scheme-handler/{scheme}"), ENTRY, on);
        }
        let applications = entry_path.parent().expect("applications folder").to_path_buf();
        if on {
            std::fs::create_dir_all(&applications)?;
            std::fs::write(&entry_path, entry(&program()?))?;
        } else if entry_path.exists() {
            std::fs::remove_file(&entry_path)?;
        }
        if let Some(dir) = list_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&list_path, list)?;
        // Desktops cache the entries; refreshing the cache is a nicety.
        let _ = std::process::Command::new("update-desktop-database").arg(&applications).status();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::io;

    use super::SCHEMES;

    /// Whether every scheme opens this copy of RusTone.
    pub fn registered() -> bool {
        let Some(ours) = rustone_mac::bundle_id() else { return false };
        SCHEMES.iter().all(|s| rustone_mac::default_handler(s).is_some_and(|h| h.eq_ignore_ascii_case(&ours)))
    }

    pub fn set(on: bool) -> io::Result<()> {
        if !on {
            // LaunchServices has no "no default": another app has to be
            // chosen for the schemes.
            return Err(io::Error::other("choose another app for these links in that app's settings"));
        }
        let ours = rustone_mac::bundle_id().ok_or_else(|| io::Error::other("only the installed RusTone.app can open links"))?;
        for scheme in SCHEMES {
            rustone_mac::set_default_handler(scheme, &ours).map_err(io::Error::other)?;
        }
        Ok(())
    }
}

pub use imp::{registered, set};

/// Default applications in an XDG `mimeapps.list` (`[Default Applications]`
/// with `type=first.desktop;second.desktop;` lines).
#[cfg_attr(not(target_os = "linux"), allow(dead_code, reason = "used on Linux; tested everywhere"))]
mod mimeapps {
    const SECTION: &str = "[Default Applications]";

    /// The first default entry for `mime`.
    pub fn default_for<'a>(list: &'a str, mime: &str) -> Option<&'a str> {
        let mut in_section = false;
        for line in list.lines().map(str::trim) {
            if line.starts_with('[') {
                in_section = line == SECTION;
            } else if in_section
                && let Some((key, value)) = line.split_once('=')
                && key.trim() == mime
            {
                return value.split(';').map(str::trim).find(|v| !v.is_empty());
            }
        }
        None
    }

    /// `list` with `entry` made the default for `mime` (`on`) or taken out
    /// of its defaults (`!on`, dropping the line once nothing is left).
    /// Other lines and sections stay as they were.
    pub fn set_default(list: &str, mime: &str, entry: &str, on: bool) -> String {
        let mut out: Vec<String> = Vec::new();
        let (mut in_section, mut seen_section, mut done) = (false, false, false);
        for line in list.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                if in_section && on && !done {
                    out.push(format!("{mime}={entry};"));
                    done = true;
                }
                in_section = trimmed == SECTION;
                seen_section |= in_section;
                out.push(line.to_owned());
                continue;
            }
            if in_section
                && let Some((key, value)) = trimmed.split_once('=')
                && key.trim() == mime
            {
                let others = value.split(';').map(str::trim).filter(|v| !v.is_empty() && *v != entry);
                let entries: Vec<&str> = if on { std::iter::once(entry).chain(others).collect() } else { others.collect() };
                if !entries.is_empty() {
                    out.push(format!("{mime}={};", entries.join(";")));
                }
                done = true;
                continue;
            }
            out.push(line.to_owned());
        }
        // Still not added: the section is the last one, or there is none.
        if on && !done {
            if !seen_section {
                if out.last().is_some_and(|l| !l.trim().is_empty()) {
                    out.push(String::new());
                }
                out.push(SECTION.to_owned());
            }
            out.push(format!("{mime}={entry};"));
        }
        let mut text = out.join("\n");
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::mimeapps::{default_for, set_default};

    const SIP: &str = "x-scheme-handler/sip";

    #[test]
    fn an_empty_list_gets_the_section() {
        let list = set_default("", SIP, "rustone.desktop", true);
        assert_eq!(list, "[Default Applications]\nx-scheme-handler/sip=rustone.desktop;\n");
        assert_eq!(default_for(&list, SIP), Some("rustone.desktop"));
    }

    #[test]
    fn rustone_goes_first_and_leaves_the_others() {
        let list = "[Added Associations]\nx-scheme-handler/sip=other.desktop;\n\n[Default Applications]\ntext/html=firefox.desktop\nx-scheme-handler/sip=other.desktop;\n";
        let on = set_default(list, SIP, "rustone.desktop", true);
        assert_eq!(default_for(&on, SIP), Some("rustone.desktop"));
        assert!(on.contains("x-scheme-handler/sip=rustone.desktop;other.desktop;"), "{on}");
        assert!(on.contains("[Added Associations]\nx-scheme-handler/sip=other.desktop;"), "{on}");
        let off = set_default(&on, SIP, "rustone.desktop", false);
        assert_eq!(default_for(&off, SIP), Some("other.desktop"));
        assert!(off.contains("text/html=firefox.desktop"), "{off}");
    }

    #[test]
    fn turning_off_drops_a_line_with_only_rustone() {
        let on = set_default("[Default Applications]\n[Other]\nkey=value\n", SIP, "rustone.desktop", true);
        assert_eq!(on, "[Default Applications]\nx-scheme-handler/sip=rustone.desktop;\n[Other]\nkey=value\n");
        let off = set_default(&on, SIP, "rustone.desktop", false);
        assert_eq!(off, "[Default Applications]\n[Other]\nkey=value\n");
        assert_eq!(default_for(&off, SIP), None);
    }
}
