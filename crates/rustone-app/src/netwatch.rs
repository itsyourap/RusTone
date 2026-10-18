//! Notices when the machine's IP addresses change, so the SIP engine can
//! re-create its transports and re-register.
//! Polls instead of using OS notifications to
//! stay portable and free of unsafe code.

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(2);

pub struct NetWatch {
    changed: Arc<AtomicBool>,
}

impl NetWatch {
    /// Starts watching on a background thread; `wake` runs after each change.
    pub fn spawn(wake: impl Fn() + Send + 'static) -> Self {
        let changed = Arc::new(AtomicBool::new(false));
        let flag = changed.clone();
        let spawned = std::thread::Builder::new().name("netwatch".into()).spawn(move || {
            let mut last = addresses();
            loop {
                std::thread::sleep(POLL);
                let now = addresses();
                if now != last {
                    log::info!("network addresses changed ({} → {} usable)", last.len(), now.len());
                    // Only act once there is a network again.
                    if !now.is_empty() {
                        flag.store(true, Ordering::SeqCst);
                        wake();
                    }
                    last = now;
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("cannot watch for network changes: {e}");
        }
        Self { changed }
    }

    /// Whether the addresses changed since the last call.
    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::SeqCst)
    }
}

fn addresses() -> BTreeSet<IpAddr> {
    match if_addrs::get_if_addrs() {
        Ok(interfaces) => usable(interfaces.iter().filter(|i| i.is_oper_up()).map(if_addrs::Interface::ip)),
        Err(e) => {
            log::debug!("cannot list network interfaces: {e}");
            BTreeSet::new()
        }
    }
}

/// Addresses SIP can use: not loopback, not link-local.
fn usable(ips: impl Iterator<Item = IpAddr>) -> BTreeSet<IpAddr> {
    ips.filter(|ip| {
        !ip.is_loopback()
            && match ip {
                IpAddr::V4(v4) => !v4.is_link_local() && !v4.is_unspecified(),
                IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80 && !v6.is_unspecified(),
            }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_routable_addresses_count() {
        let ips: Vec<IpAddr> = ["127.0.0.1", "169.254.1.2", "192.168.29.54", "fe80::1", "2001:db8::5", "::1", "0.0.0.0"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        let kept: Vec<String> = usable(ips.into_iter()).iter().map(ToString::to_string).collect();
        assert_eq!(kept, ["192.168.29.54", "2001:db8::5"]);
    }
}
