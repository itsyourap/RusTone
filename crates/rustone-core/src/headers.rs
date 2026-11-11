//! Custom SIP headers for an account's requests, one rule per line:
//!
//! ```text
//! # Every request gets this one
//! P-Access-Network-Info: GPON;PSAPId=+910000
//! # Only INVITE and PRACK requests
//! [INVITE PRACK] P-Early-Media: supported
//! # An empty value removes the header
//! User-Agent:
//! ```
//!
//! A rule replaces any header of the same name, so it can change the ones
//! RusTone sends too. `{user}` and `{domain}` stand for the account's user
//! and domain.

/// One header rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderRule {
    /// Upper-case method names; empty for every request.
    pub methods: Vec<String>,
    pub name: String,
    /// Empty: remove the header.
    pub value: String,
}

/// A header name: an RFC 3261 token.
fn is_token(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "-.!%*_+`'~".contains(c))
}

/// The rules in `text`, and a message for each line that isn't one
/// (`line 3: …`). Blank lines and `#` comments are skipped.
pub fn parse(text: &str) -> (Vec<HeaderRule>, Vec<String>) {
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut error = |e: &str| errors.push(format!("line {}: {e}", i + 1));
        let (methods, rest) = match line.strip_prefix('[') {
            Some(rest) => match rest.split_once(']') {
                Some((methods, rest)) => (methods.split(|c: char| c == ',' || c.is_whitespace()).filter(|m| !m.is_empty()).map(str::to_ascii_uppercase).collect(), rest.trim()),
                None => {
                    error("no ] after the methods");
                    continue;
                }
            },
            None => (Vec::new(), line),
        };
        if methods.iter().any(|m: &String| !is_token(m)) {
            error("methods are names like INVITE");
            continue;
        }
        let Some((name, value)) = rest.split_once(':') else {
            error("expected Name: value");
            continue;
        };
        let name = name.trim();
        if !is_token(name) {
            error(&format!("\"{name}\" isn't a header name"));
            continue;
        }
        rules.push(HeaderRule { methods, name: name.to_owned(), value: value.trim().to_owned() });
    }
    (rules, errors)
}

/// `template` with `{user}` and `{domain}` filled in.
pub fn fill(template: &str, user: &str, domain: &str) -> String {
    template.replace("{user}", user).replace("{domain}", domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_parse() {
        let text = "# comment\n\nP-Access-Network-Info: GPON;PSAPId=+910000\n[invite, PRACK] P-Early-Media: supported\nUser-Agent:\n[INVITE] Accept-Contact: *;+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.mmtel\"\n";
        let (rules, errors) = parse(text);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(rules.len(), 4);
        assert_eq!(rules[0], HeaderRule { methods: vec![], name: "P-Access-Network-Info".into(), value: "GPON;PSAPId=+910000".into() });
        assert_eq!(rules[1].methods, ["INVITE", "PRACK"]);
        assert_eq!(rules[2].value, "", "an empty value removes");
        assert_eq!(rules[3].value, "*;+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.mmtel\"");
    }

    #[test]
    fn bad_lines_are_reported() {
        let (rules, errors) = parse("no colon here\n[INVITE P-X: 1\nBad Name: 1\n[IN/VITE] X: 1\nGood: yes");
        assert_eq!(rules.len(), 1);
        assert_eq!(errors.len(), 4);
        assert!(errors[0].starts_with("line 1:"));
        assert!(errors[2].contains("\"Bad Name\""));
    }

    #[test]
    fn placeholders_fill() {
        assert_eq!(fill("<sip:{user}@{domain}>", "+910000000000", "ims.example"), "<sip:+910000000000@ims.example>");
    }
}
