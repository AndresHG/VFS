//! IPC tests of the desktop commands on Tauri's mock runtime (VFS-1 X1): the
//! frontend's exact `invoke` payloads go through the real command handler and managed state.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::MockRuntime;
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, WebviewWindow, WebviewWindowBuilder};
use vfs_core::Error;
use vfs_desktop_lib::i18n::{self, Lang};
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

/// VFS-1 AC-5: the progress command answers through IPC with the contract's camelCase
/// shape, and failures reject with an `ApiError` code.
#[test]
fn progress_and_errors_over_ipc() {
    let data = tempfile::tempdir().unwrap();
    let (app, webview) = mock_app();
    let vfs = vfs_core::Vfs::open(&data.path().join(INDEX_FILE)).unwrap();
    app.manage(AppState { vfs });

    let progress = invoke(&webview, "get_progress", json!({})).unwrap();
    assert_eq!(progress["state"], json!("idle"));
    assert_eq!(progress["filesIndexed"], json!(0));

    let err = invoke(&webview, "rescan", json!({ "id": null })).unwrap_err();
    assert_eq!(err["code"], json!(Error::Unavailable(String::new()).code()));

    let err = invoke(&webview, "reveal", json!({ "id": 999, "isDir": false })).unwrap_err();
    assert_eq!(err["code"], json!(Error::NotFound(String::new()).code()));
}

/// VFS-22 R2: the UI reads the OS locale through `system_locale`, a string (empty when the
/// OS does not say), with no index needed.
#[test]
fn system_locale_over_ipc() {
    let (_app, webview) = mock_app();
    let locale = invoke(&webview, "system_locale", json!({})).unwrap();
    assert!(locale.is_string(), "{locale}");
}

/// VFS-22 R5: the UI hands its language to the shell, which answers with nothing and ignores
/// a code that is not a language. Off macOS a real language is applied too; on macOS that
/// builds the app menu, which AppKit allows only on the main thread the mock runtime lacks.
#[test]
fn set_language_over_ipc() {
    let (_app, webview) = mock_app();
    let langs: &[&str] = if cfg!(target_os = "macos") {
        &["fr"]
    } else {
        &["en", "es", "fr"]
    };
    for lang in langs {
        let reply = invoke(&webview, "set_language", json!({ "lang": lang })).unwrap();
        assert!(reply.is_null(), "{lang}: {reply}");
    }
}

/// Every non-empty label of the app menu built in `lang`: submenus and their items.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn menu_labels(app: &App<MockRuntime>, lang: Lang) -> Vec<String> {
    let menu = i18n::menu(app.handle(), lang).expect("menu");
    let mut out = Vec::new();
    for kind in menu.items().unwrap() {
        let sub = kind.as_submenu().expect("top level is submenus");
        out.push(sub.text().unwrap());
        for item in sub.items().unwrap() {
            if let Some(p) = item.as_predefined_menuitem() {
                out.push(p.text().unwrap());
            }
        }
    }
    out.retain(|l| !l.is_empty());
    out
}

/// VFS-22 AC-6: the app menu is built in the language asked for: its submenus and items carry
/// that language's words, with the product name filled in. Off macOS only: AppKit builds menus
/// only on the main thread, which the mock runtime lacks.
#[cfg(not(target_os = "macos"))]
#[test]
fn menu_is_built_in_the_language() {
    let (app, _webview) = mock_app();
    let name = app.package_info().name.clone();
    for lang in [Lang::Es, Lang::En] {
        let m = &lang.strings().menu;
        let labels = menu_labels(&app, lang);
        for want in [m.edit, m.view, m.window, m.copy, m.paste, m.close_window] {
            assert!(
                labels.iter().any(|l| l == want),
                "{lang:?}: {want} in {labels:?}"
            );
        }
        let quit = m.quit.replace("{app}", &name);
        assert!(labels.contains(&quit), "{lang:?}: {quit} in {labels:?}");
        assert!(labels.iter().all(|l| !l.contains("{app}")), "{labels:?}");
    }
    assert_ne!(menu_labels(&app, Lang::Es), menu_labels(&app, Lang::En));
}

fn write_file(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"x").unwrap();
}

/// VFS-1 AC-4 + AC-5 + AC-10 (X1): add_root -> search -> get_progress end to end
/// over IPC, with the real indexer thread on a temporary tree.
#[test]
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
    assert_eq!(
        added["drive"],
        json!("local"),
        "a temp folder is on the system disk"
    );
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

