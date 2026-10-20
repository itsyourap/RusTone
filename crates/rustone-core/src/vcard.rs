//! Contacts as vCard (`.vcf`): reads 2.1, 3.0 and 4.0 (as phones, Google
//! Contacts and Outlook write them) and writes 3.0. RusTone's own fields that
//! vCard lacks travel as `X-RUSTONE-*` properties.

use crate::contacts::Contact;

/// One property: `group.NAME;PARAM=a,b;FLAG:value`.
struct Property {
    name: String,
    /// Parameter values, upper-cased: `TYPE=CELL,VOICE` and 2.1's bare `CELL` both give "CELL", "VOICE".
    types: Vec<String>,
    value: String,
}

/// Every contact in `text`, prepared like a new contact;
/// cards without any number are skipped.
pub fn parse(text: &str) -> Vec<Contact> {
    let mut contacts = Vec::new();
    let mut card: Option<Vec<Property>> = None;
    for line in unfold(text) {
        let Some(property) = parse_line(&line) else { continue };
        match (property.name.as_str(), property.value.to_ascii_uppercase().as_str()) {
            ("BEGIN", "VCARD") => card = Some(Vec::new()),
            ("END", "VCARD") => {
                if let Some(c) = card.take().and_then(|props| to_contact(&props).prepared()) {
                    contacts.push(c);
                }
            }
            _ => {
                if let Some(props) = &mut card {
                    props.push(property);
                }
            }
        }
    }
    contacts
}

/// Joins folded lines (a line starting with a space or tab continues the
/// previous one) and quoted-printable soft breaks (a line ending in `=`).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut qp_continues = false;
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match lines.last_mut() {
            Some(last) if qp_continues => last.push_str(line),
            Some(last) if line.starts_with([' ', '\t']) => last.push_str(&line[1..]),
            _ => lines.push(line.to_owned()),
        }
        let current = lines.last().map(String::as_str).unwrap_or_default();
        qp_continues = current.to_ascii_uppercase().contains("QUOTED-PRINTABLE") && current.ends_with('=');
        if qp_continues && let Some(last) = lines.last_mut() {
            last.pop();
        }
    }
    lines
}

fn parse_line(line: &str) -> Option<Property> {
    let (head, raw_value) = split_unquoted(line, ':')?;
    let mut parts = head.split(';');
    let name = parts.next()?;
    // Drop the group prefix (`item1.TEL`).
    let name = name.rsplit('.').next().unwrap_or(name).trim().to_ascii_uppercase();
    let mut types = Vec::new();
    let (mut quoted_printable, mut base64) = (false, false);
    for param in parts {
        let (key, values) = match param.split_once('=') {
            Some((k, v)) => (k.trim().to_ascii_uppercase(), v),
            None => ("TYPE".to_owned(), param),
        };
        for v in values.split(',') {
            let v = v.trim().trim_matches('"').to_ascii_uppercase();
            match (key.as_str(), v.as_str()) {
                ("ENCODING", "QUOTED-PRINTABLE") | ("TYPE", "QUOTED-PRINTABLE") => quoted_printable = true,
                ("ENCODING", "B" | "BASE64") | ("TYPE", "BASE64") => base64 = true,
                ("TYPE" | "PREF", _) => types.push(if key == "PREF" { "PREF".to_owned() } else { v }),
                _ => {}
            }
        }
    }
    if base64 {
        // Photos and other binary data.
        return Some(Property { name, types, value: String::new() });
    }
    let value = if quoted_printable { decode_quoted_printable(raw_value) } else { raw_value.to_owned() };
    Some(Property { name, types, value })
}

/// Splits at the first `sep` outside double quotes (parameter values may be quoted).
fn split_unquoted(s: &str, sep: char) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            c if c == sep && !quoted => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

fn decode_quoted_printable(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    while i < bytes.len() {
        if bytes[i] == b'='
            && let Some((Some(high), Some(low))) = bytes.get(i + 1..i + 3).map(|h| (hex(h[0]), hex(h[1])))
        {
            out.push(high << 4 | low);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Splits a structured value (`N`, `ADR`) at unescaped `;` and unescapes each part.
fn components(value: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n' | 'N') => parts.last_mut().expect("non-empty").push('\n'),
                Some(other) => parts.last_mut().expect("non-empty").push(other),
                None => {}
            },
            ';' => parts.push(String::new()),
            c => parts.last_mut().expect("non-empty").push(c),
        }
    }
    parts.into_iter().map(|p| p.trim().to_owned()).collect()
}

