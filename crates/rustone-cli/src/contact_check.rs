//! Checks the Contact headers of outgoing SIP messages in a pjsua log
//! against the JF router's requirement: exactly one
//! `;+sip.instance="<00000000-0000-1000-8000-0000HHHHHHHH>"` (uppercase hex,
//! no `urn:uuid:`), and the same instance id everywhere.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

static TX_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"TX \d+ bytes (?:Request|Response) msg (\S+)").unwrap());
static CONTACT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:contact|m)\s*:\s*(.*)$").unwrap());
static INSTANCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#";\+sip\.instance="<00000000-0000-1000-8000-0000([0-9A-F]{8})>""#).unwrap()
});

const END_MSG: &str = "--end msg--";

#[derive(Debug)]
pub struct Checked {
    /// e.g. `REGISTER/cseq=42873` or `200/INVITE/cseq=1`.
    pub message: String,
    pub contact: String,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct Report {
    pub checked: Vec<Checked>,
    pub instance_ids: BTreeSet<String>,
}

impl Report {
    pub fn failures(&self) -> impl Iterator<Item = &Checked> {
        self.checked.iter().filter(|c| c.error.is_some())
    }

    pub fn ok(&self) -> bool {
        !self.checked.is_empty() && self.failures().next().is_none() && self.instance_ids.len() == 1
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in &self.checked {
            match &c.error {
                None => writeln!(f, "  ok    {:<28} {}", c.message, c.contact)?,
                Some(e) => writeln!(f, "  FAIL  {:<28} {}\n        -> {e}", c.message, c.contact)?,
            }
        }
        if self.instance_ids.len() > 1 {
            writeln!(f, "  FAIL  instance id differs between messages: {:?}", self.instance_ids)?;
        }
        let failed = self.failures().count();
        write!(
            f,
            "{} Contact header(s) checked, {} failed, instance id(s): {:?} => {}",
            self.checked.len(),
            failed,
            self.instance_ids,
            if self.ok() { "PASS" } else { "FAIL" }
        )
    }
}

/// Scans every outgoing (TX) message in `log` and checks its Contact headers.
pub fn check_log(log: &str) -> Report {
    let mut report = Report::default();
    let mut current: Option<String> = None;

    for line in log.lines().map(|l| l.trim_end_matches('\r')) {
        if let Some(caps) = TX_START.captures(line) {
            current = Some(caps[1].to_owned());
            continue;
        }
        let Some(message) = &current else { continue };
        if line == END_MSG {
            current = None;
            continue;
        }
        // The blank line ends the headers; the body follows.
        if line.is_empty() {
            current = None;
            continue;
        }
        if let Some(caps) = CONTACT.captures(line) {
            let contact = caps[1].trim().to_owned();
            if contact == "*" {
                continue;
            }
            let error = check_contact(&contact, &mut report.instance_ids);
            report.checked.push(Checked { message: message.clone(), contact, error });
        }
    }
    report
}

fn check_contact(contact: &str, ids: &mut BTreeSet<String>) -> Option<String> {
    let count = contact.matches("+sip.instance=").count();
    match count {
        0 => return Some("no +sip.instance param".into()),
        1 => {}
        n => return Some(format!("+sip.instance appears {n} times")),
    }
    match INSTANCE.captures(contact) {
        Some(caps) => {
            ids.insert(caps[1].to_owned());
            None
        }
        None => Some(
            "+sip.instance is not \"<00000000-0000-1000-8000-0000HHHHHHHH>\" (uppercase hex)"
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"<sip:+910000000000@192.168.29.164:14456;transport=TLS;ob>;+sip.instance="<00000000-0000-1000-8000-00003033F4C1>""#;

    fn tx(msg: &str, contact: &str) -> String {
        format!(
            "19:34:28.133 pjsua_core.c ...TX 738 bytes Request msg {msg} (tdta0) to TLS 192.168.29.1:5068:\n\
             {msg} sip:sip.example.com;transport=tls SIP/2.0\n\
             Via: SIP/2.0/TLS 192.168.29.164:14456;rport\n\
             Contact: {contact}\n\
             Content-Length:  0\n\
             \n\
             \n\
             --end msg--\n"
        )
    }

    #[test]
    fn register_with_reg_id_passes() {
        let log = tx("REGISTER/cseq=1", &format!("{GOOD};reg-id=1"));
        let report = check_log(&log);
        assert!(report.ok(), "{report}");
        assert_eq!(report.instance_ids.iter().next().unwrap(), "3033F4C1");
    }

    #[test]
    fn duplicated_instance_fails() {
        let dup = format!(r#"{GOOD};+sip.instance="<00000000-0000-1000-8000-00003033F4C1>""#);
        let report = check_log(&tx("INVITE/cseq=2", &dup));
        assert!(!report.ok());
        assert!(report.checked[0].error.as_ref().unwrap().contains("2 times"));
    }

    #[test]
    fn stock_pjsua_format_fails() {
        let stock = r#"<sip:a@1.2.3.4:5061;transport=TLS;ob>;+sip.instance="<urn:uuid:00000000-0000-0000-0000-00003033f4c1>""#;
        assert!(!check_log(&tx("REGISTER/cseq=1", stock)).ok());
    }

    #[test]
    fn missing_instance_fails() {
        let report = check_log(&tx("MESSAGE/cseq=3", "<sip:a@1.2.3.4:5061;transport=TLS>"));
        assert!(!report.ok());
    }

    #[test]
    fn differing_ids_fail() {
        let other = GOOD.replace("3033F4C1", "DEADBEEF");
        let log = tx("REGISTER/cseq=1", GOOD) + &tx("INVITE/cseq=2", &other);
        let report = check_log(&log);
        assert_eq!(report.failures().count(), 0);
        assert!(!report.ok());
    }

    #[test]
    fn received_messages_and_bodies_are_ignored() {
        let log = format!(
            "RX 550 bytes Response msg 200/REGISTER/cseq=1 from TLS 192.168.29.1:5068:\n\
             Contact: <sip:x@y>\n--end msg--\n{}",
            tx("REGISTER/cseq=1", GOOD)
        );
        let report = check_log(&log);
        assert_eq!(report.checked.len(), 1);
        assert!(report.ok());
    }
}
