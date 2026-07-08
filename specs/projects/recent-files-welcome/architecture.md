---
status: complete
---

# Architecture: Recent Files + Welcome Screen

Single-doc architecture (well under ~300 lines of technical content; no separate component
designs needed). Follows the codebase's **pure-vs-GPUI split**: all recent-list logic lives
in the GPUI-free `freecell-core` (headlessly unit-tested); `freecell-app` adds the thin GPUI
plumbing (store ownership, record sites, welcome view, menu).

## 1. Component map

| Layer | Location | Responsibility |
|---|---|---|
| Recent-list logic | `freecell-core/src/recent.rs` (new) | model, (de)serialize, record/dedupe/cap/prune, display-entry building, formatters |
| Path resolution + ownership | `freecell-app/src/shell/recents.rs` (new) | resolve the on-disk store path (`dirs`), wrap load/save; helpers used by `FreeCellApp` |
| App wiring | `freecell-app/src/shell/app.rs` | own `RecentList`, record on open + path-adoption, push updates to welcome + menu |
| Welcome UI | `freecell-app/src/shell/welcome.rs` | two-pane render, recent rows, empty state, click→open |
| Menu | `freecell-app/src/shell/menus.rs` | Open Recent submenu build; `OpenRecent`/`ClearRecent` actions |

## 2. Data model & persistence (`freecell-core::recent`)

```rust
/// One recorded file. `last_opened` is Unix seconds (i64) — "last opened in FreeCell".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub last_opened: i64,
}

/// The most-recent-first list. Serialized as `{ "entries": [...] }`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RecentList {
    pub entries: Vec<RecentEntry>,
}

pub const STORE_CAP: usize = 10;      // max entries retained
pub const WELCOME_LIMIT: usize = 5;   // rows shown on the welcome pane
pub const MENU_LIMIT: usize = 10;     // items shown in Open Recent
```

Only `path` + `last_opened` are stored; size/folder are re-derived from disk at display time
(so they never go stale). `PathBuf` serializes as its string form via serde — acceptable for
a local cache; non-UTF-8 paths are lossy but such paths never round-trip through the `.xlsx`
open flow in practice.

### 2.1 Core API (all pure or std-fs only — no GPUI, no `dirs`)

```rust
impl RecentList {
    /// Parse from JSON bytes; any error (absent handled by caller) → Default (empty). Never panics.
    pub fn from_json(bytes: &[u8]) -> Self;
    pub fn to_json(&self) -> Vec<u8>;

    /// Read `path` if present; missing/corrupt ⇒ empty. Best-effort.
    pub fn load(path: &Path) -> Self;
    /// Serialize to `path` (creating parent dirs). Returns io::Result; caller logs+swallows.
    pub fn save(&self, path: &Path) -> io::Result<()>;

    /// Record `path` as opened at `now` (Unix secs): canonicalize-agnostic — caller passes an
    /// already-canonical path. Removes any existing entry with the same path, pushes to front,
    /// truncates to STORE_CAP. Also prunes non-existent files (keeps the store tidy).
    pub fn record(&mut self, path: PathBuf, now: i64);

    /// Empty the list (Clear Recent Files).
    pub fn clear(&mut self);

    /// Up to `limit` display rows for **existing** files, most-recent first. Stats each file
    /// for size; drops any that no longer exist (silent prune, functional_spec §1.4).
    pub fn display_entries(&self, now: i64, limit: usize) -> Vec<DisplayEntry>;
}

/// A ready-to-render row (no further disk access needed by the UI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayEntry {
    pub path: PathBuf,
    pub name: String,        // file_name
    pub subtitle: String,    // "{size} · {folder}", or just "{size}" if no parent name
    pub relative_time: String,
}
```

### 2.2 Pure formatters (unit-tested directly)

```rust
/// Human file size: B / KB / MB / GB, 1 decimal for >=10 units where sensible
/// (e.g. 12_582_912 → "12.0 MB"? use: <1KB "N B"; <1MB "N KB"; else "N.N MB"/"N.N GB").
pub fn format_size(bytes: u64) -> String;

/// Immediate parent directory name (e.g. ".../Downloads/x.xlsx" → "Downloads"); "" if none.
pub fn parent_folder_label(path: &Path) -> String;

/// Relative label per functional_spec §1.7. `now`/`then` are Unix secs.
pub fn format_relative_time(now: i64, then: i64) -> String;
```

`format_relative_time` needs calendar fields (same-day, yesterday, weekday, month/day/year).
Implement a small **dependency-free civil-date** helper (Howard Hinnant's
`civil_from_days`): `fn civil(days: i64) -> (year:i64, month:u32, day:u32)` and
`weekday(days) = ((days % 7) + 11) % 7` (0=Mon). Bucket by comparing `now`/`then` civil days.
Month names from a static `["Jan",…,"Dec"]`. Everything is a pure function of the two ints,
so tests inject fixed `now`/`then` — **no wall-clock reads in core**. (`chrono`/`time` are in
the tree but a ~15-line pure helper avoids a direct dependency and is trivially testable.)

