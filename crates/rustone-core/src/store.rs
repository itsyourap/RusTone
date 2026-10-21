//! The SQLite database (`rustone.db` in the config folder): contacts, and
//! later call history. Small and local, so it's used from the UI thread; WAL
//! with `synchronous = NORMAL` keeps commits from waiting on the disk.

use std::path::Path;

use rusqlite::{Connection, Row, params};

use crate::contacts::Contact;
use crate::history::{CallKind, CallRecord};
use crate::messages::{Conversation, MessageRecord, MessageState};

pub const FILE_NAME: &str = "rustone.db";

pub type Result<T> = std::result::Result<T, String>;

pub struct Store {
    conn: Connection,
}

/// Schema steps; `PRAGMA user_version` counts how many have run.
const MIGRATIONS: &[&str] = &["CREATE TABLE contacts (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL DEFAULT '',
        number TEXT NOT NULL DEFAULT '',
        first_name TEXT NOT NULL DEFAULT '',
        last_name TEXT NOT NULL DEFAULT '',
        phone TEXT NOT NULL DEFAULT '',
        mobile TEXT NOT NULL DEFAULT '',
        email TEXT NOT NULL DEFAULT '',
        address TEXT NOT NULL DEFAULT '',
        city TEXT NOT NULL DEFAULT '',
        state TEXT NOT NULL DEFAULT '',
        zip TEXT NOT NULL DEFAULT '',
        comment TEXT NOT NULL DEFAULT '',
        starred INTEGER NOT NULL DEFAULT 0,
        presence INTEGER NOT NULL DEFAULT 0,
        directory INTEGER NOT NULL DEFAULT 0
    );",
    "CREATE TABLE history (
        id INTEGER PRIMARY KEY,
        kind INTEGER NOT NULL,
        number TEXT NOT NULL DEFAULT '',
        name TEXT NOT NULL DEFAULT '',
        time INTEGER NOT NULL,
        duration INTEGER NOT NULL DEFAULT 0,
        info TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX history_time ON history (time);",
    "CREATE TABLE messages (
        id INTEGER PRIMARY KEY,
        peer TEXT NOT NULL,
        name TEXT NOT NULL DEFAULT '',
        outgoing INTEGER NOT NULL,
        time INTEGER NOT NULL,
        text TEXT NOT NULL,
        state INTEGER NOT NULL,
        reason TEXT NOT NULL DEFAULT '',
        unread INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX messages_peer_time ON messages (peer, time);",
];

/// Only the last 1000 calls are kept.
pub const HISTORY_LIMIT: i64 = 1000;

const INSERT_CONTACT: &str = "INSERT INTO contacts (name, number, first_name, last_name, phone, mobile, email, address, city, state, zip, comment, starred, presence, directory)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)";

const CONTACT_COLUMNS: &str =
    "id, name, number, first_name, last_name, phone, mobile, email, address, city, state, zip, comment, starred, presence, directory";

fn err(e: rusqlite::Error) -> String {
    e.to_string()
}

impl Store {
    /// Opens (creating it and its folder if needed) the database at `path`
    /// and brings its schema up to date.
    pub fn open(path: &Path) -> Result<Self> {
        // On a first run the config folder may not exist yet.
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let conn = Connection::open(path).map_err(err)?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(err)?;
        conn.pragma_update(None, "synchronous", "NORMAL").map_err(err)?;
        Self::init(conn)
    }

