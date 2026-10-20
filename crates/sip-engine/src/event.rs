//! What the engine reports to the application.

use crate::audio::AudioDevice;
use crate::call::{CallId, CallInfo, CodecInfo};
use crate::message::{IncomingMessage, MessageStatus};
use crate::presence::Presence;
use crate::quality::CallQuality;
use crate::video::VideoDevice;

/// Events are delivered on pjsua's worker threads or the engine thread; the
/// sink must be cheap and must not block (queue them and wake the UI).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// pjsua is running (after start, restart or resume).
    Started,
    /// The audio codecs pjsua has, with their priorities (after start).
    Codecs(Vec<CodecInfo>),
    /// The sound devices pjsua sees (after start and after hot-plug).
    AudioDevices(Vec<AudioDevice>),
    /// The video devices pjsua sees (after start).
    VideoDevices(Vec<VideoDevice>),
    /// A text message arrived.
    MessageReceived(IncomingMessage),
    /// A message we sent got its final answer (or couldn't be sent).
    MessageStatus(MessageStatus),
    /// A watched contact's presence changed (or was replayed).
    Presence(Presence),
    /// A live call's quality, every second while calls are monitored.
    CallQuality(CallQuality),
    /// pjsua could not start; the engine waits for a new configuration.
    Failed(String),
    /// pjsua was shut down (suspend, or before a restart).
    Stopped,
    /// The main account's registration changed.
    Registration(RegState),
    /// A new incoming call, before any response (the application answers
    /// with 180 or rejects it).
    IncomingCall(CallInfo),
    /// A call's state or media changed.
    Call(CallInfo),
    /// `Command::MakeCall` failed before a call existed.
    CallFailed { target: String, reason: String },
    /// An RFC 2833 digit from the remote.
    DtmfReceived { call: CallId, digit: char },
    /// Progress of a transfer we started (REFER NOTIFYs).
    TransferStatus { call: CallId, code: u16, text: String, done: bool },
    /// A call command failed (hold, transfer, DTMF...).
    CallError { call: Option<CallId>, reason: String },
    /// The voicemail box's state, from an MWI NOTIFY (RFC 3842).
    Voicemail(Voicemail),
    /// A recording was closed (the call ended or recording stopped).
    RecordingSaved { call: CallId, path: std::path::PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegState {
    /// No account, or unregistered.
    Unregistered,
    /// An account without a registrar: usable for calls, nothing to register.
    NoRegistrar,
    /// A REGISTER is in progress.
    Registering,
    Registered {
        /// Seconds until the registration expires, as granted by the server.
        expires: u32,
    },
    Failed {
        /// SIP status, or 0 for local/transport errors.
        code: u16,
        reason: String,
        /// The server's `P-Registrar-Error`, when it sent one.
        detail: Option<String>,
    },
}

impl RegState {
    pub fn is_registered(&self) -> bool {
        matches!(self, Self::Registered { .. } | Self::NoRegistrar)
    }
}

/// A voicemail summary (`application/simple-message-summary`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Voicemail {
    /// `Messages-Waiting: yes`.
    pub waiting: bool,
    /// New and old voice messages (`Voice-Message: 2/8`), when given.
    pub new: Option<u32>,
    pub old: Option<u32>,
}

impl Voicemail {
    /// Reads a message summary body (only
    /// Messages-Waiting counts).
    pub fn parse(body: &str) -> Self {
        let mut summary = Self::default();
        for line in body.lines() {
            let Some((key, value)) = line.split_once(':') else { continue };
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "messages-waiting" => summary.waiting = value.eq_ignore_ascii_case("yes"),
                "voice-message" => {
                    // "2/8 (0/2)": new/old, then urgent new/old.
                    let counts = value.split_whitespace().next().unwrap_or("");
                    if let Some((new, old)) = counts.split_once('/') {
                        summary.new = new.trim().parse().ok();
                        summary.old = old.trim().parse().ok();
                    }
                }
                _ => {}
            }
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_summaries_are_read() {
        let body = ["Messages-Waiting: yes", "Message-Account: sip:alice@vmail.example.com", "Voice-Message: 2/8 (0/2)"].join("\r\n");
        assert_eq!(Voicemail::parse(&body), Voicemail { waiting: true, new: Some(2), old: Some(8) });
        assert_eq!(Voicemail::parse("Messages-Waiting: no"), Voicemail { waiting: false, new: None, old: None });
    }
}
