//! Feeds the debug window: SIP message tap, pjsua log lines, stream
//! statistics and audio taps on the conference bridge.

use std::ffi::{CStr, c_char, c_int, c_short, c_uint};
use std::sync::{Arc, Mutex, RwLock};

use pjsua_sys::*;

use super::calls;
use crate::call::CallId;
use crate::debug::{AudioTap, DebugFeed, LogSource, MsgDirection, StreamStats};

static FEED: RwLock<Option<Arc<DebugFeed>>> = RwLock::new(None);

pub fn set_feed(feed: Option<Arc<DebugFeed>>) {
    *FEED.write().unwrap_or_else(|e| e.into_inner()) = feed;
}

fn feed() -> Option<Arc<DebugFeed>> {
    FEED.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn enabled() -> bool {
    feed().is_some_and(|f| f.is_enabled())
}

fn guard(f: impl FnOnce()) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
}

fn c_str(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: pjsip passes NUL-terminated strings.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// SIP messages and log lines (always collected)

unsafe extern "C" fn on_message(is_rx: c_int, buf: *const c_char, len: usize, remote: *const c_char, port: c_int, transport: *const c_char) {
    guard(|| {
        let Some(feed) = feed() else { return };
        if buf.is_null() {
            return;
        }
        // SAFETY: `len` readable bytes at `buf` for the duration of the call.
        let raw = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(buf.cast::<u8>(), len) }).into_owned();
        let direction = if is_rx != 0 { MsgDirection::Received } else { MsgDirection::Sent };
        feed.push_message(direction, c_str(transport), format!("{}:{port}", c_str(remote)), raw);
    });
}

/// Installs the SIP message tap (after pjsua_init).
pub fn install_message_tap() {
    // SAFETY: pjsua is initialised; the callback contains panics.
    let status = unsafe { shim::jf_msg_tap_install(on_message) };
    if status != PJ_SUCCESS {
        log::warn!("SIP message capture unavailable ({status})");
    }
}

pub fn remove_message_tap() {
    // SAFETY: before pjsua_destroy.
    unsafe { shim::jf_msg_tap_remove() };
}

/// A pjsua log line for the debug window.
pub fn log_line(level: c_int, text: &str) {
    if let Some(feed) = feed() {
        feed.push_log(LogSource::Pjsua, u8::try_from(level.clamp(1, 6)).unwrap_or(5), text.to_owned());
    }
}

// ---------------------------------------------------------------------------
// Debug mode: verbose log, stream statistics, audio taps

/// pjsua's log level outside debug mode (5 with a log file).
static BASE_LOG_LEVEL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(3);

pub fn set_base_log_level(level: c_int) {
    BASE_LOG_LEVEL.store(level, std::sync::atomic::Ordering::Relaxed);
}

/// Marks the feed as collecting (or not) stream statistics and audio.
pub fn feed_enabled(on: bool) {
    if let Some(feed) = feed() {
        feed.set_enabled(on);
    }
}

/// Turns the extra debug collection on or off for the running session.
pub fn apply(on: bool) {
    let base = BASE_LOG_LEVEL.load(std::sync::atomic::Ordering::Relaxed);
    // SAFETY: plain pjlib call.
    unsafe { pj_log_set_level(if on { 5 } else { base }) };
    if on { add_taps() } else { remove_taps() }
}

/// Polls every call's stream statistics into the feed.
pub fn poll_streams() {
    let Some(feed) = feed().filter(|f| f.is_enabled()) else { return };
    for id in calls::call_ids() {
        if let Some(stats) = stream_stats(id) {
            feed.update_stream(id, stats);
        }
    }
}

pub fn call_ended(call: CallId) {
    if let Some(feed) = feed() {
        feed.end_stream(call);
    }
}