    /// A throwaway database, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(err)?)
    }

    fn init(conn: Connection) -> Result<Self> {
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(err)?;
        let version = usize::try_from(version).unwrap_or(0);
        if version > MIGRATIONS.len() {
            return Err(format!("the database is from a newer RusTone (schema {version})"));
        }
        for (i, step) in MIGRATIONS.iter().enumerate().skip(version) {
            conn.execute_batch(&format!("BEGIN; {step} PRAGMA user_version = {}; COMMIT;", i + 1)).map_err(err)?;
        }
        // A message still sending when RusTone stopped will never get its answer.
        conn.execute("UPDATE messages SET state = ?1, reason = 'Not sent' WHERE state = ?2", [MessageState::Failed.code(), MessageState::Sending.code()])
            .map_err(err)?;
        Ok(Self { conn })
    }

    // -----------------------------------------------------------------------
    // Contacts

    /// Every contact, by name.
    pub fn contacts(&self) -> Result<Vec<Contact>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {CONTACT_COLUMNS} FROM contacts")).map_err(err)?;
        let mut contacts = stmt.query_map([], contact_from_row).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)?;
        contacts.sort_by_cached_key(|c| c.title().to_lowercase());
        Ok(contacts)
    }

    /// Inserts a new contact (id 0) or updates an existing one; returns its id.
    pub fn save_contact(&self, c: &Contact) -> Result<i64> {
        let values = params![
            c.name, c.number, c.first_name, c.last_name, c.phone, c.mobile, c.email, c.address, c.city, c.state, c.zip, c.comment, c.starred,
            c.presence, c.directory
        ];
        if c.id == 0 {
            self.conn
                .execute(
                    INSERT_CONTACT,
                    values,
                )
                .map_err(err)?;
            Ok(self.conn.last_insert_rowid())
        } else {
            let mut all: Vec<&dyn rusqlite::ToSql> = values.to_vec();
            all.push(&c.id);
            self.conn
                .execute(
                    "UPDATE contacts SET name = ?1, number = ?2, first_name = ?3, last_name = ?4, phone = ?5, mobile = ?6, email = ?7, address = ?8,
                     city = ?9, state = ?10, zip = ?11, comment = ?12, starred = ?13, presence = ?14, directory = ?15 WHERE id = ?16",
                    all.as_slice(),
                )
                .map_err(err)?;
            Ok(c.id)
        }
    }

    /// Saves several new contacts in one transaction (imports); returns how many.
    pub fn add_contacts(&mut self, contacts: &[Contact]) -> Result<usize> {
        let tx = self.conn.transaction().map_err(err)?;
        for c in contacts {
            tx.execute(
                INSERT_CONTACT,
                params![
                    c.name, c.number, c.first_name, c.last_name, c.phone, c.mobile, c.email, c.address, c.city, c.state, c.zip, c.comment, c.starred,
                    c.presence, c.directory
                ],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(contacts.len())
    }

    pub fn delete_contact(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM contacts WHERE id = ?1", [id]).map_err(err)?;
        Ok(())
    }
}

impl Store {
    /// Adds a call, dropping the oldest beyond [`HISTORY_LIMIT`]; returns its id.
    pub fn add_call(&self, r: &CallRecord) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO history (kind, number, name, time, duration, info) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![r.kind.code(), r.number, r.name, r.time, r.duration, r.info],
            )
            .map_err(err)?;
        let id = self.conn.last_insert_rowid();
        self.conn
            .execute("DELETE FROM history WHERE id NOT IN (SELECT id FROM history ORDER BY time DESC, id DESC LIMIT ?1)", [HISTORY_LIMIT])
            .map_err(err)?;
        Ok(id)
    }

    /// The history, newest first.
    pub fn calls(&self) -> Result<Vec<CallRecord>> {
        let mut stmt = self.conn.prepare("SELECT id, kind, number, name, time, duration, info FROM history ORDER BY time DESC, id DESC").map_err(err)?;
        stmt.query_map([], |r| {
            Ok(CallRecord {
                id: r.get(0)?,
                kind: CallKind::from_code(r.get(1)?),
                number: r.get(2)?,
                name: r.get(3)?,
                time: r.get(4)?,
                duration: r.get(5)?,
                info: r.get(6)?,
            })
        })
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)
    }

    pub fn delete_call(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM history WHERE id = ?1", [id]).map_err(err)?;
        Ok(())
    }

    pub fn clear_calls(&self) -> Result<()> {
        self.conn.execute("DELETE FROM history", []).map_err(err)?;
        Ok(())
    }
}

