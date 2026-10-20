//! Video: cameras, the video settings, and the frames drawn inside
//! the window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::call::CallId;

/// Where video frames come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoSource {
    /// The local camera.
    Preview,
    /// A call's remote video.
    Call(CallId),
}

/// One video frame, RGBA.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
    /// Counts frames, so the UI only uploads new ones.
    pub seq: u64,
}

/// The latest frame of each source, written by pjsua's media threads and
/// read by the UI.
#[derive(Debug, Default)]
pub struct VideoFrames {
    frames: Mutex<HashMap<VideoSource, Frame>>,
}

impl VideoFrames {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<VideoSource, Frame>> {
        self.frames.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stores a BGRA frame as RGBA.
    pub(crate) fn put_bgra(&self, source: VideoSource, width: u32, height: u32, bgra: &[u8]) {
        let mut rgba = bgra.to_vec();
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let mut frames = self.lock();
        let seq = frames.get(&source).map_or(1, |f| f.seq + 1);
        frames.insert(source, Frame { width, height, rgba: Arc::new(rgba), seq });
    }

    pub(crate) fn clear(&self, source: VideoSource) {
        self.lock().remove(&source);
    }

    /// The latest frame of `source`, if it has one.
    pub fn latest(&self, source: VideoSource) -> Option<Frame> {
        self.lock().get(&source).cloned()
    }
}

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
    fn frames_become_rgba_and_count() {
        let frames = VideoFrames::default();
        frames.put_bgra(VideoSource::Preview, 1, 1, &[1, 2, 3, 255]);
        frames.put_bgra(VideoSource::Preview, 1, 1, &[10, 20, 30, 255]);
        let f = frames.latest(VideoSource::Preview).unwrap();
        assert_eq!((f.seq, f.rgba.as_slice()), (2, &[30u8, 20, 10, 255][..]));
        frames.clear(VideoSource::Preview);
        assert!(frames.latest(VideoSource::Preview).is_none() && frames.latest(VideoSource::Call(0)).is_none());
    }

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
