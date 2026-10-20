//! What the engine reports to the application.

use crate::audio::AudioDevice;
use crate::call::{CallId, CallInfo, CodecInfo};
use crate::presence::Presence;
use crate::quality::CallQuality;

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
