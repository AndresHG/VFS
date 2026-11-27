// Backend bridge (VFS-1 spec, "Tauri commands and events"). In the app every call goes to the Tauri commands through
// `window.__TAURI__` (withGlobalTauri). A rejected call yields an ApiError {code, message}.
//
// Payload shapes (camelCase structs, snake_case enums; ids are numbers, times unix seconds;
// the enum values are the sets of contract.js):
//   SearchQuery    {text, filters: {kinds: FileKind[], modifiedFrom, modifiedTo, sizeMin, sizeMax, folder}, limit, offset,
//                  scope?: Scope}  (VFS-38; left out = 'all')
//                  (the order travels in search_sources' options.sort; VFS-12)
//   SearchResponse {hits: SearchHit[], total, truncated, truncatedBy: TruncatedBy|null, elapsedMs, approximateOrder}
//   SearchHit      {id, name, dir, kind: FileKind, size, modified, matched: MatchKind, isDir,
//                   copies?: FileCopy[],  (VFS-19; left out when empty, never on folders)
//                   link?: {kind: LinkKind, target: string|null, openable},  (VFS-18)
//                   cloudOnly?: true}  (VFS-25: its data is only in the cloud; left out when false)
//   FileCopy       {id, dir, modified, cloudOnly?}  (another place of the same file; id is a files.id)
//   Root           {id, path, status: RootStatus, files, dirs, lastFullScan, drive: DriveKind,
//                   unreadable, paused, unpopulated, cloudOnly, links}  (VFS-10: folders that
//                   could not be listed; VFS-32; VFS-37: cloudOnly and links null before a full walk)
//   UnreadableDir  {id, path, reason: UnreadableReason}  (id is a dirs.id; list_unreadable)
//   IndexProgress  {state: IndexerState, filesIndexed, roots: RootProgress[]}
//   RootProgress   {rootId, path, status, dirsDone, dirsPending, filesIndexed, currentDir}
//   (VFS-6) search_sources(query, options: {sources: Source[], bookmarkFolder, sort: SortOrder})
//     -> {hits: (SearchHit & {source: 'files'} | BookmarkHit & {source: 'bookmarks'})[], total,
//     truncated, truncatedBy, elapsedMs, approximateOrder}
//   BookmarkHit    {id, isFolder, title, url, root: BookmarkRoot, folder: string[], profile, added,
//                   matched, openable}
//   BookmarkStatus {enabled, state: ReadState, count, profiles, lastRead, roots: {id, root, profile}[]}
//   (VFS-21) UpdateSettings {checkOnOpen}; check_for_update -> {version} | null (null when the
//     setting is off); install_update emits 'update-progress' {downloaded, total|null}
//   (VFS-45) open_project_page(page: ProjectPage) opens a fixed project address in the browser;
//     check_for_update rejects when the check failed
//   (VFS-20, VFS-48) feedback_report(description) -> string;
//     feedback_save(description, github) -> {folder, file}
//   (VFS-27) ContentStatus {settings: {enabled, paused, offered}, phase: {kind: ContentPhase,
//                   reason?: ContentWait|ContentStop, detail?}, done, total, perHour|null,
//                   secondsLeft|null}; the switch, pause, retry, rebuild, mark-offered and delete
//                   commands answer it, and 'content-status' carries it
//   ContentOffer   {documents, seconds: [lo, hi], bytes: [lo, hi], freeBytes|null, lowSpace}
//   content_search(query, filters, offset) -> {hits: ContentHit[], partial, total, totalCapped,
//                   elapsedMs}, 20 hits a page
//   ContentHit     {id, name, dir, kind, size, modified, score, snippet, highlights: [start, end][]
//                   (UTF-8 byte offsets in snippet), passages: {page|null, …}[], title, authors, created}
//   content_unread_counts() -> {byReason: [UnreadReason, n][], pending, storeBytes}
//   content_unread(fileIds) -> [id, UnreadReason|null][]
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
import { gateOnCore } from './coregate.js';

