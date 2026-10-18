//! Editing the audio codec list: enabled codecs, space-separated,
//! in priority order (e.g. `AMR-WB/16000/1 AMR/8000/1`).

/// One row of the codec editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecRow {
    /// pjsua codec id, `name/rate/channels`.
    pub id: String,
    pub enabled: bool,
}

/// Enabled codecs first (in `list` order), then the other `available` ones.
/// Enabled codecs the engine doesn't report are kept, so a list survives an
/// engine that hasn't started yet.
pub fn rows(list: &str, available: &[String]) -> Vec<CodecRow> {
    let enabled: Vec<&str> = list.split_whitespace().collect();
    let mut rows: Vec<CodecRow> = enabled.iter().map(|id| CodecRow { id: (*id).to_owned(), enabled: true }).collect();
    for id in available {
        if !enabled.iter().any(|e| e.eq_ignore_ascii_case(id)) {
            rows.push(CodecRow { id: id.clone(), enabled: false });
        }
    }
    rows
}

/// The `audioCodecs` string for `rows` (enabled ones, in order).
pub fn list(rows: &[CodecRow]) -> String {
    rows.iter().filter(|r| r.enabled).map(|r| r.id.as_str()).collect::<Vec<_>>().join(" ")
}

/// Enables or disables `id`. A newly enabled codec goes last among the
/// enabled ones.
pub fn set_enabled(list_str: &str, available: &[String], id: &str, on: bool) -> String {
    let mut rows = rows(list_str, available);
    if let Some(i) = rows.iter().position(|r| r.id == id) {
        let mut row = rows.remove(i);
        row.enabled = on;
        let at = if on { rows.iter().take_while(|r| r.enabled).count() } else { rows.len() };
        rows.insert(at, row);
    }
    list(&rows)
}

/// Moves an enabled codec one place up (`up`) or down in priority.
pub fn move_codec(list_str: &str, id: &str, up: bool) -> String {
    let mut ids: Vec<&str> = list_str.split_whitespace().collect();
    if let Some(i) = ids.iter().position(|x| *x == id) {
        let j = if up { i.checked_sub(1) } else { (i + 1 < ids.len()).then_some(i + 1) };
        if let Some(j) = j {
            ids.swap(i, j);
        }
    }
    ids.join(" ")
}

/// "AMR-WB · 16 kHz" style label for a codec id.
pub fn label(id: &str) -> String {
    let mut parts = id.split('/');
    let name = parts.next().unwrap_or(id);
    let rate = parts.next().and_then(|r| r.parse::<u32>().ok());
    let channels = parts.next().and_then(|c| c.parse::<u32>().ok()).unwrap_or(1);
    let mut out = name.to_owned();
    if let Some(rate) = rate {
        if rate % 1000 == 0 {
            out.push_str(&format!(" · {} kHz", rate / 1000));
        } else {
            out.push_str(&format!(" · {:.1} kHz", f64::from(rate) / 1000.0));
        }
    }
    if channels > 1 {
        out.push_str(" · stereo");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn avail() -> Vec<String> {
        ["AMR-WB/16000/1", "AMR/8000/1", "PCMA/8000/1", "PCMU/8000/1", "opus/48000/2"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn rows_put_enabled_first() {
        let r = rows("PCMA/8000/1 PCMU/8000/1", &avail());
        assert_eq!(r[0], CodecRow { id: "PCMA/8000/1".into(), enabled: true });
        assert_eq!(r.len(), 5);
        assert!(!r[2].enabled);
        assert_eq!(list(&r), "PCMA/8000/1 PCMU/8000/1");
    }

    #[test]
    fn enabling_and_ordering_codecs() {
        let s = set_enabled("PCMA/8000/1 PCMU/8000/1", &avail(), "AMR-WB/16000/1", true);
        assert_eq!(s, "PCMA/8000/1 PCMU/8000/1 AMR-WB/16000/1");
        let s = move_codec(&s, "AMR-WB/16000/1", true);
        let s = move_codec(&s, "AMR-WB/16000/1", true);
        let s = move_codec(&s, "AMR-WB/16000/1", true);
        assert_eq!(s, "AMR-WB/16000/1 PCMA/8000/1 PCMU/8000/1");
        let s = set_enabled(&s, &avail(), "PCMU/8000/1", false);
        assert_eq!(s, "AMR-WB/16000/1 PCMA/8000/1");
        assert_eq!(move_codec(&s, "PCMA/8000/1", false), s, "last stays last");
    }

    #[test]
    fn unknown_enabled_codecs_are_kept() {
        assert_eq!(list(&rows("G729/8000/1", &avail())), "G729/8000/1");
    }

    #[test]
    fn labels_read_well() {
        assert_eq!(label("AMR-WB/16000/1"), "AMR-WB · 16 kHz");
        assert_eq!(label("opus/48000/2"), "opus · 48 kHz · stereo");
        assert_eq!(label("L16/44100/1"), "L16 · 44.1 kHz");
    }
}
