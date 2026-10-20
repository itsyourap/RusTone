//! Loading the users directory on a schedule.
//! Requests run on a thread of their own.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use fastframe_shell::Waker;
use rustone_core::directory::{self, Status, UrlValues};
use sip_engine::{Presence, PresenceState};

use crate::phonebook::Phonebook;

/// What a request got back.
enum Fetched {
    /// The connection failed.
    Failed(String),
    Response { status: u16, content_type: String, cache_control: Option<String>, body: String },
}

/// How long a typed URL must stay the same before it's loaded.
const SETTLE: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct UsersDirectory {
    /// The URL setting this state belongs to.
    template: String,
    /// A new URL being typed, adopted once it stops changing.
    pending: Option<(String, Instant)>,
    /// Counts requests.
    sequence: u32,
    /// Failed connections in a row.
    failures: u32,
    /// The last refresh interval the server asked for.
    refresh: Option<Duration>,
    next_load: Option<Instant>,
    inflight: Option<Receiver<Fetched>>,
    silent: bool,
    /// Presence the directory reports, by number (applied to
    /// any contact with that number).
    pub presence: HashMap<String, Presence>,
    /// The latest outcome, for Settings.
    pub status: Option<String>,
}

impl UsersDirectory {
    /// Loads when due. `account_ready` is whether the account is registered
    /// (URLs with account details wait for it). Returns a message for the user.
    pub fn tick(&mut self, template: &str, values: &UrlValues, account_ready: bool, book: &mut Phonebook, waker: &Waker) -> Option<String> {
        let template = template.trim();
        if template != self.template {
            // Wait until the URL stops changing, so typing doesn't fetch every prefix.
            match &self.pending {
                Some((pending, since)) if pending == template => {
                    if since.elapsed() >= SETTLE {
                        *self = Self { template: template.to_owned(), next_load: (!template.is_empty()).then(Instant::now), ..Self::default() };
                    }
                }
                _ => self.pending = Some((template.to_owned(), Instant::now())),
            }
        } else {
            self.pending = None;
        }
        if let Some(message) = self.finish(book) {
            return Some(message);
        }
        let due = self.next_load.is_some_and(|t| Instant::now() >= t);
        if !due || self.inflight.is_some() || (directory::needs_account(template) && !account_ready) {
            return None;
        }
        self.next_load = None;
        let url = directory::build_url(template, values, self.sequence);
        self.sequence += 1;
        let (tx, rx) = mpsc::channel();
        let waker = waker.clone();
        std::thread::spawn(move || {
            let _ = tx.send(fetch(&url));
            waker.wake();
        });
        self.inflight = Some(rx);
        None
    }

    /// When the next load (or a typed URL settling) is due, for scheduling a wake-up.
    pub fn next_load(&self) -> Option<Instant> {
        let settle = self.pending.as_ref().map(|(_, since)| *since + SETTLE);
        [self.next_load, settle].into_iter().flatten().min()
    }

