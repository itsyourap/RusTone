//! Calls, tone generators and the sound player.
//! Functions here may run on the engine thread or
//! on pjsua's callback threads; shared state is behind mutexes.

use std::collections::HashMap;
use std::ffi::{c_char, c_short, c_uint, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use pjsua_sys::*;

use super::util::{pj_default, pj_to_string};
pub use crate::call::uri_user_host;
use crate::call::{CallId, CallInfo, CallState, Direction, MediaState};

// ---------------------------------------------------------------------------
// Call snapshots and incoming headers

/// Headers captured from an incoming INVITE.
#[derive(Debug, Clone, Default)]
pub struct Headers {
    pub caller_id: Option<String>,
    pub user_agent: Option<String>,
    pub diversion: Option<String>,
}

static HEADERS: LazyLock<Mutex<HashMap<CallId, Headers>>> = LazyLock::new(Mutex::default);

/// Reads caller ID, User-Agent and Diversion from an incoming INVITE.
pub fn capture_headers(call_id: CallId, rdata: *const pjsip_rx_data) {
    let caller_id = rdata_header(rdata, c"P-Asserted-Identity")
        .or_else(|| rdata_header(rdata, c"Remote-Party-ID"))
        .and_then(|v| caller_id_value(&v));
    let user_agent = rdata_header(rdata, c"User-Agent").map(|v| user_agent_value(&v));
    let diversion = rdata_header(rdata, c"Diversion").map(|v| {
        let uri = uri_user_host(&v);
        match uri.split_once('@') {
            Some((user, _)) if !user.is_empty() => user.to_owned(),
            Some((_, host)) => host.to_owned(),
            None => uri,
        }
    });
    lock(&HEADERS).insert(call_id, Headers { caller_id, user_agent, diversion });
}

fn rdata_header(rdata: *const pjsip_rx_data, name: &std::ffi::CStr) -> Option<String> {
    let mut buf = [0 as c_char; 512];
    // SAFETY: rdata is valid during the incoming-call callback; the shim
    // writes a NUL-terminated string into `buf`.
    let len = unsafe { shim::jf_rdata_header(rdata, name.as_ptr(), buf.as_mut_ptr(), buf.len()) };
    if len < 0 {
        return None;
    }
    let printed = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
    // "Name: value" → value.
    let value = printed.split_once(':').map_or(printed.as_str(), |(_, v)| v).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

/// Keeps a caller ID only when it holds a SIP address, and drops
/// header parameters after it.
fn caller_id_value(value: &str) -> Option<String> {
    if !value.contains('@') {
        return None;
    }
    let end = match value.find('>') {
        Some(close) => close + 1,
        None => value.find(';').unwrap_or(value.len()),
    };
    Some(value[..end].trim().to_owned())
}

/// Cuts the User-Agent at the first of `~+-` (only when present).
fn user_agent_value(value: &str) -> String {
    let end = value.find(['~', '+', '-']).unwrap_or(value.len());
    value[..end].trim().to_owned()
}

pub fn forget_call(call_id: CallId) {
    lock(&HEADERS).remove(&call_id);
    drop_tonegen(ToneKey::Call(call_id));
}

/// A snapshot of `call_id`, or `None` if pjsua doesn't know it.
pub fn call_info(call_id: CallId) -> Option<CallInfo> {
    let mut info = std::mem::MaybeUninit::<pjsua_call_info>::zeroed();
    // SAFETY: filled on success and read in place (its strings point into it).
    unsafe {
        if pjsua_call_get_info(call_id, info.as_mut_ptr()) != PJ_SUCCESS {
            return None;
        }
        Some(snapshot(info.assume_init_ref()))
    }
}

fn snapshot(ci: &pjsua_call_info) -> CallInfo {
    let state = match ci.state {
        PJSIP_INV_STATE_CALLING => CallState::Calling,
        PJSIP_INV_STATE_INCOMING => CallState::Incoming,
        PJSIP_INV_STATE_EARLY => CallState::Early,
        PJSIP_INV_STATE_CONNECTING => CallState::Connecting,
        PJSIP_INV_STATE_CONFIRMED => CallState::Confirmed,
        _ => CallState::Disconnected,
    };
    let media = media_state(ci.media_status);
    let headers = lock(&HEADERS).get(&ci.id).cloned().unwrap_or_default();
    CallInfo {
        id: ci.id,
        direction: if ci.role == PJSIP_ROLE_UAS { Direction::Incoming } else { Direction::Outgoing },
        state,
        status_code: u16::try_from(ci.last_status).unwrap_or(0),
        status_text: pj_to_string(&ci.last_status_text),
        remote: pj_to_string(&ci.remote_info),
        local: pj_to_string(&ci.local_info),
        media,
        early_media: ci.media_cnt > 0 && media == MediaState::Active && state != CallState::Confirmed,
        connected_for: Duration::from_secs(u64::try_from(ci.connect_duration.sec).unwrap_or(0))
            + Duration::from_millis(u64::try_from(ci.connect_duration.msec).unwrap_or(0)),
        remote_video: ci.rem_vid_cnt > 0,
        sip_call_id: pj_to_string(&ci.call_id),
        caller_id: headers.caller_id,
        user_agent: headers.user_agent,
        diversion: headers.diversion,
    }
}

fn media_state(status: pjsua_call_media_status) -> MediaState {
    match status {
        PJSUA_CALL_MEDIA_ACTIVE => MediaState::Active,
        PJSUA_CALL_MEDIA_LOCAL_HOLD => MediaState::LocalHold,
        PJSUA_CALL_MEDIA_REMOTE_HOLD => MediaState::RemoteHold,
        PJSUA_CALL_MEDIA_ERROR => MediaState::Error,
        _ => MediaState::None,
    }
}

/// Ids of all calls pjsua knows.
pub fn call_ids() -> Vec<CallId> {
    let mut ids = [PJSUA_INVALID_ID; 32];
    let mut count = ids.len() as c_uint;
    // SAFETY: pjsua fills at most `count` ids.
    if unsafe { pjsua_enum_calls(ids.as_mut_ptr(), &mut count) } != PJ_SUCCESS {
        return Vec::new();
    }
    ids[..count as usize].to_vec()
}

// ---------------------------------------------------------------------------
// Media bridging and mute

static MIC_MUTED: AtomicBool = AtomicBool::new(false);
static SPEAKER_MUTED: AtomicBool = AtomicBool::new(false);

fn call_slot(call_id: CallId) -> Option<pjsua_conf_port_id> {
    // SAFETY: plain query.
    let slot = unsafe { pjsua_call_get_conf_port(call_id) };
    (slot != PJSUA_INVALID_ID).then_some(slot)
}

/// Bridges the call's audio with the sound device while media is active
/// or held by the remote; disconnects it during local hold.
pub fn update_media(call_id: CallId) {
    let Some(info) = call_info(call_id) else { return };
    let Some(slot) = call_slot(call_id) else { return };
    // SAFETY: valid slots of this pjsua instance.
    unsafe {
        if matches!(info.media, MediaState::Active | MediaState::RemoteHold) {
            pjsua_conf_connect(slot, 0);
            pjsua_conf_connect(0, slot);
            super::debug::tap_call(slot);
        } else {
            pjsua_conf_disconnect(slot, 0);
            pjsua_conf_disconnect(0, slot);
            drop_tonegen(ToneKey::Call(call_id));
        }
    }
    apply_levels();
}

pub fn set_mute(mic: bool, speaker: bool) {
    MIC_MUTED.store(mic, Ordering::SeqCst);
    SPEAKER_MUTED.store(speaker, Ordering::SeqCst);
    apply_levels();
}

/// Mic mute is the level of the sound device's capture into the bridge;
/// speaker mute is each call's level towards the sound device.
fn apply_levels() {
    let mic = if MIC_MUTED.load(Ordering::SeqCst) { 0.0 } else { 1.0 };
    let speaker = if SPEAKER_MUTED.load(Ordering::SeqCst) { 0.0 } else { 1.0 };
    // SAFETY: valid slots of this pjsua instance.
    unsafe {
        pjsua_conf_adjust_rx_level(0, mic);
        for id in call_ids() {
            if let Some(slot) = call_slot(id) {
                pjsua_conf_adjust_rx_level(slot, speaker);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tone generators for in-band and local DTMF

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToneKey {
    /// Tones mixed into a call, heard by the remote.
    Call(CallId),
    /// Tones played on the local speaker only.
    Local,
}

struct Tonegen {
    pool: *mut pj_pool_t,
    port: *mut pjmedia_port,
    slot: pjsua_conf_port_id,
}

// SAFETY: the pointers are only used under the TONEGENS lock, through
// pjsua/pjmedia calls that are themselves thread-safe.
unsafe impl Send for Tonegen {}

static TONEGENS: LazyLock<Mutex<HashMap<ToneKey, Tonegen>>> = LazyLock::new(Mutex::default);

/// Plays DTMF `digits` (160 ms on, 50 ms off, at most 16) on the tone
/// generator for `key`, creating it on first use.
pub fn play_digits(key: ToneKey, digits: &str) -> bool {
    let mut tones: Vec<pjmedia_tone_digit> = digits
        .chars()
        .filter(|c| c.is_ascii_digit() || "*#ABCDabcd".contains(*c))
        .take(16)
        .map(|c| pjmedia_tone_digit { digit: c.to_ascii_uppercase() as c_char, on_msec: 160, off_msec: 50, volume: 0 as c_short })
        .collect();
    if tones.is_empty() {
        return false;
    }
    let mut map = lock(&TONEGENS);
    let tg = match map.entry(key) {
        std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
        std::collections::hash_map::Entry::Vacant(e) => match create_tonegen(key) {
            Some(t) => e.insert(t),
            None => return false,
        },
    };
    // SAFETY: `tg.port` is a live tone generator; the digits are copied.
    unsafe { pjmedia_tonegen_play_digits(tg.port, tones.len() as c_uint, tones.as_mut_ptr(), 0) == PJ_SUCCESS }
}

fn create_tonegen(key: ToneKey) -> Option<Tonegen> {
    let target = match key {
        ToneKey::Call(id) => call_slot(id)?,
        ToneKey::Local => 0,
    };
    // SAFETY: standard pjmedia setup; everything is released in `destroy`.
    unsafe {
        let pool = pjsua_pool_create(c"jf-tonegen".as_ptr(), 512, 512);
        if pool.is_null() {
            return None;
        }
        let mut port = std::ptr::null_mut();
        if pjmedia_tonegen_create(pool, 8000, 1, 64, 16, 0, &mut port) != PJ_SUCCESS {
            pj_pool_release(pool);
            return None;
        }
        let mut slot = PJSUA_INVALID_ID;
        if pjsua_conf_add_port(pool, port, &mut slot) != PJ_SUCCESS {
            pjmedia_port_destroy(port);
            pj_pool_release(pool);
            return None;
        }
        if key == ToneKey::Local {
            pjsua_conf_adjust_rx_level(slot, 0.4);
        }
        pjsua_conf_connect(slot, target);
        Some(Tonegen { pool, port, slot })
    }
}

pub fn drop_tonegen(key: ToneKey) {
    if let Some(tg) = lock(&TONEGENS).remove(&key) {
        destroy_tonegen(tg);
    }
}

pub fn drop_all_tonegens() {
    for (_, tg) in lock(&TONEGENS).drain() {
        destroy_tonegen(tg);
    }
}

fn destroy_tonegen(tg: Tonegen) {
    // SAFETY: created by `create_tonegen`, removed from the map first.
    unsafe {
        pjsua_conf_remove_port(tg.slot);
        pjmedia_port_destroy(tg.port);
        pj_pool_release(tg.pool);
    }
}

// ---------------------------------------------------------------------------
// SIP INFO DTMF

/// Sends each digit as `INFO` with `application/dtmf-relay`. Stops at the
/// first failure.
pub fn send_dtmf_info(call_id: CallId, digits: &str) -> bool {
    let method = pj_str_t { ptr: c"INFO".as_ptr().cast_mut(), slen: 4 };
    let ctype = pj_str_t { ptr: c"application/dtmf-relay".as_ptr().cast_mut(), slen: 22 };
    for digit in digits.chars() {
        let body = format!("Signal={digit}\r\nDuration=160");
        let body_c = std::ffi::CString::new(body).expect("no NUL");
        pj_default!(msg: pjsua_msg_data = pjsua_msg_data_init);
        msg.content_type = ctype;
        msg.msg_body = pj_str_t { ptr: body_c.as_ptr().cast_mut(), slen: body_c.as_bytes().len() as pj_ssize_t };
        // SAFETY: `msg` initialised in place; strings outlive the call.
        if unsafe { pjsua_call_send_request(call_id, &method, &msg) } != PJ_SUCCESS {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// The sound player: one at a time.

pub struct Player {
    id: pjsua_player_id,
    slot: pjsua_conf_port_id,
}

impl Player {
    /// Starts playing `path` to the sound device. Non-looping sounds call
    /// `on_end(id)` from a media thread when they finish.
    pub fn start(path: &std::path::Path, looping: bool, level: f32, on_end: unsafe extern "C" fn(*mut pjmedia_port, *mut c_void)) -> Result<Self, String> {
        let file = std::ffi::CString::new(path.to_string_lossy().into_owned()).map_err(|e| e.to_string())?;
        let name = pj_str_t { ptr: file.as_ptr().cast_mut(), slen: file.as_bytes().len() as pj_ssize_t };
        let options = if looping { 0 } else { PJMEDIA_FILE_NO_LOOP as c_uint };
        let mut id = PJSUA_INVALID_ID;
        // SAFETY: pjsua copies the file name.
        let status = unsafe { pjsua_player_create(&name, options, &mut id) };
        super::util::check(status, &format!("playing {}", path.display()))?;
        // SAFETY: valid player id.
        let slot = unsafe { pjsua_player_get_conf_port(id) };
        if !looping {
            let mut port = std::ptr::null_mut();
            unsafe {
                if pjsua_player_get_port(id, &mut port) == PJ_SUCCESS {
                    pjmedia_wav_player_set_eof_cb2(port, id as isize as *mut c_void, Some(on_end));
                }
            }
        }
        unsafe {
            pjsua_conf_adjust_rx_level(slot, level.clamp(0.0, 1.0));
            pjsua_conf_connect(slot, 0);
        }
        Ok(Self { id, slot })
    }

    pub fn id(&self) -> pjsua_player_id {
        self.id
    }

    pub fn stop(self) {
        // SAFETY: valid player of this pjsua instance, stopped once.
        unsafe {
            pjsua_conf_disconnect(self.slot, 0);
            pjsua_player_destroy(self.id);
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_id_keeps_sip_addresses() {
        assert_eq!(caller_id_value("\"Alice\" <sip:+911234@ims.example;user=phone>;privacy=off").unwrap(), "\"Alice\" <sip:+911234@ims.example;user=phone>");
        assert_eq!(caller_id_value("sip:1001@pbx;foo=bar").unwrap(), "sip:1001@pbx");
        assert_eq!(caller_id_value("tel:+911234"), None);
    }

    #[test]
    fn user_agent_is_trimmed_only_when_needed() {
        assert_eq!(user_agent_value("Softphone/3.22.3"), "Softphone/3.22.3");
        assert_eq!(user_agent_value("Phone 1.2~beta"), "Phone 1.2");
    }

}
