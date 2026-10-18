//! pjsua callbacks. They run on pjsua's worker threads, copy what they need
//! into owned [`Event`]s, and never unwind into C.

use std::ffi::{c_char, c_int};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::RwLock;

use pjsua_sys::*;

use super::util::{error_text, pj_to_string};
use crate::{Event, RegState, Sink};

static SINK: RwLock<Option<Sink>> = RwLock::new(None);
/// pjsua id of the main (registering) account, or -1.
static MAIN_ACCOUNT: AtomicI32 = AtomicI32::new(PJSUA_INVALID_ID);

pub fn install_sink(sink: Sink) {
    *SINK.write().unwrap_or_else(|e| e.into_inner()) = Some(sink);
}

pub fn remove_sink() {
    *SINK.write().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn set_main_account(acc_id: pjsua_acc_id) {
    MAIN_ACCOUNT.store(acc_id, Ordering::SeqCst);
}

pub fn emit(event: Event) {
    if let Some(sink) = SINK.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        sink(event);
    }
}

fn guard(f: impl FnOnce()) {
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
        emit(Event::Registration(reg_state(info, param)));
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