/// Unescapes a text value (`\n`, `\;`, `\,`, `\\`) without splitting it.
fn text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out.trim().to_owned()
}

fn to_contact(props: &[Property]) -> Contact {
    let mut c = Contact::default();
    let mut tels: Vec<(&Property, String)> = Vec::new();
    let mut own_number = None;
    let mut sip = None;
    let is = |p: &Property, t: &str| p.types.iter().any(|x| x == t);
    for p in props {
        match p.name.as_str() {
            "FN" => c.name = text(&p.value),
            "N" => {
                let parts = components(&p.value);
                c.last_name = parts.first().cloned().unwrap_or_default();
                c.first_name = parts.get(1).cloned().unwrap_or_default();
            }
            "TEL" => {
                let number = text(&p.value);
                let number = number.strip_prefix("tel:").unwrap_or(&number).to_owned();
                if !number.is_empty() {
                    tels.push((p, number));
                }
            }
            "IMPP" | "X-SIP" => {
                let v = text(&p.value);
                if let Some(address) = v.strip_prefix("sip:").or(v.strip_prefix("SIP:")).or((p.name == "X-SIP").then_some(v.as_str())) {
                    sip.get_or_insert_with(|| address.to_owned());
                }
            }
            "X-RUSTONE-NUMBER" => own_number = Some(text(&p.value)),
            "X-RUSTONE-STARRED" => c.starred = p.value.trim() == "1",
            "X-RUSTONE-PRESENCE" => c.presence = p.value.trim() == "1",
            "EMAIL" if c.email.is_empty() => c.email = text(&p.value),
            "ADR" if c.address.is_empty() && c.city.is_empty() => {
                // PO box; extended; street; locality; region; postal code; country
                let parts = components(&p.value);
                let get = |i: usize| parts.get(i).cloned().unwrap_or_default();
                c.address = [get(0), get(1), get(2)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
                c.city = get(3);
                c.state = get(4);
                c.zip = get(5);
            }
            "NOTE" if c.comment.is_empty() => c.comment = text(&p.value),
            _ => {}
        }
    }
    let cell = tels.iter().find(|(p, _)| is(p, "CELL")).map(|(_, n)| n.clone());
    // Our own export repeats the dial number as a TEL for other apps; that isn't the phone.
    let phone = tels
        .iter()
        .find(|(p, n)| !is(p, "CELL") && !is(p, "FAX") && own_number.as_deref() != Some(n.as_str()))
        .map(|(_, n)| n.clone());
    let preferred = tels.iter().find(|(p, _)| is(p, "PREF")).or(tels.first()).map(|(_, n)| n.clone());
    c.mobile = cell.unwrap_or_default();
    c.phone = phone.unwrap_or_default();
    c.number = own_number.or(sip).or(preferred).unwrap_or_default();
    // A card whose only name is the number shows the number anyway.
    if c.name == c.number {
        c.name.clear();
    }
    c
}

/// `contacts` as vCard 3.0 (CRLF line ends, folded at 75 bytes).
pub fn write(contacts: &[Contact]) -> String {
    let mut out = String::new();
    for c in contacts {
        let mut lines = vec!["BEGIN:VCARD".to_owned(), "VERSION:3.0".to_owned()];
        lines.push(format!("FN:{}", escape(&c.title())));
        lines.push(format!("N:{};{};;;", escape(&c.last_name), escape(&c.first_name)));
        let number = c.number.trim();
        if !number.is_empty() {
            lines.push(format!("X-RUSTONE-NUMBER:{}", escape(number)));
            // So phones and address books see it too.
            if number.contains('@') || number.to_ascii_lowercase().starts_with("sip:") {
                let address = number.strip_prefix("sip:").unwrap_or(number);
                lines.push(format!("IMPP:sip:{}", escape(address)));
            } else if number != c.phone.trim() && number != c.mobile.trim() {
                lines.push(format!("TEL;TYPE=VOICE,PREF:{}", escape(number)));
            }
        }
        if !c.phone.trim().is_empty() {
            lines.push(format!("TEL;TYPE=WORK,VOICE:{}", escape(c.phone.trim())));
        }
        if !c.mobile.trim().is_empty() {
            lines.push(format!("TEL;TYPE=CELL:{}", escape(c.mobile.trim())));
        }
        if !c.email.trim().is_empty() {
            lines.push(format!("EMAIL;TYPE=INTERNET:{}", escape(c.email.trim())));
        }
        if [&c.address, &c.city, &c.state, &c.zip].iter().any(|s| !s.trim().is_empty()) {
            lines.push(format!("ADR:;;{};{};{};{};", escape(&c.address), escape(&c.city), escape(&c.state), escape(&c.zip)));
        }
        if !c.comment.trim().is_empty() {
            lines.push(format!("NOTE:{}", escape(&c.comment)));
        }
        if c.starred {
            lines.push("X-RUSTONE-STARRED:1".to_owned());
        }
        if c.presence {
            lines.push("X-RUSTONE-PRESENCE:1".to_owned());
        }
        lines.push("END:VCARD".to_owned());
        for line in lines {
            fold(&line, &mut out);
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace("\r\n", "\\n").replace('\n', "\\n")
}

/// Appends `line` with CRLF, folding at 75 bytes without splitting a character.
fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for ch in line.chars() {
        if width + ch.len_utf8() > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += ch.len_utf8();
    }
    out.push_str("\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_field() {
        let c = Contact {
            id: 0,
            name: "Ada, Countess; of Lovelace".into(),
            number: "1002".into(),
            first_name: "Ada".into(),
            last_name: "Lovelace".into(),
            phone: "+44 20 7946 0000".into(),
            mobile: "+44 7700 900123".into(),
            email: "ada@example.com".into(),
            address: "12 St James's Square".into(),
            city: "London".into(),
            state: "Greater London".into(),
            zip: "SW1Y 4JH".into(),
            comment: "Line one\nline two — with a long tail that has to be folded because it is longer than seventy-five bytes".into(),
            starred: true,
            presence: true,
            directory: false,
        };
        let sip = Contact { name: "Desk".into(), number: "sip:desk@pbx.example".into(), ..Contact::default() };
        let text = write(&[c.clone(), sip]);
        assert!(text.lines().all(|l| l.len() <= 76), "{text}");
        assert!(text.contains("IMPP:sip:desk@pbx.example"));
        let back = parse(&text);
        assert_eq!(back[0], c);
        assert_eq!(back[1].number, "sip:desk@pbx.example");
    }

    #[test]
    fn reads_phone_exports() {
        // vCard 3.0 as Google Contacts / iOS write it, with groups and folding.
        let google = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Charles Babbage\r\nN:Babbage;Charles;;;\r\nitem1.TEL;TYPE=CELL:+44 7700 900456\r\n\
                      TEL;TYPE=WORK,VOICE;TYPE=pref:020 7946\r\n  0111\r\nitem2.EMAIL;TYPE=INTERNET:cb@example.com\r\nPHOTO;ENCODING=b;TYPE=JPEG:AAAA\r\nEND:VCARD\r\n";
        // vCard 2.1 with quoted-printable UTF-8 and a soft line break, as older phones write it.
        let old = "BEGIN:VCARD\nVERSION:2.1\nN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:=E0=A6=85=E0=A6=99=E0=A7=8D=E0=A6=95=E0=A6=A8;=\n=E0=A6=B0=E0=A6=BE=E0=A6=AF=E0=A6=BC;;;\nTEL;CELL;PREF:98765 43210\nEND:VCARD\n";
        // vCard 4.0 with a tel: URI and a SIP address.
        let v4 = "BEGIN:VCARD\nVERSION:4.0\nFN:Front desk\nTEL;VALUE=uri;TYPE=\"voice,work\":tel:+1-555-0100\nIMPP:sip:frontdesk@pbx.example\nEND:VCARD\n";
        let all = parse(&format!("{google}{old}{v4}BEGIN:VCARD\nVERSION:3.0\nEND:VCARD\n"));
        assert_eq!(all.len(), 3, "the empty card is skipped");
        let cb = &all[0];
        assert_eq!((cb.title().as_str(), cb.first_name.as_str(), cb.last_name.as_str()), ("Charles Babbage", "Charles", "Babbage"));
        assert_eq!((cb.mobile.as_str(), cb.phone.as_str(), cb.number.as_str()), ("+44 7700 900456", "020 7946 0111", "020 7946 0111"));
        assert_eq!(cb.email, "cb@example.com");
        let bn = &all[1];
        assert_eq!((bn.last_name.as_str(), bn.first_name.as_str(), bn.mobile.as_str(), bn.number.as_str()), ("অঙ্কন", "রায়", "98765 43210", "98765 43210"));
        assert_eq!(bn.title(), "রায় অঙ্কন");
        let desk = &all[2];
        assert_eq!((desk.number.as_str(), desk.phone.as_str()), ("frontdesk@pbx.example", "+1-555-0100"));
    }

    #[test]
    fn broken_quoted_printable_is_kept() {
        assert_eq!(decode_quoted_printable("=C3=A9t=C3=A9 =ZZ =é="), "été =ZZ =é=");
    }
}
