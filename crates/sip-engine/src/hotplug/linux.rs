//! Sound-device hot-plug on Linux. pjproject talks to ALSA, whose cards
//! show up in `/proc/asound/cards` and as nodes in `/dev/snd`; a thread
//! compares both every couple of seconds, which needs no udev or sound
//! server.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(2);

/// Keeps the polling thread running while alive.
pub struct DeviceWatcher {
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl DeviceWatcher {
    pub fn start(on_change: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        let (stop, stopped) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("sound-devices".into())
            .spawn(move || {
                let mut last = fingerprint();
                while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(POLL) {
                    let now = fingerprint();
                    if now != last {
                        last = now;
                        on_change();
                    }
                }
            })
            .map_err(|e| format!("sound device thread: {e}"))?;
        Ok(Self { stop: Some(stop), thread: Some(thread) })
    }
}

impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The ALSA cards and device nodes present now.
fn fingerprint() -> (String, Vec<String>) {
    let cards = std::fs::read_to_string("/proc/asound/cards").unwrap_or_default();
    let mut nodes: Vec<String> = std::fs::read_dir("/dev/snd")
        .map(|dir| dir.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    nodes.sort();
    (cards, nodes)
}
