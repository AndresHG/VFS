//! API keys in the OS credential store, with the fallbacks of VFS-28 R2.
//!
//! The store is Windows Credential Manager (local-machine persistence) or the macOS login
//! keychain, service `com.vfs.app`, one account per provider (D6). If it fails, the caller
//! offers the user a protected file («protección básica»: DPAPI on Windows, mode 0600
//! elsewhere) or memory for this session only. Errors never carry the key: the store crate's
//! errors can hold the secret bytes, so only their category is kept.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::provider::Provider;
use crate::secret::ApiKey;

/// The credential store service name (R2).
pub const SERVICE: &str = "com.vfs.app";

/// Why a key could not be saved, read or deleted. No variant carries key text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The text typed is not a plausible key.
    #[error("the key text is not valid")]
    Invalid,
    /// The OS store refused the key: offer the protected file or session memory (R2).
    #[error("the credential store is unavailable ({0})")]
    StoreUnavailable(String),
    /// A saved key exists but cannot be read: «No se puede leer la clave guardada; vuelva a
    /// introducirla» (R3).
    #[error("the saved key cannot be read ({0})")]
    Unreadable(String),
    /// The protected file could not be written or removed.
    #[error("key file: {0}")]
    File(String),
}

/// A credential store failure, by category only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// No credential for this account.
    NoEntry,
    /// Any other failure; the text names its category, never the secret.
    Failure(String),
}

/// The OS credential store, or a test double.
pub trait CredentialBackend: Send + Sync {
    fn set(&self, account: &str, secret: &str) -> Result<(), StoreError>;
    fn get(&self, account: &str) -> Result<String, StoreError>;
    fn delete(&self, account: &str) -> Result<(), StoreError>;
}

/// The native store of this platform through `keyring-core` (D6).
pub struct OsStore {
    store: Option<Arc<keyring_core::CredentialStore>>,
}

impl OsStore {
    pub fn new() -> OsStore {
        OsStore {
            store: native_store(),
        }
    }

    fn entry(&self, account: &str) -> Result<keyring_core::Entry, StoreError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| StoreError::Failure("no credential store on this platform".into()))?;
        // Windows: local-machine persistence, so the key does not roam with the profile (R2).
        let modifiers: HashMap<&str, &str> = if cfg!(windows) {
            HashMap::from([("persistence", "Local")])
        } else {
            HashMap::new()
        };
        keyring_core::Entry::new_in_store_with_modifiers(store, SERVICE, account, &modifiers)
            .map_err(store_error)
    }
}

impl Default for OsStore {
    fn default() -> Self {
        OsStore::new()
    }
}

#[cfg(windows)]
fn native_store() -> Option<Arc<keyring_core::CredentialStore>> {
    let store: Arc<keyring_core::CredentialStore> =
        windows_native_keyring_store::Store::new().ok()?;
    Some(store)
}

#[cfg(target_os = "macos")]
fn native_store() -> Option<Arc<keyring_core::CredentialStore>> {
    let store: Arc<keyring_core::CredentialStore> =
        apple_native_keyring_store::keychain::Store::new().ok()?;
    Some(store)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn native_store() -> Option<Arc<keyring_core::CredentialStore>> {
    None
}

/// Keeps the category of a store error and drops any payload, which may hold the secret.
fn store_error(e: keyring_core::Error) -> StoreError {
    use keyring_core::Error as E;
    StoreError::Failure(
        match e {
            E::NoEntry => return StoreError::NoEntry,
            E::PlatformFailure(_) => "platform failure",
            E::NoStorageAccess(_) => "no storage access",
            E::BadEncoding(_) => "bad encoding",
            E::BadDataFormat(..) => "bad data format",
            E::BadStoreFormat(_) => "bad store format",
            E::TooLong(..) => "too long",
            E::Invalid(..) => "invalid",
            E::Ambiguous(_) => "ambiguous",
            E::NoDefaultStore => "no store",
            E::NotSupportedByStore(_) => "not supported",
            _ => "other",
        }
        .into(),
    )
}

impl CredentialBackend for OsStore {
    fn set(&self, account: &str, secret: &str) -> Result<(), StoreError> {
        self.entry(account)?
            .set_password(secret)
            .map_err(store_error)
    }

    fn get(&self, account: &str) -> Result<String, StoreError> {
        self.entry(account)?.get_password().map_err(store_error)
    }

    fn delete(&self, account: &str) -> Result<(), StoreError> {
        match self
            .entry(account)?
            .delete_credential()
            .map_err(store_error)
        {
            Err(StoreError::NoEntry) => Ok(()),
            other => other,
        }
    }
}

/// The fallback the user picked when the OS store failed (R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// A file only this user can read, «protección básica».
    ProtectedFile,
    /// Memory, for this session only.
    SessionOnly,
}

