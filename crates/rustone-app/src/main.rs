//! RusTone: a SIP softphone built on pjproject, egui and fastframe.

// No console window for release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod calls;
mod icons;
mod netwatch;
mod sip;
mod theme;
mod tray;
mod ui;
mod window;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fastframe_instance::{Claim, Slot};
use fastframe_shell::{Shell, Waker};
use rustone_core::settings::{self, Settings};

use crate::app::{App, Paths, Request, Requests};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTANCE_ID: &str = "dev.itsyourap.rustone";

const DIRS: fastframe_dirs::App = fastframe_dirs::App {
    // Not copied into a portable folder: the running copy's lock and logs.
    not_copied: &["instance", "rustone.log", "panic.log"],
    ..fastframe_dirs::App::new("rustone", "dev", "itsyourap", "RusTone")
};

#[derive(Debug, Default)]
struct Args {
    /// Start in the tray.
    minimized: bool,
}

impl Args {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut out = Self::default();
        for arg in args {
            match arg.to_ascii_lowercase().as_str() {
                "--minimized" | "/minimized" | "-m" => out.minimized = true,
                _ => {}
            }
        }
        out
    }
}

fn main() -> ExitCode {
    let args = Args::parse(std::env::args().skip(1));
    let waker = Waker::default();
    let requests = Requests::default();

    // One running copy per user (and per portable folder). A second launch
    // asks the running copy to show its window, then exits.
    let mut slot = Slot::new(INSTANCE_ID);
    if let Some(scope) = DIRS.portable().instance_scope() {
        slot = slot.scoped(&scope);
    }
    let (handler_requests, handler_waker) = (requests.clone(), waker.clone());
    let request = if args.minimized { "ping" } else { "show" };
    let _instance = match slot.claim(request, move |request| match request {
        "show" => {
            handler_requests.lock().unwrap_or_else(|e| e.into_inner()).push(Request::Show);
            handler_waker.wake();
            Some("ok".to_owned())
        }
        "ping" => Some("ok".to_owned()),
        _ => None,
    }) {
        Claim::First(guard) => guard,
        Claim::Running(_) | Claim::Declined => return ExitCode::SUCCESS,
        Claim::Unanswered => return ExitCode::FAILURE,
    };

    let found = DIRS.discover(|_, _| Ok(()));
    let dirs = found.dirs.unwrap_or_else(fallback_dirs);
    if let Err(e) = fastframe_log::Logging::new("rustone", VERSION)
        .filter("warn,rustone=info,sip_engine=info")
        .file(dirs.state.join("rustone.log"))
        .panic_log(dirs.state.join("panic.log"))
        .init()
    {
        eprintln!("logging unavailable: {e}");
    }
    log::info!("data folders: {:?}", found.mode);

    let settings_path = dirs.config.join(settings::FILE_NAME);
    let settings = load_settings(&settings_path);

    fastframe_titlebar::init();
    let start_hidden = args.minimized || settings.ui.start_minimized;
    let paths = Paths { settings: settings_path, sounds: dirs.cache.join("sounds"), sip_log: dirs.state.join("sip.log") };
    let app = App::new(settings, paths, requests, &waker);

    let shell = Shell::new(app, &waker).start_hidden(start_hidden);
    // macOS needs AppKit's loop to keep the menu-bar item alive; elsewhere
    // the shell's own sleep answers window messages.
    #[cfg(target_os = "macos")]
    let shell = shell.idle(fastframe_tray::idle);

    let result = shell.run(|lease| {
        let options = lease.peek(window::native_options);
        eframe::run_native(
            "RusTone",
            options,
            Box::new(move |cc| {
                let mut app = lease.take(&cc.egui_ctx);
                app.attach(&cc.egui_ctx);
                Ok(Box::new(window::Window::new(app)))
            }),
        )
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("window failed: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Settings from disk. An unreadable file is kept aside and replaced by defaults.
fn load_settings(path: &Path) -> Settings {
    match Settings::load(path) {
        Ok(settings) => settings,
        Err(e) => {
            let aside = path.with_extension("toml.bad");
            log::warn!("{e}; keeping it as {} and using defaults", aside.display());
            let _ = std::fs::rename(path, aside);
            Settings::default()
        }
    }
}

/// Used only when no per-user folder exists: a folder beside the executable.
fn fallback_dirs() -> fastframe_dirs::Dirs {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    fastframe_dirs::Dirs::inside(&exe_dir.join("rustone-data"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimized_switch_accepts_every_spelling() {
        for arg in ["--minimized", "/minimized", "/MINIMIZED", "-m"] {
            assert!(Args::parse([arg.to_owned()]).minimized, "{arg}");
        }
        assert!(!Args::parse(["sip:123@example.com".to_owned()]).minimized);
    }
}
