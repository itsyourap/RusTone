//! Starting with Windows, through
//! fastframe-autostart. Reading and changing it touch the registry, so
//! both run on a worker thread that wakes the app when done.

use std::sync::{Arc, Mutex};

use fastframe_autostart::{Autostart, Start, State};

pub struct StartAtLogin {
    autostart: Autostart,
    /// What's registered; `None` until first read.
    state: Arc<Mutex<Option<State>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl StartAtLogin {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        // A hidden start passes the argument RusTone already reads.
        let autostart = Autostart::new("rustone", "RusTone").hidden_argument("--minimized");
        Self { autostart, state: Arc::default(), wake: Arc::new(wake) }
    }

    /// The registration, as last read (`None` while reading).
    pub fn state(&self) -> Option<State> {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reads the registration again (Task Manager can change it).
    pub fn refresh(&self) {
        self.work(None);
    }

    /// Starts with Windows this way, or not at all.
    pub fn set(&self, start: Option<Start>) {
        self.work(Some(start));
    }

    fn work(&self, change: Option<Option<Start>>) {
        let (autostart, state, wake) = (self.autostart.clone(), self.state.clone(), self.wake.clone());
        let spawned = std::thread::Builder::new().name("autostart".into()).spawn(move || {
            let now = match change {
                Some(start) => autostart.set(start).unwrap_or_else(|e| {
                    log::warn!("cannot change starting with Windows: {e}");
                    autostart.state()
                }),
                None => autostart.state(),
            };
            *state.lock().unwrap_or_else(|e| e.into_inner()) = Some(now);
            wake();
        });
        if let Err(e) = spawned {
            log::warn!("cannot read starting with Windows: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registers and removes a separate test entry, never RusTone's own.
    #[test]
    #[ignore = "changes the user's startup apps"]
    fn starts_hidden_with_the_minimized_argument() {
        let autostart = Autostart::new("rustone-autostart-test", "RusTone autostart test").hidden_argument("--minimized");
        assert_eq!(autostart.set(Some(Start::Hidden)).expect("registered"), State::On(Start::Hidden));
        #[cfg(windows)]
        {
            let run = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run").expect("Run key");
            let value: String = run.get_value("RusTone autostart test").expect("the entry");
            assert!(value.ends_with("--minimized"), "{value}");
        }
        assert_eq!(autostart.set(None).expect("removed"), State::Off);
    }
}
