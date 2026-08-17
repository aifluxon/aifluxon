use crate::error::AuthError;
use crate::secret::SecretString;
use std::collections::HashMap;
use std::sync::Mutex;

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<SecretString>, AuthError>;
    fn set(&self, key: &str, value: &SecretString) -> Result<(), AuthError>;
    fn delete(&self, key: &str) -> Result<(), AuthError>;
}

#[derive(Default)]
pub struct MemorySecretStore {
    values: Mutex<HashMap<String, String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> Result<Option<SecretString>, AuthError> {
        let values = self
            .values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(values.get(key).cloned().map(SecretString::new))
    }

    fn set(&self, key: &str, value: &SecretString) -> Result<(), AuthError> {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.to_string(), value.expose().to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), AuthError> {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip() {
        let store = MemorySecretStore::new();
        store.set("k", &SecretString::new("v")).unwrap();
        assert_eq!(store.get("k").unwrap().unwrap().expose(), "v");
    }

    #[test]
    fn memory_store_delete() {
        let store = MemorySecretStore::new();
        store.set("k", &SecretString::new("v")).unwrap();
        store.delete("k").unwrap();
        assert!(store.get("k").unwrap().is_none());
    }

    #[test]
    fn memory_store_isolated_instances() {
        let a = MemorySecretStore::new();
        let b = MemorySecretStore::new();
        a.set("k", &SecretString::new("v")).unwrap();
        assert!(b.get("k").unwrap().is_none());
    }

    #[test]
    fn memory_store_never_persists_to_disk() {
        let store = MemorySecretStore::new();
        store.set("token", &SecretString::new("secret")).unwrap();
        drop(store);
        let store = MemorySecretStore::new();
        assert!(store.get("token").unwrap().is_none());
    }
}
