//! Notices when the user has been away from the keyboard and mouse for a
//! while, to publish "Away" (checked
//! every 5 s, away after 10 minutes).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(5);
/// 120 polls of 5 s.
const AWAY_AFTER: Duration = Duration::from_secs(600);

pub struct IdleWatch {
    away: Arc<AtomicBool>,
}

impl IdleWatch {
    /// Starts watching on a background thread; `wake` runs when the user
    /// goes away or comes back.
    pub fn spawn(wake: impl Fn() + Send + 'static) -> Self {
        let away = Arc::new(AtomicBool::new(false));
        let flag = away.clone();
        let spawned = std::thread::Builder::new().name("idlewatch".into()).spawn(move || {
            loop {
                std::thread::sleep(POLL);
                let idle = match user_idle::UserIdle::get_time() {
                    Ok(idle) => idle.duration(),
                    Err(e) => {
                        log::debug!("cannot read the idle time: {e:?}");
                        continue;
                    }
                };
                let now_away = idle >= AWAY_AFTER;
                if flag.swap(now_away, Ordering::SeqCst) != now_away {
                    log::info!("user {}", if now_away { "away" } else { "back" });
                    wake();
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("cannot watch for idleness: {e}");
        }
        Self { away }
    }

    pub fn is_away(&self) -> bool {
        self.away.load(Ordering::SeqCst)
    }
}
