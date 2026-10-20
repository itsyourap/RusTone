//! The contacts (and later the call history) in `rustone.db`, cached for the
//! UI, plus vCard import/export through native file dialogs, which run on a
//! thread of their own so the window keeps drawing.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use rustone_core::contacts::Contact;
use rustone_core::store::Store;
use rustone_core::vcard;

/// A file dialog's outcome, delivered to the UI thread.
enum Picked {
    Import(PathBuf),
    Export(PathBuf),
    Cancelled,
}

pub struct Phonebook {
    store: Option<Store>,
    /// Every contact, starred first, then by name.
    pub contacts: Vec<Contact>,
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
        let mut book = Self { store, contacts: Vec::new(), dialog: None };
        book.reload();
        book
    }

    fn reload(&mut self) {
        let Some(store) = &self.store else { return };
        match store.contacts() {
            Ok(mut contacts) => {
                // Starred contacts sort first in every column.
                contacts.sort_by_key(|c| !c.starred);
                self.contacts = contacts;
            }
            Err(e) => log::warn!("cannot read contacts: {e}"),
        }
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

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Phonebook {
        Phonebook { store: Store::open_in_memory().ok(), contacts: Vec::new(), dialog: None }
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
}
