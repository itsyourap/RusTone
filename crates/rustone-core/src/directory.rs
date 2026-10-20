//! The users directory: building its URL,
//! reading what the server returns (JSON, XML, Yealink phone books, CSV)
//! and merging it into the contacts.

use std::collections::HashSet;
use std::time::Duration;

use md5::{Digest, Md5};

use crate::contacts::Contact;

/// Values for the URL's placeholders (empty when there's no account).
#[derive(Debug, Clone, Default)]
pub struct UrlValues {
    pub username: String,
    pub password: String,
    pub server: String,
    pub domain: String,
    pub label: String,
    pub version: String,
    /// Unix seconds, for `{time}`.
    pub time: i64,
}

/// Whether the URL needs account details, so it waits for registration.
pub fn needs_account(template: &str) -> bool {
    template.contains("%s") || template.contains('{')
}

/// The URL: `%s` placeholders take username, password and server in
/// order; `{username}`-style ones are URL-encoded; `sequence=N` is appended.
pub fn build_url(template: &str, v: &UrlValues, sequence: u32) -> String {
    let mut url = String::new();
    let mut printf = [v.username.as_str(), v.password.as_str(), v.server.as_str()].into_iter();
    let mut rest = template.trim();
    while let Some(i) = rest.find("%s") {
        url.push_str(&rest[..i]);
        url.push_str(&encode(printf.next().unwrap_or_default()));
        rest = &rest[i + 2..];
    }
    url.push_str(rest);
    let server = if v.server.is_empty() { "localhost".to_owned() } else { v.server.rsplit_once(':').map_or(v.server.clone(), |(host, _)| host.to_owned()) };
    let md5_password: String = Md5::digest(v.password.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    for (key, value) in [
        ("{time}", v.time.to_string()),
        ("{version}", v.version.clone()),
        ("{label}", v.label.clone()),
        ("{server}", server),
        ("{domain}", v.domain.clone()),
        ("{username}", v.username.clone()),
        ("{password}", v.password.clone()),
        ("{md5_password}", md5_password),
    ] {
        url = url.replace(key, &encode(&value));
    }
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}sequence={sequence}")
}

/// Percent-encodes everything but unreserved characters.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// A directory contact and the fields the server gave (only those are
/// updated on an existing contact).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub contact: Contact,
    pub fields: HashSet<&'static str>,
}

/// A presence status the server reports for a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Offline,
    Online,
    Away,
    Busy,
    Ringing,
    OnThePhone,
    Unknown,
}

impl Status {
    fn parse(s: &str) -> Self {
        match s {
            "offline" => Self::Offline,
            "online" => Self::Online,
            "away" => Self::Away,
            "busy" => Self::Busy,
            "ring" => Self::Ringing,
            "phone" => Self::OnThePhone,
            _ => Self::Unknown,
        }
    }
}

/// What one response contained.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// (number, status, info) from JSON's `presence` array.
    pub presence: Vec<(String, Status, String)>,
    /// Seconds until the next load, if the server said.
    pub refresh: Option<u32>,
    /// Don't show errors to the user.
    pub silent: bool,
}

const TEXT_FIELDS: [&str; 12] = ["name", "number", "firstname", "lastname", "phone", "mobile", "email", "address", "city", "state", "zip", "comment"];

/// Builds an entry from `field -> value` pairs.
fn entry(get: impl Fn(&str) -> Option<String>) -> Option<Entry> {
    let mut c = Contact { directory: true, ..Contact::default() };
    let mut fields = HashSet::new();
    for key in TEXT_FIELDS {
        let Some(value) = get(key) else { continue };
        let slot = match key {
            "name" => &mut c.name,
            "number" => &mut c.number,
            "firstname" => &mut c.first_name,
            "lastname" => &mut c.last_name,
            "phone" => &mut c.phone,
            "mobile" => &mut c.mobile,
            "email" => &mut c.email,
            "address" => &mut c.address,
            "city" => &mut c.city,
            "state" => &mut c.state,
            "zip" => &mut c.zip,
            _ => &mut c.comment,
        };
        *slot = value.trim().to_owned();
        fields.insert(key);
    }
    // "telephone" also works for the number.
    if c.number.is_empty()
        && let Some(t) = get("telephone")
    {
        c.number = t.trim().to_owned();
        fields.insert("number");
    }
    for key in ["presence", "starred"] {
        if let Some(v) = get(key) {
            let on = v.trim() != "0" && !v.trim().is_empty() && v.trim() != "false";
            if key == "presence" { c.presence = on } else { c.starred = on }
            fields.insert(key);
        }
    }
    let had_number = !c.number.is_empty();
    let mut c = c.prepared()?;
    if !had_number {
        fields.insert("number");
    }
    // An empty name becomes first + last, else the number.
    if c.name.is_empty() {
        c.name = c.title();
    }
    Some(Entry { contact: c, fields })
}

