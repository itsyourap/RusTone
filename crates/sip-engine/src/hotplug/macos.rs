//! Sound-device hot-plug on macOS: CoreAudio property listeners on the
//! system object's device list and default input and output devices.
//! CoreAudio calls them on a thread of its own.

use std::ffi::c_void;

type AudioObjectId = u32;
type OsStatus = i32;

#[repr(C)]
struct PropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

type ListenerProc = unsafe extern "C" fn(AudioObjectId, u32, *const PropertyAddress, *mut c_void) -> OsStatus;

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectAddPropertyListener(
        object: AudioObjectId,
        address: *const PropertyAddress,
        listener: ListenerProc,
        client_data: *mut c_void,
    ) -> OsStatus;
    fn AudioObjectRemovePropertyListener(
        object: AudioObjectId,
        address: *const PropertyAddress,
        listener: ListenerProc,
        client_data: *mut c_void,
    ) -> OsStatus;
}

/// `kAudioObjectSystemObject`.
const SYSTEM_OBJECT: AudioObjectId = 1;
/// `kAudioObjectPropertyScopeGlobal` ('glob').
const SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
/// `kAudioObjectPropertyElementMain`.
const ELEMENT_MAIN: u32 = 0;
/// `kAudioHardwarePropertyDevices`, `…DefaultInputDevice`,
/// `…DefaultOutputDevice` ('dev#', 'dIn ', 'dOut').
const SELECTORS: [u32; 3] = [u32::from_be_bytes(*b"dev#"), u32::from_be_bytes(*b"dIn "), u32::from_be_bytes(*b"dOut")];

type Callback = Box<dyn Fn() + Send + Sync>;

unsafe extern "C" fn changed(_object: AudioObjectId, _count: u32, _addresses: *const PropertyAddress, data: *mut c_void) -> OsStatus {
    // SAFETY: `data` is the boxed callback registered in `start`, alive
    // until the listeners are removed in `drop`.
    let on_change = unsafe { &*(data as *const Callback) };
    on_change();
    0
}

fn address(selector: u32) -> PropertyAddress {
    PropertyAddress { selector, scope: SCOPE_GLOBAL, element: ELEMENT_MAIN }
}

/// Keeps the listeners registered while alive.
pub struct DeviceWatcher {
    on_change: Box<Callback>,
}

impl DeviceWatcher {
    pub fn start(on_change: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        let watcher = Self { on_change: Box::new(Box::new(on_change)) };
        for selector in SELECTORS {
            // SAFETY: the address and listener are valid; the client data
            // outlives the registration (removed in `drop`).
            let status = unsafe { AudioObjectAddPropertyListener(SYSTEM_OBJECT, &address(selector), changed, watcher.data()) };
            if status != 0 {
                return Err(format!("CoreAudio listener: OSStatus {status}"));
            }
        }
        Ok(watcher)
    }

    fn data(&self) -> *mut c_void {
        (&*self.on_change as *const Callback).cast_mut().cast()
    }
}

impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        for selector in SELECTORS {
            // SAFETY: removes what `start` added, with the same arguments.
            unsafe { AudioObjectRemovePropertyListener(SYSTEM_OBJECT, &address(selector), changed, self.data()) };
        }
    }
}
