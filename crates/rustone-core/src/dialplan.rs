//! Number formatting: the account's
//! dialling prefix and dial plan, applied to numbers dialled from the main
//! account.
//!
//! A dial plan is rules separated by `|` (surrounding spaces and brackets
//! ignored), the first matching rule wins, and no match blocks the call.
//! In a rule `x` stands for any digit, `.` repeats what comes before it,
//! `*`, `#` and `+` are the keypad's, `[2-9]` and other regular-expression
//! syntax work, and
//! `<match:replace>` swaps the part matching `match` for `replace` (`<:44>`
//! inserts, `<0:>` removes). `<abc>` alone is matched as is.

use regex::Regex;

/// Why a number can't be dialled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked;

/// Applies the dialling prefix and dial plan to `number`. SIP addresses
/// and other non-numbers pass through the prefix untouched; the dial plan
/// sees them too.
pub fn format_number(number: &str, prefix: &str, plan: &str) -> Result<String, Blocked> {
    let number = number.trim();
    if number.contains('<') && number.contains('>') {
        return Ok(number.to_owned());
    }
    let mut formatted = number.to_owned();
    if is_pstn(number) {
        formatted.retain(|c| !matches!(c, '.' | '-' | '(' | ')' | '/' | ' '));
        if !prefix.is_empty() && formatted.chars().count() > 3 {
            if let Some(rest) = formatted.strip_prefix('+') {
                formatted = rest.to_owned();
            }
            formatted = format!("{prefix}{formatted}");
        }
    }
    let plan = plan.trim_matches(|c| matches!(c, ' ' | '(' | ')'));
    if plan.is_empty() {
        return Ok(formatted);
    }
    for rule in plan.split('|').filter(|r| !r.is_empty()) {
        if let Some(result) = apply_rule(rule, &formatted) {
            return Ok(result);
        }
    }
    Err(Blocked)
}

/// Digits and dialling punctuation, or anything
/// starting with `+` (its check lets those through).
fn is_pstn(number: &str) -> bool {
    number.starts_with('+') || number.chars().all(|c| c.is_ascii_digit() || matches!(c, '*' | '#' | '.' | '-' | '(' | ')' | '/' | ' '))
}

/// One rule: `None` when it doesn't match.
fn apply_rule(rule: &str, number: &str) -> Option<String> {
    let mut pattern = String::from("^");
    let mut replaces = Vec::new();
    let mut chars = rule.chars();
    while let Some(c) = chars.next() {
        if c != '<' {
            pattern.push_str(&translate(c));
            continue;
        }
        let group: String = chars.by_ref().take_while(|&c| c != '>').collect();
        match group.split_once(':') {
            Some((matched, replace)) => {
                pattern.push('(');
                pattern.extend(matched.chars().map(translate));
                pattern.push(')');
                replaces.push(replace.to_owned());
            }
            None => pattern.extend(group.chars().map(translate)),
        }
    }
    pattern.push('$');
    let re = Regex::new(&pattern).ok()?;
    let captures = re.captures(number)?;
    let mut out = String::new();
    let mut last = 0;
    for (i, replace) in replaces.iter().enumerate() {
        let Some(m) = captures.get(i + 1) else { continue };
        out.push_str(&number[last..m.start()]);
        out.push_str(replace);
        last = m.end();
    }
    out.push_str(&number[last..]);
    Some(out)
}

/// The rewrite: `.` repeats (`*`), `x` is any digit; the rest is
/// regular-expression syntax, except that the keypad's `*` and `+` are
/// matched as themselves, and braces are literal (ATL used them for groups).
fn translate(c: char) -> String {
    match c {
        '.' => "*".into(),
        'x' | 'X' => "[0-9]".into(),
        '*' | '+' | '{' | '}' => format!("\\{c}"),
        c => c.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_goes_before_longer_numbers() {
        assert_eq!(format_number("+44 (20) 7946-0018", "00", ""), Ok("00442079460018".into()));
        assert_eq!(format_number("123", "9", ""), Ok("123".into()), "short numbers are extensions");
        assert_eq!(format_number("5551234", "9", ""), Ok("95551234".into()));
        assert_eq!(format_number("alice", "9", ""), Ok("alice".into()));
        assert_eq!(format_number("<sip:1234@pbx>", "9", "xxx"), Ok("<sip:1234@pbx>".into()));
    }

    #[test]
    fn dial_plans_match_rewrite_and_block() {
        let plan = "(<9:>1xxxxxxxxxx|xxx|<:0>[2-9]xxxxxxxxx|*x.)";
        assert_eq!(format_number("912125551234", "", plan), Ok("12125551234".into()), "9 removed");
        assert_eq!(format_number("101", "", plan), Ok("101".into()));
        assert_eq!(format_number("2125551234", "", plan), Ok("02125551234".into()), "0 inserted");
        assert_eq!(format_number("*72", "", plan), Ok("*72".into()));
        assert_eq!(format_number("1234", "", plan), Err(Blocked), "nothing matches");
        assert_eq!(format_number("1234", "", ""), Ok("1234".into()), "no plan, no blocking");
        // A literal group, and the prefix applied before the plan.
        assert_eq!(format_number("5551234", "8", "<8>x."), Ok("85551234".into()));
    }
}
