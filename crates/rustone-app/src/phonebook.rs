//! The contacts, call history and messages in `rustone.db`, cached for the
//! UI, plus vCard import/export through native file dialogs, which run on a
//! thread of their own so the window keeps drawing.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use rustone_core::contacts::Contact;
use rustone_core::history::{self, CallRecord};
use rustone_core::messages::{Conversation, MessageRecord, MessageState};
use sip_engine::uri_user_host;

use crate::calls::{EndedCall, Party};
use rustone_core::store::Store;
use rustone_core::vcard;

/// A file dialog's outcome, delivered to the UI thread.
enum Picked {
    Import(PathBuf),
    Export(PathBuf),
    ExportHistory(PathBuf),
    Cancelled,
}

pub struct Phonebook {
    store: Option<Store>,
    /// Every contact, starred first, then by name.
    pub contacts: Vec<Contact>,
    /// The call history, newest first.
    pub calls: Vec<CallRecord>,
    /// Message conversations, the most recent first.
    pub conversations: Vec<Conversation>,
    /// Changes whenever messages change, so views can reload.
    pub revision: u64,
    dialog: Option<Receiver<Picked>>,
}

impl Phonebook {
    /// Opens the database; without one, contacts live in memory for this run.
    pub fn open(path: &Path) -> Self {
        let store = match Store::open(path) {
            Ok(store) => Some(store),
            Err(e) => {
                log::error!("cannot open {}: {e}; contacts won't be saved", path.display());
                Store::open_in_memory().ok()
            }
        };
        let mut book = Self { store, contacts: Vec::new(), calls: Vec::new(), conversations: Vec::new(), revision: 0, dialog: None };
        book.reload();
        book
    }

    fn reload(&mut self) {
        self.reload_messages();
        let Some(store) = &self.store else { return };
        match store.calls() {
            Ok(calls) => self.calls = calls,
            Err(e) => log::warn!("cannot read the call history: {e}"),
        }
        match store.contacts() {
            Ok(mut contacts) => {
                // Starred contacts sort first in every column.
                contacts.sort_by_key(|c| !c.starred);
                self.contacts = contacts;
            }
            Err(e) => log::warn!("cannot read contacts: {e}"),
        }
    }

    fn reload_messages(&mut self) {
        self.revision += 1;
        let Some(store) = &self.store else { return };
        match store.conversations() {
            Ok(list) => self.conversations = list,
            Err(e) => log::warn!("cannot read messages: {e}"),
        }
    }

    /// One conversation, oldest first.
    pub fn messages_with(&self, peer: &str) -> Vec<MessageRecord> {
        let Some(store) = &self.store else { return Vec::new() };
        store.messages_with(peer).unwrap_or_else(|e| {
            log::warn!("cannot read the conversation: {e}");
            Vec::new()
        })
    }

    /// Unread messages over all conversations.
    pub fn unread(&self) -> u32 {
        self.conversations.iter().map(|c| c.unread).sum()
    }

    /// Saves a message; returns its id (0 if it couldn't be saved).
    pub fn add_message(&mut self, message: &MessageRecord) -> i64 {
        let Some(store) = &self.store else { return 0 };
        let id = store.add_message(message).unwrap_or_else(|e| {
            log::warn!("cannot save the message: {e}");
            0
        });
        self.reload_messages();
        id
    }

    pub fn set_message_state(&mut self, id: i64, state: MessageState, reason: &str) {
        let Some(store) = &self.store else { return };
        if let Err(e) = store.set_message_state(id, state, reason) {
            log::warn!("cannot update the message: {e}");
        }
        self.reload_messages();
    }

    pub fn mark_read(&mut self, peer: &str) {
        if !self.conversations.iter().any(|c| c.peer == peer && c.unread > 0) {
            return;
        }
        if let Some(store) = &self.store
            && let Err(e) = store.mark_read(peer)
        {
            log::warn!("cannot mark the conversation read: {e}");
        }
        self.reload_messages();
    }

    pub fn delete_conversation(&mut self, peer: &str) -> Result<(), String> {
        let store = self.store.as_ref().ok_or("no database")?;
        store.delete_conversation(peer)?;
        self.reload_messages();
        Ok(())
    }

    /// The numbers of contacts with presence on, without repeats.
    pub fn watched_numbers(&self) -> Vec<String> {
        let mut numbers: Vec<String> = self.contacts.iter().filter(|c| c.presence).map(|c| c.number.trim().to_owned()).filter(|n| !n.is_empty()).collect();
        numbers.sort();
        numbers.dedup();
        numbers
    }

