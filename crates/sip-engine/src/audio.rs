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

/// Speaker and microphone levels, 0..=100.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levels {
    pub speaker: u8,
    pub mic: u8,
    /// Boost the microphone above the middle of the slider.
    pub mic_amplification: bool,
    /// Adjust the microphone in software only, leaving the device's
    /// volume alone.
    pub software_mic: bool,
}

impl Default for Levels {
    fn default() -> Self {
        Self { speaker: 100, mic: 100, mic_amplification: false, software_mic: false }
    }
}

impl Levels {
    /// Each call's level towards the speaker (software only).
    pub fn speaker_gain(&self) -> f32 {
        f32::from(self.speaker.min(100)) / 100.0
    }

    /// The device input volume to ask for: with amplification, the lower
    /// half of the slider covers the device's whole range.
    pub fn mic_device_level(&self) -> u32 {
        let mic = u32::from(self.mic.min(100));
        if self.mic_amplification { (mic * 2).min(100) } else { mic }
    }

    /// Software gain on the microphone:
    /// unity when the device took the volume, plus a boost curve above 50
    /// with amplification.
    pub fn mic_gain(&self, device_took_volume: bool) -> f32 {
        let mic = f32::from(self.mic.min(100));
        let percent = match (device_took_volume, self.mic_amplification && mic > 50.0) {
            (true, true) => 100.0 + (mic - 50.0).powf(1.68),
            (true, false) => 100.0,
            (false, true) => 50.0 + (mic - 50.0).powf(1.7),
            (false, false) => mic,
        };
        percent / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_map_to_device_and_gain() {
        let l = Levels { mic: 40, ..Levels::default() };
        assert_eq!((l.mic_device_level(), l.mic_gain(true), l.mic_gain(false)), (40, 1.0, 0.4));
        let boosted = Levels { mic: 100, mic_amplification: true, ..Levels::default() };
        assert_eq!(boosted.mic_device_level(), 100);
        assert!((boosted.mic_gain(true) - 8.149).abs() < 0.01, "{}", boosted.mic_gain(true));
        assert!((boosted.mic_gain(false) - 8.231).abs() < 0.01, "{}", boosted.mic_gain(false));
        let low = Levels { mic: 30, mic_amplification: true, ..Levels::default() };
        assert_eq!((low.mic_device_level(), low.mic_gain(false)), (60, 0.3));
        assert_eq!(Levels { speaker: 75, ..Levels::default() }.speaker_gain(), 0.75);
    }

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