/// VFS-6 AC-8, AC-11, AC-12 over IPC: with the setting off `search_sources` returns files only
/// and the status says so; once on and read, `["bookmarks"]` returns bookmark results with
/// the contract's shape; `open_bookmark` refuses an unknown id and a non-web address.
#[test]
fn bookmark_commands_over_ipc() {
    use rusqlite::Connection;
    use std::sync::Arc;
    use vfs_bookmarks::testing::{FakeFirefox, Places, MENU};
    use vfs_bookmarks::Bookmarks;
    use vfs_desktop_lib::bookmarks::BookmarkState;

    let data = tempfile::tempdir().unwrap();
    let (app, webview) = mock_app();
    let index = data.path().join(INDEX_FILE);
    let vfs = vfs_core::Vfs::open(&index).unwrap();
    let root = data.path().join("datos");
    write_file(&root.join("Recetas.pdf"));
    vfs.add_root(&root).unwrap();
    vfs.scan_to_completion(&std::sync::atomic::AtomicBool::new(false))
        .unwrap();
    app.manage(AppState { vfs });

    let mut ff = FakeFirefox::new(&data.path().join("Firefox"));
    let profile = ff.add_profile("default", "p.default");
    {
        let conn = Connection::open(profile.join("places.sqlite")).unwrap();
        let p = Places { conn: &conn };
        p.bookmark(MENU, Some("Recetas de la abuela"), "https://r.es/", 10);
        p.bookmark(MENU, Some("Recetas locales"), "file:///r.pdf", 10);
    }
    let bm =
        Arc::new(Bookmarks::open(&vfs_bookmarks::bookmarks_db_path(&index), ff.locator()).unwrap());
    app.manage(BookmarkState::new(Arc::clone(&bm)));

    let query = json!({ "text": "recetas", "filters": {}, "limit": 50, "offset": 0 });
    let st = invoke(&webview, "get_bookmarks_status", json!({})).unwrap();
    assert_eq!(
        (st["enabled"].clone(), st["state"].clone()),
        (json!(false), json!("never"))
    );
    let r = invoke(
        &webview,
        "search_sources",
        json!({ "query": query, "options": { "sources": [] } }),
    )
    .unwrap();
    let hits = r["hits"].as_array().unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h["source"] == json!("files")));

    let st = invoke(
        &webview,
        "set_bookmarks_enabled",
        json!({ "enabled": true }),
    )
    .unwrap();
    assert_eq!(st["enabled"], json!(true));
    bm.refresh().unwrap();
    let r = invoke(
        &webview,
        "search_sources",
        json!({ "query": query, "options": { "sources": ["bookmarks"], "sort": "relevance" } }),
    )
    .unwrap();
    let hits = r["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|h| h["isFolder"] == json!(false)));
    let web = hits.iter().find(|h| h["openable"] == json!(true)).unwrap();
    assert_eq!(web["source"], json!("bookmarks"));
    assert_eq!(web["root"], json!("menu"));
    assert_eq!(web["url"], json!("https://r.es/"));
    assert!(web["folder"].as_array().unwrap().is_empty());
    let local = hits.iter().find(|h| h["openable"] == json!(false)).unwrap();
    let err = invoke(&webview, "open_bookmark", json!({ "id": local["id"] })).unwrap_err();
    assert_eq!(err["code"], json!(Error::Unavailable(String::new()).code()));
    let err = invoke(&webview, "open_bookmark", json!({ "id": 987654 })).unwrap_err();
    assert_eq!(err["code"], json!(Error::NotFound(String::new()).code()));

    let st = invoke(
        &webview,
        "set_bookmarks_enabled",
        json!({ "enabled": false }),
    )
    .unwrap();
    assert_eq!(st["count"], json!(0));
}

/// VFS-21 AC-3 + AC-4: the update setting round-trips over IPC, and with it off the check
/// answers "no update" without building an updater (the mock app has no updater plugin, so
/// building one would fail the call).
#[test]
fn update_check_off_makes_no_connection() {
    use vfs_desktop_lib::updates::UpdateState;
    let data = tempfile::tempdir().unwrap();
    let (app, webview) = mock_app();
    app.manage(UpdateState::new(data.path()));

    let settings = invoke(&webview, "get_update_settings", json!({})).unwrap();
    assert_eq!(settings, json!({ "checkOnOpen": true }));

    let settings = invoke(&webview, "set_update_check", json!({ "enabled": false })).unwrap();
    assert_eq!(settings, json!({ "checkOnOpen": false }));
    let settings = invoke(&webview, "get_update_settings", json!({})).unwrap();
    assert_eq!(settings, json!({ "checkOnOpen": false }));

    assert_eq!(
        invoke(&webview, "check_for_update", json!({})).unwrap(),
        Value::Null
    );

    let err = invoke(&webview, "install_update", json!({})).unwrap_err();
    assert_eq!(err["code"], json!(Error::NotFound(String::new()).code()));
}