    /// Handles a finished request.
    fn finish(&mut self, book: &mut Phonebook) -> Option<String> {
        let fetched = match self.inflight.as_ref()?.try_recv() {
            Ok(f) => f,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Fetched::Failed("the request stopped".into()),
        };
        self.inflight = None;
        let first = self.sequence == 1;
        let message = match fetched {
            Fetched::Failed(e) => {
                self.failures += 1;
                self.next_load = Some(Instant::now() + directory::retry_after(self.failures));
                self.status = Some(format!("Connection failed: {e}"));
                first.then(|| "Users directory: connection failed".to_owned())
            }
            Fetched::Response { status, .. } if status >= 300 => {
                self.failures = 0;
                self.schedule_refresh();
                self.status = Some(format!("The server returned an error code: {status}"));
                first.then(|| format!("Users directory: the server returned an error code: {status}"))
            }
            Fetched::Response { content_type, cache_control, body, .. } => {
                self.failures = 0;
                match directory::parse(&body, &content_type) {
                    Ok(listing) => {
                        self.silent = listing.silent;
                        self.refresh = directory::refresh_after(listing.refresh, cache_control.as_deref());
                        self.schedule_refresh();
                        self.presence = listing.presence.iter().map(|(number, status, info)| (number.clone(), presence(number, *status, info))).collect();
                        let plan = directory::merge(&book.contacts, &listing.entries);
                        let count = listing.entries.len();
                        match book.apply_directory(&plan) {
                            Ok(()) => {
                                let at = jiff::Zoned::now().strftime("%H:%M");
                                self.status = Some(format!("Loaded {count} contact{} at {at}", if count == 1 { "" } else { "s" }));
                                None
                            }
                            Err(e) => {
                                self.status = Some(format!("Couldn't save the directory: {e}"));
                                Some(format!("Users directory: couldn't save it: {e}"))
                            }
                        }
                    }
                    Err(e) => {
                        self.schedule_refresh();
                        self.status = Some(e.clone());
                        first.then(|| format!("Users directory: {e}"))
                    }
                }
            }
        };
        if self.silent { None } else { message }
    }

    fn schedule_refresh(&mut self) {
        self.next_load = self.refresh.map(|r| Instant::now() + r);
    }
}

fn presence(number: &str, status: Status, info: &str) -> Presence {
    let state = match status {
        Status::Offline => PresenceState::Offline,
        Status::Online => PresenceState::Online,
        Status::Away => PresenceState::Away,
        Status::Busy => PresenceState::Busy,
        Status::Ringing => PresenceState::Ringing,
        Status::OnThePhone => PresenceState::OnThePhone,
        Status::Unknown => PresenceState::Unknown,
    };
    Presence { number: number.to_owned(), state, note: info.to_owned() }
}

/// One GET. HTTP errors are results, not failures.
fn fetch(url: &str) -> Fetched {
    let (url, auth) = split_userinfo(url);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    let mut request = agent.get(&url).header("Cache-Control", "no-cache");
    if let Some((user, password)) = auth {
        request = request.header("Authorization", format!("Basic {}", base64(format!("{user}:{password}").as_bytes())));
    }
    match request.call() {
        Ok(mut response) => {
            let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
            let (content_type, cache_control) = (header("content-type").unwrap_or_default(), header("cache-control"));
            let status = response.status().as_u16();
            match response.body_mut().read_to_string() {
                Ok(body) => Fetched::Response { status, content_type, cache_control, body },
                Err(e) => Fetched::Failed(e.to_string()),
            }
        }
        Err(e) => Fetched::Failed(e.to_string()),
    }
}

/// Takes `user:password@` out of a URL (to pass as
/// credentials); returns the URL without it and the decoded pair.
fn split_userinfo(url: &str) -> (String, Option<(String, String)>) {
    let Some(scheme_end) = url.find("://") else { return (url.to_owned(), None) };
    let rest = &url[scheme_end + 3..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let Some(at) = rest[..authority_end].rfind('@') else { return (url.to_owned(), None) };
    let (user, password) = rest[..at].split_once(':').unwrap_or((&rest[..at], ""));
    let clean = format!("{}{}", &url[..scheme_end + 3], &rest[at + 1..]);
    (clean, Some((percent_decode(user), percent_decode(password))))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_come_out_of_the_url() {
        assert_eq!(split_userinfo("https://bob:p%40ss@dir.example/list?x=1"), ("https://dir.example/list?x=1".to_owned(), Some(("bob".to_owned(), "p@ss".to_owned()))));
        assert_eq!(split_userinfo("https://dir.example/a@b"), ("https://dir.example/a@b".to_owned(), None));
        assert_eq!(base64(b"bob:p@ss"), "Ym9iOnBAc3M=");
        assert_eq!((base64(b"a"), base64(b"ab"), base64(b"abc")), ("YQ==".to_owned(), "YWI=".to_owned(), "YWJj".to_owned()));
    }
}
