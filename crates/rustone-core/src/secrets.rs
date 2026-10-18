//! Passwords live in the OS credential store (Windows Credential Manager),
//! never in settings.toml.

use std::collections::HashMap;
use std::sync::Mutex;

pub trait SecretStore: Send + Sync {
    /// The secret stored under `key`, or `None` if there is none.
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    /// Stores `secret`; an empty secret deletes the entry.
    fn set(&self, key: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// The platform credential store, entries named `<service>` / `<key>`.
pub struct Keyring {
    service: String,
}

impl Keyring {
    /// Installs the platform store. Call once at startup.
    pub fn init(service: &str) -> Result<Self, String> {
        #[cfg(windows)]
        keyring_core::set_default_store(windows_native_keyring_store::Store::new().map_err(|e| e.to_string())?);
        #[cfg(not(windows))]
        return Err("no credential store on this platform yet (Phase 10)".into());
        #[cfg(windows)]
        Ok(Self { service: service.to_owned() })
    }

    fn entry(&self, key: &str) -> Result<keyring_core::Entry, String> {
        keyring_core::Entry::new(&self.service, key).map_err(|e| e.to_string())
    }
}

impl SecretStore for Keyring {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        match self.entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        if secret.is_empty() {
            return self.delete(key);
        }
        self.entry(key)?.set_password(secret).map_err(|e| e.to_string())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// In-memory store, for tests and as a fallback when the OS store fails.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).get(key).cloned())
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if secret.is_empty() {
            map.remove(key);
        } else {
            map.insert(key.to_owned(), secret.to_owned());
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(store: &dyn SecretStore, key: &str) {
        store.delete(key).unwrap();
        assert_eq!(store.get(key).unwrap(), None);
        store.set(key, "pässwörd 1").unwrap();
        assert_eq!(store.get(key).unwrap().as_deref(), Some("pässwörd 1"));
        store.set(key, "").unwrap();
        assert_eq!(store.get(key).unwrap(), None);
        store.delete(key).unwrap();
    }

    #[test]
    fn memory_store() {
        exercise(&MemoryStore::default(), "k");
    }

    /// Writes and removes one entry in Windows Credential Manager.
    #[cfg(windows)]
    #[test]
    fn windows_credential_manager() {
        let store = Keyring::init("RusTone-test").unwrap();
        exercise(&store, "unit-test-entry");
    }
}
