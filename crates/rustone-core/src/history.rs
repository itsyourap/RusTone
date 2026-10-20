//! Call history entries and their CSV export.

/// Call types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallKind {
    Outgoing,
    Incoming,
    Missed,
    /// Answered on another device (CANCEL with "Call completed elsewhere").
    Elsewhere,
}

impl CallKind {
    pub fn code(self) -> i64 {
        match self {
            Self::Outgoing => 0,
            Self::Incoming => 1,
            Self::Missed => 2,
            Self::Elsewhere => 3,
        }
    }

    /// Unknown codes read as outgoing.
    pub fn from_code(code: i64) -> Self {
        match code {
            1 => Self::Incoming,
            2 => Self::Missed,
            3 => Self::Elsewhere,
            _ => Self::Outgoing,
        }
    }

    /// Names used in the export.
    pub fn name(self) -> &'static str {
        match self {
            Self::Outgoing => "out",
            Self::Incoming => "in",
            Self::Missed => "miss",
            Self::Elsewhere => "else",
        }
    }
}

/// One call in the history. `id` is 0 until it's saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    pub id: i64,
    pub kind: CallKind,
    /// What to call back: a number, or `user@host` for other domains.
    pub number: String,
    /// The SIP display name at the time (contacts' names are looked up when shown).
    pub name: String,
    /// When the call started, Unix seconds.
    pub time: i64,
    /// Talk time in seconds; 0 when it wasn't answered.
    pub duration: u32,
    /// How it ended: "Call ended", "Declined", "Busy"...
    pub info: String,
}

/// The history as CSV, columns: Type,Name,Number,Time,Duration,Info.
/// Time is RFC 3339 (UTC), `format_time` turns Unix seconds into it.
pub fn to_csv(records: &[CallRecord], format_time: impl Fn(i64) -> String) -> String {
    let mut out = String::from("Type,Name,Number,Time,Duration,Info\r\n");
    for r in records {
        let fields = [r.kind.name().to_owned(), r.name.clone(), r.number.clone(), format_time(r.time), r.duration.to_string(), r.info.clone()];
        out.push_str(&fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_owned() }
}

/// `m:ss` or `h:mm:ss`, empty for 0.
pub fn format_duration(seconds: u32) -> String {
    match seconds {
        0 => String::new(),
        s if s < 3600 => format!("{}:{:02}", s / 60, s % 60),
        s => format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_when_needed() {
        let r = CallRecord { id: 1, kind: CallKind::Missed, number: "1002".into(), name: "Lovelace, Ada".into(), time: 0, duration: 0, info: "Missed call".into() };
        assert_eq!(to_csv(&[r], |_| "1970-01-01T00:00:00Z".into()), "Type,Name,Number,Time,Duration,Info\r\nmiss,\"Lovelace, Ada\",1002,1970-01-01T00:00:00Z,0,Missed call\r\n");
    }

    #[test]
    fn durations_read_as_clock_time() {
        assert_eq!(format_duration(0), "");
        assert_eq!(format_duration(65), "1:05");
        assert_eq!(format_duration(3725), "1:02:05");
        for kind in [CallKind::Outgoing, CallKind::Incoming, CallKind::Missed, CallKind::Elsewhere] {
            assert_eq!(CallKind::from_code(kind.code()), kind);
        }
        assert_eq!(CallKind::from_code(9), CallKind::Outgoing);
    }
}