/// Reads a response body. CSV is recognised by its content type, then
/// JSON, then XML.
pub fn parse(body: &str, content_type: &str) -> Result<Listing, String> {
    let body = body.trim_start_matches('\u{feff}').trim();
    if body.is_empty() {
        return Err("The directory is empty".into());
    }
    if content_type.to_ascii_lowercase().contains("text/csv") {
        return Ok(parse_csv(body));
    }
    if body.starts_with('[') || body.starts_with('{') {
        return parse_json(body);
    }
    if body.starts_with('<') {
        return parse_xml(body);
    }
    Err("The received data cannot be recognized".into())
}

fn parse_json(body: &str) -> Result<Listing, String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| "The received data cannot be recognized")?;
    let mut listing = Listing::default();
    let items = match &value {
        serde_json::Value::Array(items) => items.as_slice(),
        serde_json::Value::Object(o) => {
            listing.refresh = o.get("refresh").and_then(serde_json::Value::as_u64).map(|r| r.min(u64::from(u32::MAX)) as u32);
            listing.silent = o.get("silent").and_then(serde_json::Value::as_i64).is_some_and(|s| s != 0);
            for p in o.get("presence").and_then(serde_json::Value::as_array).into_iter().flatten() {
                let text = |k| p.get(k).and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
                if !text("number").is_empty() {
                    listing.presence.push((text("number"), Status::parse(&text("status")), text("info")));
                }
            }
            o.get("items").and_then(serde_json::Value::as_array).map_or(&[][..], Vec::as_slice)
        }
        _ => return Err("The received data cannot be recognized".into()),
    };
    listing.entries = items
        .iter()
        .filter_map(|item| {
            entry(|key| match item.get(key)? {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                serde_json::Value::Bool(b) => Some(if *b { "1".into() } else { "0".into() }),
                _ => None,
            })
        })
        .collect();
    Ok(listing)
}

fn parse_xml(body: &str) -> Result<Listing, String> {
    let doc = roxmltree::Document::parse(body).map_err(|_| "The received data cannot be recognized")?;
    let root = doc.root_element();
    let mut listing = Listing::default();
    match root.tag_name().name() {
        "contacts" => {
            listing.refresh = root.attribute("refresh").and_then(|r| r.trim().parse().ok());
            listing.silent = root.attribute("silent").is_some_and(|s| s.trim() != "0" && !s.trim().is_empty());
            listing.entries = root.children().filter(|n| n.has_tag_name("contact")).filter_map(|n| entry(|k| n.attribute(k).map(str::to_owned))).collect();
        }
        "YealinkIPPhoneBook" => {
            let units = root.descendants().filter(|n| n.has_tag_name("Unit"));
            listing.entries = units
                .filter_map(|u| {
                    entry(|k| match k {
                        "name" => u.attribute("Name").map(str::to_owned),
                        "number" => u.attribute("Phone1").map(str::to_owned),
                        "phone" => u.attribute("Phone2").map(str::to_owned),
                        "mobile" => u.attribute("Phone3").map(str::to_owned),
                        _ => None,
                    })
                })
                .collect();
        }
        _ => {
            let entries = root.descendants().filter(|n| n.has_tag_name("entry") || n.has_tag_name("DirectoryEntry"));
            listing.entries = entries
                .filter_map(|e| {
                    let child = |names: &[&str]| e.children().find(|c| names.iter().any(|n| c.has_tag_name(*n))).and_then(|c| c.text()).map(str::to_owned);
                    let numbers: Vec<String> =
                        e.children().filter(|c| c.has_tag_name("extension") || c.has_tag_name("Telephone")).filter_map(|c| c.text().map(str::to_owned)).collect();
                    entry(|k| match k {
                        "name" => child(&["name", "Name"]),
                        "number" => numbers.first().cloned(),
                        "phone" => numbers.get(1).cloned(),
                        "mobile" => numbers.get(2).cloned(),
                        "presence" => child(&["presence"]),
                        "starred" => child(&["starred"]),
                        _ => None,
                    })
                })
                .collect();
        }
    }
    Ok(listing)
}