function tauriBackend(t) {
  // VFS-39 R2: index commands wait for the index to open (coregate.js).
  const invoke = gateOnCore(t.core.invoke);
  const { listen } = t.event;
  return {
    isFake: false,
    // VFS-22 R2: the OS locale (e.g. "es-ES"); WKWebView's navigator.language is not it.
    systemLocale: () => invoke('system_locale'),
    setLanguage: (lang) => invoke('set_language', { lang }),
    waitCore: () => invoke('wait_core'),
    listRoots: () => invoke('list_roots'),
    listUnreadable: (rootId) => invoke('list_unreadable', { rootId }),
    pickFolder: () => invoke('pick_folder'),
    addRoot: (path) => invoke('add_root', { path }),
    removeRoot: (id) => invoke('remove_root', { id }),
    search: (query) => invoke('search', { query }),
    suggest: (query) => invoke('suggest', { query }),
    // File and folder ids are separate id spaces, so the shell needs `isDir` (-> is_dir).
    reveal: (id, isDir) => invoke('reveal', { id, isDir }),
    revealTarget: (id) => invoke('reveal_target', { id }),
    openLinkUrl: (id) => invoke('open_link_url', { id }),
    getProgress: () => invoke('get_progress'),
    rescan: (id) => invoke('rescan', { id }),
    setRootPaused: (id, paused) => invoke('set_root_paused', { id, paused }),
    setPaused: (paused) => invoke('set_paused', { paused }),
    // VFS-33: «Incluir carpetas del sistema y de programas» -> boolean.
    getIncludeAllFolders: () => invoke('get_include_all_folders'),
    setIncludeAllFolders: (on) => invoke('set_include_all_folders', { on }),
    // VFS-34: «Filtro de archivos»; the list calls return the patterns, count -> number.
    getFileExclusions: () => invoke('get_file_exclusions'),
    countFileExclusion: (pattern) => invoke('count_file_exclusion', { pattern }),
    addFileExclusion: (pattern) => invoke('add_file_exclusion', { pattern }),
    removeFileExclusion: (pattern) => invoke('remove_file_exclusion', { pattern }),
    resetFileExclusions: () => invoke('reset_file_exclusions'),
    getContentMaxMb: () => invoke('get_content_max_mb'),
    setContentMaxMb: (mb) => invoke('set_content_max_mb', { mb }),
    searchSources: (query, options) => invoke('search_sources', { query, options }),
    getBookmarksStatus: () => invoke('get_bookmarks_status'),
    setBookmarksEnabled: (enabled) => invoke('set_bookmarks_enabled', { enabled }),
    openBookmark: (id) => invoke('open_bookmark', { id }),
    // VFS-20: the report text, and saving it (-> {folder, file}); `github` also opens the issue.
    feedbackReport: (description) => invoke('feedback_report', { description }),
    feedbackSave: (description, github) => invoke('feedback_save', { description, github }),
    onBookmarksChanged: (cb) => listen('bookmarks-changed', (e) => cb(e.payload)),
    getUpdateSettings: () => invoke('get_update_settings'),
    setUpdateCheck: (enabled) => invoke('set_update_check', { enabled }),
    checkForUpdate: () => invoke('check_for_update'),
    // VFS-45: the app version (Tauri core, no index) and the project page, at a fixed address.
    appVersion: () => t.app.getVersion(),
    openProjectPage: (page) => invoke('open_project_page', { page }),
    installUpdate: () => invoke('install_update'),
    onUpdateProgress: (cb) => listen('update-progress', (e) => cb(e.payload)),
    // VFS-27: content search.
    contentStatus: () => invoke('content_status'),
    contentSetEnabled: (on) => invoke('content_set_enabled', { on }),
    contentSetPaused: (paused) => invoke('content_set_paused', { paused }),
    contentRetry: () => invoke('content_retry'),
    contentRebuild: () => invoke('content_rebuild'),
    contentMarkOffered: () => invoke('content_mark_offered'),
    contentOffer: () => invoke('content_offer'),
    contentDeleteStore: () => invoke('content_delete_store'),
    contentSearch: (query, filters, offset) => invoke('content_search', { query, filters, offset }),
    contentUnreadCounts: () => invoke('content_unread_counts'),
    contentUnread: (fileIds) => invoke('content_unread', { fileIds }),
    onContentStatus: (cb) => listen('content-status', (e) => cb(e.payload)),
    // VFS-29: search by meaning.
    meaningStatus: () => invoke('meaning_status'),
    meaningOffer: () => invoke('meaning_offer'),
    meaningSetEnabled: (on) => invoke('meaning_set_enabled', { on }),
    meaningSetPaused: (paused) => invoke('meaning_set_paused', { paused }),
    meaningWarm: () => invoke('meaning_warm'),
    onMeaningStatus: (cb) => listen('meaning-status', (e) => cb(e.payload)),
    // VFS-28: «Preguntar a sus documentos». The key is sent once, when typed; nothing returns it.
    aiStatus: () => invoke('ai_status'),
    aiSetSettings: (enabled, provider, cheap) => invoke('ai_set_settings', { enabled, provider, cheap }),
    aiNoticeSeen: () => invoke('ai_notice_seen'),
    aiSaveKey: (provider, key) => invoke('ai_save_key', { provider, key }),
    aiSaveKeyFallback: (provider, key, fallback) => invoke('ai_save_key_fallback', { provider, key, fallback }),
    aiDeleteKey: (provider) => invoke('ai_delete_key', { provider }),
    aiTestKey: (provider) => invoke('ai_test_key', { provider }),
    aiSetCap: (capNano) => invoke('ai_set_cap', { capNano }),
    aiRecords: () => invoke('ai_records'),
    aiClearRecords: () => invoke('ai_clear_records'),
    // VFS-49: chats kept in ai.db; a message runs the whole ask flow over its chat's scope.
    aiChatSend: (askId, chatId, scope, question, confirmed, lang) => invoke('ai_chat_send', { askId, chatId, scope, question, confirmed, lang }),
    aiChats: () => invoke('ai_chats'),
    aiChat: (chatId) => invoke('ai_chat', { chatId }),
    aiChatRename: (chatId, title) => invoke('ai_chat_rename', { chatId, title }),
    aiChatDelete: (chatId) => invoke('ai_chat_delete', { chatId }),
    aiChatsClear: () => invoke('ai_chats_clear'),
    aiCancel: (askId) => invoke('ai_cancel', { askId }),
    aiChatBibliography: (chatId, ord, all) => invoke('ai_chat_bibliography', { chatId, ord, all }),
    aiChatExportRis: (chatId, ord, all) => invoke('ai_chat_export_ris', { chatId, ord, all }),
    onAnswerText: (cb) => listen('ai-answer-text', (e) => cb(e.payload)),
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
    waitCore: fail, listRoots: fail, listUnreadable: fail, pickFolder: fail, addRoot: fail, removeRoot: fail, search: fail,
    suggest: fail,
    reveal: fail, revealTarget: fail, openLinkUrl: fail, getProgress: fail, rescan: fail, setRootPaused: fail, setPaused: fail,
    getIncludeAllFolders: fail, setIncludeAllFolders: fail,
    searchSources: fail, getBookmarksStatus: fail, setBookmarksEnabled: fail, openBookmark: fail,
    getUpdateSettings: fail, setUpdateCheck: fail, checkForUpdate: fail, installUpdate: fail,
    appVersion: fail, openProjectPage: fail,
    onUpdateProgress: async () => () => {},
    feedbackReport: fail, feedbackSave: fail,
    contentStatus: fail, contentSetEnabled: fail, contentSetPaused: fail, contentRetry: fail, contentRebuild: fail,
    contentMarkOffered: fail, contentOffer: fail, contentDeleteStore: fail, contentSearch: fail,
    contentUnreadCounts: fail, contentUnread: fail, onContentStatus: async () => () => {},
    meaningStatus: fail, meaningOffer: fail, meaningSetEnabled: fail, meaningSetPaused: fail, meaningWarm: fail,
    onMeaningStatus: async () => () => {},
    aiStatus: fail, aiSetSettings: fail, aiNoticeSeen: fail, aiSaveKey: fail, aiSaveKeyFallback: fail,
    aiDeleteKey: fail, aiTestKey: fail, aiSetCap: fail, aiRecords: fail, aiClearRecords: fail,
    aiChatSend: fail, aiChats: fail, aiChat: fail, aiChatRename: fail, aiChatDelete: fail, aiChatsClear: fail,
    aiCancel: fail, aiChatBibliography: fail, aiChatExportRis: fail,
    onAnswerText: async () => () => {},
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
