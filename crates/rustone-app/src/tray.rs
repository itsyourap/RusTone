//! The notification-area icon and its menu.

use fastframe_tray::{Config, MenuItem, Tray};

use crate::icons;
use crate::sip::Status;

pub const SHOW: &str = "show";
pub const QUIT: &str = "quit";
pub const DEBUG: &str = "debug";
const STATUS: &str = "status";

/// Creates the tray item. `None` if the platform has no tray.
pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Option<Tray> {
    let tray = Tray::spawn(
        Config {
            id: "rustone",
            title: "RusTone".into(),
            icon: icons::logo_rgba_offline,
            template_icon: None,
            themed_icon: false,
            menu: vec![
                // Registration status, kept current by `show_status`.
                MenuItem::action(STATUS, "Starting…").enabled(false),
                MenuItem::Separator,
                MenuItem::action(SHOW, "Show RusTone"),
                MenuItem::action(DEBUG, "Debug window"),
                MenuItem::Separator,
                MenuItem::action(QUIT, "Quit RusTone"),
            ],
        },
        wake,
    );
    if tray.is_none() {
        log::warn!("no system tray available; closing the window will quit");
    }
    tray
}

/// Shows the registration status: menu line, tooltip, a dimmed icon while
/// not registered, and a red dot after missed calls.
pub fn show_status(tray: &mut Tray, status: &Status, account: Option<&str>, missed: bool) {
    let line = match (status, account) {
        (Status::Error(e), _) => format!("Error: {e}"),
        (Status::NoAccount, _) => "No account set up".to_owned(),
        (s, Some(account)) => format!("{account}: {}", s.label()),
        (s, None) => s.label().to_owned(),
    };
    tray.set_label(STATUS, line.clone());
    let tooltip = if missed { format!("RusTone\n{line}\nMissed calls") } else { format!("RusTone\n{line}") };
    tray.set_tooltip(tooltip);
    // The offline icon wins over the missed one.
    let icon = match (status.is_online(), missed) {
        (false, _) => icons::logo_rgba_offline,
        (true, true) => icons::logo_rgba_missed,
        (true, false) => icons::logo_rgba,
    };
    tray.set_icon(icon, None);
}
