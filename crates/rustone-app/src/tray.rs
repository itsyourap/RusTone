//! The notification-area icon and its menu.

use fastframe_tray::{Config, MenuItem, Tray};

use crate::icons;

pub const SHOW: &str = "show";
pub const QUIT: &str = "quit";
const STATUS: &str = "status";

/// Creates the tray item. `None` if the platform has no tray.
pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Option<Tray> {
    let tray = Tray::spawn(
        Config {
            id: "rustone",
            title: "RusTone".into(),
            icon: icons::logo_rgba,
            template_icon: None,
            themed_icon: false,
            menu: vec![
                // Registration status; filled in by the SIP engine (Phase 2).
                MenuItem::action(STATUS, "Not connected").enabled(false),
                MenuItem::Separator,
                MenuItem::action(SHOW, "Show RusTone"),
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
