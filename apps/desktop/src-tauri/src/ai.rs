//! «Preguntar a sus documentos (IA)» (VFS-28), PR 28d: the settings, the key, the monthly cap
//! and the record of what was sent, as app commands. The key goes from the web UI to Rust once,
//! when typed, and never back (R2): no command returns it, and only its last 4 characters are
//! shown.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::State;
use vfs_ai::{
    chat_model, AiDb, AiError, ApiKey, CallKind, Endpoint, Fallback, KeyError, KeyLocation, Keys,
    Provider,
};
use vfs_core::api::ApiError;

/// The AI settings, apart from the index; never the key (R2).
pub const SETTINGS_FILE: &str = "ai.json";
/// The record of what was sent and the monthly totals (R11).
pub const DB_FILE: &str = "ai.db";
/// Protected key files, when the OS store fails (R2).
pub const KEYS_DIR: &str = "ai-keys";

/// What the user set in Ajustes (R1). The cap lives in `ai.db` with the totals it limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiSettings {
    pub enabled: bool,
    pub provider: Provider,
    /// «Respuestas más baratas» (D7).
    pub cheap: bool,
    /// The one-time notice before the first question was accepted (R6).
    pub notice_seen: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            enabled: false,
            provider: Provider::Anthropic,
            cheap: false,
            notice_seen: false,
        }
    }
}

fn load_settings(path: &Path) -> AiSettings {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_settings(path: &Path, settings: &AiSettings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec(settings).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, path)
}

/// Error codes of the AI commands (`ApiError.code`), mirrored in `ui/contract.js`. Messages
/// never carry the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiErrorCode {
    /// R13: «La clave no es válida».
    InvalidKey,
    NoCredit,
    SpendLimit,
    Saturated,
    ModelGone,
    Offline,
    Cancelled,
    /// Any other provider error; the message is the provider's.
    Provider,
    BadResponse,
    /// The text typed is not a plausible key.
    KeyInvalid,
    /// No key is saved for the provider.
    NoKey,
    /// R3: «No se puede leer la clave guardada; vuelva a introducirla».
    KeyUnreadable,
    /// R2: the OS store refused the key; offer the protected file or session memory.
    StoreUnavailable,
    /// The record or the settings could not be read or written.
    Storage,
}

impl AiErrorCode {
    pub fn all() -> Vec<AiErrorCode> {
        use AiErrorCode::*;
        vec![
            InvalidKey,
            NoCredit,
            SpendLimit,
            Saturated,
            ModelGone,
            Offline,
            Cancelled,
            Provider,
            BadResponse,
            KeyInvalid,
            NoKey,
            KeyUnreadable,
            StoreUnavailable,
            Storage,
        ]
    }

    fn id(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    }
}

fn api_error(code: AiErrorCode, message: impl Into<String>) -> ApiError {
    ApiError {
        code: code.id(),
        message: message.into(),
    }
}

impl From<&AiError> for AiErrorCode {
    fn from(e: &AiError) -> AiErrorCode {
        match e {
            AiError::InvalidKey => AiErrorCode::InvalidKey,
            AiError::NoCredit => AiErrorCode::NoCredit,
            AiError::SpendLimit => AiErrorCode::SpendLimit,
            AiError::Saturated => AiErrorCode::Saturated,
            AiError::ModelGone => AiErrorCode::ModelGone,
            AiError::Offline => AiErrorCode::Offline,
            AiError::Cancelled { .. } => AiErrorCode::Cancelled,
            AiError::Provider { .. } => AiErrorCode::Provider,
            AiError::BadResponse(_) => AiErrorCode::BadResponse,
        }
    }
}

pub(crate) fn ai_error(e: AiError) -> ApiError {
    let message = match &e {
        AiError::Provider { message, .. } => message.clone(),
        other => other.to_string(),
    };
    api_error(AiErrorCode::from(&e), message)
}

