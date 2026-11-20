// Backend bridge (VFS-1 spec, "Tauri commands and events"). In the app every call goes to the Tauri commands through
// `window.__TAURI__` (withGlobalTauri). A rejected call yields an ApiError {code, message}.
//
// Payload shapes (camelCase structs, snake_case enums; ids are numbers, times unix seconds;
// the enum values are the sets of contract.js):
//   SearchQuery    {text, filters: {kinds: FileKind[], modifiedFrom, modifiedTo, sizeMin, sizeMax, folder}, limit, offset}
//                  (the order travels in search_sources' options.sort; VFS-12)
//   SearchResponse {hits: SearchHit[], total, truncated, truncatedBy: TruncatedBy|null, elapsedMs, approximateOrder}
//   SearchHit      {id, name, dir, kind: FileKind, size, modified, matched: MatchKind, isDir,
//                   copies?: FileCopy[],  (VFS-19; left out when empty, never on folders)
//                   link?: {kind: LinkKind, target: string|null, openable}}  (VFS-18)
//   FileCopy       {id, dir, modified}    (another place of the same file; id is a files.id)
//   Root           {id, path, status: RootStatus, files, dirs, lastFullScan, drive: DriveKind}
//   IndexProgress  {state: IndexerState, filesIndexed, roots: RootProgress[]}
//   RootProgress   {rootId, path, status, dirsDone, dirsPending, filesIndexed, currentDir}
//   (VFS-6) search_sources(query, options: {sources: Source[], bookmarkFolder, sort: SortOrder})
//     -> {hits: (SearchHit & {source: 'files'} | BookmarkHit & {source: 'bookmarks'})[], total,
//     truncated, truncatedBy, elapsedMs, approximateOrder}
//   BookmarkHit    {id, isFolder, title, url, root: BookmarkRoot, folder: string[], profile, added,
//                   matched, openable}
//   BookmarkStatus {enabled, state: ReadState, count, profiles, lastRead, roots: {id, root, profile}[]}
//
// Development-only fake backend (VFS-1 U5). It lives in `apps/desktop/ui-dev/`, outside the
// bundled `ui/` folder (frontendDist), so it is not in the packaged app at all. On top of that
// the dynamic import below runs only when ALL of these hold:
//   1. there is no Tauri runtime (`__TAURI__` and `__TAURI_INTERNALS__` are both absent);
//   2. the page is served over plain http from exactly `localhost` or `127.0.0.1` (the app is
//      served from `tauri://localhost` on macOS and `http://tauri.localhost` on Windows);
//   3. the URL carries `?fake`, which nothing in the app ever adds.
// Dev use: serve `apps/desktop/` and open http://localhost:<port>/ui/?fake

import { UiErrorCode } from './contract.js';

function tauriBackend(t) {
  const { invoke } = t.core;
  const { listen } = t.event;
  return {
    isFake: false,
    // VFS-22 R2: the OS locale (e.g. "es-ES"); WKWebView's navigator.language is not it.
    systemLocale: () => invoke('system_locale'),
    setLanguage: (lang) => invoke('set_language', { lang }),
    listRoots: () => invoke('list_roots'),
    pickFolder: () => invoke('pick_folder'),
    addRoot: (path) => invoke('add_root', { path }),
    removeRoot: (id) => invoke('remove_root', { id }),
    search: (query) => invoke('search', { query }),
    // File and folder ids are separate id spaces, so the shell needs `isDir` (-> is_dir).
    reveal: (id, isDir) => invoke('reveal', { id, isDir }),
    revealTarget: (id) => invoke('reveal_target', { id }),
    openLinkUrl: (id) => invoke('open_link_url', { id }),
    getProgress: () => invoke('get_progress'),
    rescan: (id) => invoke('rescan', { id }),
    setPaused: (paused) => invoke('set_paused', { paused }),
    searchSources: (query, options) => invoke('search_sources', { query, options }),
    getBookmarksStatus: () => invoke('get_bookmarks_status'),
    setBookmarksEnabled: (enabled) => invoke('set_bookmarks_enabled', { enabled }),
    openBookmark: (id) => invoke('open_bookmark', { id }),
    onBookmarksChanged: (cb) => listen('bookmarks-changed', (e) => cb(e.payload)),
    onProgress: (cb) => listen('index-progress', (e) => cb(e.payload)),
    onIndexChanged: (cb) => listen('index-changed', () => cb()),
  };
}

function fakeAllowed() {
  return !('__TAURI__' in window) && !('__TAURI_INTERNALS__' in window)
    && location.protocol === 'http:'
    && (location.hostname === 'localhost' || location.hostname === '127.0.0.1')
    && new URLSearchParams(location.search).has('fake');
}

function missingBackend() {
  const fail = () => Promise.reject({ code: UiErrorCode.BACKEND_MISSING, message: 'no Tauri runtime' });
  return {
    isFake: false,
    missing: true,
    systemLocale: async () => navigator.language,
    setLanguage: async () => {},
    listRoots: fail, pickFolder: fail, addRoot: fail, removeRoot: fail, search: fail,
    reveal: fail, revealTarget: fail, openLinkUrl: fail, getProgress: fail, rescan: fail, setPaused: fail,
    searchSources: fail, getBookmarksStatus: fail, setBookmarksEnabled: fail, openBookmark: fail,
    onProgress: async () => () => {}, onIndexChanged: async () => () => {},
    onBookmarksChanged: async () => () => {},
  };
}

async function load() {
  if (window.__TAURI__?.core) return tauriBackend(window.__TAURI__);
  if (fakeAllowed()) {
    const { createFakeBackend } = await import('../ui-dev/fake-backend.js');
    return createFakeBackend(new URLSearchParams(location.search));
  }
  return missingBackend();
}

export const api = await load();
