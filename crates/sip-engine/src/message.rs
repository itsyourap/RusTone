//! Instant messages (SIP MESSAGE, text/plain).

/// The app's id for a message it sends, to match its delivery status.
pub type MessageId = u64;

/// A message that arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMessage {
    /// The sender: P-Asserted-Identity / Remote-Party-ID when present,
    /// else the From header, e.g. `"Ada" <sip:1001@pbx>`.
    pub from: String,
    pub text: String,
}

/// How sending a message ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageStatus {
    pub id: MessageId,
    /// The final SIP status (200, 202, 404...), or 0 when it couldn't be sent.
    pub code: u16,
    pub reason: String,
}

impl MessageStatus {
    pub fn delivered(&self) -> bool {
        (200..300).contains(&self.code)
    }
}

/// Whether a MESSAGE body type is plain text (`text/plain`, any parameters).
pub fn is_plain_text(mime: &str) -> bool {
    mime.split(';').next().is_some_and(|t| t.trim().eq_ignore_ascii_case("text/plain"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_with_or_without_parameters() {
        assert!(is_plain_text("text/plain") && is_plain_text("Text/Plain; charset=UTF-8"));
        assert!(!is_plain_text("application/im-iscomposing+xml") && !is_plain_text("message/cpim"));
        assert!(MessageStatus { id: 1, code: 202, reason: "Accepted".into() }.delivered());
        assert!(!MessageStatus { id: 1, code: 0, reason: "no account".into() }.delivered());
    }
}
