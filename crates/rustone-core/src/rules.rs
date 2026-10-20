//! Incoming-call rules: which calls are
//! rejected. Pure functions over already-parsed addresses.

use crate::settings::DenyIncoming;

/// The account a call came in on: its user and domain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub user: String,
    pub domain: String,
}

/// An incoming call's addresses as `user@host` (or just `host`).
#[derive(Debug, Clone, Copy)]
pub struct Addresses<'a> {
    /// Who the call is addressed to (our side, the request's To).
    pub local: &'a str,
    /// Who is calling.
    pub remote: &'a str,
}

fn split(address: &str) -> (&str, &str) {
    address.split_once('@').unwrap_or(("", address))
}

/// Whether `mode` rejects a call. Without an account only "all" does, as
/// the domain modes are checked against the account.
pub fn denied(mode: DenyIncoming, account: Option<&Identity>, call: Addresses<'_>) -> bool {
    let (local_user, local_domain) = split(call.local);
    let (_, remote_domain) = split(call.remote);
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    match (mode, account) {
        (DenyIncoming::Off, _) => false,
        (DenyIncoming::All, _) => true,
        (_, None) => false,
        (DenyIncoming::OtherUser, Some(a)) => local_user != a.user,
        (DenyIncoming::OtherDomain, Some(a)) => !same(local_domain, &a.domain),
        (DenyIncoming::RemoteDomain, Some(a)) => !same(remote_domain, &a.domain),
        (DenyIncoming::OtherUserOrDomain, Some(a)) => local_user != a.user || (!a.domain.is_empty() && !same(local_domain, &a.domain)),
    }
}

/// Whether the caller asks to be answered automatically:
/// `X-AUTOANSWER: true|1`, or a `Call-Info` with
/// "auto answer" or `answer-after=N`. `Some(Some(n))` carries the delay.
pub fn auto_answer_requested(x_autoanswer: Option<&str>, call_info: Option<&str>) -> Option<Option<u32>> {
    if x_autoanswer.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1")) {
        return Some(None);
    }
    let info = call_info?.to_ascii_lowercase();
    if info.contains("auto answer") {
        return Some(None);
    }
    let after = info.split_once("answer-after=")?.1;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().map(Some)
}

/// Whether `number` matches one of `patterns` (separated by `;` or `|`,
/// with `*` and `?` wildcards). No patterns matches everyone.
pub fn number_allowed(patterns: &str, number: &str) -> bool {
    let mut any = false;
    for pattern in patterns.split([';', '|']).map(str::trim).filter(|p| !p.is_empty()) {
        any = true;
        if wildcard(pattern.as_bytes(), number.as_bytes()) {
            return true;
        }
    }
    !any
}

/// `*` matches any run of characters, `?` any one; case-insensitive.
fn wildcard(pattern: &[u8], text: &[u8]) -> bool {
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some(b'*'), _) => wildcard(&pattern[1..], text) || (!text.is_empty() && wildcard(pattern, &text[1..])),
        (Some(b'?'), Some(_)) => wildcard(&pattern[1..], &text[1..]),
        (Some(p), Some(t)) if p.eq_ignore_ascii_case(t) => wildcard(&pattern[1..], &text[1..]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callers_ask_for_auto_answer() {
        assert_eq!(auto_answer_requested(Some("TRUE"), None), Some(None));
        assert_eq!(auto_answer_requested(Some("1"), None), Some(None));
        assert_eq!(auto_answer_requested(Some("false"), None), None);
        assert_eq!(auto_answer_requested(None, Some("<http://x>;answer-after=5")), Some(Some(5)));
        assert_eq!(auto_answer_requested(None, Some("<sip:x>;Answer-After=0")), Some(Some(0)));
        assert_eq!(auto_answer_requested(None, Some("<sip:x>;info=Auto Answer")), Some(None));
        assert_eq!(auto_answer_requested(None, Some("<sip:x>;purpose=icon")), None);
        assert_eq!(auto_answer_requested(None, None), None);
    }

    #[test]
    fn numbers_match_with_wildcards() {
        assert!(number_allowed("", "100"));
        assert!(number_allowed(" ; ", "100"));
        assert!(number_allowed("100;200", "200"));
        assert!(number_allowed("1*|5??", "1999"));
        assert!(number_allowed("1*|5??", "512"));
        assert!(!number_allowed("1*|5??", "5123"));
        assert!(!number_allowed("100", "1000"));
        assert!(number_allowed("+91*", "+915550100001"));
    }

    fn me() -> Identity {
        Identity { user: "+915550100001".into(), domain: "sip.example.com".into() }
    }

    #[test]
    fn deny_modes_pick_calls() {
        let to_me = Addresses { local: "+915550100001@sip.example.com", remote: "05550100002@192.168.29.1" };
        let to_other = Addresses { local: "100@sip.example.com", remote: "200@sip.example.com" };
        let other_domain = Addresses { local: "+915550100001@192.168.29.54", remote: "200@SIP.EXAMPLE.COM" };
        assert!(!denied(DenyIncoming::Off, Some(&me()), to_other));
        assert!(denied(DenyIncoming::All, None, to_me));
        assert!(!denied(DenyIncoming::OtherUser, Some(&me()), to_me));
        assert!(denied(DenyIncoming::OtherUser, Some(&me()), to_other));
        assert!(denied(DenyIncoming::OtherDomain, Some(&me()), other_domain));
        assert!(!denied(DenyIncoming::OtherDomain, Some(&me()), to_me));
        assert!(denied(DenyIncoming::RemoteDomain, Some(&me()), to_me));
        assert!(!denied(DenyIncoming::RemoteDomain, Some(&me()), other_domain));
        assert!(denied(DenyIncoming::OtherUserOrDomain, Some(&me()), other_domain));
        assert!(denied(DenyIncoming::OtherUserOrDomain, Some(&me()), to_other));
        assert!(!denied(DenyIncoming::OtherUserOrDomain, Some(&me()), to_me));
        // Without an account, only "all" rejects.
        assert!(!denied(DenyIncoming::OtherUser, None, to_other));
    }
}
