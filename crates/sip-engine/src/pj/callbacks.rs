//! pjsua callbacks. They run on pjsua's worker threads, copy what they need
//! into owned [`Event`]s, and never unwind into C.

use std::ffi::{c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::RwLock;

use pjsua_sys::*;

use super::calls::{self, ToneKey};
use super::util::{error_text, pj_to_string};
use crate::call::{CallState, DtmfMethod};
use crate::{Event, Message, RegState, Sink};

static SINK: RwLock<Option<Sink>> = RwLock::new(None);
/// pjsua id of the main (registering) account, or -1.
static MAIN_ACCOUNT: AtomicI32 = AtomicI32::new(PJSUA_INVALID_ID);

/// The engine thread's queue, for callbacks that need it to act.
static ENGINE_TX: std::sync::Mutex<Option<std::sync::mpsc::Sender<Message>>> = std::sync::Mutex::new(None);

/// Call options that change without a restart.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallOptions {
    pub dtmf_method: DtmfMethod,
    /// Play DTMF digits locally.
    pub local_dtmf: bool,
    /// Answer incoming calls with our codec order rather than the
    /// caller's.
    pub force_codec: bool,
}

static OPTIONS: RwLock<CallOptions> = RwLock::new(CallOptions { dtmf_method: DtmfMethod::Auto, local_dtmf: true, force_codec: false });

pub fn set_engine_tx(tx: Option<std::sync::mpsc::Sender<Message>>) {
    *ENGINE_TX.lock().unwrap_or_else(|e| e.into_inner()) = tx;
}

pub fn set_options(options: CallOptions) {
    *OPTIONS.write().unwrap_or_else(|e| e.into_inner()) = options;
}

pub fn options() -> CallOptions {
    *OPTIONS.read().unwrap_or_else(|e| e.into_inner())
}

pub fn install_sink(sink: Sink) {
    *SINK.write().unwrap_or_else(|e| e.into_inner()) = Some(sink);
}

pub fn remove_sink() {
    *SINK.write().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn set_main_account(acc_id: pjsua_acc_id) {
    MAIN_ACCOUNT.store(acc_id, Ordering::SeqCst);
}

/// Hands a message to the engine thread.
fn send_engine(message: Message) {
    if let Some(tx) = ENGINE_TX.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(message);
    }
}

pub fn emit(event: Event) {
    if let Some(sink) = SINK.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        sink(event);
    }
}

pub(super) fn guard(f: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(f)).is_err() {
        log::error!("panic in a pjsua callback (contained)");
    }
}

fn is_main(acc_id: pjsua_acc_id) -> bool {
    acc_id != PJSUA_INVALID_ID && acc_id == MAIN_ACCOUNT.load(Ordering::SeqCst)
}

pub unsafe extern "C" fn on_log(level: c_int, data: *const c_char, len: c_int) {
    guard(|| {
        // SAFETY: pjsua passes `len` readable bytes.
        let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len.max(0) as usize) };
        let text = String::from_utf8_lossy(bytes);
        let text = strip_timestamp(text.trim_end());
        super::debug::log_line(level, text);
        match level {
            ..=1 => log::error!(target: "sip_engine::pjsua", "{text}"),
            2 => log::warn!(target: "sip_engine::pjsua", "{text}"),
            3 => log::info!(target: "sip_engine::pjsua", "{text}"),
            _ => log::debug!(target: "sip_engine::pjsua", "{text}"),
        }
    });
}

/// pjsua prefixes `HH:MM:SS.mmm ` (the file log needs it; `log` adds its own).
fn strip_timestamp(line: &str) -> &str {
    let b = line.as_bytes();
    let is_stamp = b.len() > 13
        && b[2] == b':'
        && b[5] == b':'
        && b[8] == b'.'
        && b[12] == b' '
        && [0, 1, 3, 4, 6, 7, 9, 10, 11].iter().all(|&i| b[i].is_ascii_digit());
    if is_stamp { line[13..].trim_start() } else { line }
}

pub unsafe extern "C" fn on_reg_started2(acc_id: pjsua_acc_id, info: *mut pjsua_reg_info) {
    guard(|| {
        // SAFETY: pjsua passes a valid info for the duration of the call.
        if is_main(acc_id) && !info.is_null() && unsafe { (*info).renew } != 0 {
            emit(Event::Registration(RegState::Registering));
        }
    });
}

pub unsafe extern "C" fn on_reg_state2(acc_id: pjsua_acc_id, info: *mut pjsua_reg_info) {
    guard(|| {
        if !is_main(acc_id) || info.is_null() {
            return;
        }
        // SAFETY: valid for the duration of the callback.
        let info = unsafe { &*info };
        if info.cbparam.is_null() {
            return;
        }
        let param = unsafe { &*info.cbparam };
        let state = reg_state(info, param);
        if matches!(state, RegState::Registered { .. }) {
            // Presence subscriptions start once registered.
            send_engine(Message::Registered);
        }
        emit(Event::Registration(state));
    });
}

fn reg_state(info: &pjsua_reg_info, param: &pjsip_regc_cbparam) -> RegState {
    let code = u16::try_from(param.code).unwrap_or(0);
    if info.renew == 0 || param.is_unreg != 0 {
        return RegState::Unregistered;
    }
    if (200..300).contains(&code) && param.status == PJ_SUCCESS && param.expiration > 0 {
        return RegState::Registered { expires: param.expiration };
    }
    let mut reason = pj_to_string(&param.reason);
    if param.status != PJ_SUCCESS && (reason.is_empty() || code == 0) {
        reason = error_text(param.status);
    }
    RegState::Failed { code, reason, detail: registrar_error(info) }
}