/// CSV with a header row naming the columns.
fn parse_csv(body: &str) -> Listing {
    let mut rows = body.lines().filter(|l| !l.trim().is_empty()).map(csv_row);
    let Some(header) = rows.next() else { return Listing::default() };
    let header: Vec<String> = header.iter().map(|h| h.trim().to_lowercase().replace([' ', '_'], "")).collect();
    let entries = rows
        .filter_map(|row| {
            entry(|key| {
                let i = header.iter().position(|h| h == key)?;
                row.get(i).cloned()
            })
        })
        .collect();
    Listing { entries, ..Listing::default() }
}

fn csv_row(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                fields.last_mut().expect("non-empty").push('"');
            }
            ('"', _) => quoted = !quoted,
            (',' | ';', false) => fields.push(String::new()),
            (c, _) => fields.last_mut().expect("non-empty").push(c),
        }
    }
    fields
}

/// How to bring the contacts in line with a listing.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MergePlan {
    pub update: Vec<Contact>,
    pub add: Vec<Contact>,
    pub remove: Vec<i64>,
}

impl MergePlan {
    pub fn is_empty(&self) -> bool {
        self.update.is_empty() && self.add.is_empty() && self.remove.is_empty()
    }
}

/// Merging the directory: a directory contact with the
/// same number and name gets the fields the server gave; directory contacts
/// the server no longer lists go; new entries are added. The user's own
/// contacts are never changed, and an entry that repeats one isn't added.
pub fn merge(existing: &[Contact], entries: &[Entry]) -> MergePlan {
    let key = |c: &Contact| (c.number.trim().to_owned(), c.name.trim().to_owned());
    let mut plan = MergePlan::default();
    let mut matched = HashSet::new();
    for e in entries {
        let k = key(&e.contact);
        if let Some(old) = existing.iter().find(|c| c.directory && key(c) == k) {
            matched.insert(old.id);
            let mut new = old.clone();
            let src = &e.contact;
            for f in &e.fields {
                match *f {
                    "firstname" => new.first_name = src.first_name.clone(),
                    "lastname" => new.last_name = src.last_name.clone(),
                    "phone" => new.phone = src.phone.clone(),
                    "mobile" => new.mobile = src.mobile.clone(),
                    "email" => new.email = src.email.clone(),
                    "address" => new.address = src.address.clone(),
                    "city" => new.city = src.city.clone(),
                    "state" => new.state = src.state.clone(),
                    "zip" => new.zip = src.zip.clone(),
                    "comment" => new.comment = src.comment.clone(),
                    "presence" => new.presence = src.presence,
                    "starred" => new.starred = src.starred,
                    _ => {}
                }
            }
            if new != *old {
                plan.update.push(new);
            }
        } else if !existing.iter().any(|c| !c.directory && key(c) == k) && !plan.add.iter().any(|a| key(a) == k) {
            plan.add.push(e.contact.clone());
        }
    }
    plan.remove = existing.iter().filter(|c| c.directory && !matched.contains(&c.id)).map(|c| c.id).collect();
    plan
}

/// When to load again: the listing's `refresh`, else `Cache-Control:
/// max-age` clamped to a minute..a day. `None` means never.
pub fn refresh_after(listing_refresh: Option<u32>, cache_control: Option<&str>) -> Option<Duration> {
    let secs = match listing_refresh {
        Some(r) => r,
        None => {
            let max_age = cache_control?.split(',').find_map(|d| d.trim().strip_prefix("max-age=")?.trim().parse::<u32>().ok())?;
            max_age.clamp(60, 86400)
        }
    };
    (secs > 0).then(|| Duration::from_secs(u64::from(secs)))
}