### 2.3 Store path (`freecell-app::shell::recents`, uses `dirs`)

```rust
/// `<data_dir>/FreeCell/recents.json`, or None if no data dir (headless/no-HOME).
/// macOS: ~/Library/Application Support/FreeCell/recents.json
/// Linux: $XDG_DATA_HOME|~/.local/share/FreeCell/recents.json
pub fn recents_store_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("FreeCell").join("recents.json"))
}
```

Add `dirs = "6"` to `freecell-app/Cargo.toml` (already resolved at 6.0.0 in the lockfile).
`serde`/`serde_json` added to `freecell-core/Cargo.toml` (both are workspace deps).

## 3. App wiring (`FreeCellApp`)

Add one field: `recents: RecentList`, loaded in `init` via
`recents_store_path().map(RecentList::load).unwrap_or_default()`.

### 3.1 Recording

A single private helper is the choke point:

```rust
fn record_recent(&mut self, canonical: PathBuf, cx: &mut App) {
    let now = /* Unix secs from SystemTime::now() — see note */;
    self.recents.record(canonical, now);
    if let Some(p) = recents_store_path() { let _ = self.recents.save(&p); } // log+swallow err
    self.refresh_recents_ui(cx);
}
```

Called from:

- `do_open_path` — after `path.canonicalize()` succeeds, for **both** `OpenOutcome::Activate`
  and `OpenNew` (a successful canonicalize means the file exists). One record covers the
  Open… panel, CLI argv, welcome-row click, and Open Recent menu.
- `note_window_path(key, path, …)` — the Save/Save-As path-adoption choke point. Record the
  adopted `path` (already canonical from the save flow).

> **Wall-clock note.** `SystemTime::now()` is the only impure input and lives *only* in
> `record_recent` (production `FreeCellApp`), never in `freecell-core`. gpui window tests that
> record can assert ordering/labels via core unit tests + by injecting through a small
> test seam if needed; the app-level tests assert the *entry set* (paths), not exact times.

### 3.2 Pushing updates to the UI

```rust
fn refresh_recents_ui(&mut self, cx: &mut App) {
    // Welcome, if open: hand it fresh display rows.
    if let Some(w) = self.welcome.clone() {
        let rows = self.recents.display_entries(now, WELCOME_LIMIT);
        w.update(cx, |w, cx| w.set_recents(rows, cx));
    }
    // macOS menu bar: rebuild from the current list.
    menus::install_menus_with(&self.recents, cx);
}
```

Also: when `do_show_welcome` first creates the welcome view, seed it with the current
display rows (constructor arg or an immediate `set_recents`).

Clear path: `OpenRecent`/`ClearRecent` global action handlers (registered in `init`
alongside the existing `NewWorkbook`/`OpenFile`/…):

```rust
cx.on_action(|a: &OpenRecent, cx| FreeCellApp::open_recent_index(a.index, cx));
cx.on_action(|_: &ClearRecent, cx| FreeCellApp::clear_recents(cx));
```

- `open_recent_index(i)` — snapshot `display_entries(now, MENU_LIMIT)`, index into it, and
  `open_path(&entry.path)` (guards against a stale index → no-op).
- `clear_recents` — `self.recents.clear()`, persist, `refresh_recents_ui`.

## 4. Welcome view (`WelcomeView`)

Add `recents: Vec<DisplayEntry>` state + `pub fn set_recents(&mut self, rows, cx)` (stores +
`cx.notify()`). `WelcomeView::new` gains the initial rows (or is seeded immediately after
construction by `FreeCellApp`).

`render` becomes the two-pane layout (`ui_design.md §1–3`): a horizontal flex under the
optional titlebar row; left pane (wordmark/tagline/buttons — the existing button wiring
moves in unchanged); right pane (RECENT header + list-or-empty). Each recent row is a
`div().id(...).on_mouse_down`/`on_click` listener calling `FreeCellApp::open_path(row.path)`.
Row hover via gpui `.hover(|s| s.bg(CHROME_BG))`. The About/error modal overlay
(`render_modal`) is unchanged.

Colors: replace `welcome.rs`'s local `BG`/`CARD_BG` etc. with the shared token *values*
(`ui_design.md §0`): `CHROME_BG 0xF3F3F3`, `ACTIVE_TAB_BG 0xFFFFFF`, `HAIRLINE 0xD9D9D9`,
`TEXT 0x1F1F1F`, `MUTED_TEXT 0x555555`.

## 5. Menu (`menus.rs`)

