//! The SQLite database (`rustone.db` in the config folder): contacts, and
//! later call history. Small and local, so it's used from the UI thread; WAL
//! with `synchronous = NORMAL` keeps commits from waiting on the disk.

use std::path::Path;

use rusqlite::{Connection, Row, params};

use crate::contacts::Contact;

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
    );"];

const INSERT_CONTACT: &str = "INSERT INTO contacts (name, number, first_name, last_name, phone, mobile, email, address, city, state, zip, comment, starred, presence, directory)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)";

const CONTACT_COLUMNS: &str =
    "id, name, number, first_name, last_name, phone, mobile, email, address, city, state, zip, comment, starred, presence, directory";

fn err(e: rusqlite::Error) -> String {
    e.to_string()
}

impl Store {
    /// Opens (creating if needed) the database at `path` and brings its schema up to date.
    pub fn open(path: &Path) -> Result<Self> {
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
