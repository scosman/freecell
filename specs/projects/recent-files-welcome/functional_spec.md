---
status: complete
---

# Functional Spec: Recent Files + Welcome Screen

Adds a persisted **recent files** list and rebuilds the **welcome window** around it. Two
surfaces consume the list: the welcome window (all platforms) and the macOS **File → Open
Recent** submenu. Builds on the existing app shell (`crates/freecell-app/src/shell/`:
`app.rs`, `welcome.rs`, `menus.rs`, `registry.rs`).

## 1. Recent files list

### 1.1 What counts as "recent"

An entry is recorded whenever the app **successfully associates a window with a file on
disk**:

- **Opening a file** — via the `Open…` panel, a Finder/CLI argv path, a click on a welcome
  recent row, or the Open Recent menu. Recorded once the path canonicalizes (i.e. the file
  exists) in `FreeCellApp::do_open_path`.
- **Adopting a path via Save / Save As** — when a window takes a canonical path
  (`FreeCellApp::note_window_path`, fired after a successful Save As / first save of a new
  workbook).

**Not** recorded: creating a new unsaved workbook (**New Spreadsheet** — it has no path), a
failed open (bad/missing path never canonicalizes), or an in-memory workbook that is never
saved.

### 1.2 Recency semantics

- The entry's timestamp is **when it was last opened/saved in FreeCell**, not the file's own
  mtime. Re-opening or re-saving a file updates its timestamp and moves it to the front.
- The list is ordered **most-recent-first**.
- A path already in the list is **de-duplicated**: recording an existing (canonical) path
  updates its timestamp and moves it to the front rather than adding a second row.

### 1.3 Retention

- The store holds up to **10** entries (older ones fall off the end when a new distinct file
  is recorded).
- The **welcome pane** displays up to **5** existing entries.
- The **Open Recent menu** displays up to **10** existing entries.

### 1.4 Missing / moved files

- Entries whose file no longer exists on disk are **silently dropped** — never shown on the
  welcome pane or in the menu, and pruned from the persisted store the next time it is
  written. No error, no greyed-out row.
- Because of pruning, the number of *visible* rows may be fewer than the number stored.

### 1.5 Persistence

- The list persists across launches in a small JSON file in the per-user app data directory
  (see `architecture.md §2`). Corrupt/unreadable/absent file ⇒ treated as an empty list (no
  crash, no dialog).
- Writes are best-effort: a failure to persist (e.g. read-only disk) is logged and swallowed
  — it never blocks opening/saving a document or raises a dialog.

### 1.6 Per-row displayed data

Each visible row shows, derived at display time from the path + a filesystem stat:

- **Name** — the file name (e.g. `Q3 Revenue Forecast.xlsx`).
- **Subtitle** — `"{size} · {folder}"` where `size` is the file's size formatted (e.g.
  `1.2 MB`, `480 KB`, `12 KB`) and `folder` is the immediate parent directory's name (e.g.
  `Downloads`, `Documents`, `Desktop`).
- **Timestamp** — a relative label of *last opened in FreeCell* (§1.2): see §1.7.

If the stat fails (file vanished between prune and render), the row is dropped (§1.4).

### 1.7 Relative-time formatting

Given `now` and the entry's `last_opened`, bucket the delta:

| Condition | Label | Example |
|---|---|---|
| < 1 minute | `Just now` | `Just now` |
| < 1 hour | `{n}m ago` | `5m ago` |
| Same calendar day | `{n}h ago` | `2h ago` |
| Previous calendar day | `Yesterday` | `Yesterday` |
| 2–6 days ago | weekday name | `Mon` |
| Same calendar year | `{Mon} {D}` | `Jul 1` |
| Earlier year | `{Mon} {D}, {YYYY}` | `Dec 3, 2024` |

Times in the future (clock skew) clamp to `Just now`. All buckets are pure functions of
(`now`, `last_opened`) and are unit-tested with injected values.

## 2. Welcome window

Redesigned per the mockups (`ui_design.md`), keeping the existing lifecycle
(`functional_spec` of the MVP): opens at launch, closes when any document window loads,
closing the last window quits the app, and it can still host the About / app-level error
dialog when no document window exists.

### 2.1 Layout (two panes)

- **Left pane:** FreeCell wordmark, the tagline **"The open spreadsheet"**, and two
  full-width stacked buttons — **New Spreadsheet** (primary) and **Open…** (outline).
  Behaviour unchanged (New = `FreeCellApp::new_workbook`, Open = `open_via_panel`).
- **Right pane:** a `RECENT` header over the recent list (up to 5 rows) or the empty state.

### 2.2 Recent rows (welcome)

- Each row shows name + subtitle + relative-time (§1.6) and is clickable. **No icon/glyph —
  pure text.**
- **Click** opens that file via `FreeCellApp::open_path` (which dedupes against already-open
  windows and, on success, moves the entry to the front + closes the welcome window when the
  document loads — same as any open).
- Hover gives subtle feedback (see `ui_design.md`).
- A row's file is guaranteed to exist at render time (missing ones are pre-filtered). In the
  rare race where it disappears between render and click, `open_path`'s existing
  canonicalize-failure path shows the standard "Couldn't open the file" dialog on the
  welcome window — no new error surface needed.

### 2.3 Empty state

When there are no existing recent files, the right pane shows a centered **text-only**
placeholder (no glyph/icon): **"No recent spreadsheets"** and **"Create a new spreadsheet or
open a file to get started."** The `RECENT` header remains visible above it.

### 2.4 Live updates

If a recent file is recorded while the welcome window is open (e.g. Save As in a document
window, or an open that fails to close the welcome), the welcome list refreshes to reflect
the new ordering.

## 3. File → Open Recent menu (macOS)

- The **File** menu gains an **Open Recent** submenu positioned after **Open…**.
- The submenu lists up to 10 existing recent files (name only as the item label), most-recent
  first; selecting one opens it via `FreeCellApp::open_path`.
- A separator and a **Clear Recent Files** item follow the list; selecting it empties the
  store (welcome + menu update to their empty states).
- When the list is empty, the submenu contains a single **disabled** `No Recent Files` item
  (standard macOS behaviour) and no Clear item.
- The submenu rebuilds whenever the store changes (open, save-as, clear).
- **Linux:** unchanged — there is no menu bar in the MVP (`menus.rs`), so Open Recent is
  macOS-only. The welcome pane is the cross-platform recents surface.

## 4. Out of scope

- Pinning/favoriting, drag-reorder, right-click context menus on rows, or "reveal in Finder".
- Thumbnails/previews.
- Syncing recents across machines.
- Tracking recents for documents that are never saved to disk.
- Recent-files support in any non-`.xlsx` flow (there is only `.xlsx`).

## 5. Constraints

- Reuse the **existing app palette** (`ui_design.md §0`) — the mockups' ad-hoc colors are
  not the design system.
- No new blocking I/O on the UI thread beyond a bounded number of small `stat`s (≤10) and one
  small JSON read/write; these are cheap and synchronous is acceptable (matches how the shell
  already does synchronous path canonicalization).
- Pure logic (ordering, dedupe, cap, prune, size/folder/relative-time formatting) lives in
  `freecell-core` and is headlessly unit-tested; GPUI wiring is thin.