New actions (add to the `actions!` list in `shell/mod.rs`, or a data-carrying action for the
index):

```rust
// zero-data:
ClearRecent
// data-carrying (index into the current MENU_LIMIT display snapshot):
#[derive(…Action…)] struct OpenRecent { index: usize }
```

`OpenRecent` carries a `usize`, so it uses gpui's data-carrying action derive
(`impl_actions!`/`#[derive(Action)]`) rather than the zero-data `actions!` macro. **Risk /
fallback:** if data-carrying menu actions prove unavailable at the pinned gpui rev, fall back
to a fixed set of ten distinct zero-data actions (`OpenRecent0..=OpenRecent9`) generated by a
macro — ugly but bounded (the menu shows ≤10). Decide during Phase 2; this is the one spot
that could trip the "new technical constraint" rule.

`build_menus` gains a `&RecentList` param and constructs the **Open Recent** submenu
(`MenuItem::submenu`) from `display_entries(now, MENU_LIMIT)`:

- non-empty → one `MenuItem::action(name, OpenRecent { index })` per row, then
  `MenuItem::separator()`, then `MenuItem::action("Clear Recent Files", ClearRecent)`.
- empty → one disabled `No Recent Files` item (use gpui's disabled-menu-item affordance;
  if none exists at the rev, a `ClearRecent`-bound item that no-ops on empty is acceptable —
  prefer disabled).

`install_menus_with(recents, cx)` replaces the current `install_menus` (macOS-only
`set_menus(build_menus(recents))`); the existing `install_menus(cx)` call in `init` becomes
`install_menus_with(&app.recents, cx)`.

## 6. Error handling

- Missing/corrupt store file → empty list (§2.1 `load`).
- Store write failure → `tracing::warn!` + swallow (never blocks open/save, never a dialog).
- Stat failure on a row → row dropped (§2.1 `display_entries`).
- Click/menu-open of a file that vanished mid-flight → existing `do_open_path`
  canonicalize-failure dialog ("Couldn't open the file"); the entry is pruned on the next
  record/write.

## 7. Testing strategy

**`freecell-core` unit tests (headless, deterministic — the bulk of coverage):**

- `record`: front-insert, dedupe-by-path moves to front + updates time, cap at `STORE_CAP`,
  ordering preserved.
- `clear`: empties.
- round-trip `to_json`/`from_json`; `from_json` on garbage/empty ⇒ empty (no panic).
- `load` on absent path ⇒ empty; `save` then `load` round-trips (temp dir).
- `display_entries`: limit honored; **missing files pruned** (create temp files, delete some,
  assert dropped); subtitle = `"{size} · {folder}"`; ordering.
- `format_size`: B/KB/MB/GB boundaries.
- `parent_folder_label`: nested path, root, no-parent.
- `format_relative_time`: every §1.7 bucket via injected `now`/`then`, incl. future-clamp,
  yesterday boundary, weekday range, cross-year.
- civil-date helper: spot dates + weekday for known epochs.

**`freecell-app` gpui tests (worker-less, existing `TestAppContext` harness in `app.rs`):**

- Recording on open: `open_path_detached` a temp `.xlsx` ⇒ store contains its canonical path.
- Recording on path adoption: drive the `Saved`→`note_window_path` seam (mirrors
  `saved_adopts_canonical_path_and_closes_after_save`) ⇒ store contains the saved path.
- Dedupe at the app layer: open the same path twice ⇒ one entry.
- `clear_recents` empties the store.
- `WelcomeView::set_recents` → the view exposes N rows; empty ⇒ empty-state predicate true
  (add `#[cfg(test)]` accessors like the existing `has_modal`).
- `menus::build_menus(&list)`: File menu contains an **Open Recent** submenu; item count =
  min(existing, 10); empty list ⇒ the disabled placeholder; extend the existing
  `menu_bar_has_the_three_specced_menus` test.

**Render suite:** the welcome window is **not** part of the pixel render suite (which targets
the `GridView`); this redesign touches no grid/chrome/titlebar render code, so the existing
baselines are unaffected and must still pass (`render_tests.sh test`). Welcome correctness is
covered by the gpui view tests above + the Xvfb smoke launch (`cargo run -p freecell-app`
opens the welcome window). Adding a dedicated welcome render case is possible but out of
scope; noted here as a conscious decision per `CLAUDE.md`.

**Standard gates (every phase):** `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace`,
`cargo deny check` (new `dirs`/`serde_json` direct deps are MIT/Apache — already in-tree).

## 8. Dependencies added

- `freecell-core`: `serde` (derive) + `serde_json` — both workspace deps, add to manifest.
- `freecell-app`: `dirs = "6"` (path resolution) + `serde_json` if it constructs JSON
  directly (it does not — core owns (de)serialization; app only needs `dirs`).
