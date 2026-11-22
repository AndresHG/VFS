// The English catalog (VFS-22 R1): the same keys as `strings.es.js`, same tone: short, plain
// words, no "index" jargon. Functions take already-formatted values.

import { DateChoice, LanguageChoice, SizeChoice, SourceChoice } from './choices.js';
import { BookmarkRoot, ErrorCode, FileKind, LinkKind, ReadState, RootStatus, SortOrder, UnreadableReason } from './contract.js';

const files = (n) => `${n} files`;

export const en = {
  appName: 'File Finder',
  windowTitle: 'VFS — File Finder',
  searchLabel: 'Search by file or folder name',
  searchPlaceholder: 'For example: tax return 2023',
  searchHint: 'Use the up and down arrows to move through the results, Enter to show the chosen one in its folder, Shift and Enter to show where a shortcut leads and Escape to clear the search.',
  clearSearch: 'Clear the search',
  devBadge: 'Test data',

  filtersTitle: 'Filters',
  fType: 'Type',
  fDate: 'Date',
  fSize: 'Size',
  fFolder: 'Folder',
  fFrom: 'From',
  fTo: 'To',
  typeButton: (...names) => (!names.length ? 'All types'
    : names.length <= 2 ? names.join(', ') : `${names[0]} and ${names.length - 1} more`),
  clearFilters: 'Clear filters',
  clearFiltersN: (n) => `Clear filters (${n})`,
  rangeInverted: 'The “From” date is after the “To” date.',

  kindsPlural: {
    [FileKind.DOCUMENT]: 'Documents', [FileKind.PHOTO]: 'Photos', [FileKind.VIDEO]: 'Videos',
    [FileKind.AUDIO]: 'Audio', [FileKind.FOLDER]: 'Folders', [FileKind.OTHER]: 'Other',
  },
  kindSingular: {
    [FileKind.DOCUMENT]: 'Document', [FileKind.PHOTO]: 'Photo', [FileKind.VIDEO]: 'Video',
    [FileKind.AUDIO]: 'Audio', [FileKind.FOLDER]: 'Folder', [FileKind.OTHER]: 'Other',
  },

  dateOptions: {
    [DateChoice.ANY]: 'Any date', [DateChoice.WEEK]: 'Last 7 days',
    [DateChoice.MONTH]: 'Last 30 days', [DateChoice.YEAR]: 'Last year',
    [DateChoice.CUSTOM]: 'Choose dates…',
  },
  sizeOptions: {
    [SizeChoice.ANY]: 'Any size', [SizeChoice.SMALL]: 'Under 1 MB',
    [SizeChoice.MEDIUM]: '1 MB to 100 MB', [SizeChoice.LARGE]: '100 MB to 1 GB',
    [SizeChoice.HUGE]: 'Over 1 GB',
  },
  fSource: 'Search in',
  sourceOptions: {
    [SourceChoice.BOTH]: 'Files and bookmarks', [SourceChoice.FILES]: 'Files',
    [SourceChoice.BOOKMARKS]: 'Bookmarks',
  },
  fSort: 'Order',
  // Option order is display order (VFS-12).
  sortOptions: {
    [SortOrder.RELEVANCE]: 'Best match first', [SortOrder.NEWEST]: 'Most recent first',
    [SortOrder.OLDEST]: 'Oldest first', [SortOrder.NAME]: 'By name (A-Z)',
    [SortOrder.SIZE]: 'Largest first', [SortOrder.SOURCE]: 'Files first, then bookmarks',
  },
  folderGroupBookmarks: 'Firefox bookmarks',
  folderAll: 'All folders',
  folderPick: 'Choose another folder…',
  folderInside: (name) => `In: ${name}`,

  resultsTitle: 'Results',
  results: (n, one) => (one ? '1 result' : `${n} results`),
  resultsCapped: (n) => `More than ${n} results. Type a little more to narrow them down.`,
  resultsApproximate: (n) => `More than ${n} results: too many to sort them all. Type a little more to narrow them down.`,
  resultsPartial: (n) => `${n} results. The search did not quite finish; some may be missing.`,
  searching: 'Searching…',
  fuzzyNote: 'No exact matches; showing similar results.',
  loadingRow: 'Loading…',
  revealButton: 'Show in folder',
  revealKey: 'Enter',
  unavailableBadge: 'Unavailable',
  openButton: 'Open in Firefox',
  folderContentsButton: 'See what is inside',
  bookmarkKind: 'Bookmark',
  bookmarkFolderKind: 'Bookmark folder',
  bookmarkRoots: {
    [BookmarkRoot.MENU]: 'Bookmarks Menu', [BookmarkRoot.TOOLBAR]: 'Bookmarks Toolbar',
    [BookmarkRoot.UNFILED]: 'Other Bookmarks', [BookmarkRoot.MOBILE]: 'Mobile Bookmarks',
  },
  bookmarkRowLabel: (title, kind, path, site, date) =>
    [title, kind, `in ${path}`, site, date && `saved on ${date}`].filter(Boolean).join(', '),
  detailsBookmark: 'Address:',
  detailsBookmarkFolder: 'Saved in:',
  rowLabel: (name, kind, dir, date, size, unavailable, places = 1) =>
    [name, kind, `in ${dir}`, date && `modified on ${date}`, size, unavailable && 'unavailable',
      places > 1 && `in ${places} places, Shift plus Enter to see them`]
      .filter(Boolean).join(', '),
  copiesBadge: (n) => `in ${n} places`,
  copiesBadgeLabel: (n) => `See the ${n} places where it is`,
  copiesTitle: (name, n) => `“${name}” is in ${n} places`,
  copiesIntro: 'These files have the same name and the same size: almost certainly copies of the same file. Choose the one you want to see.',
  copiesModified: (date) => `Modified on ${date}`,
  detailsLabel: 'Location:',

  // A file whose data is only in the cloud (VFS-25 R4).
  cloudOnlyBadge: 'Cloud only',

  // Shortcuts and links (VFS-18 D2, D3, D6).
  linkBadge: {
    [LinkKind.SHORTCUT]: 'Shortcut', [LinkKind.INTERNET]: 'Web page', [LinkKind.SYMLINK]: 'Link',
  },
  linkTargetPrefix: 'Leads to: ',
  targetButton: 'Show the destination',
  secondKey: 'Shift+Enter',
  linkRowLabel: (badge, target) => [badge, target && `leads to ${target}`].filter(Boolean).join(', '),
  detailsLinkAt: {
    [LinkKind.SHORTCUT]: 'Shortcut in:', [LinkKind.INTERNET]: 'Web page saved in:',
    [LinkKind.SYMLINK]: 'Link in:',
  },
  detailsLinkTarget: 'Leads to:',
  targetMissing: (t) => `The destination “${t}” cannot be found. It may have been moved or deleted, or its disk may not be connected.`,

  emptyNoRootsTitle: 'Add a folder to get started',
  emptyNoRootsBody: 'Choose the folder where you keep your files. The app will go through it and then you can search it. Your files are never moved or changed.',
  emptyFirstScanTitle: 'Getting the search ready…',
  emptyFirstScanBody: (n) => `${n} files checked so far. You can search in the meantime; if you close the app, it carries on when you open it again.`,
  emptyIdleTitle: (n) => `Type to search ${n} files`,
  emptyIdleBody: 'You can type part of the name or of the folder. Capitals, accents and a wrong letter do not matter.',
  emptyNoHitsTitle: (q) => (q ? `Nothing found for “${q}”` : 'No files match these filters'),
  emptyNoHitsBody: 'Check what you typed or try fewer words.',
  emptyNoHitsFilters: 'Some filters are on and may be hiding results.',
  // One search that finds something (VFS-13); `n` is a formatted count or suggestCapped.
  suggestDropFilters: (n) => `No results with these filters. Search without filters? (${n})`,
  suggestSpelling: (word, n) => `Did you mean “${word}”? (${n})`,
  suggestDropTerm: (word, n) => `Search without “${word}”? (${n})`,
  suggestCapped: (n) => `more than ${n}`,
  emptyDeadlineTitle: 'The search did not finish; please try again.',
  emptyDeadlineBody: 'The computer is very busy right now. If it happens again, type another word.',
  emptyErrorTitle: 'The search could not be done',
  emptyErrorBody: 'Please try again in a moment.',
  retry: 'Try again',
  addFolderEllipsis: 'Add folder…',

  firstScanNote: 'The search is getting ready. You can search in the meantime; if you close the app, it carries on when you open it again.',
  unavailableOne: (p) => `The folder “${p}” cannot be found. Is the drive connected? Its files stay in the list, marked “Unavailable”.`,
  quoted: (s) => `“${s}”`,
  unavailableMany: (list) => `These folders cannot be found: ${list}. Are the drives connected? Their files stay in the list, marked “Unavailable”.`,
  understood: 'Got it',
  dismiss: 'Close notice',

  statusScanning: (n, dirs) => `Checking folders… ${files(n)} found · ${dirs} folders to go`,
  statusChecking: (n) => `Looking for changes… · ${files(n)}`,
  statusPaused: (n) => `Check paused · ${files(n)} found`,
  statusChecked: (when, n) => `Last checked: ${when} · ${files(n)}`,
  statusFiles: files,
  today: (time) => `today ${time}`,
  yesterday: (time) => `yesterday ${time}`,
  dayAndTime: (day, time) => `${day}, ${time}`,
  statusReadyUnavailable: (n, k) => `${files(n)} · ${k === 1 ? '1 folder unavailable' : `${k} folders unavailable`}`,
  statusNoRoots: 'No folders yet',
  statusConnecting: 'Connecting…',
  pause: 'Pause',
  resume: 'Resume',
  foldersButton: (n) => (n ? `Folders (${n})` : 'Folders'),

  foldersTitle: 'Folders to search',
  foldersIntro: 'The app only searches these folders and everything inside them. It never moves, changes or deletes your files.',
  foldersEmpty: 'You have not added any folders yet.',
  addFolder: 'Add folder…',
  rescanAll: 'Look for changes now',
  rescan: 'Check again',
  rootPause: 'Pause',
  rootResume: 'Resume',
  rootPaused: 'Paused',
  remove: 'Remove',
  close: 'Close',
  rootStatus: {
    [RootStatus.PENDING]: 'Waiting', [RootStatus.SCANNING]: 'Checking…', [RootStatus.READY]: 'Ready',
    [RootStatus.UNAVAILABLE]: 'Unavailable', [RootStatus.REMOVING]: 'Removing…',
  },
  rootFiles: files,
  // Folders that could not be read (VFS-10 R5): empty for none; `n` is `count` formatted.
  unreadableLine: (count, n) => (!count ? '' : count === 1 ? '1 folder could not be checked'
    : `${n} folders could not be checked`),
  unreadableSee: 'View',
  unreadableTitle: 'Folders that could not be checked',
  unreadableIntro: 'These folders could not be opened. What they contain does not appear in searches.',
  unreadableReason: {
    [UnreadableReason.NO_PERMISSION]: 'No permission', [UnreadableReason.UNAVAILABLE]: 'Not available',
    [UnreadableReason.CLOUD_ONLY]: 'Cloud only', [UnreadableReason.DISK_ERROR]: 'Disk error',
  },
  unreadableMore: (n) => `and ${n} more`,
  unreadableHint: 'When the folder is available again, press “Look for changes now” to check it again.',
  rootChecked: (d) => `Checked on ${d}`,
  rootNeverChecked: 'Not fully checked yet',
  rescanStarted: 'Changes will be looked for in the background. You can keep searching meanwhile.',
  rootAdded: (p) => `“${p}” was added. It is being checked; you can search in the meantime.`,
  rootTakesIn: (p, inner) => { const l = [].concat(inner); return `“${p}” was added. It includes ${l.map((x) => `“${x}”`).join(', ')}, which ${l.length > 1 ? 'were' : 'was'} already in the list; ${l.length > 1 ? 'their' : 'its'} files still appear in searches while it is checked.`; },
  remoteWarning: 'This folder is on a network or removable drive. If it is disconnected, its files will be marked “Unavailable”.',
  bookmarksTitle: 'Firefox bookmarks',
  bookmarksToggle: 'Include Firefox bookmarks',
  bookmarksIntro: 'Also searches the bookmarks saved in Firefox. Only bookmarks are read, never history or passwords, and nothing leaves this computer.',

  // VFS-21: updates. The check is the app's only connection to the internet; say so.
  updatesTitle: 'Updates',
  updatesToggle: 'Check for updates when opening',
  updatesIntro: 'When it opens, VFS checks GitHub for a new version. This is the only internet connection VFS makes: it sends nothing about your files, and if you turn this off it never connects.',
  // VFS-33: the default folder exclusions, folded under "Advanced".
  advancedTitle: 'Advanced',
  includeAllToggle: 'Include system and program folders',
  includeAllIntro: 'Normally VFS skips folders that only the system and programs use: system files, app data (Library on Mac, AppData on Windows), development folders (node_modules, .git, Python environments) and other programs\' caches. If you add one of those folders to your list, it is checked. Changing this option checks all your folders again.',
  updateAvailable: (version) => `A new version of VFS is available (${version}).`,
  updateInstall: 'Update',
  updateLater: 'Not now',
  updateDownloading: (pct) => (pct == null ? 'Downloading the update…' : `Downloading the update… ${pct}%`),
  updateInstalling: 'Installing the update. VFS will reopen by itself in a few seconds.',
  updateFailed: 'The update could not be downloaded. You can try again now or the next time you open VFS.',
  updateInstallFailed: 'The update could not be installed. VFS keeps working with the current version; you can try again now or the next time you open it.',
  updateRetry: 'Try again',
  bookmarksCount: (n, when) => `${n} bookmarks · read ${when}`,
  bookmarksState: {
    [ReadState.NEVER]: 'The bookmarks have not been read yet.',
    [ReadState.READING]: 'Reading Firefox bookmarks…',
    [ReadState.OK]: '',
    [ReadState.FROM_BACKUP]: 'Firefox did not allow reading its latest bookmarks; its last backup is used instead.',
    [ReadState.NO_FIREFOX]: 'No Firefox bookmarks were found on this computer.',
    [ReadState.FAILED]: 'The Firefox bookmarks could not be read. It will be tried again later.',
  },
  settingsButton: 'Settings',
  settingsTitle: 'Settings',
  languageLabel: 'Language',
  languageOptions: {
    [LanguageChoice.SYSTEM]: 'Same as the system', [LanguageChoice.ES]: 'Español',
    [LanguageChoice.EN]: 'English',
  },
  removeTitle: 'Remove this folder?',
  removeBody: 'It will no longer appear in searches. Its files are not deleted from the disk.',
  cancel: 'Cancel',
  removeConfirm: 'Remove folder',

  feedbackButton: 'Send feedback',
  feedbackTitle: 'Send feedback',
  feedbackIntro: 'Tell us what happened or what you would like. A report from the app that helps find bugs is attached. Nothing leaves this computer until you send it.',
  feedbackLabel: 'What happened, or what would you like?',
  feedbackNames: 'Include folder and file names in the report',
  feedbackNamesHint: 'Helps find some bugs. Unticked, the report holds no folder or file name.',
  feedbackPreview: 'See the report',
  feedbackPreviewNote: 'Only the latest log lines are shown here; the .zip file holds the whole report. To see your text updated, close and reopen this section.',
  feedbackLoading: 'Preparing the report…',
  feedbackHowTo: '“Send via GitHub” saves the report and opens a GitHub page (a free account is needed) to drag it into. Without an account, use “Export diagnostics” and send the file to whoever installed the app for you.',
  feedbackPublic: 'The GitHub page is public: anyone can see what you write and the file you attach. Folder and file names never go in the page text; only in the .zip, if you ticked the box.',
  feedbackGithub: 'Send via GitHub',
  feedbackExport: 'Export diagnostics (.zip)',
  feedbackSaved: (folder, file) => `Report saved in “${folder}” as “${file}”. Send it to whoever installed the app for you, or attach it on GitHub.`,
  feedbackSavedGithub: (folder, file) => `Report saved in “${folder}” as “${file}”. Drag it into the GitHub page that just opened.`,

  revealedParent: 'The folder opened, but the file could not be selected.',
  overlapInside: (p) => `That folder is already included, because it is inside “${p}”.`,
  overlapGeneric: 'That folder is already in the list or inside another folder in the list.',

  errors: {
    generic: 'Something went wrong. Please try again.',
    reveal: {
      [ErrorCode.NOT_FOUND]: 'The file is no longer in that location.',
      [ErrorCode.UNAVAILABLE]: 'That folder is not available right now. Check that the drive is connected.',
    },
    revealTarget: {
      [ErrorCode.UNAVAILABLE]: 'This shortcut does not say where it leads.',
    },
    openLinkUrl: {
      [ErrorCode.NOT_FOUND]: 'The file is no longer in that location.',
      [ErrorCode.UNAVAILABLE]: 'This address is not a web page and cannot be opened.',
    },
    openBookmark: {
      [ErrorCode.NOT_FOUND]: 'That bookmark no longer exists in Firefox.',
      [ErrorCode.UNAVAILABLE]: 'This bookmark is not a web page and cannot be opened.',
    },
    addRoot: {
      [ErrorCode.NOT_FOUND]: 'That folder cannot be found.',
      [ErrorCode.NOT_A_DIRECTORY]: 'That is not a folder. Please choose a folder.',
      [ErrorCode.UNAVAILABLE]: 'That folder cannot be opened. Check that the drive is connected.',
    },
    any: {
      [ErrorCode.SCHEMA_TOO_NEW]: 'The search data was made by a newer version of the app. Please install the latest version.',
    },
  },
  backendMissing: 'Could not connect to the app. Close it and open it again.',
};
