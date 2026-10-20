//! Contacts and how they're shown and searched.

/// A contact. `id` is 0 until it's saved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Contact {
    pub id: i64,
    /// Display name; empty means first + last name, or the number.
    pub name: String,
    /// What a call dials: a number, extension or SIP address.
    pub number: String,
    pub first_name: String,
    pub last_name: String,
    pub phone: String,
    pub mobile: String,
    pub email: String,
    pub address: String,
    pub city: String,
    pub state: String,
    pub zip: String,
    pub comment: String,
    pub starred: bool,
    /// Subscribe to the contact's presence.
    pub presence: bool,
    /// From the users directory: replaced on every refresh.
    pub directory: bool,
}

impl Contact {
    /// The name to show: `name`, else first and last name, else the number.
    pub fn title(&self) -> String {
        if !self.name.trim().is_empty() {
            return self.name.trim().to_owned();
        }
        let full = format!("{} {}", self.first_name.trim(), self.last_name.trim());
        if !full.trim().is_empty() { full.trim().to_owned() } else { self.number.trim().to_owned() }
    }

    /// Whether every word of `query` appears in the name or one of the numbers (case-insensitive).
    pub fn matches(&self, query: &str) -> bool {
        let haystack = [&self.title(), &self.number, &self.phone, &self.mobile, &self.email, &self.comment]
            .iter()
            .map(|s| s.to_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        query.split_whitespace().all(|word| haystack.contains(&word.to_lowercase()))
    }

    /// An empty number takes the phone, then
    /// the mobile; a contact with no number at all is rejected.
    pub fn prepared(mut self) -> Option<Self> {
        if self.number.trim().is_empty() {
            self.number = if !self.phone.trim().is_empty() { self.phone.clone() } else { self.mobile.clone() };
        }
        (!self.number.trim().is_empty()).then_some(self)
    }

    /// The numbers a call could dial: number, phone, mobile (non-empty, in that order).
    pub fn numbers(&self) -> impl Iterator<Item = &str> {
        [&self.number, &self.phone, &self.mobile].into_iter().map(|s| s.trim()).filter(|s| !s.is_empty())
    }
}

/// How well a number matches a contact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NumberMatch {
    /// The caller's number has up to 3 extra leading characters (a prefix
    /// or country code), and the contact's number is longer than 3.
    Suffix,
    Exact,
}

/// A number or SIP address reduced for comparing: no `sip:`, no host, and
/// no formatting characters (` -()./`).
pub fn comparable(number: &str) -> String {
    let n = number.trim();
    let n = n.strip_prefix('<').unwrap_or(n);
    let n = n.split('>').next().unwrap_or(n);
    let lower = n.to_ascii_lowercase();
    let n = ["sips:", "sip:", "tel:"].iter().find_map(|p| lower.strip_prefix(p).map(|_| &n[p.len()..])).unwrap_or(n);
    let user = n.split(['@', ';']).next().unwrap_or(n);
    user.chars().filter(|c| !" -()./".contains(*c)).collect::<String>().to_lowercase()
}

impl Contact {
    /// The best match of `number` against this contact's numbers.
    pub fn match_number(&self, number: &str) -> Option<NumberMatch> {
        let wanted = comparable(number);
        if wanted.is_empty() {
            return None;
        }
        self.numbers()
            .filter_map(|own| {
                let own = comparable(own);
                if own.is_empty() {
                    None
                } else if own == wanted {
                    Some(NumberMatch::Exact)
                } else if own.chars().count() > 3 && wanted.ends_with(&own) && wanted.chars().count() - own.chars().count() <= 3 {
                    Some(NumberMatch::Suffix)
                } else {
                    None
                }
            })
            .max()
    }
}

/// The contact for `number`: an exact match, else the last suffix match.
pub fn find_by_number<'a>(contacts: &'a [Contact], number: &str) -> Option<&'a Contact> {
    let mut suffix = None;
    for c in contacts {
        match c.match_number(number) {
            Some(NumberMatch::Exact) => return Some(c),
            Some(NumberMatch::Suffix) => suffix = Some(c),
            None => {}
        }
    }
    suffix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_fall_back() {
        let mut c = Contact { number: "1002".into(), ..Contact::default() };
        assert_eq!(c.title(), "1002");
        c.first_name = "Ada".into();
        c.last_name = "Lovelace".into();
        assert_eq!(c.title(), "Ada Lovelace");
        c.name = " Countess ".into();
        assert_eq!(c.title(), "Countess");
    }

    #[test]
    fn search_matches_every_word() {
        let c = Contact { name: "Ada Lovelace".into(), number: "1002".into(), mobile: "+91 98765".into(), ..Contact::default() };
        assert!(c.matches("ada"));
        assert!(c.matches("love 1002"));
        assert!(c.matches("98765"));
        assert!(!c.matches("ada babbage"));
        assert!(c.matches(""));
        assert_eq!(c.numbers().collect::<Vec<_>>(), ["1002", "+91 98765"]);
    }

    #[test]
    fn numbers_match_exactly_or_by_suffix() {
        let ada = Contact { name: "Ada".into(), number: "98765 43210".into(), phone: "sip:1002@pbx.example".into(), ..Contact::default() };
        let bob = Contact { name: "Bob".into(), number: "+91 98765 43210".into(), ..Contact::default() };
        assert_eq!(ada.match_number("9876543210"), Some(NumberMatch::Exact));
        assert_eq!(ada.match_number("+919876543210"), Some(NumberMatch::Suffix), "country code in front");
        assert_eq!(ada.match_number("<sip:1002@192.168.29.1:5068;transport=tls>"), Some(NumberMatch::Exact));
        assert_eq!(ada.match_number("19876543210999"), None);
        assert_eq!(Contact { number: "101".into(), ..Contact::default() }.match_number("9101"), None, "short numbers match exactly only");
        let book = [ada.clone(), bob.clone()];
        assert_eq!(find_by_number(&book, "+919876543210").unwrap().name, "Bob", "exact beats suffix");
        assert_eq!(find_by_number(&book, "09876543210").unwrap().name, "Ada", "a trunk prefix in front");
        let twins = [Contact { name: "First".into(), number: "4321".into(), ..Contact::default() }, Contact { name: "Last".into(), number: "54321".into(), ..Contact::default() }];
        assert_eq!(find_by_number(&twins, "654321").unwrap().name, "Last", "the last suffix match wins");
        assert!(find_by_number(&book, "").is_none());
        assert_eq!(comparable("sip:+91 (98) 765-43@x"), "+919876543");
    }

    #[test]
    fn prepare_needs_a_number() {
        let c = Contact { name: "Ada".into(), mobile: "98765".into(), ..Contact::default() };
        assert_eq!(c.prepared().unwrap().number, "98765");
        assert!(Contact { name: "Nobody".into(), ..Contact::default() }.prepared().is_none());
    }
}