fn key_error(e: KeyError) -> ApiError {
    let code = match e {
        KeyError::Invalid => AiErrorCode::KeyInvalid,
        KeyError::StoreUnavailable(_) => AiErrorCode::StoreUnavailable,
        KeyError::Unreadable(_) => AiErrorCode::KeyUnreadable,
        KeyError::File(_) => AiErrorCode::Storage,
    };
    api_error(code, e.to_string())
}

fn storage_error(e: impl std::fmt::Display) -> ApiError {
    api_error(AiErrorCode::Storage, e.to_string())
}

/// Where a saved key is kept, as the UI names it (R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyPlace {
    CredentialStore,
    ProtectedFile,
    SessionOnly,
}

impl KeyPlace {
    pub fn all() -> Vec<KeyPlace> {
        vec![
            KeyPlace::CredentialStore,
            KeyPlace::ProtectedFile,
            KeyPlace::SessionOnly,
        ]
    }
}

impl From<KeyLocation> for KeyPlace {
    fn from(l: KeyLocation) -> KeyPlace {
        match l {
            KeyLocation::CredentialStore => KeyPlace::CredentialStore,
            KeyLocation::ProtectedFile => KeyPlace::ProtectedFile,
            KeyLocation::SessionOnly => KeyPlace::SessionOnly,
        }
    }
}

/// The key of the chosen provider, as Ajustes shows it (R1, R3): never the key itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum KeyState {
    None,
    Saved { last4: String, place: KeyPlace },
    Unreadable,
}

/// The `state` tags of [`KeyState`], for the contract check.
pub fn key_states() -> Vec<String> {
    [
        KeyState::None,
        KeyState::Saved {
            last4: String::new(),
            place: KeyPlace::SessionOnly,
        },
        KeyState::Unreadable,
    ]
    .iter()
    .map(|s| {
        serde_json::to_value(s).unwrap_or_default()["state"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    })
    .collect()
}

/// Everything Ajustes and the search screen need to know about the AI (R1, R5, R10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    pub settings: AiSettings,
    pub key: KeyState,
    pub cap_nano: i64,
    /// This calendar month's spend, local time (R10).
    pub month_nano: i64,
    /// Content search has a store to ask over (R5).
    pub store: bool,
}

/// One passage of the record, exactly as sent (R11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentPassageView {
    pub file_name: String,
    pub page: Option<u32>,
    pub text: String,
}

/// One priced call of the record (R11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallView {
    pub follow_up: bool,
    pub rewrite: bool,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_nano: i64,
}

/// One question or follow-up of «Qué se ha enviado» (R11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordView {
    pub id: i64,
    /// RFC 3339, local offset.
    pub asked_at: String,
    pub provider: Provider,
    pub question: String,
    /// The rewrite's output as returned; null for a follow-up.
    pub rewrite: Option<String>,
    pub passages: Vec<SentPassageView>,
    pub calls: Vec<CallView>,
    pub cost_nano: i64,
}

/// The AI's managed state.
pub struct AiState {
    settings_path: PathBuf,
    db_path: PathBuf,
    index_db: PathBuf,
    keys: Keys,
    /// A provider base URL for tests; `None` is the provider's public API.
    endpoint: Option<String>,
    settings: Mutex<AiSettings>,
    db: Mutex<Option<AiDb>>,
}

impl AiState {
    /// The state for the app data folder `dir`, with keys in the OS store.
    pub fn new(dir: &Path) -> AiState {
        AiState::with(dir, Keys::new(dir.join(KEYS_DIR)), None)
    }

    /// The state with the given keys and, for tests, a provider base URL.
    pub fn with(dir: &Path, keys: Keys, endpoint: Option<String>) -> AiState {
        let settings_path = dir.join(SETTINGS_FILE);
        AiState {
            settings: Mutex::new(load_settings(&settings_path)),
            settings_path,
            db_path: dir.join(DB_FILE),
            index_db: dir.join(crate::INDEX_FILE),
            keys,
            endpoint,
            db: Mutex::new(None),
        }
    }