fn stream_stats(call: CallId) -> Option<StreamStats> {
    let mut s = std::mem::MaybeUninit::<shim::StreamSnapshot>::zeroed();
    // SAFETY: the shim fills the plain struct.
    if unsafe { shim::jf_call_stream_snapshot(call, s.as_mut_ptr()) } != PJ_SUCCESS {
        return None;
    }
    let s = unsafe { s.assume_init() };
    let text = |chars: &[c_char]| -> String {
        let bytes: Vec<u8> = chars.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let ms = |us: c_uint| us as f32 / 1000.0;
    Some(StreamStats {
        codec: text(&s.codec),
        clock_rate: s.clock_rate,
        channels: s.channels,
        remote: text(&s.remote),
        duration_ms: s.duration_ms,
        tx_packets: s.tx_pkt,
        tx_bytes: s.tx_bytes,
        rx_packets: s.rx_pkt,
        rx_bytes: s.rx_bytes,
        rx_lost: s.rx_loss,
        rx_discarded: s.rx_discard,
        rx_duplicates: s.rx_dup,
        rx_reordered: s.rx_reorder,
        rx_jitter_ms: ms(s.rx_jitter_last_us),
        rx_jitter_mean_ms: ms(s.rx_jitter_mean_us),
        rx_jitter_max_ms: ms(s.rx_jitter_max_us),
        tx_lost: s.tx_loss,
        tx_jitter_ms: ms(s.tx_jitter_last_us),
        rtt_ms: ms(s.rtt_last_us),
        rtt_mean_ms: ms(s.rtt_mean_us),
        jb_frame_bytes: s.jb_frame_bytes,
        jb_frames: s.jb_size,
        jb_prefetch: s.jb_prefetch,
        jb_delay_ms: s.jb_avg_delay_ms,
        jb_lost: s.jb_lost,
        jb_discarded: s.jb_discard,
        jb_empty: s.jb_empty,
    })
}

struct Tap {
    port: *mut pjmedia_port,
    slot: pjsua_conf_port_id,
}

// SAFETY: only touched under the TAPS lock through thread-safe pjsua calls.
unsafe impl Send for Tap {}

#[derive(Default)]
struct Taps {
    mic: Option<Tap>,
    speaker: Option<Tap>,
}

static TAPS: Mutex<Taps> = Mutex::new(Taps { mic: None, speaker: None });

unsafe extern "C" fn on_audio(tap_id: c_int, samples: *const c_short, count: usize, clock_rate: c_uint) {
    guard(|| {
        let Some(feed) = feed() else { return };
        if samples.is_null() {
            return;
        }
        // SAFETY: `count` samples for the duration of the call.
        let samples = unsafe { std::slice::from_raw_parts(samples, count) };
        let tap = if tap_id == 0 { AudioTap::Mic } else { AudioTap::Speaker };
        feed.push_audio(tap, clock_rate, samples);
    });
}

fn create_tap(id: c_int) -> Option<Tap> {
    let mut port = std::ptr::null_mut();
    // SAFETY: the shim adds a port to the running bridge.
    let slot = unsafe { shim::jf_tap_add(id, on_audio, &mut port) };
    (slot >= 0).then_some(Tap { port, slot })
}

fn destroy_tap(tap: Tap) {
    // SAFETY: created by `create_tap`, removed once. The tap frees its own
    // pool when the bridge has finished removing it.
    unsafe {
        pjsua_conf_remove_port(tap.slot);
        pjmedia_port_destroy(tap.port);
    }
}

fn add_taps() {
    let mut taps = TAPS.lock().unwrap_or_else(|e| e.into_inner());
    if taps.mic.is_none() {
        taps.mic = create_tap(0);
        if let Some(t) = &taps.mic {
            // SAFETY: valid slots.
            unsafe { pjsua_conf_connect(0, t.slot) };
        }
    }
    if taps.speaker.is_none() {
        taps.speaker = create_tap(1);
    }
    if let Some(t) = &taps.speaker {
        for id in calls::call_ids() {
            // SAFETY: valid slots; connecting twice is harmless.
            let slot = unsafe { pjsua_call_get_conf_port(id) };
            if slot != PJSUA_INVALID_ID {
                unsafe { pjsua_conf_connect(slot, t.slot) };
            }
        }
    }
}

/// Connects a call whose media became active to the speaker tap.
pub fn tap_call(slot: pjsua_conf_port_id) {
    if !enabled() {
        return;
    }
    if let Some(t) = &TAPS.lock().unwrap_or_else(|e| e.into_inner()).speaker {
        // SAFETY: valid slots.
        unsafe { pjsua_conf_connect(slot, t.slot) };
    }
}

pub fn remove_taps() {
    let mut taps = TAPS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(t) = taps.mic.take() {
        destroy_tap(t);
    }
    if let Some(t) = taps.speaker.take() {
        destroy_tap(t);
    }
}