/// Where a provider's key is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyLocation {
    CredentialStore,
    ProtectedFile,
    SessionOnly,
}

/// The keys of both providers (R2). Only Rust reads them, to make a call.
pub struct Keys {
    os: Box<dyn CredentialBackend>,
    dir: PathBuf,
    memory: Mutex<HashMap<Provider, ApiKey>>,
}

impl Keys {
    /// Keys in the OS store, with protected files under `dir` (the app data folder's `ai-keys`).
    pub fn new(dir: impl Into<PathBuf>) -> Keys {
        Keys::with_backend(Box::new(OsStore::new()), dir)
    }

    pub fn with_backend(os: Box<dyn CredentialBackend>, dir: impl Into<PathBuf>) -> Keys {
        Keys {
            os,
            dir: dir.into(),
            memory: Mutex::new(HashMap::new()),
        }
    }

    fn file(&self, provider: Provider) -> PathBuf {
        self.dir.join(format!("{}.key", provider.id()))
    }

    /// Saves `key` in the OS store and removes any fallback copy. On
    /// [`KeyError::StoreUnavailable`] nothing is kept: offer [`Keys::save_fallback`].
    pub fn save(&self, provider: Provider, key: &ApiKey) -> Result<(), KeyError> {
        match self.os.set(provider.id(), key.expose()) {
            Ok(()) => {
                self.forget_fallbacks(provider)?;
                Ok(())
            }
            Err(StoreError::NoEntry) => Err(KeyError::StoreUnavailable("no entry".into())),
            Err(StoreError::Failure(why)) => {
                log::warn!("vfs-ai: credential store refused the {provider} key: {why}");
                Err(KeyError::StoreUnavailable(why))
            }
        }
    }

    /// Keeps `key` where the user chose after the OS store failed (R2).
    pub fn save_fallback(
        &self,
        provider: Provider,
        key: &ApiKey,
        fallback: Fallback,
    ) -> Result<(), KeyError> {
        self.forget_fallbacks(provider)?;
        match fallback {
            Fallback::ProtectedFile => write_protected(&self.file(provider), key),
            Fallback::SessionOnly => {
                self.memory_lock().insert(provider, key.clone());
                Ok(())
            }
        }
    }

    /// The saved key, from the OS store, the protected file or session memory, in that order.
    /// `Ok(None)` when none is saved; [`KeyError::Unreadable`] when one exists but cannot be
    /// read (R3).
    pub fn load(&self, provider: Provider) -> Result<Option<ApiKey>, KeyError> {
        Ok(self.load_with_location(provider)?.map(|(key, _)| key))
    }

    /// Where the key is kept, if any.
    pub fn location(&self, provider: Provider) -> Result<Option<KeyLocation>, KeyError> {
        Ok(self.load_with_location(provider)?.map(|(_, at)| at))
    }

    /// The last 4 characters of the saved key, for «Clave guardada · termina en …» (R1).
    pub fn hint(&self, provider: Provider) -> Result<Option<String>, KeyError> {
        Ok(self.load(provider)?.map(|k| k.last4()))
    }

