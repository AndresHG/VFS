# Domain pre-mortem

Walk every question before approving an M or L change. Answer it, or mark it
"n/a because …". Each line exists because shipping without asking it breaks a
local file-search app. The seed lines come from the VFS-1 research, not from
incidents; every line added from now on cites the bug that taught it.

| Ask before building | Learned from |
|---------------------|--------------|
| How long does this take on a slow PC scanning a multi-TB spinning HDD? Is the I/O one sequential stream per disk, low priority, and resumable if the app closes halfway? | seed (VFS-1) |
| Does it work for paths over 260 characters on Windows (`\\?\` prefix, manifest long-path support) and for case-insensitive and case-sensitive volumes on both platforms? | seed (VFS-1) |
| Do names in NFC and NFD (macOS stores NFD; Windows keeps whatever was written) index, compare, fold accents and display the same? | seed (VFS-1) |
| What happens when an antivirus or Windows Defender scans, locks or slows the files we enumerate, or the index database itself? | seed (VFS-1) |
| If the process is killed or power is cut mid-write, does the index open again (WAL, transactions), and how does it rebuild when the file is corrupt? | seed (VFS-1) |
| What does a query that matches a million files do to latency, memory and the UI? Are results capped, paged or streamed? | seed (VFS-1) |
| What happens on permission-denied folders, locked files, symlinks, junctions and cloud placeholders (OneDrive, iCloud): skipped, followed once, or looped? | seed (VFS-1) |
| Does any step open, write, move or delete a user file? v1 only reads metadata and asks the OS to reveal a file. | seed (VFS-1) |
| Does any of this already ship? Before writing the Problem or a requirement, grep every module of the area (all of `apps/desktop/ui/*.js`, not one file) for the behavior. | VFS-38: the draft re-specified highlighting that #61 already shipped in `format.js` |

When a bug escapes because a question wasn't asked, add the question here
with the spec or PR that taught it, in the same PR as the fix.
