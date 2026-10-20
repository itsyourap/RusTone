//! RusTone: a SIP softphone built on pjproject, egui and fastframe.

// No console window for release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod calls;
mod directory;
mod hookrun;
mod icons;
mod idle;
mod netwatch;
mod phonebook;
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
    ..fastframe_dirs::App::new("rustone", "dev", "itsyourap", APP_FOLDER)
};
/// The data folders' name under `%APPDATA%\itsyourap` and `%LOCALAPPDATA%\itsyourap`.
const APP_FOLDER: &str = "RusTone";

/// Renames data folders from the app's old name ("RusTone") to [`APP_FOLDER`].
/// Windows paths ignore case, so they keep working either way; a failure
/// (e.g. another copy still has them open) is only reported. Returns what
/// was renamed and what failed, for the log.
fn rename_old_folders() -> (Vec<PathBuf>, Vec<String>) {
    let (mut renamed, mut failed) = (Vec::new(), Vec::new());
    for var in ["APPDATA", "LOCALAPPDATA"] {
        let Some(base) = std::env::var_os(var).map(|b| PathBuf::from(b).join("itsyourap")) else { continue };
        let Ok(entries) = std::fs::read_dir(&base) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.eq_ignore_ascii_case(APP_FOLDER) && name != APP_FOLDER {
                let target = base.join(APP_FOLDER);
                match std::fs::rename(entry.path(), &target) {
                    Ok(()) => renamed.push(target),
                    Err(e) => failed.push(format!("{}: {e}", entry.path().display())),
                }
            }
        }
    }
    (renamed, failed)
}

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
    // Before anything opens files in them (the instance lock lives there).
    let (renamed, rename_failed) = rename_old_folders();
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
        .redact(sip::app_log_tap)
        .init()
    {
        eprintln!("logging unavailable: {e}");
    }
    log::info!("data folders: {:?}", found.mode);
    for folder in &renamed {
        log::info!("renamed data folder to {}", folder.display());
    }
    for failure in &rename_failed {
        log::warn!("cannot rename data folder {failure}");
    }

    let settings_path = dirs.config.join(settings::FILE_NAME);
    let settings = load_settings(&settings_path);

    fastframe_titlebar::init();
    let start_hidden = args.minimized || settings.ui.start_minimized;
    let paths = Paths { settings: settings_path, sounds: dirs.cache.join("sounds"), sip_log: dirs.state.join("sip.log"), database: dirs.config.join(rustone_core::store::FILE_NAME), renamed_from_rustone: !renamed.is_empty() };
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
        Ok(mut settings) => {
            if settings.grow_window_to_default() {
                // Saved now, so the step runs once.
                if let Err(e) = fastframe_file::write(path, settings.to_toml()) {
                    log::warn!("cannot save {}: {e}", path.display());
                }
            }
            settings
        }
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
