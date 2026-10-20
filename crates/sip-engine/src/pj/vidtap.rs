//! Video taps: frames from the video bridge for drawing inside the window.
//! Remote video gets a tap per call; the preview gets one too. Taps may be
//! added from pjsua's callback threads, so state is behind mutexes.

use std::collections::{HashMap, HashSet};
use std::ffi::{c_int, c_uint};
use std::sync::{Arc, LazyLock, Mutex};

use pjsua_sys::*;

use crate::call::CallId;
use crate::video::{VideoFrames, VideoSource};

/// Frame sizes the bridge scales into (it keeps the aspect ratio).
const REMOTE_SIZE: (c_uint, c_uint) = (960, 540);
const PREVIEW_SIZE: (c_uint, c_uint) = (320, 240);
/// The preview's tap id; calls use their call id.
const PREVIEW_TAP: c_int = 1000;

static FRAMES: Mutex<Option<Arc<VideoFrames>>> = Mutex::new(None);

struct Tap {
    port: *mut pjmedia_port,
    slot: c_int,
    /// The bridge slot it's fed from.
    source: c_int,
}
// SAFETY: only used under the TAPS lock, through thread-safe pjsua calls.
unsafe impl Send for Tap {}

static TAPS: LazyLock<Mutex<HashMap<VideoSource, Tap>>> = LazyLock::new(Mutex::default);
/// Sources whose frames are kept. Removing a port from the bridge is only
/// queued, so a removed tap can still deliver a frame; it's dropped here.
/// Held while storing a frame, so a frame can't land after `disconnect`
/// cleared the source. Never held across pjsua calls (the bridge thread
/// takes it from inside the bridge).
static LIVE: LazyLock<Mutex<HashSet<VideoSource>>> = LazyLock::new(Mutex::default);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where frames go (set by the engine for its lifetime).
pub fn set_frames(frames: Option<Arc<VideoFrames>>) {
    *lock(&FRAMES) = frames;
}

fn source_for(tap_id: c_int) -> VideoSource {
    if tap_id == PREVIEW_TAP { VideoSource::Preview } else { VideoSource::Call(tap_id) }
}

unsafe extern "C" fn on_frame(tap_id: c_int, bgra: *const u8, width: c_uint, height: c_uint) {
    let _ = std::panic::catch_unwind(|| {
        let Some(frames) = lock(&FRAMES).clone() else { return };
        let source = source_for(tap_id);
        let live = lock(&LIVE);
        if !live.contains(&source) {
            return;
        }
        let len = width as usize * height as usize * 4;
        // SAFETY: the shim checked the frame holds width*height BGRA pixels.
        let bgra = unsafe { std::slice::from_raw_parts(bgra, len) };
        frames.put_bgra(source, width, height, bgra);
        drop(live);
    });
}

/// Feeds `source` from bridge slot `from`, replacing a tap on another slot.
fn connect(source: VideoSource, from: c_int) {
    let mut taps = lock(&TAPS);
    if taps.get(&source).is_some_and(|t| t.source == from) {
        return;
    }
    if let Some(old) = taps.remove(&source) {
        destroy(old);
    }
    let (tap_id, (w, h)) = match source {
        VideoSource::Preview => (PREVIEW_TAP, PREVIEW_SIZE),
        VideoSource::Call(id) => (id, REMOTE_SIZE),
    };
    let mut port = std::ptr::null_mut();
    // SAFETY: the shim adds and connects the port; `port` is filled on success.
    let slot = unsafe { shim::jf_video_tap_add(tap_id, on_frame, from, w, h, &mut port) };
    if slot >= 0 {
        taps.insert(source, Tap { port, slot, source: from });
        lock(&LIVE).insert(source);
    } else {
        log::warn!("cannot show video from bridge slot {from}");
    }
}

fn disconnect(source: VideoSource) {
    {
        let mut live = lock(&LIVE);
        live.remove(&source);
        if let Some(frames) = lock(&FRAMES).clone() {
            frames.clear(source);
        }
    }
    if let Some(tap) = lock(&TAPS).remove(&source) {
        destroy(tap);
    }
}

fn destroy(tap: Tap) {
    // SAFETY: created by `connect`; the port frees its pool once the bridge lets go.
    unsafe {
        pjsua_vid_conf_remove_port(tap.slot);
        pjmedia_port_destroy(tap.port);
    }
}

/// Follows a call's video: a tap on its decoder while video flows.
pub fn update_call(call: CallId, decoder_slot: Option<c_int>) {
    match decoder_slot {
        Some(slot) => connect(VideoSource::Call(call), slot),
        None => disconnect(VideoSource::Call(call)),
    }
}

pub fn forget_call(call: CallId) {
    disconnect(VideoSource::Call(call));
}

/// Taps the running preview of `camera`.
pub fn tap_preview(camera: i32) {
    // SAFETY: plain query.
    let slot = unsafe { pjsua_vid_preview_get_vid_conf_port(camera) };
    if slot != PJSUA_INVALID_ID {
        connect(VideoSource::Preview, slot);
    }
}

pub fn untap_preview() {
    disconnect(VideoSource::Preview);
}

/// Removes every tap (session end).
pub fn clear() {
    let taps: Vec<(VideoSource, Tap)> = lock(&TAPS).drain().collect();
    {
        let mut live = lock(&LIVE);
        live.clear();
        if let Some(frames) = lock(&FRAMES).clone() {
            for (source, _) in &taps {
                frames.clear(*source);
            }
        }
    }
    for (_, tap) in taps {
        destroy(tap);
    }
}
