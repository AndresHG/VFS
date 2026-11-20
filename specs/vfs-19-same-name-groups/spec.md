# VFS-19 — Copies of a file shown as one result

- **Status**: implemented (this PR)
- **Size**: M (search result shape and one UI row)
- **Links**: VFS-1 (AC-10 ranking, AC-11), VFS-4 (lazy folder reads), issue #96

## Problem

A personal archive holds many copies of the same file: backups of a phone, a disk copied
into another disk, "copia de seguridad" folders. A search for one of them fills the list
with the same name again and again, each in a different folder, and pushes other matches
down. The owner asked (2026-11-20) to show such copies once, saying in how many places the
file is, with every place still one click away. Different files that merely share a name
(`IMG_0001.jpg` from two cameras, `factura.pdf`) must not be merged.

## Requirements (EARS)

- R1. WHEN the ranked list holds several files with the same folded name and the same size,
  THE search SHALL return them as one result: the best-ranked one, carrying the others as
  its `copies` (id, folder, modification date), in ranking order.
- R2. THE search SHALL never group folders, nor files whose size differs.
- R3. WHEN results are paged, THE offsets and `total` SHALL count results (a group is one),
  and every copy of a returned result SHALL name its folder.
- R4. WHEN a result has copies, THE UI SHALL mark it "en N sitios" (N = copies + 1) and, on a
  click of that mark or Mayús+Intro on the selected row, list every place with its own
  "Mostrar en carpeta".

## Acceptance scenarios

- AC-1 (R1) Given three files with the same name and size in three folders and a fourth
  with that name and another size, When the name is searched, Then two results come back:
  one with two copies, one with none.
- AC-2 (R1) Given names that differ only in case or accents and have the same size, Then
  they are one result (the same folding as matching, VFS-1 AC-10).
- AC-3 (R2) Given folders with the same name, Then each is its own result.
- AC-4 (R3) Given more results than a page, When later pages are requested (served from the
  kept list), Then `total` counts results and every copy names its folder.
- AC-5 (R4) Given a result with copies in the UI, Then the row shows "en N sitios", and the
  list of places opens from it and reveals each place.

## Non-goals

- Finding copies that do not match the query, or comparing file contents (hashes).
- Grouping bookmarks (VFS-6) or grouping by name alone.

## Pre-mortem

- Slow PC / HDD: grouping runs over at most 4,000 ranked candidates in memory; no extra
  disk reads except the folders of the copies on the returned page (one row each).
- Long paths, links, permissions, user files: n/a because only the result shape changes.
- NFC/NFD: copies are matched on the folded name, so an NFC and an NFD copy group together.
- A million-hit query: the same caps; groups only make the 2,000 results cover more files.
- Antivirus, killed mid-write: n/a because nothing is written.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Group or show copies separately | Group, with every place one click away | owner | 2026-11-20 |
| 2 | What counts as a copy | Same folded name and same size; not the name alone | recommended, owner agreed | 2026-11-20 |
| 3 | Which copy represents the group | The best-ranked (ties: shorter name, newer, lower id) | engineering | 2026-11-20 |
