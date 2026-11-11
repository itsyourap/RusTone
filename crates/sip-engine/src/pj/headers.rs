//! Custom headers: the shim's module changes the main account's requests
//! (those pjsip makes by itself too, like PRACK) by the account's rules.

use std::ffi::CString;

use pjsua_sys::{PJ_SUCCESS, shim};

use crate::config::AccountConfig;

/// Registers the module (after pjsua_init).
pub fn install() {
    // SAFETY: pjsua is initialised.
    let status = unsafe { shim::jf_headers_install() };
    if status != PJ_SUCCESS {
        log::warn!("custom headers unavailable ({status})");
    }
}

/// Unregisters it (before pjsua_destroy).
pub fn remove() {
    // SAFETY: before pjsua_destroy.
    unsafe { shim::jf_headers_remove() };
}

/// Applies `account`'s rules, or none.
pub fn set(account: Option<&AccountConfig>) {
    let (host, rules) = account.map_or_else(Default::default, |a| (a.from_host(), a.header_rules_text()));
    let (Ok(host), Ok(rules)) = (CString::new(host), CString::new(rules)) else {
        log::warn!("custom headers contain a NUL; not applied");
        return;
    };
    // SAFETY: the shim copies both strings.
    let status = unsafe { shim::jf_headers_set(host.as_ptr(), rules.as_ptr()) };
    if status != PJ_SUCCESS {
        log::warn!("custom headers not applied ({status})");
    } else if account.is_some_and(|a| !a.headers.is_empty()) {
        log::info!("{} custom header rule(s) for the account's requests", account.map_or(0, |a| a.headers.len()));
    }
}