    pub fn settings(&self) -> AiSettings {
        *self.settings.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn update(&self, f: impl FnOnce(&mut AiSettings)) -> Result<(), ApiError> {
        let mut s = self.settings.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = *s;
        f(&mut next);
        save_settings(&self.settings_path, &next).map_err(storage_error)?;
        *s = next;
        Ok(())
    }

    /// `ai.db`, opened on first use.
    pub(crate) fn db(&self) -> Result<DbGuard<'_>, ApiError> {
        let mut g = self.db.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            *g = Some(AiDb::open(&self.db_path).map_err(storage_error)?);
        }
        Ok(DbGuard(g))
    }

    pub(crate) fn endpoint(&self, provider: Provider) -> Endpoint {
        match &self.endpoint {
            Some(url) => Endpoint::local(url.clone()),
            None => Endpoint::public(provider),
        }
    }

    /// The saved key of `provider`, for a call.
    pub(crate) fn key(&self, provider: Provider) -> Result<ApiKey, ApiError> {
        self.keys
            .load(provider)
            .map_err(key_error)?
            .ok_or_else(|| api_error(AiErrorCode::NoKey, "no key saved"))
    }

    pub fn index_db(&self) -> &Path {
        &self.index_db
    }

    pub fn status(&self) -> Result<AiStatus, ApiError> {
        let settings = self.settings();
        let key = match self.keys.location(settings.provider) {
            Ok(None) => KeyState::None,
            Ok(Some(place)) => KeyState::Saved {
                last4: self
                    .keys
                    .hint(settings.provider)
                    .map_err(key_error)?
                    .unwrap_or_default(),
                place: place.into(),
            },
            Err(KeyError::Unreadable(_)) => KeyState::Unreadable,
            Err(e) => return Err(key_error(e)),
        };
        let now = chrono::Local::now();
        let db = self.db()?;
        let cap = db.get().cap_status(&now).map_err(storage_error)?;
        Ok(AiStatus {
            settings,
            key,
            cap_nano: cap.cap_nano,
            month_nano: cap.spent_nano,
            store: vfs_content::Searcher::open(&self.index_db).is_ok(),
        })
    }
}

/// A lock on the opened `ai.db`.
pub(crate) struct DbGuard<'a>(MutexGuard<'a, Option<AiDb>>);

impl DbGuard<'_> {
    pub(crate) fn get(&self) -> &AiDb {
        self.0.as_ref().expect("opened by AiState::db")
    }

    pub(crate) fn get_mut(&mut self) -> &mut AiDb {
        self.0.as_mut().expect("opened by AiState::db")
    }
}

/// Runs a blocking provider call on its own thread: blocking `reqwest` must not run on the
/// async runtime's threads.
pub(crate) fn off_runtime<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| s.spawn(f).join().expect("provider call panicked"))
}

#[tauri::command(async)]
pub fn ai_status(ai: State<'_, AiState>) -> Result<AiStatus, ApiError> {
    ai.status()
}

/// The switch, the provider and «Respuestas más baratas» (R1).
#[tauri::command(async)]
pub fn ai_set_settings(
    ai: State<'_, AiState>,
    enabled: bool,
    provider: Provider,
    cheap: bool,
) -> Result<AiStatus, ApiError> {
    ai.update(|s| {
        s.enabled = enabled;
        s.provider = provider;
        s.cheap = cheap;
    })?;
    ai.status()
}

/// The first-use notice was accepted («Entendido», R6).
#[tauri::command(async)]
pub fn ai_notice_seen(ai: State<'_, AiState>) -> Result<AiStatus, ApiError> {
    ai.update(|s| s.notice_seen = true)?;
    ai.status()
}

/// Saves the typed key in the OS store (R2). Rejects with `store_unavailable` when the store
/// refuses it: then offer `ai_save_key_fallback`.
#[tauri::command(async)]
pub fn ai_save_key(
    ai: State<'_, AiState>,
    provider: Provider,
    key: String,
) -> Result<AiStatus, ApiError> {
    let key = ApiKey::new(key).map_err(key_error)?;
    ai.keys.save(provider, &key).map_err(key_error)?;
    ai.status()
}

