//! Sound devices: the devices pjsua sees, the ones the user picked (by
//! name), and the rules for resolving one
//! to the other.

/// A sound device pjsua can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    /// pjmedia's device index; changes when devices come and go.
    pub index: i32,
    pub name: String,
    /// Audio backend, e.g. "WMME".
    pub driver: String,
    pub inputs: u32,
    pub outputs: u32,
}

/// The devices to use, by name; empty means the system default.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioDevices {
    pub input: String,
    pub output: String,
    /// Plays the ringtone and other sounds while no call has audio.
    pub ring: String,
}

/// pjmedia's "default capture device" index.
pub const DEFAULT_CAPTURE: i32 = -1;
/// pjmedia's "default playback device" index.
pub const DEFAULT_PLAYBACK: i32 = -2;

/// Device indexes for [`AudioDevices`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceIds {
    pub input: i32,
    pub output: i32,
    pub ring: i32,
}

impl Default for DeviceIds {
    fn default() -> Self {
        Self { input: DEFAULT_CAPTURE, output: DEFAULT_PLAYBACK, ring: DEFAULT_PLAYBACK }
    }
}

impl DeviceIds {
    /// Exact, case-sensitive name
    /// matches (the last one wins); a missing or empty name keeps the
    /// system default.
    pub fn resolve(wanted: &AudioDevices, devices: &[AudioDevice]) -> Self {
        let mut ids = Self::default();
        for d in devices {
            if d.inputs > 0 && !wanted.input.is_empty() && d.name == wanted.input {
                ids.input = d.index;
            }
            if d.outputs > 0 {
                if !wanted.output.is_empty() && d.name == wanted.output {
                    ids.output = d.index;
                }
                if !wanted.ring.is_empty() && d.name == wanted.ring {
                    ids.ring = d.index;
                }
            }
        }
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(index: i32, name: &str, inputs: u32, outputs: u32) -> AudioDevice {
        AudioDevice { index, name: name.into(), driver: "WMME".into(), inputs, outputs }
    }

    #[test]
    fn names_resolve_to_devices() {
        let devices = [dev(0, "Microphone (USB)", 1, 0), dev(1, "Speakers (Realtek)", 0, 2), dev(2, "Headset", 1, 2), dev(3, "Headset", 1, 2)];
        let wanted = AudioDevices { input: "Headset".into(), output: "Speakers (Realtek)".into(), ring: "Headset".into() };
        assert_eq!(DeviceIds::resolve(&wanted, &devices), DeviceIds { input: 3, output: 1, ring: 3 }, "last match wins");
        // Empty, missing, wrong-direction and differently-cased names fall back.
        let wanted = AudioDevices { input: "Speakers (Realtek)".into(), output: "headset".into(), ring: "Gone".into() };
        assert_eq!(DeviceIds::resolve(&wanted, &devices), DeviceIds::default());
        assert_eq!(DeviceIds::resolve(&AudioDevices::default(), &devices), DeviceIds::default());
    }
}
