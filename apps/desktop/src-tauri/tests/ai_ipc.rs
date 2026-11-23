//! VFS-28 28d: the AI commands over IPC on Tauri's mock runtime, with a fake credential store
//! and the mock provider. No test needs a real key or the network.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::MockRuntime;
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, WebviewWindow, WebviewWindowBuilder};
use vfs_ai::mock::{MockResponse, MockServer};
use vfs_ai::{CredentialBackend, Keys, StoreError};
use vfs_desktop_lib::ai::{AiState, DB_FILE, SETTINGS_FILE};
use vfs_desktop_lib::commands;

const KEY: &str = "sk-ant-test-NOT-REAL-0123456789wxyz";

#[cfg(windows)]
const APP_ORIGIN: &str = "http://tauri.localhost";
#[cfg(not(windows))]
const APP_ORIGIN: &str = "tauri://localhost";

/// An in-memory credential store; `broken` makes every call fail, as a locked keychain would.
#[derive(Clone, Default)]
struct FakeStore {
    items: Arc<Mutex<HashMap<String, String>>>,
    broken: bool,
}

impl CredentialBackend for FakeStore {
    fn set(&self, account: &str, secret: &str) -> Result<(), StoreError> {
        if self.broken {
            return Err(StoreError::Failure("locked".into()));
        }
        self.items
            .lock()
            .unwrap()
            .insert(account.into(), secret.into());
        Ok(())
    }
    fn get(&self, account: &str) -> Result<String, StoreError> {
        if self.broken {
            return Err(StoreError::NoEntry);
        }
        self.items
            .lock()
            .unwrap()
            .get(account)
            .cloned()
            .ok_or(StoreError::NoEntry)
    }
    fn delete(&self, account: &str) -> Result<(), StoreError> {
        self.items
            .lock()
            .unwrap()
            .remove(account)
            .map(|_| ())
            .ok_or(StoreError::NoEntry)
    }
}

struct Harness {
    _app: App<MockRuntime>,
    webview: WebviewWindow<MockRuntime>,
    dir: tempfile::TempDir,
    store: FakeStore,
}

fn harness(store: FakeStore, endpoint: Option<String>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let app = mock_builder()
        .invoke_handler(commands::handler())
        .build(mock_context(noop_assets()))
        .expect("mock app");
    let keys = Keys::with_backend(Box::new(store.clone()), dir.path().join("ai-keys"));
    app.manage(AiState::with(dir.path(), keys, endpoint));
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview");
    Harness {
        _app: app,
        webview,
        dir,
        store,
    }
}

impl Harness {
    fn invoke(&self, cmd: &str, args: Value) -> Result<Value, Value> {
        let r = get_ipc_response(
            &self.webview,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: APP_ORIGIN.parse().unwrap(),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<Value>().expect("json response"));
        // VFS-28 AC-1: no IPC response, resolved or rejected, carries the key.
        let text = format!("{r:?}");
        assert!(!text.contains(KEY), "{cmd} returned the key: {text}");
        r
    }
}

/// VFS-28 AC-1 (UI part, R1, R2): a saved key reads back as «termina en …» with its last 4
/// characters, no IPC response carries it, it is in neither the settings file nor `ai.db`, and
/// «Borrar» removes it.
#[test]
fn saved_key_shows_only_its_last_four() {
    let h = harness(FakeStore::default(), None);
    let st = h.invoke("ai_status", json!({})).unwrap();
    assert_eq!(st["settings"]["enabled"], false, "AI is off by default");
    assert_eq!(st["key"]["state"], "none");

    h.invoke(
        "ai_set_settings",
        json!({"enabled": true, "provider": "anthropic", "cheap": false}),
    )
    .unwrap();
    let st = h
        .invoke("ai_save_key", json!({"provider": "anthropic", "key": KEY}))
        .unwrap();
    assert_eq!(
        st["key"],
        json!({"state": "saved", "last4": "wxyz", "place": "credential_store"})
    );
    assert_eq!(h.store.items.lock().unwrap()["anthropic"], KEY);
    h.invoke("ai_records", json!({})).unwrap();
    for file in [SETTINGS_FILE, DB_FILE] {
        let bytes = std::fs::read(h.dir.path().join(file)).unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(KEY),
            "key in {file}"
        );
    }

    let st = h
        .invoke("ai_delete_key", json!({"provider": "anthropic"}))
        .unwrap();
    assert_eq!(st["key"]["state"], "none");
    assert!(h.store.items.lock().unwrap().is_empty());
}

/// VFS-28 AC-2 (UI part, R2): when the store refuses the key, saving rejects with
/// `store_unavailable`, and the session-only choice then keeps it in memory.
#[test]
fn refused_store_offers_the_fallbacks() {
    let h = harness(
        FakeStore {
            broken: true,
            ..FakeStore::default()
        },
        None,
    );
    let err = h
        .invoke("ai_save_key", json!({"provider": "openai", "key": KEY}))
        .unwrap_err();
    assert_eq!(err["code"], "store_unavailable");
    h.invoke(
        "ai_set_settings",
        json!({"enabled": true, "provider": "openai", "cheap": false}),
    )
    .unwrap();
    let st = h
        .invoke(
            "ai_save_key_fallback",
            json!({"provider": "openai", "key": KEY, "fallback": "session_only"}),
        )
        .unwrap();
    assert_eq!(st["key"]["place"], "session_only");
}

/// VFS-28 AC-4 (R4): «Probar» resolves for a correct key and rejects with the mapped code for
/// a wrong one; without a key it says so.
#[test]
fn test_key_maps_the_providers_answer() {
    let mock = MockServer::start(vec![
        MockResponse::json(200, r#"{"data":[]}"#),
        MockResponse::json(
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
        ),
    ]);
    let h = harness(FakeStore::default(), Some(mock.url()));
    let err = h
        .invoke("ai_test_key", json!({"provider": "anthropic"}))
        .unwrap_err();
    assert_eq!(err["code"], "no_key");
    h.invoke("ai_save_key", json!({"provider": "anthropic", "key": KEY}))
        .unwrap();
    assert_eq!(
        h.invoke("ai_test_key", json!({"provider": "anthropic"})),
        Ok(Value::Null)
    );
    let err = h
        .invoke("ai_test_key", json!({"provider": "anthropic"}))
        .unwrap_err();
    assert_eq!(err["code"], "invalid_key");
}

/// VFS-28 R10, AC-9 (data over IPC): the cap is set and read back; the record lists and clears.
#[test]
fn cap_and_record_over_ipc() {
    let h = harness(FakeStore::default(), None);
    let st = h
        .invoke("ai_set_cap", json!({"capNano": 10_000_000}))
        .unwrap();
    assert_eq!(st["capNano"], 10_000_000);
    assert_eq!(st["monthNano"], 0);
    assert_eq!(st["store"], false, "no content store in this folder");
    assert_eq!(h.invoke("ai_records", json!({})).unwrap(), json!([]));
    assert_eq!(h.invoke("ai_clear_records", json!({})), Ok(Value::Null));
}
