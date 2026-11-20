//! IPC tests of the desktop commands on Tauri's mock runtime (VFS-1 addendum X1): the
//! frontend's exact `invoke` payloads go through the real command handler and managed state.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::MockRuntime;
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, WebviewWindow, WebviewWindowBuilder};
use vfs_desktop_lib::{commands, init_core, AppState, INDEX_FILE};

fn mock_app() -> (App<MockRuntime>, WebviewWindow<MockRuntime>) {
    let app = mock_builder()
        .invoke_handler(commands::handler())
        .build(mock_context(noop_assets()))
        .expect("mock app");
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview");
    (app, webview)
}

/// Origin of the bundled frontend; only a local origin may call app commands.
#[cfg(windows)]
const APP_ORIGIN: &str = "http://tauri.localhost";
#[cfg(not(windows))]
const APP_ORIGIN: &str = "tauri://localhost";

/// Sends `cmd` with `args` as the frontend's `invoke(cmd, args)` would; `Err` is the
/// rejection value (`ApiError`).
fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(
        webview,
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
    .map(|body| body.deserialize::<Value>().expect("json response"))
}

/// VFS-1 AC-5: stats and progress commands answer through IPC with the contract's
/// camelCase shapes, and an unknown root id rejects with an `ApiError` code.
#[test]
fn stats_and_progress_over_ipc() {
    let data = tempfile::tempdir().unwrap();
    let (app, webview) = mock_app();
    let vfs = vfs_core::Vfs::open(&data.path().join(INDEX_FILE)).unwrap();
    app.manage(AppState { vfs });

    let stats = invoke(&webview, "get_stats", json!({})).unwrap();
    assert_eq!(stats["files"], json!(0));
    assert_eq!(stats["roots"], json!(0));
    assert!(stats["schemaVersion"].as_u64().unwrap() >= 1);

    let progress = invoke(&webview, "get_progress", json!({})).unwrap();
    assert_eq!(progress["state"], json!("idle"));
    assert_eq!(progress["filesIndexed"], json!(0));

    let err = invoke(&webview, "rescan", json!({ "id": null })).unwrap_err();
    assert_eq!(err["code"], json!("unavailable"));

    let err = invoke(&webview, "reveal", json!({ "id": 999, "isDir": false })).unwrap_err();
    assert_eq!(err["code"], json!("not_found"));
}

fn write_file(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"x").unwrap();
}

/// VFS-1 AC-4 + AC-5 + AC-10 (addendum X1): add_root -> search -> get_progress end to end
/// over IPC, with the real indexer thread on a temporary tree.
///
/// ENABLE AFTER MERGE: needs the core roots, search and indexer (T2a/T3a). On the skeleton
/// branch those are `todo!` or `Unavailable`, so the test is ignored there. The lead removes
/// the `#[ignore]` once they are merged; it must then pass on macOS and Windows CI.
#[test]
#[ignore = "enable after T2a/T3a are merged (core roots/search/indexer are stubs)"]
fn add_root_search_progress_end_to_end() {
    let data = tempfile::tempdir().unwrap();
    let tree = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tree.path()).unwrap();
    write_file(
        &root
            .join("Gestoría")
            .join("Año 2019")
            .join("Notaría Pérez escritura.pdf"),
    );
    write_file(&root.join("Fotos").join("IMG_0001.jpg"));
    std::fs::create_dir_all(root.join("Vacía")).unwrap();

    let (app, webview) = mock_app();
    init_core(app.handle(), &data.path().join("app").join(INDEX_FILE)).expect("init core");

    let added = invoke(
        &webview,
        "add_root",
        json!({ "path": root.to_string_lossy() }),
    )
    .expect("add_root");
    assert_eq!(added["path"], json!(root.to_string_lossy()));
    let root_id = added["id"].as_i64().unwrap();

    let roots = invoke(&webview, "list_roots", json!({})).unwrap();
    assert_eq!(roots.as_array().unwrap().len(), 1);

    let query = json!({ "query": { "text": "notaria", "filters": { "kinds": ["document"] } } });
    let deadline = Instant::now() + Duration::from_secs(30);
    let hit = loop {
        let res = invoke(&webview, "search", query.clone()).expect("search");
        if let Some(hit) = res["hits"].as_array().and_then(|h| h.first()).cloned() {
            break hit;
        }
        assert!(
            Instant::now() < deadline,
            "file not indexed within 30 s: {res}"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(hit["name"], json!("Notaría Pérez escritura.pdf"));
    assert_eq!(hit["kind"], json!("document"));
    assert_eq!(hit["isDir"], json!(false));
    assert!(hit["dir"].as_str().unwrap().ends_with("Año 2019"));

    let progress = invoke(&webview, "get_progress", json!({})).unwrap();
    assert!(["idle", "scanning", "paused"].contains(&progress["state"].as_str().unwrap()));
    let roots = progress["roots"].as_array().unwrap();
    assert!(roots.iter().any(|r| r["rootId"] == json!(root_id)));

    invoke(&webview, "set_paused", json!({ "paused": true })).unwrap();
    invoke(&webview, "set_paused", json!({ "paused": false })).unwrap();
    invoke(&webview, "remove_root", json!({ "id": root_id })).unwrap();

    app.state::<AppState>().vfs.shutdown();
}
