//! SIP MESSAGE.

use std::ffi::{c_int, c_void};

use pjsua_sys::*;

use super::util::pj_to_string;
use crate::Event;
use crate::message::{IncomingMessage, MessageId, MessageStatus, is_plain_text};

/// Sends `text` as text/plain to `uri` (a bare SIP URI) from `acc`.
pub fn send(acc: pjsua_acc_id, uri: &str, text: &str, id: MessageId) -> Result<(), String> {
    let to = std::ffi::CString::new(uri).map_err(|e| e.to_string())?;
    let body = std::ffi::CString::new(text.replace('\0', "")).map_err(|e| e.to_string())?;
    let to_str = pj_str_t { ptr: to.as_ptr().cast_mut(), slen: to.as_bytes().len() as pj_ssize_t };
    let content = pj_str_t { ptr: body.as_ptr().cast_mut(), slen: body.as_bytes().len() as pj_ssize_t };
    // The id rides along as user_data, to match the delivery status.
    let user_data = id as usize as *mut c_void;
    // SAFETY: pjsua copies the strings; a NULL mime type means text/plain.
    let status = unsafe { pjsua_im_send(acc, &to_str, std::ptr::null(), &content, std::ptr::null(), user_data) };
    super::util::check(status, "sending the message")
}

/// pjsua's on_pager2: plain-text messages only.
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn on_pager2(
    _call_id: pjsua_call_id,
    from: *const pj_str_t,
    _to: *const pj_str_t,
    _contact: *const pj_str_t,
    mime_type: *const pj_str_t,
    body: *const pj_str_t,
    rdata: *mut pjsip_rx_data,
    _acc_id: pjsua_acc_id,
) {
    super::callbacks::guard(|| {
        // SAFETY: valid for the duration of the callback.
        let (mime, from, text) = unsafe { (pj_to_string(&*mime_type), pj_to_string(&*from), pj_to_string(&*body)) };
        if !is_plain_text(&mime) {
            return;
        }
        let from = super::calls::caller_id(rdata).unwrap_or(from);
        super::callbacks::emit(Event::MessageReceived(IncomingMessage { from, text: text.trim().to_owned() }));
    });
}

/// pjsua's on_pager_status2: every final answer to a message we sent.
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn on_pager_status2(
    _call_id: pjsua_call_id,
    _to: *const pj_str_t,
    _body: *const pj_str_t,
    user_data: *mut c_void,
    status: pjsip_status_code,
    reason: *const pj_str_t,
    _tdata: *mut pjsip_tx_data,
    _rdata: *mut pjsip_rx_data,
    _acc_id: pjsua_acc_id,
) {
    super::callbacks::guard(|| {
        let code = u16::try_from(status as c_int).unwrap_or(0);
        // SAFETY: valid for the duration of the callback.
        let reason = unsafe { pj_to_string(&*reason) };
        let id = user_data as usize as MessageId;
        super::callbacks::emit(Event::MessageStatus(MessageStatus { id, code, reason: reason.trim().to_owned() }));
    });
}
