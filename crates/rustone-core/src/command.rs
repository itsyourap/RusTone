//! Command-line verbs:
//! what a launch with arguments, a `msip:` link or a `sip:`/`tel:`/
//! `callto:` link asks the running copy to do.

/// One thing a launch asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    /// Call this number or address.
    Dial(String),
    /// Answer the ringing call (`/answer`, `msip:answer`).
    Answer,
    /// Hang up every call that isn't ringing in (`/hangupall`).
    HangupAll,
    /// Decline ringing calls (`/hangupincoming`).
    HangupIncoming,
    /// Hold or resume the current call.
    Hold,
    /// Transfer the current call (`msip:transfer_1002`).
    Transfer(String),
    /// Mute (`Some(true)`), unmute or toggle (`None`) the microphone.
    MicMute(Option<bool>),
    /// The same for the speaker.
    SpeakerMute(Option<bool>),
    /// Change the microphone or speaker level by this much.
    MicLevel(i8),
    SpeakerLevel(i8),
    /// Hide the window (`msip:minimize`).
    Minimize,
    /// Quit (`/exit`).
    Exit,
}

/// Step for the level verbs.
pub const LEVEL_STEP: i8 = 5;

/// The verb in a command line (the arguments joined by spaces). `None` for
/// nothing to do (no arguments, or only `/minimized`).
pub fn parse(line: &str) -> Option<Verb> {
    let mut line = line.trim();
    if line.len() >= 2 && line.starts_with('"') && line.ends_with('"') {
        line = &line[1..line.len() - 1];
    }
    let lower = line.to_ascii_lowercase();
    if line.is_empty() || matches!(lower.as_str(), "/minimized" | "--minimized" | "-m") {
        return None;
    }
    match lower.as_str() {
        "/answer" => return Some(Verb::Answer),
        "/hangupall" => return Some(Verb::HangupAll),
        "/hangupincoming" => return Some(Verb::HangupIncoming),
        "/exit" => return Some(Verb::Exit),
        _ => {}
    }
    if let Some(cmd) = lower.strip_prefix("msip:") {
        let verb = match cmd {
            "minimize" => Verb::Minimize,
            "answer" => Verb::Answer,
            "hangupall" => Verb::HangupAll,
            "hold" => Verb::Hold,
            "micmute" => Verb::MicMute(Some(true)),
            "micunmute" => Verb::MicMute(Some(false)),
            "micmuteclick" => Verb::MicMute(None),
            "speakmute" => Verb::SpeakerMute(Some(true)),
            "speakunmute" => Verb::SpeakerMute(Some(false)),
            "speakmuteclick" => Verb::SpeakerMute(None),
            "micup" => Verb::MicLevel(LEVEL_STEP),
            "micdown" => Verb::MicLevel(-LEVEL_STEP),
            "speakup" => Verb::SpeakerLevel(LEVEL_STEP),
            "speakdown" => Verb::SpeakerLevel(-LEVEL_STEP),
            _ if cmd.starts_with("transfer_") => Verb::Transfer(line[5 + 9..].to_owned()),
            "" => return None,
            _ => return dial(&line[5..]),
        };
        return Some(verb);
    }
    dial(line)
}

/// A number from a link: `tel:`, `sip:`, `callto:`, `dialpad:` and `dial:`
/// prefixes, a `//` and a trailing `/` go.
fn dial(target: &str) -> Option<Verb> {
    let mut target = target.trim();
    for prefix in ["tel:", "sip:", "callto:", "dialpad:", "dial:"] {
        if target.len() >= prefix.len() && target[..prefix.len()].eq_ignore_ascii_case(prefix) {
            target = &target[prefix.len()..];
            break;
        }
    }
    if let Some(rest) = target.strip_prefix("//") {
        target = rest.strip_suffix('/').unwrap_or(rest);
    }
    // Links may be percent-encoded ("tel:%2B91…").
    let target = percent_decode(target);
    (!target.trim().is_empty()).then(|| Verb::Dial(target.trim().to_owned()))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_numbers_dial() {
        assert_eq!(parse("tel:+915550100001"), Some(Verb::Dial("+915550100001".into())));
        assert_eq!(parse("\"sip:100@pbx\""), Some(Verb::Dial("100@pbx".into())));
        assert_eq!(parse("callto://200/"), Some(Verb::Dial("200".into())));
        assert_eq!(parse("tel:%2B91%20345"), Some(Verb::Dial("+91 345".into())));
        assert_eq!(parse("msip:300"), Some(Verb::Dial("300".into())));
        assert_eq!(parse("1002"), Some(Verb::Dial("1002".into())));
        assert_eq!(parse(""), None);
        assert_eq!(parse("/minimized"), None);
    }

    #[test]
    fn verbs_parse() {
        assert_eq!(parse("/answer"), Some(Verb::Answer));
        assert_eq!(parse("/hangupall"), Some(Verb::HangupAll));
        assert_eq!(parse("/hangupincoming"), Some(Verb::HangupIncoming));
        assert_eq!(parse("/exit"), Some(Verb::Exit));
        assert_eq!(parse("msip:hold"), Some(Verb::Hold));
        assert_eq!(parse("msip:transfer_1002"), Some(Verb::Transfer("1002".into())));
        assert_eq!(parse("msip:micmuteclick"), Some(Verb::MicMute(None)));
        assert_eq!(parse("msip:speakunmute"), Some(Verb::SpeakerMute(Some(false))));
        assert_eq!(parse("msip:speakdown"), Some(Verb::SpeakerLevel(-5)));
        assert_eq!(parse("msip:minimize"), Some(Verb::Minimize));
    }
}
