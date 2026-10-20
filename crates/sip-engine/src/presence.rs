//! Contacts' presence: PIDF status, RPID
//! activity and the note, which Asterisk and FreeSWITCH use for "Ringing"
//! and "On the phone".

/// Presence states, one icon each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PresenceState {
    #[default]
    Unknown,
    Offline,
    Online,
    Away,
    Busy,
    OnThePhone,
    /// On the phone and ringing: the call can be picked up.
    Ringing,
}

/// A contact's presence, keyed by the number it was subscribed with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    pub number: String,
    pub state: PresenceState,
    /// The PIDF note, e.g. "On the phone", or pjsua's "Online"/"Offline".
    pub note: String,
}

/// The buddy's basic status as pjsua reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuddyStatus {
    Unknown,
    Online,
    Offline,
}

/// The RPID activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Away,
    Busy,
    Other,
}

/// Maps buddy status and activity to a presence state.
pub fn classify(status: BuddyStatus, activity: Activity, note: &str) -> PresenceState {
    match status {
        BuddyStatus::Offline => PresenceState::Offline,
        BuddyStatus::Unknown => PresenceState::Unknown,
        BuddyStatus::Online => match activity {
            Activity::Away => PresenceState::Away,
            Activity::Busy => PresenceState::Busy,
            Activity::Other if note == "On the phone" => PresenceState::OnThePhone,
            Activity::Other if note.starts_with("Ring") => PresenceState::Ringing,
            Activity::Other => PresenceState::Online,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_from_status_and_activity() {
        use PresenceState as S;
        assert_eq!(classify(BuddyStatus::Offline, Activity::Busy, "Ringing"), S::Offline);
        assert_eq!(classify(BuddyStatus::Unknown, Activity::Other, ""), S::Unknown);
        assert_eq!(classify(BuddyStatus::Online, Activity::Away, ""), S::Away);
        assert_eq!(classify(BuddyStatus::Online, Activity::Busy, "On the phone"), S::Busy, "activity wins over the note");
        assert_eq!(classify(BuddyStatus::Online, Activity::Other, "On the phone"), S::OnThePhone);
        assert_eq!(classify(BuddyStatus::Online, Activity::Other, "Ringing"), S::Ringing);
        assert_eq!(classify(BuddyStatus::Online, Activity::Other, "Ready"), S::Online);
    }
}

/// What we publish about ourselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnlineStatus {
    #[default]
    Online,
    /// Idle for a while.
    Away,
    /// Do not disturb.
    Busy,
}
