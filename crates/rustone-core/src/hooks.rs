//! Command hooks: programs run on call events.
//!
//! A hook is a command line. `{number}`, `{name}`, `{callid}`,
//! `{direction}` (`in`/`out`), `{duration}` (seconds) and `{time}` (Unix
//! seconds) are replaced; without any of them the number is added as the
//! last argument.

use serde::{Deserialize, Serialize};

/// The hooks' command lines; empty ones are off.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HookSettings {
    pub outgoing_call: String,
    pub incoming_call: String,
    pub call_ring: String,
    pub call_answer: String,
    pub call_answer_video: String,
    pub call_busy: String,
    pub call_start: String,
    pub call_end: String,
}

/// When a hook runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    /// We place a call.
    OutgoingCall,
    /// A call comes in (before any rules).
    IncomingCall,
    /// It starts ringing here.
    CallRing,
    /// We answer an incoming call.
    CallAnswer,
    /// We answer an incoming call that offers video.
    CallAnswerVideo,
    /// A call ends busy or declined (486, 600, 603).
    CallBusy,
    /// A call is answered, either way.
    CallStart,
    /// A call ends otherwise.
    CallEnd,
}

/// The call a hook runs for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HookCall {
    pub number: String,
    pub name: String,
    /// The SIP Call-ID.
    pub call_id: String,
    pub incoming: bool,
    pub duration_secs: u64,
}

impl HookSettings {
    pub fn get(&self, kind: HookKind) -> &str {
        match kind {
            HookKind::OutgoingCall => &self.outgoing_call,
            HookKind::IncomingCall => &self.incoming_call,
            HookKind::CallRing => &self.call_ring,
            HookKind::CallAnswer => &self.call_answer,
            HookKind::CallAnswerVideo => &self.call_answer_video,
            HookKind::CallBusy => &self.call_busy,
            HookKind::CallStart => &self.call_start,
            HookKind::CallEnd => &self.call_end,
        }
    }
}

/// The program and its arguments for `template`, or `None` when empty.
pub fn command_line(template: &str, call: &HookCall, now_unix: i64) -> Option<(String, String)> {
    let template = template.trim();
    if template.is_empty() {
        return None;
    }
    let values = [
        ("{number}", call.number.clone()),
        ("{name}", call.name.clone()),
        ("{callid}", call.call_id.clone()),
        ("{direction}", if call.incoming { "in" } else { "out" }.to_owned()),
        ("{duration}", call.duration_secs.to_string()),
        ("{time}", now_unix.to_string()),
    ];
    let has_placeholder = values.iter().any(|(key, _)| template.contains(key));
    let mut line = template.to_owned();
    for (key, value) in &values {
        line = line.replace(key, value);
    }
    let (program, args) = split_program(&line);
    let args = if has_placeholder || call.number.is_empty() {
        args.to_owned()
    } else if args.is_empty() {
        call.number.clone()
    } else {
        format!("{args} {}", call.number)
    };
    Some((program, args))
}

/// The first token (quoted or not) and the rest.
fn split_program(line: &str) -> (String, &str) {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix('"') {
        return match rest.split_once('"') {
            Some((program, args)) => (program.to_owned(), args.trim()),
            None => (rest.to_owned(), ""),
        };
    }
    match line.split_once(char::is_whitespace) {
        Some((program, args)) => (program.to_owned(), args.trim()),
        None => (line.to_owned(), ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call() -> HookCall {
        HookCall { number: "100".into(), name: "Alice".into(), call_id: "abc@pbx".into(), incoming: true, duration_secs: 42 }
    }

    #[test]
    fn the_number_is_added_as_last_argument() {
        assert_eq!(command_line("notify.exe", &call(), 0), Some(("notify.exe".into(), "100".into())));
        assert_eq!(command_line(r#""C:\Program Files\x\crm.exe" /log"#, &call(), 0), Some((r"C:\Program Files\x\crm.exe".into(), "/log 100".into())));
        assert_eq!(command_line("  ", &call(), 0), None);
    }

    #[test]
    fn placeholders_replace_the_number_argument() {
        let line = command_line(r#"curl "https://crm/?n={number}&id={callid}&d={direction}&t={duration}&at={time}""#, &call(), 1_700_000_000);
        assert_eq!(line, Some(("curl".into(), r#""https://crm/?n=100&id=abc@pbx&d=in&t=42&at=1700000000""#.into())));
        assert_eq!(command_line("say.exe {name}", &call(), 0), Some(("say.exe".into(), "Alice".into())));
    }
}