    /// The contact a caller's number belongs to, for showing names.
    pub fn lookup(&self, number: &str) -> Option<&Contact> {
        rustone_core::contacts::find_by_number(&self.contacts, number)
    }

    /// The contact whose number is exactly `number`.
    pub fn with_number(&self, number: &str) -> Option<&Contact> {
        let number = number.trim();
        self.contacts.iter().find(|c| c.number.trim() == number)
    }

    /// Saves a new or edited contact; returns its id.
    pub fn save(&mut self, contact: &Contact) -> Result<i64, String> {
        let store = self.store.as_ref().ok_or("no contacts database")?;
        let id = store.save_contact(contact)?;
        self.reload();
        Ok(id)
    }

    pub fn delete(&mut self, id: i64) -> Result<(), String> {
        let store = self.store.as_ref().ok_or("no contacts database")?;
        store.delete_contact(id)?;
        self.reload();
        Ok(())
    }

    /// Applies a users-directory merge.
    pub fn apply_directory(&mut self, plan: &rustone_core::directory::MergePlan) -> Result<(), String> {
        if plan.is_empty() {
            return Ok(());
        }
        let store = self.store.as_mut().ok_or("no contacts database")?;
        for c in &plan.update {
            store.save_contact(c)?;
        }
        for id in &plan.remove {
            store.delete_contact(*id)?;
        }
        store.add_contacts(&plan.add)?;
        self.reload();
        Ok(())
    }

    /// Adds the contacts in a vCard file, skipping numbers already in the
    /// book. Returns (added, skipped).
    pub fn import(&mut self, text: &str) -> Result<(usize, usize), String> {
        let mut seen: std::collections::HashSet<String> = self.contacts.iter().map(|c| c.number.trim().to_owned()).collect();
        let cards = vcard::parse(text);
        let total = cards.len();
        let new: Vec<Contact> = cards.into_iter().filter(|c| seen.insert(c.number.trim().to_owned())).collect();
        let store = self.store.as_mut().ok_or("no contacts database")?;
        let added = store.add_contacts(&new)?;
        self.reload();
        Ok((added, total - added))
    }

    /// The user's own contacts (not the users directory's) as vCard.
    pub fn export(&self) -> String {
        let own: Vec<Contact> = self.contacts.iter().filter(|c| !c.directory).cloned().collect();
        vcard::write(&own)
    }

    /// Records a finished call. `own_hosts` are the account's registrar,
    /// domain and proxies: numbers there are stored without the host.
    pub fn record(&mut self, call: &EndedCall, own_hosts: &[String]) {
        let record = record_for(call, own_hosts);
        let Some(store) = &self.store else { return };
        match store.add_call(&record) {
            Ok(_) => self.reload(),
            Err(e) => log::warn!("cannot save the call to the history: {e}"),
        }
    }

    pub fn delete_call(&mut self, id: i64) -> Result<(), String> {
        let store = self.store.as_ref().ok_or("no database")?;
        store.delete_call(id)?;
        self.reload();
        Ok(())
    }

    pub fn clear_calls(&mut self) -> Result<(), String> {
        let store = self.store.as_ref().ok_or("no database")?;
        store.clear_calls()?;
        self.reload();
        Ok(())
    }

    /// Asks where to export the call history (CSV).
    pub fn pick_export_history(&mut self) {
        self.run_dialog(|| {
            rfd::FileDialog::new()
                .set_title("Export call history")
                .add_filter("CSV", &["csv"])
                .set_file_name("RusTone calls.csv")
                .save_file()
                .map(Picked::ExportHistory)
        });
    }

    /// Asks for a .vcf file to import.
    pub fn pick_import(&mut self) {
        self.run_dialog(|| rfd::FileDialog::new().set_title("Import contacts").add_filter("vCard", &["vcf", "vcard"]).pick_file().map(Picked::Import));
    }

    /// Asks where to export the contacts.
    pub fn pick_export(&mut self) {
        self.run_dialog(|| {
            rfd::FileDialog::new()
                .set_title("Export contacts")
                .add_filter("vCard", &["vcf"])
                .set_file_name("RusTone contacts.vcf")
                .save_file()
                .map(Picked::Export)
        });
    }