pub unsafe extern "C" fn on_incoming_call(_acc_id: pjsua_acc_id, call_id: pjsua_call_id, rdata: *mut pjsip_rx_data) {
    guard(|| {
        calls::capture_headers(call_id, rdata);
        // SAFETY: rdata is the incoming INVITE, valid during the callback.
        if options().force_codec && unsafe { shim::jf_prefer_local_codec_order(rdata) } != PJ_SUCCESS {
            log::warn!("cannot prefer our codec order for call {call_id}");
        }
        if let Some(info) = calls::call_info(call_id) {
            emit(Event::IncomingCall(info));
        }
    });
}

pub unsafe extern "C" fn on_call_state(call_id: pjsua_call_id, _e: *mut pjsip_event) {
    guard(|| {
        let Some(info) = calls::call_info(call_id) else { return };
        // The 481 that follows a replaced call is ignored.
        if info.state == CallState::Disconnected && info.status_code == 481 {
            calls::forget_call(call_id);
            return;
        }
        let ended = info.state == CallState::Disconnected;
        emit(Event::Call(info));
        if ended {
            calls::forget_call(call_id);
            super::debug::call_ended(call_id);
        }
    });
}

pub unsafe extern "C" fn on_call_media_state(call_id: pjsua_call_id) {
    guard(|| {
        calls::update_media(call_id);
        if let Some(info) = calls::call_info(call_id) {
            emit(Event::Call(info));
        }
    });
}

pub unsafe extern "C" fn on_call_rx_offer(call_id: pjsua_call_id, offer: *const pjmedia_sdp_session, _reserved: *mut c_void, _code: *mut pjsip_status_code, opt: *mut pjsua_call_setting) {
    guard(|| calls::on_reoffer(call_id, offer, opt));
}

pub unsafe extern "C" fn on_dtmf_digit(call_id: pjsua_call_id, digit: c_int) {
    guard(|| {
        let Some(digit) = u8::try_from(digit).ok().map(char::from) else { return };
        if options().local_dtmf {
            calls::play_digits(ToneKey::Local, &digit.to_string());
        }
        emit(Event::DtmfReceived { call: call_id, digit });
    });
}

/// Declines (603) transfers of unconfirmed calls, and transfers to
/// a party already in a call with us.
pub unsafe extern "C" fn on_call_transfer_request2(call_id: pjsua_call_id, dst: *const pj_str_t, code: *mut pjsip_status_code, _opt: *mut pjsua_call_setting) {
    guard(|| {
        // SAFETY: valid for the duration of the callback.
        let target = if dst.is_null() { String::new() } else { pj_to_string(unsafe { &*dst }) };
        let confirmed = calls::call_info(call_id).is_some_and(|i| i.state == CallState::Confirmed);
        let target_key = calls::uri_user_host(&target);
        let duplicate = calls::call_ids().into_iter().filter(|&id| id != call_id).any(|id| {
            calls::call_info(id).is_some_and(|i| calls::uri_user_host(&i.remote) == target_key)
        });
        if (!confirmed || duplicate) && !code.is_null() {
            unsafe { *code = 603 };
        }
        log::info!("transfer request for call {call_id} ({})", if confirmed && !duplicate { "accepted" } else { "declined" });
    });
}

pub unsafe extern "C" fn on_call_transfer_status(call_id: pjsua_call_id, st_code: c_int, st_text: *const pj_str_t, final_: pj_bool_t, p_cont: *mut pj_bool_t) {
    guard(|| {
        let text = if st_text.is_null() { String::new() } else { pj_to_string(unsafe { &*st_text }) };
        let code = u16::try_from(st_code).unwrap_or(0);
        let success = (200..300).contains(&code);
        emit(Event::TransferStatus { call: call_id, code, text, done: final_ != 0 || success });
        // Transferred: stop following and hang up our leg.
        if success {
            if !p_cont.is_null() {
                unsafe { *p_cont = PJ_FALSE };
            }
            unsafe { pjsua_call_hangup(call_id, 0, std::ptr::null(), std::ptr::null()) };
        }
    });
}

pub unsafe extern "C" fn on_call_redirected(_call_id: pjsua_call_id, _target: *const pjsip_uri, _e: *const pjsip_event) -> pjsip_redirect_op {
    PJSIP_REDIRECT_ACCEPT_REPLACE
}

/// A non-looping sound finished: ask the engine thread to destroy the player
/// (players can't be destroyed from their own callback).
pub unsafe extern "C" fn on_player_eof(_port: *mut pjmedia_port, user_data: *mut c_void) {
    guard(|| {
        let id = user_data as isize as pjsua_player_id;
        if let Some(tx) = ENGINE_TX.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = tx.send(Message::PlayerEnded(id));
        }
    });
}

/// The server's `P-Registrar-Error`, shown for rejected registrations.
fn registrar_error(info: &pjsua_reg_info) -> Option<String> {
    let mut buf = [0 as c_char; 512];
    // SAFETY: the shim only reads `info` and writes a NUL-terminated string into `buf`.
    let len = unsafe { shim::jf_reg_info_header(info, c"P-Registrar-Error".as_ptr(), buf.as_mut_ptr(), buf.len()) };
    if len < 0 {
        return None;
    }
    // SAFETY: NUL-terminated by the shim.
    let value = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().trim().to_owned();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pjsua_timestamps_are_stripped() {
        assert_eq!(strip_timestamp("21:43:57.672    pjsua_acc.c  ..SIP registration failed"), "pjsua_acc.c  ..SIP registration failed");
        assert_eq!(strip_timestamp("no stamp here"), "no stamp here");
        assert_eq!(strip_timestamp("12:34"), "12:34");
    }
}