const MESSAGE_COLUMNS: &str = "id, peer, name, outgoing, time, text, state, reason, unread";

fn message_from_row(r: &Row<'_>) -> rusqlite::Result<MessageRecord> {
    Ok(MessageRecord {
        id: r.get(0)?,
        peer: r.get(1)?,
        name: r.get(2)?,
        outgoing: r.get(3)?,
        time: r.get(4)?,
        text: r.get(5)?,
        state: MessageState::from_code(r.get(6)?),
        reason: r.get(7)?,
        unread: r.get(8)?,
    })
}

impl Store {
    /// Saves a message; returns its id.
    pub fn add_message(&self, m: &MessageRecord) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO messages (peer, name, outgoing, time, text, state, reason, unread) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![m.peer, m.name, m.outgoing, m.time, m.text, m.state.code(), m.reason, m.unread],
            )
            .map_err(err)?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn set_message_state(&self, id: i64, state: MessageState, reason: &str) -> Result<()> {
        self.conn.execute("UPDATE messages SET state = ?1, reason = ?2 WHERE id = ?3", params![state.code(), reason, id]).map_err(err)?;
        Ok(())
    }

    /// One conversation, oldest first.
    pub fn messages_with(&self, peer: &str) -> Result<Vec<MessageRecord>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE peer = ?1 ORDER BY time, id")).map_err(err)?;
        stmt.query_map([peer], message_from_row).map_err(err)?.collect::<std::result::Result<Vec<_>, _>>().map_err(err)
    }

    /// Every conversation, the most recent first.
    pub fn conversations(&self) -> Result<Vec<Conversation>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {MESSAGE_COLUMNS}, (SELECT count(*) FROM messages u WHERE u.peer = m.peer AND u.unread = 1)
                 FROM messages m WHERE id = (SELECT id FROM messages l WHERE l.peer = m.peer ORDER BY time DESC, id DESC LIMIT 1)
                 ORDER BY time DESC, id DESC"
            ))
            .map_err(err)?;
        stmt.query_map([], |r| Ok(Conversation { peer: r.get(1)?, last: message_from_row(r)?, unread: r.get(9)? }))
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)
    }

    /// The conversation has been seen.
    pub fn mark_read(&self, peer: &str) -> Result<()> {
        self.conn.execute("UPDATE messages SET unread = 0 WHERE peer = ?1 AND unread = 1", [peer]).map_err(err)?;
        Ok(())
    }

    pub fn delete_conversation(&self, peer: &str) -> Result<()> {
        self.conn.execute("DELETE FROM messages WHERE peer = ?1", [peer]).map_err(err)?;
        Ok(())
    }
}

