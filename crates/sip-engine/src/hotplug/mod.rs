//! Sound-device hot-plug: [`DeviceWatcher::start`] calls `on_change` (from
//! a thread of its own) when a sound device appears or disappears, or the
//! default device changes. The engine then re-enumerates pjmedia's devices.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::DeviceWatcher;
#[cfg(target_os = "macos")]
pub use macos::DeviceWatcher;
#[cfg(windows)]
pub use windows::DeviceWatcher;