/// After `failures` failed connections in a row: 10 s × failures³.
pub fn retry_after(failures: u32) -> Duration {
    Duration::from_secs(10 * u64::from(failures.min(20)).pow(3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> UrlValues {
        UrlValues {
            username: "1001".into(),
            password: "p&ss".into(),
            server: "pbx.example:5061".into(),
            domain: "example.com".into(),
            label: "Office".into(),
            version: "0.1.0".into(),
            time: 42,
        }
    }

    #[test]
    fn urls_fill_placeholders() {
        let v = values();
        assert_eq!(build_url("https://dir/%s/%s", &v, 0), "https://dir/1001/p%26ss?sequence=0");
        assert_eq!(build_url("https://dir/?u={username}&s={server}&t={time}&d={domain}", &v, 3), "https://dir/?u=1001&s=pbx.example&t=42&d=example.com&sequence=3");
        assert_eq!(build_url("https://dir/{md5_password}", &v, 1), "https://dir/947760a6249e930930ddbea4e4b5202e?sequence=1");
        assert_eq!(build_url("https://dir/{server}", &UrlValues::default(), 0), "https://dir/localhost?sequence=0");
        assert!(needs_account("https://dir/{username}") && !needs_account("https://dir/list.json"));
    }

    #[test]
    fn json_with_refresh_and_presence() {
        let body = r#"{"refresh": 300, "silent": 1, "items": [
            {"name": "Ada", "number": "1001", "mobile": "98765", "presence": 1},
            {"firstname": "Charles", "lastname": "Babbage", "telephone": "1002"},
            {"name": "No number"}
        ], "presence": [{"number": "1001", "status": "ring", "info": "Ringing"}]}"#;
        let l = parse(body, "application/json").unwrap();
        assert_eq!((l.refresh, l.silent, l.entries.len()), (Some(300), true, 2));
        assert!(l.entries[0].contact.presence && l.entries[0].contact.directory);
        assert_eq!((l.entries[1].contact.name.as_str(), l.entries[1].contact.number.as_str()), ("Charles Babbage", "1002"));
        assert_eq!(l.presence, [("1001".to_owned(), Status::Ringing, "Ringing".to_owned())]);
        assert_eq!(parse(r#"[{"name": "Bare", "number": 7}]"#, "").unwrap().entries[0].contact.number, "7");
    }

    #[test]
    fn xml_shapes() {
        let ours = r#"<contacts refresh="60"><contact name="Ada" number="1001" starred="1"/></contacts>"#;
        let l = parse(ours, "text/xml").unwrap();
        assert_eq!((l.refresh, l.entries[0].contact.starred), (Some(60), true));
        let yealink = r#"<YealinkIPPhoneBook><Menu Name="All"><Unit Name="Desk" Phone1="2001" Phone2="2002" Phone3="2003"/></Menu></YealinkIPPhoneBook>"#;
        let d = &parse(yealink, "").unwrap().entries[0].contact;
        assert_eq!((d.name.as_str(), d.number.as_str(), d.phone.as_str(), d.mobile.as_str()), ("Desk", "2001", "2002", "2003"));
        let generic = r#"<IPPhoneDirectory><DirectoryEntry><Name>Lab</Name><Telephone>3001</Telephone><Telephone>3002</Telephone></DirectoryEntry></IPPhoneDirectory>"#;
        let lab = &parse(generic, "").unwrap().entries[0].contact;
        assert_eq!((lab.name.as_str(), lab.number.as_str(), lab.phone.as_str()), ("Lab", "3001", "3002"));
        assert!(parse("not a directory", "text/plain").is_err());
    }

    #[test]
    fn csv_with_a_header() {
        let l = parse("Name,Number,Mobile\r\n\"Lovelace, Ada\",1001,98765\r\nBob;1002;\r\n", "text/csv; charset=utf-8").unwrap();
        assert_eq!(l.entries.len(), 2);
        assert_eq!((l.entries[0].contact.name.as_str(), l.entries[0].contact.mobile.as_str()), ("Lovelace, Ada", "98765"));
    }

    #[test]
    fn directory_entries_merge() {
        let mine = Contact { id: 1, name: "Ada".into(), number: "1001".into(), ..Contact::default() };
        let listed = Contact { id: 2, name: "Desk".into(), number: "2001".into(), comment: "old".into(), directory: true, ..Contact::default() };
        let gone = Contact { id: 3, name: "Gone".into(), number: "9".into(), directory: true, ..Contact::default() };
        let l = parse(
            r#"[{"name": "Ada", "number": "1001"}, {"name": "Desk", "number": "2001", "comment": "new"}, {"name": "Lab", "number": "3001"}]"#,
            "",
        )
        .unwrap();
        let plan = merge(&[mine, listed, gone], &l.entries);
        assert_eq!(plan.update.len(), 1);
        assert_eq!((plan.update[0].id, plan.update[0].comment.as_str()), (2, "new"));
        assert_eq!(plan.add.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Lab"], "the user's own Ada isn't duplicated");
        assert_eq!(plan.remove, [3]);
    }

    #[test]
    fn refresh_and_retry_timing() {
        assert_eq!(refresh_after(Some(30), Some("max-age=999")), Some(Duration::from_secs(30)), "the body wins, unclamped");
        assert_eq!(refresh_after(None, Some("public, max-age=5")), Some(Duration::from_secs(60)), "clamped to a minute");
        assert_eq!(refresh_after(None, Some("max-age=999999")), Some(Duration::from_secs(86400)));
        assert_eq!(refresh_after(None, None), None);
        assert_eq!(refresh_after(Some(0), None), None);
        assert_eq!((retry_after(1), retry_after(2)), (Duration::from_secs(10), Duration::from_secs(80)));
    }
}
