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

#[cfg(test)]
mod tests {
    use super::*;

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