    fn load_with_location(
        &self,
        provider: Provider,
    ) -> Result<Option<(ApiKey, KeyLocation)>, KeyError> {
        let os_failure = match self.os.get(provider.id()) {
            Ok(text) => {
                let key = ApiKey::new(text)
                    .map_err(|_| KeyError::Unreadable("stored text is not a key".into()))?;
                return Ok(Some((key, KeyLocation::CredentialStore)));
            }
            Err(StoreError::NoEntry) => None,
            Err(StoreError::Failure(why)) => Some(why),
        };
        let file = self.file(provider);
        if file.exists() {
            let key = read_protected(&file)?;
            return Ok(Some((key, KeyLocation::ProtectedFile)));
        }
        if let Some(key) = self.memory_lock().get(&provider) {
            return Ok(Some((key.clone(), KeyLocation::SessionOnly)));
        }
        match os_failure {
            Some(why) => Err(KeyError::Unreadable(why)),
            None => Ok(None),
        }
    }

    /// Deletes the key everywhere it may be kept («Borrar», R1).
    pub fn delete(&self, provider: Provider) -> Result<(), KeyError> {
        self.forget_fallbacks(provider)?;
        self.os.delete(provider.id()).map_err(|e| match e {
            StoreError::NoEntry => KeyError::Unreadable("no entry".into()),
            StoreError::Failure(why) => KeyError::Unreadable(why),
        })
    }

    fn forget_fallbacks(&self, provider: Provider) -> Result<(), KeyError> {
        self.memory_lock().remove(&provider);
        match fs::remove_file(self.file(provider)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(KeyError::File(e.kind().to_string()))
            }
            _ => Ok(()),
        }
    }

    fn memory_lock(&self) -> std::sync::MutexGuard<'_, HashMap<Provider, ApiKey>> {
        self.memory.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Writes the key to `path` readable only by this user: a DPAPI blob on Windows, a mode 0600
/// file elsewhere (R2). Written to a temporary file first, then renamed over.
fn write_protected(path: &Path, key: &ApiKey) -> Result<(), KeyError> {
    let io = |e: std::io::Error| KeyError::File(e.kind().to_string());
    let dir = path
        .parent()
        .ok_or_else(|| KeyError::File("no folder".into()))?;
    fs::create_dir_all(dir).map_err(io)?;
    let bytes = protect(key.expose().as_bytes())?;
    let tmp = path.with_extension("key.tmp");
    let _ = fs::remove_file(&tmp);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(io)?;
    file.write_all(&bytes).map_err(io)?;
    file.sync_all().map_err(io)?;
    drop(file);
    fs::rename(&tmp, path).map_err(io)
}

fn read_protected(path: &Path) -> Result<ApiKey, KeyError> {
    let bytes = fs::read(path).map_err(|e| KeyError::Unreadable(e.kind().to_string()))?;
    let plain = unprotect(&bytes)?;
    let text = String::from_utf8(plain)
        .map_err(|_| KeyError::Unreadable("key file is not text".into()))?;
    ApiKey::new(text).map_err(|_| KeyError::Unreadable("key file is not a key".into()))
}

