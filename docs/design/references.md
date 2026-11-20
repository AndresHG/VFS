# Design references

What the VFS search screen takes from other calm, minimal search and file tools, and what it
leaves behind. The reader is an older, non-technical person on a slow Windows PC with a 4–5 TB
archive; every borrowed idea is judged against that person, not against a power user.

## claude.ai (Anthropic)

<https://claude.ai/> — the owner's named reference for calm minimalism.

Take:
- **One thing in the middle.** An empty conversation is a greeting and a single input, centred,
  with nothing else competing. The VFS idle screen should be the search box and one sentence.
- **Warm neutrals, not grey-blue.** An ivory/paper background with near-black text and a single
  warm accent reads as quiet and friendly rather than clinical.
- **Restraint with colour.** The accent is used for one primary action at a time; everything
  else is ink, muted ink and hairlines.
- **Generous line height and measure.** Body copy breathes; nothing is packed to the edges.
- **Soft, honest loading states.** A small, slow indicator next to the thing that is working,
  never a full-screen spinner; text stays readable while work goes on.
- **Chrome recedes.** Settings, history and account live in a side panel or menu, not on the
  main surface.

Reject:
- **A serif display face.** It is lovely for a chat greeting but adds a second family to bundle
  offline and lowers legibility of short labels at small sizes for low-vision readers.
- **Very light, low-contrast secondary text.** Fine for younger eyes; for this reader muted text
  stays at 4.5:1 or better.
- **Hidden affordances.** Hover-only icons and tiny ghost buttons work for daily users of
  claude.ai; here every action is visible and labelled.

## macOS Spotlight

<https://support.apple.com/guide/mac-help/search-with-spotlight-mchlp1008/mac>

Take:
- **Results while typing, best match first, keyboard-first.** Arrows move, Return acts. VFS
  already does this; keep it.
- **The type of each result is shown by an icon, not a column.** A text "Tipo" column repeats
  what the icon already says.
- **The selected result reveals its full location** (Spotlight's preview pane, Finder's path
  bar). VFS's "Ubicación" line does this; it should sit next to the selection, not in the footer.

Reject:
- **Grouping by category headers.** With thousands of hits, groups fragment the list and make
  arrow-key travel unpredictable.
- **Previews.** The product never opens or previews files (spec R9); reading thumbnails off a
  slow HDD would also hurt speed.

## Finder search

<https://developer.apple.com/design/human-interface-guidelines/searching>

Take:
- **Scope control near the box.** "This Mac / current folder" is the same idea as the VFS
  folder filter.
- **Search tokens.** A chosen filter shows as a removable token, so the person sees what is
  narrowing the list and can undo it in one click.

Reject:
- **The "+" rule builder.** Powerful, but a nested query editor is exactly the kind of option an
  older non-technical reader will not use and may break.

## Raycast

<https://www.raycast.com/>

Take:
- **One primary action per row, shown with its key.** The selected row carries the action
  ("Mostrar en carpeta ↵"); other rows stay quiet. This removes a column of identical buttons.
- **A thin footer that carries state** (what is happening, what Return does) rather than a
  second toolbar.

Reject:
- **Command palette density and shortcuts as the main path.** ⌘K menus, action panels and
  tiny 13 px type assume a power user.

## Linear

<https://linear.app/now/how-we-redesigned-the-linear-ui>

Take:
- **Neutral chrome, colour only where it carries meaning.** Linear's redesign reduced chromatic
  colour in the UI so status colours stand out. In VFS, colour marks the selection, the match and
  a warning; nothing decorative.
- **Consistent spacing scale and alignment on a single left edge.** Fewer, stronger alignments
  make a dense screen calm.
- **Hairline separators instead of boxes.** Reduce nested borders (card in a panel in a frame).

Reject:
- **Small type and low-contrast grey on grey.** Linear is designed for daily, all-day use by
  engineers; its density is wrong for this reader.

## Everything (voidtools)

<https://www.voidtools.com/support/everything/>

Take:
- **Instant results on millions of files with a plain list.** Name, path, size, date. No
  thumbnails, no animation; the list *is* the interface. Proof that a flat virtualised list is
  enough at 5 M files.
- **Path visible on every row**, because names alone ("IMG_0042.JPG") are ambiguous.

Reject:
- **Spreadsheet density and grid chrome.** 20 px rows, column headers, menu bar and regex
  syntax are hostile to an older reader. VFS keeps the content model and drops the chrome.

## Accessibility guidance for older users

- W3C WAI, *Older Users and Web Accessibility*: <https://www.w3.org/WAI/older-users/> —
  age-related declines in vision (contrast sensitivity, colour perception, focus), dexterity and
  short-term memory; the WCAG criteria that matter most for them.
- WCAG 2.2 *Contrast (Minimum)* 1.4.3: <https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html>
  — 4.5:1 for text. VFS aims higher: 7:1 for primary text, 4.5:1 for muted text, 3:1 for
  borders that define a control (1.4.11).
- WCAG 2.2 *Target Size (Minimum)* 2.5.8: <https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html>
  — 24 px minimum; VFS keeps the 44 px enhanced target (2.5.5) for every button, as Microsoft's
  touch/pointer guidance also recommends: <https://learn.microsoft.com/en-us/windows/apps/design/input/guidelines-for-targeting>.
- WCAG 2.2 *Resize Text* 1.4.4 and *Reflow* 1.4.10:
  <https://www.w3.org/WAI/WCAG22/Understanding/resize-text.html>,
  <https://www.w3.org/WAI/WCAG22/Understanding/reflow.html> — at 200 % text nothing is lost.
  The audit found the current layout fails this at 150 % in a 900×600 window.
- WCAG 2.2 *Text Spacing* 1.4.12: <https://www.w3.org/WAI/WCAG22/Understanding/text-spacing.html>.
- Nielsen Norman Group, *Usability for Seniors*: <https://www.nngroup.com/articles/usability-for-senior-citizens/>
  — larger text and targets, plain words, visible state, forgiving errors, fewer simultaneous
  choices, no reliance on hover or memory.
- Braille Institute, *Atkinson Hyperlegible*: <https://www.brailleinstitute.org/freefont/> —
  the typeface VFS already bundles, designed to separate confusable letters for low-vision
  readers. Keep it.
- Microsoft, *Typography in Windows*: <https://learn.microsoft.com/en-us/windows/apps/design/style/typography>
  — Windows text scaling changes the root size; every VFS size stays in `rem` so it follows.

## What this adds up to

1. The idle screen is a search box and one sentence; filters and folders do not shout.
2. One typeface (Atkinson Hyperlegible), warm neutrals, one accent, colour only for meaning.
3. One action per row, on the selected row; type shown by icon, not a column.
4. Big text and targets by default; the layout must survive 150–200 % text in a small window.
5. State in plain Spanish, in one quiet place, never as a modal.
