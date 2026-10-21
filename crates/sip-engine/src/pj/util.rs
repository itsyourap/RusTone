//! Small helpers for calling pjsua safely.

use std::ffi::{CString, c_char};

use pjsua_sys::*;

/// Declares `$var`, zero-initialises it and runs pjsua's `*_default`
/// function on it **in place**. Some of these structs (e.g.
/// `pjsua_acc_config::reg_hdr_list`) contain self-referential `pj_list`
/// heads, so they must never be moved after initialisation.
macro_rules! pj_default {
    ($var:ident : $ty:ty = $init:path) => {
        // SAFETY: pjsua structs are plain C data; all-zero is a valid
        // starting point for their `*_default` functions.
        let mut $var: $ty = unsafe { std::mem::zeroed() };
        unsafe { $init(&mut $var) };
    };
}
pub(crate) use pj_default;

/// Keeps C strings alive while pjsua holds pointers to them.
#[derive(Default)]
pub struct Arena(Vec<CString>);

impl Arena {
    /// A `pj_str_t` borrowing a copy of `s` that lives as long as the arena.
    /// Interior NULs are dropped (they can't appear in SIP strings anyway).
    pub fn pj(&mut self, s: &str) -> pj_str_t {
        let c = CString::new(s.replace('\0', "")).expect("NULs removed");
        let out = pj_str_t { ptr: c.as_ptr().cast_mut(), slen: c.as_bytes().len() as pj_ssize_t };
        self.0.push(c);
        out
    }
}

pub fn check(status: pj_status_t, what: &str) -> Result<(), String> {
    if status == PJ_SUCCESS { Ok(()) } else { Err(format!("{what} failed: {}", error_text(status))) }
}

pub fn error_text(status: pj_status_t) -> String {
    let mut buf = [0 as c_char; 256];
    // SAFETY: the buffer outlives the returned pj_str_t, which points into it.
    let s = unsafe { pj_strerror(status, buf.as_mut_ptr(), buf.len()) };
    pj_to_string(&s)
}

/// Copies a `pj_str_t` into an owned string (lossy UTF-8).
pub fn pj_to_string(s: &pj_str_t) -> String {
    if s.ptr.is_null() || s.slen <= 0 {
        return String::new();
    }
    // SAFETY: pjsua guarantees `slen` readable bytes at `ptr`.
    let bytes = unsafe { std::slice::from_raw_parts(s.ptr.cast::<u8>(), s.slen as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

/// Copies a fixed-size, NUL-terminated C char array (lossy UTF-8).
pub fn cstr_to_string(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars.iter().take_while(|&&c| c != 0).map(|&c| c.to_ne_bytes()[0]).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Lends the account's info to `f`. `pjsua_acc_info` keeps its strings in an
/// internal buffer, so it must not be moved out.
pub fn with_acc_info<R>(acc_id: pjsua_acc_id, f: impl FnOnce(&pjsua_acc_info) -> R) -> Option<R> {
    let mut info = std::mem::MaybeUninit::<pjsua_acc_info>::zeroed();
    // SAFETY: on success pjsua has filled the struct; it isn't moved.
    unsafe { (pjsua_acc_get_info(acc_id, info.as_mut_ptr()) == PJ_SUCCESS).then(|| f(info.assume_init_ref())) }
}

/// A `pjsua_msg_data` with extra headers, for requests and responses
/// pjsua builds (302 with Contact, 486 with a reason, BYE with Warning).
/// Boxed because its header list points into itself.
pub struct MsgData {
    msg: Box<pjsua_msg_data>,
    _strings: Vec<CString>,
    _headers: Vec<Box<[u64]>>,
}

impl MsgData {
    pub fn new(headers: &[(String, String)]) -> Self {
        // SAFETY: plain C data; pjsua_msg_data_init fills it in place, and
        // the box keeps its address while the list points into it.
        let mut msg: Box<pjsua_msg_data> = Box::new(unsafe { std::mem::zeroed() });
        unsafe { pjsua_msg_data_init(&mut *msg) };
        let mut strings = Vec::new();
        let mut storage = Vec::new();
        // SAFETY: plain size query.
        let words = unsafe { shim::jf_sizeof_generic_string_hdr() }.div_ceil(8);
        for (name, value) in headers {
            let (Ok(name), Ok(value)) = (CString::new(name.as_str()), CString::new(value.as_str())) else { continue };
            let mut hdr: Box<[u64]> = vec![0u64; words].into_boxed_slice();
            // SAFETY: `hdr` is large and aligned enough for the header; it,
            // the strings and `msg` are kept (and don't move) with `self`.
            unsafe { shim::jf_msg_data_add_hdr(&mut *msg, hdr.as_mut_ptr().cast(), name.as_ptr(), value.as_ptr()) };
            strings.push(name);
            strings.push(value);
            storage.push(hdr);
        }
        Self { msg, _strings: strings, _headers: storage }
    }

    /// Uses `uri` as the From of a new call or request (pjsua's
    /// `local_uri`) instead of the account's identity.
    pub fn with_local_uri(mut self, uri: &str) -> Self {
        if let Ok(uri) = CString::new(uri) {
            self.msg.local_uri = pj_str_t { ptr: uri.as_ptr().cast_mut(), slen: uri.as_bytes().len() as pj_ssize_t };
            self._strings.push(uri);
        }
        self
    }

    pub fn as_ptr(&self) -> *const pjsua_msg_data {
        &*self.msg
    }
}
