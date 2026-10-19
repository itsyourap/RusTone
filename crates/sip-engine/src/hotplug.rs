//! Sound-device hot-plug on Windows:
//! device state and default-device changes call `on_change`, from a COM
//! worker thread. Adding and removing endpoints is reported through their
//! state, so those notifications are ignored.

use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::core::{PCWSTR, implement};

type Callback = Box<dyn Fn() + Send + Sync>;

#[implement(IMMNotificationClient)]
struct Client {
    on_change: Callback,
}

impl IMMNotificationClient_Impl for Client_Impl {
    fn OnDeviceStateChanged(&self, _device: &PCWSTR, _state: DEVICE_STATE) -> windows::core::Result<()> {
        (self.on_change)();
        Ok(())
    }

    fn OnDeviceAdded(&self, _device: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _device: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, _flow: EDataFlow, _role: ERole, _device: &PCWSTR) -> windows::core::Result<()> {
        (self.on_change)();
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _device: &PCWSTR, _key: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

/// Keeps the notification registered while alive.
pub struct DeviceWatcher {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
}

impl DeviceWatcher {
    /// Registers for notifications. Joins the thread to COM's
    /// multithreaded apartment.
    pub fn start(on_change: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        // SAFETY: plain COM setup; S_FALSE (already initialised) is fine.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| format!("device enumerator: {e}"))?;
            let client: IMMNotificationClient = Client { on_change: Box::new(on_change) }.into();
            enumerator.RegisterEndpointNotificationCallback(&client).map_err(|e| format!("device notifications: {e}"))?;
            Ok(Self { enumerator, client })
        }
    }
}

impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        // SAFETY: registered in `start` with the same objects.
        let _ = unsafe { self.enumerator.UnregisterEndpointNotificationCallback(&self.client) };
    }
}