    fn run_dialog(&mut self, dialog: impl FnOnce() -> Option<Picked> + Send + 'static) {
        if self.dialog.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(dialog().unwrap_or(Picked::Cancelled));
        });
        self.dialog = Some(rx);
    }

    /// Whether a file dialog is open (the UI keeps polling while it is).
    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// Finishes an import or export whose dialog has closed; returns a
    /// message for the user.
    pub fn poll_dialog(&mut self) -> Option<String> {
        let picked = match self.dialog.as_ref()?.try_recv() {
            Ok(picked) => picked,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Picked::Cancelled,
        };
        self.dialog = None;
        match picked {
            Picked::Cancelled => None,
            Picked::Import(path) => Some(match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| self.import(&String::from_utf8_lossy(&bytes))) {
                Ok((0, 0)) => "No contacts found in that file".to_owned(),
                Ok((added, 0)) => format!("Imported {added} contact{}", plural(added)),
                Ok((added, skipped)) => format!("Imported {added} contact{}; {skipped} already in your contacts", plural(added)),
                Err(e) => format!("Couldn't import {}: {e}", path.display()),
            }),
            Picked::ExportHistory(path) => {
                let csv = history::to_csv(&self.calls, rfc3339);
                Some(match fastframe_file::write(&path, csv) {
                    Ok(()) => format!("Exported {} call{}", self.calls.len(), plural(self.calls.len())),
                    Err(e) => format!("Couldn't export to {}: {e}", path.display()),
                })
            }
            Picked::Export(path) => {
                let count = self.contacts.iter().filter(|c| !c.directory).count();
                Some(match fastframe_file::write(&path, self.export()) {
                    Ok(()) => format!("Exported {count} contact{}", plural(count)),
                    Err(e) => format!("Couldn't export to {}: {e}", path.display()),
                })
            }
        }
    }
}

/// Unix seconds as RFC 3339 UTC.
fn rfc3339(seconds: i64) -> String {
    jiff::Timestamp::from_second(seconds).map(|t| t.to_string()).unwrap_or_default()
}

/// The history entry for a finished call: the
/// caller ID when there is one, the user part for the account's own hosts
/// and `user@host` for others, and the SIP display name.
pub fn record_for(call: &EndedCall, own_hosts: &[String]) -> CallRecord {
    let source = call.info.caller_id.as_deref().unwrap_or(&call.info.remote);
    let (number, name) = peer_for(source, own_hosts);
    let time = call.started.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    CallRecord {
        id: 0,
        kind: call.kind(),
        number,
        name,
        time,
        duration: if call.answered { call.info.connected_for.as_secs() as u32 } else { 0 },
        info: call.reason.clone(),
    }
}

/// Who a SIP address is, for the history and messages: the user part for
/// the account's own hosts, `user@host` for others, and the display name.
pub fn peer_for(address: &str, own_hosts: &[String]) -> (String, String) {
    let party = Party::parse(address);
    let user_host = uri_user_host(address);
    let host_only = |h: &str| h.rsplit_once(':').map_or(h, |(host, _)| host).trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    let number = match user_host.split_once('@') {
        Some((user, host)) if !user.is_empty() && !own_hosts.iter().any(|own| host_only(own) == host_only(host)) => format!("{user}@{host}"),
        _ => party.user,
    };
    (number, party.name.unwrap_or_default())
}

/// The key a conversation is stored under: no `sip:` or brackets, and no
/// formatting characters in the user part, so "98765 43210" typed and
/// "9876543210" received meet in one conversation.
pub fn conversation_key(number: &str) -> String {
    let n = number.trim().trim_start_matches('<').trim_end_matches('>');
    let n = n.strip_prefix("sip:").unwrap_or(n);
    let (user, host) = match n.split_once('@') {
        Some((user, host)) => (user, Some(host)),
        None => (n, None),
    };
    // Only phone numbers lose their formatting; names, hosts and IP
    // addresses keep their dots and dashes.
    let phone_like = user.chars().any(|c| c.is_ascii_digit()) && user.chars().all(|c| c.is_ascii_digit() || " -()./+".contains(c));
    let user: String = if phone_like && !is_ip(user) { user.chars().filter(|c| !" -()./".contains(*c)).collect() } else { user.to_owned() };
    match host {
        Some(host) => format!("{user}@{}", host.to_ascii_lowercase()),
        None => user,
    }
}

