//! Opening `sip:`, `tel:` and `callto:` links with RusTone,
//! per user: a `RusTone.Url` class that runs
//! `rustone.exe "%1"`, the schemes pointing at it, and the capabilities
//! that list RusTone in Windows' Default apps (where Windows lets the user
//! choose when another app has claimed a scheme).

/// The link schemes RusTone handles.
#[cfg_attr(not(windows), expect(dead_code, reason = "registration exists only on Windows so far"))]
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

#[cfg(not(windows))]
mod imp {
    pub fn registered() -> bool {
        false
    }

    pub fn set(_on: bool) -> std::io::Result<()> {
        Err(std::io::Error::other("not supported on this system yet"))
    }
}

pub use imp::{registered, set};
