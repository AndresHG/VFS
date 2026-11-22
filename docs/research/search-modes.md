# Search modes: choosing name, path, content (and later meaning)

Research for [VFS-38](../../specs/vfs-38-search-modes/spec.md). The question is how a user
chooses which kinds of match a VFS search uses, and how mixed results are shown. Today one
query matches file names and folder paths together, in one list ranked by tier (VFS-1 S2).
v0.3 adds matches inside documents (VFS-2, VFS-27), and VFS-29 adds meaning search later.
Sources were read on 2026-11-22. Claims marked **[unverified]** come only from third-party
pages or could not be confirmed.

## What mature tools do

| Tool | How the user scopes | Default | How results are shown |
|---|---|---|---|
| Everything | The *Match Path* toggle (Search menu, Ctrl+U), plus `path:`, `nopath:` and `content:` in the query [1][2][3]. An active option shows in the status bar, and a double-click turns it off [1]. | Name only. Content is unindexed and "can be extremely slow" [1]. | One flat list. |
| Windows Explorer | A setting, "Always search file names and contents (this might take several minutes)", plus a per-search "File contents" item [4][5] **[unverified wording]**. | Names; contents where indexed. | One list. |
| macOS Finder | One box searches names and contents. Typing offers a «Name matches» token that narrows to names, and `name:` does the same [6][7] **[third-party sources]**. | Name and contents. | One list. Spotlight groups by category [6]. |
| DEVONthink | A scope bar picks *where* to search. The prefixes `name:` and `text:` pick *what* field [8][9]. A preference picks the scope each new search starts with, including "Last scope" [10]. | All fields. | One list. |
| Recoll | A mode selector: All terms, Any term, File name, Query language [11]. History keeps the text, not the mode [11]. | Query language (all terms). | One list of paragraphs with snippets. |
| DocFetcher | Syntax only: `filename:` [12]. Folder paths are never searched [12]. | Name and contents. | A table plus a preview with highlights [13]. |
| Listary | A term ending in `\` or `/` matches the parent path instead of the name [14]. No content search [15]. | Name. | One list. |
| Obsidian | Two UIs: a quick switcher for names and a search pane with `file:`, `path:` and `content:` [16]. | The pane searches content, paths and names. | One mixed list. A long-standing request asks for name hits first, under a separator: exact names get buried, and typing `file:` every time is a chore [17]. |
| VS Code | Two UIs: Quick Open (names) and Search (content), with include and exclude globs [18][19]. | Each UI has one kind. | Content results are grouped by file, with matching lines [19]. |
| Google Drive | Chips (Type, People, Modified) in the search dropdown, cleared by «Clear filters». The `title:` operator limits to names [20]. A «Title only» chip was reported in 2023 [21] **[unverified today]**. | Name and content. | One list. |

Guidance from design research:
- NN/g: default the scope to everything. Show the scope near the box and above the results,
  and offer a one-click widen, especially when nothing is found. Users change scope *after*
  seeing results [22].
- Carbon: a scope dropdown is single-select and defaults to "All" [23].
- Baymard: users misread a scope selector that is not self-evident, and then think the
  site lacks what they want [24].

## Findings

1. **Control.**
   - Mainstream tools start with one broad search and narrow it with a visible choice: a
     token or chip (Finder, Drive) or a menu or setting (Explorer, Everything).
   - Query syntax exists everywhere, but only as the expert layer. Syntax-only tools draw
     complaints (Obsidian [17]).
   - Name, path and content are not mutually exclusive, so a multi-select fits better than a
     single-select dropdown.
   - Whatever the widget, an active narrowing must stay visible. Everything's status bar [1]
     and Baymard [24] agree.
2. **Default.**
   - Search everything that is indexed: NN/g, Finder, Drive, Obsidian and DocFetcher all do.
   - Everything defaults to names only because its content search is unindexed and slow.
     VFS has the same cost, which is why content is opt-in in Ajustes (VFS-27 R4, R5).
3. **Grouped vs one list.**
   - Separating by match kind is common where kinds differ in cost or meaning: VS Code's two
     UIs, Spotlight's categories, and Obsidian's most-asked fix [17].
   - No tool examined shows a "matched in: name / folder / content" badge. They explain a
     match with highlighted words or a text fragment instead (DocFetcher, VS Code,
     Obsidian).
4. **Small screens and non-technical users.**
   - Only Finder's token and Drive's chips need no syntax.
   - A row of 3–4 extra chips would not fit VFS's filter line. At 1366×768 and 125 % the
     window is about 1093 px wide, and the line already holds five or six pills on one
     scrolling row (VFS-35 R2).
   - A single pill whose text states the current scope, with a checklist behind it, keeps
     the state visible in one pill's width. VFS's «Tipo» pill already works this way.
   - This point is our inference, not a source's claim.
5. **Remembering the choice.**
   - Remembered: Explorer's setting and DEVONthink's "Last scope".
   - Not remembered: Recoll's history, and VFS-6 D7, where the bookmark source resets at
     each start.
   - NN/g argues for a broad start. A remembered narrowing that is not visible can make the
     archive look empty. A remembered one that *is* visible, and that the empty-result
     suggestion can widen, removes most of that risk.

## Second round: the filter line and match cues

The owner asked for a wider look: the filter line already has too many buttons, and a
per-row «coincide en» badge was tempting.

- **How many visible controls.**
  - The Australian Government Design System: show 1–2 filters always; put 3–6 behind a
    "Show filters" control and 6 or more in a drawer. Keep the 1–2 most-used outside, and
    show applied filters as removable tags [25].
  - Baymard: a horizontal filter toolbar gets troublesome past 6–8 filter types [26]. An
    "All filters" button is overlooked by some users, so its label must be clear.
  - Baymard: 28 % of sites lack an applied-filters overview, which disorients users [27].
  - NN/g: progressive disclosure by frequency of use, at most two levels, with a visible,
    labelled trigger [28].
- **Older users.** NN/g measured 55 % task success for seniors against 75 % for younger
  users. They found drop-down menus and moving widgets hard, and recommend large, static
  controls [29].
- **Scoping.** Nielsen calls up-front scoping "in general … dangerous" and finds that most
  users cannot use query syntax [30]. If there is a scope, it should default to everything
  and be visible (NN/g [22]).
- **Products.**
  - Drive and Gmail: a few chips under the box, with "Clear all" and a panel for the rest
    [20][31].
  - Slack, Linear and Notion: one "Filters" control [32][33][34].
  - Raycast: content search is opt-in in Settings, and content hits come after name and
    path hits [35].
  - Alfred: content search is a separate keyword [36].
- **Why it matched.**
  - Hearst: the result surrogate, a keyword-in-context fragment with highlighted query
    terms, is what lets users judge relevance [37].
  - Dropbox shows a text fragment under each content hit [38].
  - Algolia highlights on every searchable field by default [39].
  - SharePoint uses hit-highlighted summaries [40].
  - No guideline or product examined recommends a per-row "matched in" badge. One
    unverified product note («Match found in …», Guru) is the only label found.
  - A badge costs about 70–90 px on every row and says less than a highlight does.

## Recommendation (as decided in VFS-38)

- **Scope as three static buttons**, «Buscar en: Todo | Solo nombres | Solo documentos».
  - There is no drop-down, which helps seniors [29].
  - «Todo» is the default, as NN/g advises [22].
  - «Solo documentos» exists only while content search is on.
  - A single choice covers the cases the owner named. Checkboxes would allow empty or odd
    combinations, such as a path without a name.
- **Two filters visible.** «Tipo» and «Fecha» stay on the line. «Tamaño», «Carpeta» and the
  bookmark source move behind «Más filtros (N)». Active filters show as removable chips on a
  line below [25][27]. «Orden» moves to the results header.
  - The line becomes five controls, under the 6–8 limit [26].
  - It fits 1280×720 at 125 % without scrolling sideways.
- **Default: everything indexed.** Existing users see no change.
- **Remember the choice** across restarts. The owner's cases are standing preferences.
  Three things keep a remembered narrowing from hiding files:
  - the selected button is always on screen;
  - the empty view names the scope and offers «Todo»;
  - «Quitar filtros» resets it.
- **Results.**
  - Name and path hits stay in one list ranked by the VFS-1 tiers.
  - Content hits keep their own section (VFS-2 D9).
  - Meaning hits fuse into the content section (VFS-29).
- **Why it matched.**
  - Highlight the query words in the name and the folder line, and keep the fragment for
    content hits.
  - Add no badge.
  - Only meaning-only hits, which have no word to highlight, carry «relacionado por tema».
- **No new query syntax.** P8 (VFS-17), if built, adds the expert layer.

## Sources

1. voidtools, Searching. https://www.voidtools.com/support/everything/searching/
2. voidtools, Search modifiers. https://voidtools.com/support/everything/search_modifiers
3. voidtools, Keyboard shortcuts. https://www.voidtools.com/support/everything/keyboard_shortcuts/
4. Microsoft Q&A, file names and contents. https://learn.microsoft.com/en-us/answers/a/4615904
5. Digital Citizen, searching in File Explorer. https://www.digitalcitizen.life/search-windows-10-file-explorer/
6. Eclectic Light, searching in Spotlight GUI tools (2025). https://eclecticlight.co/2025/06/02/how-to-search-successfully-in-spotlight-gui-tools/
7. MacMost, search by file name only. https://macmost.com/search-by-file-name-only.html
8. DEVONtechnologies, narrowing a search. https://www.devontechnologies.com/blog/20231107-narrow-search-devonthink
9. DEVONthink help, search prefixes. https://download.devontechnologies.com/download/devonthink/3.8.2/DEVONthink.help/Contents/Resources/pgs/appendix-searchprefixes.html
10. DEVONthink help, General preferences. https://download.devontechnologies.com/download/devonthink/3.8.2/DEVONthink.help/Contents/Resources/pgs/preferences-general.html
11. Recoll user manual §3.1, §3.2.1. https://www.recoll.org/usermanual/usermanual.html
12. DocFetcher FAQ. https://sourceforge.net/p/docfetcher/wiki/FAQ
13. DocFetcher Pro. https://docfetcherpro.com/miscellaneous/
14. Listary, advanced search syntax. https://help.listary.com/advanced-search-syntax
15. Listary forum, content search. https://discussion.listary.com/t/possible-to-search-inside-file-contents/8255
16. Obsidian help, Search. https://obsidian.md/help/plugins/search
17. Obsidian forum, separate filename results from content results. https://forum.obsidian.md/t/separate-filename-search-results-from-content-results/4779
18. VS Code, tips and tricks. https://code.visualstudio.com/docs/getstarted/tips-and-tricks
19. VS Code, basic editing (search across files). https://code.visualstudio.com/docs/editing/codebasics
20. Google Drive help, find files. https://support.google.com/drive/answer/2375114
21. Engadget, Drive search filters (2023). https://www.engadget.com/google-drives-improved-search-filters-are-rolling-out-to-all-users-110232054.html
22. NN/g, scoped search. https://www.nngroup.com/articles/scoped-search/
23. Carbon Design System, search pattern. https://carbondesignsystem.com/patterns/search-pattern
24. Baymard, search scope. https://baymard.com/blog/search-scope
25. Australian Government Design System (Agriculture), search filters. https://design-system.agriculture.gov.au/patterns/search-filters
26. Baymard, horizontal filtering and sorting design. https://baymard.com/blog/horizontal-filtering-sorting-design
27. Baymard, applied filters. https://baymard.com/blog/how-to-design-applied-filters
28. NN/g, progressive disclosure. https://www.nngroup.com/articles/progressive-disclosure/
29. NN/g, usability for seniors. https://www.nngroup.com/articles/usability-seniors-improvements/
30. NN/g, search: visible and simple. https://www.nngroup.com/articles/search-visible-and-simple/
31. Gmail help, search options. https://support.google.com/mail/answer/6593
32. Slack help, search. https://slack.com/help/articles/202528808
33. Linear docs, filters. https://linear.app/docs/filters
34. Notion help, search. https://www.notion.com/help/search
35. Raycast manual, file search. https://manual.raycast.com/file-search
36. Alfred help, file search. https://www.alfredapp.com/help/features/file-search/
37. Hearst, Search User Interfaces, ch. 5. https://searchuserinterfaces.com/book/sui_ch5_retrieval_results.html
38. Dropbox help, search file contents. https://help.dropbox.com/files-folders/search-content
39. Algolia, highlighting and snippeting. https://www.algolia.com/doc/guides/building-search-ui/resources/ui-and-ux-patterns/in-depth/highlighting-snippeting/
40. Microsoft Learn, SharePoint search results. https://learn.microsoft.com/en-us/sharepoint/search/how-to-display-values-from-custom-managed-properties-in-search-resultsoption-2
