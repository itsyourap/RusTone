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
    /// SIP Call-ID.
    pub sip_call_id: String,
    /// From P-Asserted-Identity / Remote-Party-ID (incoming calls).
    pub caller_id: Option<String>,
    pub user_agent: Option<String>,
    /// Who the call was diverted from (Diversion header).
    pub diversion: Option<String>,
}

/// A sound to play through the call audio device.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub path: PathBuf,
    pub looping: bool,
    /// 0..=1 (in-call sounds play at 0.4).
    pub level: f32,
}
