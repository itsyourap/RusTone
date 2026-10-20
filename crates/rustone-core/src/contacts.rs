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

    /// The numbers a call could dial: number, phone, mobile (non-empty, in that order).
    pub fn numbers(&self) -> impl Iterator<Item = &str> {
        [&self.number, &self.phone, &self.mobile].into_iter().map(|s| s.trim()).filter(|s| !s.is_empty())
    }
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
}
