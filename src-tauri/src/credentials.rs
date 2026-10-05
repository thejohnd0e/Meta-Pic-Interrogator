use std::collections::BTreeMap;

use crate::domain::{AppError, AppResult};

pub trait CredentialStore {
    fn set(&mut self, provider: &str, secret: &str) -> AppResult<()>;
    fn get(&self, provider: &str) -> AppResult<Option<String>>;
    fn delete(&mut self, provider: &str) -> AppResult<()>;
}

#[derive(Default)]
pub struct MemoryCredentialStore {
    values: BTreeMap<String, String>,
}

impl CredentialStore for MemoryCredentialStore {
    fn set(&mut self, provider: &str, secret: &str) -> AppResult<()> {
        if provider.trim().is_empty() || secret.is_empty() {
            return Err(AppError::Authentication(
                "credential values are required".to_owned(),
            ));
        }
        self.values.insert(provider.to_owned(), secret.to_owned());
        Ok(())
    }
    fn get(&self, provider: &str) -> AppResult<Option<String>> {
        Ok(self.values.get(provider).cloned())
    }
    fn delete(&mut self, provider: &str) -> AppResult<()> {
        self.values.remove(provider);
        Ok(())
    }
}

#[cfg(windows)]
pub struct WindowsCredentialStore;

#[cfg(windows)]
impl CredentialStore for WindowsCredentialStore {
    fn set(&mut self, provider: &str, secret: &str) -> AppResult<()> {
        if provider.trim().is_empty() || secret.is_empty() {
            return Err(AppError::Authentication(
                "credential values are required".to_owned(),
            ));
        }
        keyring::Entry::new("metapic-interrogator", provider)
            .map_err(|_| AppError::Authentication("credential store unavailable".to_owned()))?
            .set_password(secret)
            .map_err(|_| AppError::Authentication("credential write failed".to_owned()))
    }
    fn get(&self, provider: &str) -> AppResult<Option<String>> {
        match keyring::Entry::new("metapic-interrogator", provider)
            .map_err(|_| AppError::Authentication("credential store unavailable".to_owned()))?
            .get_password()
        {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(AppError::Authentication(
                "credential read failed".to_owned(),
            )),
        }
    }
    fn delete(&mut self, provider: &str) -> AppResult<()> {
        match keyring::Entry::new("metapic-interrogator", provider)
            .map_err(|_| AppError::Authentication("credential store unavailable".to_owned()))?
            .delete_credential()
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(AppError::Authentication(
                "credential delete failed".to_owned(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CredentialStore, MemoryCredentialStore};

    #[test]
    fn memory_store_never_returns_missing_secrets() {
        let mut store = MemoryCredentialStore::default();
        assert!(store.get("openai").expect("lookup succeeds").is_none());
        store.set("openai", "secret").expect("write succeeds");
        assert_eq!(
            store.get("openai").expect("lookup succeeds").as_deref(),
            Some("secret")
        );
        store.delete("openai").expect("delete succeeds");
        assert!(store.get("openai").expect("lookup succeeds").is_none());
    }
}
