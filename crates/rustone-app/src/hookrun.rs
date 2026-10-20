//! Runs command hooks as hidden processes, without waiting for them.

use rustone_core::hooks::{HookCall, HookKind, HookSettings, command_line};

pub fn run(settings: &HookSettings, kind: HookKind, call: &HookCall) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let Some((program, args)) = command_line(settings.get(kind), call, now) else { return };
    log::info!("running the {kind:?} hook: {program} {args}");
    let mut command = std::process::Command::new(&program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// No console window for console programs.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
        // Passed as typed, as ShellExecute does.
        if !args.is_empty() {
            command.raw_arg(&args);
        }
    }
    #[cfg(not(windows))]
    command.args(args.split_whitespace());
    match command.spawn() {
        // Reap it in the background.
        Ok(mut child) => {
            let _ = std::thread::Builder::new().name("hook".into()).spawn(move || child.wait());
        }
        Err(e) => log::warn!("the {kind:?} hook ({program}) didn't start: {e}"),
    }
}
