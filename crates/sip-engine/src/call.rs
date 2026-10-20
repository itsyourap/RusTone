//! Call types shared by commands and events.

use std::path::PathBuf;
use std::time::Duration;

/// pjsua's call id. Ids are reused after a call ends.
pub type CallId = i32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    /// INVITE sent, no answer yet.
    Calling,
    /// INVITE received, not answered.
    Incoming,
    /// Provisional response (180/183) sent or received.
    Early,
    /// 200 OK sent or received, waiting for ACK.
    Connecting,
    Confirmed,
    Disconnected,
}

impl CallState {
    pub fn is_ringing_in(self, direction: Direction) -> bool {
        direction == Direction::Incoming && matches!(self, Self::Incoming | Self::Early)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaState {
    None,
    Active,
    /// We put the call on hold.
    LocalHold,
    /// The other side put the call on hold.
    RemoteHold,
    Error,
}

/// How DTMF digits are sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DtmfMethod {
    /// RFC 2833, falling back to in-band tones when the remote lacks it.
    #[default]
    Auto,
    InBand,
    Rfc2833,
    SipInfo,
}

/// A snapshot of a call, sent with every change.
#[derive(Debug, Clone, PartialEq)]
pub struct CallInfo {
    pub id: CallId,
    pub direction: Direction,
    pub state: CallState,
    /// Last SIP status (e.g. 180, 200, 486) and its text.
    pub status_code: u16,
    pub status_text: String,
    /// Remote party as `"Name" <sip:user@host>`.
    pub remote: String,
    pub local: String,
    pub media: MediaState,
    /// Media flowing before answer (183 with SDP).
    pub early_media: bool,
    /// Time since the call was answered.
    pub connected_for: Duration,
    pub remote_video: bool,
    /// The call uses video, in either direction.
    pub video: bool,
    /// The peer's video reaches us (off while its camera is off).
    pub receiving_video: bool,
    /// The peer accepts our video (our camera may still be off).
    pub sending_video: bool,
    /// Mixed with other calls in a conference.
    pub in_conference: bool,
    /// Being recorded to a WAV file.
    pub recording: bool,
    /// We stopped sending our camera (or the peer added video and it
    /// hasn't been turned on yet).
    pub camera_off: bool,
    /// SIP Call-ID.
    pub sip_call_id: String,
    /// From P-Asserted-Identity / Remote-Party-ID (incoming calls).
    pub caller_id: Option<String>,
    pub user_agent: Option<String>,
    /// Who the call was diverted from (Diversion header).
    pub diversion: Option<String>,
    /// The caller's `X-AUTOANSWER` header, if any.
    pub auto_answer_header: Option<String>,
    /// The caller's `Call-Info` header, if any (`answer-after=N`).
    pub call_info_header: Option<String>,
}

/// An audio codec pjsua offers, e.g. `AMR-WB/16000/1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecInfo {
    /// pjsua's codec id: `name/clock-rate/channels`.
    pub id: String,
    /// 0 = disabled; higher is preferred.
    pub priority: u8,
}

/// A sound to play through the call audio device.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub path: PathBuf,
    pub looping: bool,
    /// 0..=1 (in-call sounds play at 0.4).
    pub level: f32,
    /// Play on the ring device when no call has audio (the ringtone);
    /// otherwise on the speaker.
    pub ring: bool,
}

/// `"Name" <sip:user@host:port;params>` → `user@host:port` (lowercase).
pub fn uri_user_host(uri: &str) -> String {
    let inner = match (uri.find('<'), uri.rfind('>')) {
        (Some(open), Some(close)) if open < close => &uri[open + 1..close],
        _ => uri,
    };
    let inner = inner.trim();
    let lower = inner.to_ascii_lowercase();
    let start = ["sips:", "sip:", "tel:"].iter().find(|s| lower.starts_with(*s)).map_or(0, |s| s.len());
    let rest = &inner[start..];
    let end = rest.find([';', '?', '>']).unwrap_or(rest.len());
    rest[..end].to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_reduce_to_user_and_host() {
        assert_eq!(uri_user_host("\"Bob\" <sip:05550100002@192.168.29.1:5068;transport=tls>"), "05550100002@192.168.29.1:5068");
        assert_eq!(uri_user_host("sips:Alice@Example.com?subject=x"), "alice@example.com");
        assert_eq!(uri_user_host("<tel:+911234>"), "+911234");
    }
}
