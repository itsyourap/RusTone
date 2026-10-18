//! What the engine reports to the application.

/// Events are delivered on pjsua's worker threads or the engine thread; the
/// sink must be cheap and must not block (queue them and wake the UI).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// pjsua is running (after start, restart or resume).
    Started,
    /// pjsua could not start; the engine waits for a new configuration.
    Failed(String),
    /// pjsua was shut down (suspend, or before a restart).
    Stopped,
    /// The main account's registration changed.
    Registration(RegState),
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