/// Where the user chose to keep the key after the OS store failed (R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyFallback {
    ProtectedFile,
    SessionOnly,
}

impl KeyFallback {
    pub fn all() -> Vec<KeyFallback> {
        vec![KeyFallback::ProtectedFile, KeyFallback::SessionOnly]
    }
}

#[tauri::command(async)]
pub fn ai_save_key_fallback(
    ai: State<'_, AiState>,
    provider: Provider,
    key: String,
    fallback: KeyFallback,
) -> Result<AiStatus, ApiError> {
    let key = ApiKey::new(key).map_err(key_error)?;
    let fallback = match fallback {
        KeyFallback::ProtectedFile => Fallback::ProtectedFile,
        KeyFallback::SessionOnly => Fallback::SessionOnly,
    };
    ai.keys
        .save_fallback(provider, &key, fallback)
        .map_err(key_error)?;
    ai.status()
}

/// «Borrar» (R1): the key is gone from every place it may be kept.
#[tauri::command(async)]
pub fn ai_delete_key(ai: State<'_, AiState>, provider: Provider) -> Result<AiStatus, ApiError> {
    ai.keys.delete(provider).map_err(key_error)?;
    ai.status()
}

/// «Probar» (R4): the provider's model list, which costs no tokens. Resolves when the key is
/// correct; rejects with the R13 code otherwise.
#[tauri::command(async)]
pub fn ai_test_key(ai: State<'_, AiState>, provider: Provider) -> Result<(), ApiError> {
    let key = ai.key(provider)?;
    let endpoint = ai.endpoint(provider);
    off_runtime(move || chat_model(provider, key, endpoint)?.validate_key()).map_err(ai_error)
}

/// The monthly cap, in nano-dollars (R10).
#[tauri::command(async)]
pub fn ai_set_cap(ai: State<'_, AiState>, cap_nano: i64) -> Result<AiStatus, ApiError> {
    ai.db()?
        .get()
        .set_cap(cap_nano.max(0))
        .map_err(storage_error)?;
    ai.status()
}

/// «Qué se ha enviado» (R11), oldest first.
#[tauri::command(async)]
pub fn ai_records(ai: State<'_, AiState>) -> Result<Vec<RecordView>, ApiError> {
    let records = ai.db()?.get().records().map_err(storage_error)?;
    Ok(records
        .into_iter()
        .map(|r| RecordView {
            id: r.id,
            asked_at: r.asked_at,
            provider: r.provider,
            question: r.question,
            rewrite: r.rewrite,
            passages: r
                .passages
                .into_iter()
                .map(|p| SentPassageView {
                    file_name: p.file_name,
                    page: p.page,
                    text: p.text,
                })
                .collect(),
            calls: r
                .calls
                .into_iter()
                .map(|c| CallView {
                    follow_up: c.kind == CallKind::FollowUp,
                    rewrite: c.kind == CallKind::Rewrite,
                    model: c.model,
                    input_tokens: c.usage.input_tokens
                        + c.usage.cache_read_tokens
                        + c.usage.cache_write_tokens,
                    output_tokens: c.usage.output_tokens,
                    cost_nano: c.cost_nano,
                })
                .collect(),
            cost_nano: r.cost_nano,
        })
        .collect())
}

/// «Borrar todo» (R11): the record is emptied; the month's spend still counts for the cap.
#[tauri::command(async)]
pub fn ai_clear_records(ai: State<'_, AiState>) -> Result<(), ApiError> {
    ai.db()?.get_mut().clear_records().map_err(storage_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_off_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        assert_eq!(load_settings(&path), AiSettings::default());
        assert!(!AiSettings::default().enabled);
        let s = AiSettings {
            enabled: true,
            provider: Provider::OpenAi,
            cheap: true,
            notice_seen: true,
        };
        save_settings(&path, &s).unwrap();
        assert_eq!(load_settings(&path), s);
    }
}