/// An IPv4 address, possibly with a port (dots there aren't formatting).
fn is_ip(s: &str) -> bool {
    let host = s.rsplit_once(':').map_or(s, |(h, _)| h);
    host.parse::<std::net::Ipv4Addr>().is_ok()
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Phonebook {
        Phonebook { store: Store::open_in_memory().ok(), contacts: Vec::new(), calls: Vec::new(), conversations: Vec::new(), revision: 0, dialog: None }
    }

    #[test]
    fn starred_first_and_duplicates_skipped_on_import() {
        let mut b = book();
        b.save(&Contact { name: "Zed".into(), number: "9".into(), starred: true, ..Contact::default() }).unwrap();
        b.save(&Contact { name: "Ada".into(), number: "1".into(), ..Contact::default() }).unwrap();
        assert_eq!(b.contacts.iter().map(|c| c.title()).collect::<Vec<_>>(), ["Zed", "Ada"]);
        let vcf = "BEGIN:VCARD\nFN:Again\nTEL:1\nEND:VCARD\nBEGIN:VCARD\nFN:New\nTEL:2\nEND:VCARD\nBEGIN:VCARD\nFN:Twice\nTEL:2\nEND:VCARD\n";
        assert_eq!(b.import(vcf).unwrap(), (1, 2));
        assert_eq!(b.with_number("2").unwrap().name, "New");
        assert!(b.export().contains("FN:Zed"));
    }

    #[test]
    fn conversation_keys_meet() {
        assert_eq!(conversation_key("98765 43210"), "9876543210");
        assert_eq!(conversation_key("<sip:+91 (98) 765-43210@Example.COM>"), "+919876543210@example.com");
        assert_eq!(conversation_key("1001"), "1001");
        assert_eq!(conversation_key("192.168.29.54"), "192.168.29.54", "IP addresses keep their dots");
        assert_eq!(conversation_key("front-desk"), "front-desk", "names keep their dashes");
        assert_eq!(conversation_key("+1 (555) 010-0100"), "+15550100100");
    }

    #[test]
    fn messages_are_kept_per_conversation() {
        let mut b = book();
        let msg = |peer: &str, outgoing, state, unread| MessageRecord {
            id: 0,
            peer: peer.into(),
            name: String::new(),
            outgoing,
            time: 1,
            text: "hi".into(),
            state,
            reason: String::new(),
            unread,
        };
        let before = b.revision;
        let id = b.add_message(&msg("1001", true, MessageState::Sending, false));
        b.add_message(&msg("1001", false, MessageState::Received, true));
        b.add_message(&msg("1002", false, MessageState::Received, true));
        assert!(b.revision > before);
        assert_eq!(b.unread(), 2);
        b.set_message_state(id, MessageState::Sent, "200 OK");
        b.mark_read("1001");
        assert_eq!(b.unread(), 1);
        assert_eq!(b.messages_with("1001")[0].state, MessageState::Sent);
        b.delete_conversation("1002").unwrap();
        assert_eq!(b.conversations.len(), 1);
    }

    fn ended(remote: &str, caller_id: Option<&str>) -> EndedCall {
        let info = sip_engine::CallInfo {
            id: 1,
            direction: sip_engine::Direction::Incoming,
            state: sip_engine::CallState::Disconnected,
            status_code: 200,
            status_text: "OK".into(),
            remote: remote.into(),
            local: String::new(),
            media: sip_engine::MediaState::None,
            early_media: false,
            connected_for: std::time::Duration::from_secs(65),
            remote_video: false,
            video: false,
            receiving_video: false,
            sending_video: false,
            camera_off: false,
            in_conference: false,
            recording: false,
            sip_call_id: "x".into(),
            caller_id: caller_id.map(str::to_owned),
            user_agent: None,
            diversion: None,
            auto_answer_header: None,
            call_info_header: None,
        };
        EndedCall { info, started: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000), answered: true, reason: "Call ended".into() }
    }

    #[test]
    fn records_for_finished_calls() {
        let own = ["sip.example.com".to_owned(), "192.168.29.1:5068".to_owned()];
        let r = record_for(&ended("<sip:+911234@192.168.29.1:5068>", Some("\"Ada\" <sip:+915678@sip.example.com;user=phone>")), &own);
        assert_eq!((r.number.as_str(), r.name.as_str(), r.duration, r.time), ("+915678", "Ada", 65, 1_000));
        assert_eq!(r.kind, history::CallKind::Incoming);
        let foreign = record_for(&ended("\"Desk\" <sip:1001@192.168.1.5>", None), &own);
        assert_eq!((foreign.number.as_str(), foreign.name.as_str()), ("1001@192.168.1.5", "Desk"));
        assert_eq!(record_for(&ended("<sip:192.168.1.5>", None), &own).number, "192.168.1.5");
    }
}