#[cfg(not(windows))]
fn protect(plain: &[u8]) -> Result<Vec<u8>, KeyError> {
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
fn unprotect(stored: &[u8]) -> Result<Vec<u8>, KeyError> {
    Ok(stored.to_vec())
}

#[cfg(windows)]
use dpapi::{protect, unprotect};

/// DPAPI for the protected file on Windows: only this user, on this PC, can decrypt it.
#[cfg(windows)]
pub(crate) mod dpapi {
    use super::KeyError;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn run(input: &[u8], encrypt: bool) -> Result<Vec<u8>, KeyError> {
        let len = u32::try_from(input.len()).map_err(|_| KeyError::File("too long".into()))?;
        let blob_in = CRYPT_INTEGER_BLOB {
            cbData: len,
            pbData: input.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: the input blob points at `input`, alive for the call; the output buffer is
        // allocated by the system, copied, then freed with LocalFree as documented.
        let ok = unsafe {
            if encrypt {
                CryptProtectData(
                    &blob_in,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            } else {
                CryptUnprotectData(
                    &blob_in,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            }
        };
        if ok == 0 || out.pbData.is_null() {
            let why = std::io::Error::last_os_error().kind().to_string();
            return Err(if encrypt {
                KeyError::File(why)
            } else {
                KeyError::Unreadable(why)
            });
        }
        // SAFETY: on success `out` describes `cbData` bytes owned by us until LocalFree.
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        unsafe { LocalFree(out.pbData.cast()) };
        Ok(bytes)
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, KeyError> {
        run(plain, true)
    }

    pub fn unprotect(stored: &[u8]) -> Result<Vec<u8>, KeyError> {
        run(stored, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An OS store that refuses everything, as when the keychain is locked.
    struct Broken;
    impl CredentialBackend for Broken {
        fn set(&self, _: &str, _: &str) -> Result<(), StoreError> {
            Err(StoreError::Failure("no storage access".into()))
        }
        fn get(&self, _: &str) -> Result<String, StoreError> {
            Err(StoreError::NoEntry)
        }
        fn delete(&self, _: &str) -> Result<(), StoreError> {
            Ok(())
        }
    }

    /// VFS-28 AC-2 (R2): when the store fails, the key goes to a file only this user can read:
    /// mode 0600 on macOS; on Windows a DPAPI blob without the key's bytes that
    /// `CryptUnprotectData` turns back into the key.
    #[test]
    fn failed_store_offers_a_protected_file() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::with_backend(Box::new(Broken), dir.path().join("ai-keys"));
        let key = ApiKey::new("sk-ant-fallback-file-7777").unwrap();
        assert!(matches!(
            keys.save(Provider::Anthropic, &key),
            Err(KeyError::StoreUnavailable(_))
        ));
        assert_eq!(
            keys.load(Provider::Anthropic).unwrap().map(|k| k.last4()),
            None
        );

        keys.save_fallback(Provider::Anthropic, &key, Fallback::ProtectedFile)
            .unwrap();
        let file = dir.path().join("ai-keys").join("anthropic.key");
        let bytes = fs::read(&file).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        #[cfg(windows)]
        {
            let needle = key.expose().as_bytes();
            assert!(
                !bytes.windows(needle.len()).any(|w| w == needle),
                "file holds the key"
            );
            assert_eq!(dpapi::unprotect(&bytes).unwrap(), needle);
        }
        let _ = bytes;
        assert_eq!(
            keys.location(Provider::Anthropic).unwrap(),
            Some(KeyLocation::ProtectedFile)
        );
        assert_eq!(
            keys.hint(Provider::Anthropic).unwrap().as_deref(),
            Some("7777")
        );

        keys.delete(Provider::Anthropic).unwrap();
        assert!(!file.exists());
        assert_eq!(keys.location(Provider::Anthropic).unwrap(), None);
    }

    /// VFS-28 AC-2 (R2): the session-only choice keeps the key in memory, gone with the process.
    #[test]
    fn session_only_key_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::with_backend(Box::new(Broken), dir.path().join("ai-keys"));
        let key = ApiKey::new("sk-proj-memory-only-5555").unwrap();
        keys.save_fallback(Provider::OpenAi, &key, Fallback::SessionOnly)
            .unwrap();
        assert_eq!(
            keys.location(Provider::OpenAi).unwrap(),
            Some(KeyLocation::SessionOnly)
        );
        assert!(!dir.path().join("ai-keys").exists());
        let fresh = Keys::with_backend(Box::new(Broken), dir.path().join("ai-keys"));
        assert_eq!(fresh.location(Provider::OpenAi).unwrap(), None);
    }

    /// An OS store that holds a credential it cannot read (R3).
    struct Damaged;
    impl CredentialBackend for Damaged {
        fn set(&self, _: &str, _: &str) -> Result<(), StoreError> {
            Ok(())
        }
        fn get(&self, _: &str) -> Result<String, StoreError> {
            Err(StoreError::Failure("no storage access".into()))
        }
        fn delete(&self, _: &str) -> Result<(), StoreError> {
            Ok(())
        }
    }

    /// VFS-28 R3: a saved key that cannot be read is reported as unreadable, not as absent.
    #[test]
    fn unreadable_key_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::with_backend(Box::new(Damaged), dir.path());
        assert!(matches!(
            keys.load(Provider::Anthropic),
            Err(KeyError::Unreadable(_))
        ));
    }
}
