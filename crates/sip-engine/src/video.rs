//! Video: cameras and the video settings.

/// A video device pjsua sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoDevice {
    /// pjmedia's index; changes when devices come and go.
    pub index: i32,
    pub name: String,
    /// Backend, e.g. "DirectShow" or "SDL".
    pub driver: String,
    /// Captures (a camera) rather than only renders.
    pub capture: bool,
}

/// pjmedia's "default camera" index.
pub const DEFAULT_CAMERA: i32 = -1;

/// Video settings that reach pjsua at start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoConfig {
    pub enabled: bool,
    /// Camera name as pjsua reports it; empty for the default camera.
    pub camera: String,
    /// H.264 bitrate in kbit/s; 0 keeps the codec's default.
    pub bitrate_kbps: u32,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self { enabled: true, camera: String::new(), bitrate_kbps: 0 }
    }
}

/// The camera with exactly that name, else
/// the default camera.
pub fn camera_index(name: &str, devices: &[VideoDevice]) -> i32 {
    if name.is_empty() {
        return DEFAULT_CAMERA;
    }
    devices.iter().find(|d| d.capture && d.name == name).map_or(DEFAULT_CAMERA, |d| d.index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cameras_resolve_by_exact_name() {
        let devices = [
            VideoDevice { index: 0, name: "Integrated Camera".into(), driver: "DirectShow".into(), capture: true },
            VideoDevice { index: 1, name: "SDL renderer".into(), driver: "SDL".into(), capture: false },
        ];
        assert_eq!(camera_index("Integrated Camera", &devices), 0);
        assert_eq!(camera_index("SDL renderer", &devices), DEFAULT_CAMERA, "renderers aren't cameras");
        assert_eq!(camera_index("integrated camera", &devices), DEFAULT_CAMERA);
        assert_eq!(camera_index("", &devices), DEFAULT_CAMERA);
    }
}