fn contact_from_row(r: &Row<'_>) -> rusqlite::Result<Contact> {
    Ok(Contact {
        id: r.get(0)?,
        name: r.get(1)?,
        number: r.get(2)?,
        first_name: r.get(3)?,
        last_name: r.get(4)?,
        phone: r.get(5)?,
        mobile: r.get(6)?,
        email: r.get(7)?,
        address: r.get(8)?,
        city: r.get(9)?,
        state: r.get(10)?,
        zip: r.get(11)?,
        comment: r.get(12)?,
        starred: r.get(13)?,
        presence: r.get(14)?,
        directory: r.get(15)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contacts_crud() {
        let mut store = Store::open_in_memory().unwrap();
        let mut ada = Contact { name: "Ada".into(), number: "1002".into(), starred: true, ..Contact::default() };
        ada.id = store.save_contact(&ada).unwrap();
        assert!(ada.id > 0);
        store.add_contacts(&[Contact { first_name: "Charles".into(), last_name: "Babbage".into(), ..Contact::default() }]).unwrap();
        let all = store.contacts().unwrap();
        assert_eq!(all.iter().map(Contact::title).collect::<Vec<_>>(), ["Ada", "Charles Babbage"]);
        assert_eq!(all[0], ada);

        ada.mobile = "+91 98765".into();
        store.save_contact(&ada).unwrap();
        assert_eq!(store.contacts().unwrap()[0].mobile, "+91 98765");
        store.delete_contact(ada.id).unwrap();
        assert_eq!(store.contacts().unwrap().len(), 1);
    }

    #[test]
    fn history_keeps_the_newest_calls() {
        let store = Store::open_in_memory().unwrap();
        let call = |time| CallRecord { id: 0, kind: CallKind::Missed, number: "1002".into(), name: String::new(), time, duration: 0, info: "Missed call".into() };
        for t in 0..HISTORY_LIMIT + 5 {
            store.add_call(&call(t)).unwrap();
        }
        let calls = store.calls().unwrap();
        assert_eq!(calls.len() as i64, HISTORY_LIMIT);
        assert_eq!((calls[0].time, calls.last().unwrap().time), (HISTORY_LIMIT + 4, 5), "newest first, oldest dropped");
        assert_eq!(calls[0].kind, CallKind::Missed);
        store.delete_call(calls[0].id).unwrap();
        assert_eq!(store.calls().unwrap().len() as i64, HISTORY_LIMIT - 1);
        store.clear_calls().unwrap();
        assert!(store.calls().unwrap().is_empty());
    }

    #[test]
    fn conversations_and_their_messages() {
        let store = Store::open_in_memory().unwrap();
        let msg = |peer: &str, time, outgoing, state, unread| MessageRecord {
            id: 0,
            peer: peer.into(),
            name: String::new(),
            outgoing,
            time,
            text: format!("{peer} at {time}"),
            state,
            reason: String::new(),
            unread,
        };
        store.add_message(&msg("1001", 1, false, MessageState::Received, true)).unwrap();
        store.add_message(&msg("1002", 2, false, MessageState::Received, true)).unwrap();
        let sent = store.add_message(&msg("1001", 3, true, MessageState::Sending, false)).unwrap();
        store.add_message(&msg("1001", 4, false, MessageState::Received, true)).unwrap();

        let list = store.conversations().unwrap();
        assert_eq!(list.iter().map(|c| (c.peer.as_str(), c.last.time, c.unread)).collect::<Vec<_>>(), [("1001", 4, 2), ("1002", 2, 1)]);
        store.set_message_state(sent, MessageState::Failed, "404 Not Found").unwrap();
        let ada = store.messages_with("1001").unwrap();
        assert_eq!(ada.iter().map(|m| m.time).collect::<Vec<_>>(), [1, 3, 4]);
        assert_eq!((ada[1].state, ada[1].reason.as_str()), (MessageState::Failed, "404 Not Found"));
        store.mark_read("1001").unwrap();
        assert_eq!(store.conversations().unwrap()[0].unread, 0);
        store.delete_conversation("1002").unwrap();
        assert_eq!(store.conversations().unwrap().len(), 1);
    }

    #[test]
    fn unanswered_sends_fail_on_reopen() {
        let dir = std::env::temp_dir().join(format!("rustone-store-msg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        {
            let store = Store::open(&path).unwrap();
            let m = MessageRecord {
                id: 0,
                peer: "1001".into(),
                name: String::new(),
                outgoing: true,
                time: 1,
                text: "hi".into(),
                state: MessageState::Sending,
                reason: String::new(),
                unread: false,
            };
            store.add_message(&m).unwrap();
        }
        let store = Store::open(&path).unwrap();
        let m = &store.messages_with("1001").unwrap()[0];
        assert_eq!((m.state, m.reason.as_str()), (MessageState::Failed, "Not sent"));
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn opening_creates_the_folder() {
        let dir = std::env::temp_dir().join(format!("rustone-store-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir.join("config").join(FILE_NAME)).expect("a first run creates the folder");
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reopening_keeps_data_and_schema() {
        let dir = std::env::temp_dir().join(format!("rustone-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        {
            let store = Store::open(&path).unwrap();
            store.save_contact(&Contact { name: "Ada".into(), ..Contact::default() }).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.contacts().unwrap().len(), 1);
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
