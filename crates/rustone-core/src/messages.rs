//! Text messages, kept per conversation and saved.

/// Where a message stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageState {
    /// Received from the other side.
    Received,
    /// Sent, waiting for the answer.
    Sending,
    /// The other side (or its server) accepted it.
    Sent,
    /// Not delivered; `reason` says why.
    Failed,
}

impl MessageState {
    pub fn code(self) -> i64 {
        match self {
            Self::Received => 0,
            Self::Sending => 1,
            Self::Sent => 2,
            Self::Failed => 3,
        }
    }

    pub fn from_code(code: i64) -> Self {
        match code {
            1 => Self::Sending,
            2 => Self::Sent,
            3 => Self::Failed,
            _ => Self::Received,
        }
    }
}

/// One message. `id` is 0 until it's saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRecord {
    pub id: i64,
    /// The other side: a number, or `user@host` for other domains (as in the call history).
    pub peer: String,
    /// The other side's SIP display name, when it sent one.
    pub name: String,
    pub outgoing: bool,
    /// Unix seconds.
    pub time: i64,
    pub text: String,
    pub state: MessageState,
    /// Why it failed, or the server's answer.
    pub reason: String,
    /// A received message that hasn't been seen yet.
    pub unread: bool,
}

/// A conversation in the list: its latest message and how many are unread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub peer: String,
    pub last: MessageRecord,
    pub unread: u32,
}
