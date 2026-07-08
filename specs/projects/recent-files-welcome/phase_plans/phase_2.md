---
status: complete
---

# Phase 2: App integration + Open Recent menu (`freecell-app` shell)

## Overview

Phase 2 wires the pure Phase-1 `freecell_core::recent` store into the `freecell-app` shell:
`FreeCellApp` owns a live `RecentList`, records into it at the two choke points where a window
successfully associates with a file on disk (open + Save-As path adoption), persists it
best-effort to `<data_dir>/FreeCell/recents.json`, and rebuilds the macOS **File → Open
Recent** submenu whenever the store changes. This is exactly `architecture.md §3` + `§5`, the
manifest change in `§8`, and the `freecell-app` half of the testing strategy in `§7`.

Per the pure-vs-GPUI split, the only impure input the feature adds — the wall-clock read that
turns "opened just now" into Unix seconds — lives in **one** app-level helper
(`shell::recents::now_unix_secs`), never in `freecell-core` (`architecture.md §3.1`). Every
recorded path is already canonical (the open flow canonicalizes before `resolve_open`; the
save flow canonicalizes before adopting), so the store dedupes by exact path equality.

**Startup load, not `init` load (`architecture.md §3`).** `init` installs the global with an
**empty** list plus the resolved store path; the disk read happens once at startup via a
separate `FreeCellApp::load_recents(cx)` call from `main.rs` right after `init`. Keeping the
load out of `init` means gpui tests — which call `init` then reset the store — never read the
real per-user data dir.

**Boundary (welcome view untouched).** This phase does **not** modify `welcome.rs` rendering
— the welcome recents wiring is Phase 3. `refresh_recents_ui` is the seam that Phase 3 extends
to push fresh rows into `WelcomeView::set_recents`; in Phase 2 it rebuilds the **menu only**.

**Design-time risk resolved (data-carrying menu action).** `OpenRecent` carries the file's
**`PathBuf`** (the exact file to open), so it needs gpui's data-carrying action, not the
zero-data `actions!` macro. At the pinned gpui rev this is available via
`#[derive(gpui::Action)]`. The derive normally also requires `serde::Deserialize` +
`schemars::JsonSchema`, but `#[action(no_json)]` drops that requirement (the action is
dispatched only programmatically from the menu bar, never built from keymap JSON), so it needs
only `Clone` + `PartialEq`. The `architecture.md §5` ten-distinct-actions fallback is therefore
**not** needed. Carrying the path (not an index into the self-pruning `display_entries`
snapshot) is what guarantees a click opens *that* file and reaches the vanished-file dialog
(`architecture.md §6`) if it moved — an index could open the wrong file if an earlier recent
file were deleted between menu install and click.

## Steps

1. **`crates/freecell-app/Cargo.toml`** — add `dirs = "6"` to `[dependencies]` (path
   resolution). Already resolved at 6.0.0 in the lockfile, so no new crate enters
   `Cargo.lock`.

2. **`crates/freecell-app/src/shell/mod.rs`** — declare the new module (`mod recents;`) and
   the two new actions:
   - Add `ClearRecent` (zero-data) to the existing `actions!(freecell, [ … ])` list.
   - Add `OpenRecent { path: PathBuf }` as a standalone data-carrying action next to the macro,
     `pub use`-able from `super::`:
     ```rust
     #[derive(Clone, PartialEq, Debug, gpui::Action)]
     #[action(namespace = freecell, no_json)]
     pub struct OpenRecent {
         pub path: PathBuf,
     }
     ```

3. **`crates/freecell-app/src/shell/recents.rs`** (new) — store-path + wall-clock seam:
   ```rust
   /// `<data_dir>/FreeCell/recents.json`, or None if no per-user data dir resolves.
   pub fn recents_store_path() -> Option<PathBuf> {
       dirs::data_dir().map(|d| d.join("FreeCell").join("recents.json"))
   }

   /// Unix seconds now — the ONE wall-clock read in the recents feature (never in core).
   pub(crate) fn now_unix_secs() -> i64 {
       SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
   }
   ```

4. **`crates/freecell-app/src/shell/menus.rs`** — the Open Recent submenu:
   - `build_menus(recents: &RecentList, now: i64) -> Vec<Menu>` (was `build_menus()`): the File
     menu inserts `MenuItem::submenu(open_recent_submenu(recents, now))` **after "Open…"**,
     before the separator.
   - `fn open_recent_submenu(recents: &RecentList, now: i64) -> Menu` from
     `recents.display_entries(now, MENU_LIMIT)`:
     - non-empty → one `MenuItem::action(row.name, OpenRecent { path: row.path })` per row, then
       `MenuItem::separator()`, then `MenuItem::action("Clear Recent Files", ClearRecent)`.
     - empty → a single disabled placeholder
       `MenuItem::action("No Recent Files", ClearRecent).disabled(true)` (gpui's disabled
       affordance; the bound action never fires while disabled).
   - `install_menus_with(recents: &RecentList, cx: &App)` replaces `install_menus`: macOS-only
     `cx.set_menus(build_menus(recents, recents::now_unix_secs()))`.

5. **`crates/freecell-app/src/shell/app.rs`** — `FreeCellApp` wiring:
   - Fields: `recents: RecentList`, `recents_store: Option<PathBuf>`.
   - `init`: install the global with an **empty** list + `recents_store =
     recents::recents_store_path()` (no disk read here — `architecture.md §3`); register the
     `OpenRecent`/`ClearRecent` handlers alongside the existing ones — the `OpenRecent` handler
     is `cx.on_action(|a: &OpenRecent, cx| FreeCellApp::open_path(&a.path, cx))`; the trailing
     `menus::install_menus(cx)` becomes `menus::install_menus_with(&cx.global::<FreeCellApp>().recents, cx)`.
   - `pub fn load_recents(cx: &mut App)`: the startup disk read — if `recents_store` is `Some`,
     `self.recents = RecentList::load(path)` then `refresh_recents_ui`. Invoked from `main.rs`
     right after `init` (production loads once at launch; tests never call it against the real
     path).
   - Choke point `fn record_recent(&mut self, canonical: PathBuf, cx: &mut App)`:
     `self.recents.record(canonical, recents::now_unix_secs())`, `self.persist_recents()`,
     `self.refresh_recents_ui(cx)`.
   - `fn persist_recents(&self)`: if `recents_store` is `Some`, `save` and `tracing::warn!` +
     swallow any error (never blocks / never a dialog — `functional_spec.md §1.5`).
   - `fn refresh_recents_ui(&mut self, cx: &mut App)`: rebuild the menu via
     `menus::install_menus_with(&self.recents, cx)`. Documented seam for Phase 3's welcome
     `set_recents`.
   - Call `record_recent` from: `do_open_path` (after `path.canonicalize()` succeeds, for both
     `Activate` and `OpenNew`), `do_open_path_detached` (test mirror, same spot), and
     `note_window_path` (record the adopted canonical `path`).
   - **`OpenRecent` has no `FreeCellApp` method** — its handler dispatches `a.path` straight to
     `open_path` (the existing dedupe/open/vanished-file-dialog path). There is **no**
     snapshot-index helper.
   - `fn clear_recents(cx)`: `self.recents.clear()`, `persist_recents`, `refresh_recents_ui`.
   - `#[cfg(test)]` accessors: `recents_paths(cx) -> Vec<PathBuf>` and
     `reset_recents_for_test(store: Option<PathBuf>, cx)` (empty the list + point persistence at
     `store`). `boot()` calls `reset_recents_for_test(None, cx)` so every gpui test is hermetic
     (no reads of / writes to the real user data dir — `init` no longer loads, so the reset is
     belt-and-suspenders and lets a test inject a temp store).

   **`crates/freecell-app/src/main.rs`** — call `FreeCellApp::load_recents(cx)` immediately
   after `FreeCellApp::init(cx)` so production still loads the persisted list once at startup.

6. Run the gates from `/home/user/freecell/app` (`cargo fmt --all --check`, `cargo clippy
   --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build
   --workspace`); iterate until clean. (`cargo deny` runs in CI; no new crate enters
   `Cargo.lock`, so it is unaffected.)

## Tests

**`freecell-app` gpui tests (in `app.rs` `#[cfg(test)]`, worker-less `TestAppContext`):**

- `open_records_the_canonical_path_in_recents` — `open_path_detached` a temp `.xlsx` ⇒
  `recents_paths` == `[canonical]`.
- `save_as_path_adoption_records_in_recents` — drive `Saved` → `note_window_path` (mirrors
  `saved_adopts_canonical_path_and_closes_after_save`) ⇒ `recents_paths` contains the saved
  canonical path.
- `reopening_a_file_dedupes_to_one_recent_entry` — open the same path twice ⇒ one entry.
- `clear_recents_empties_the_store` — record one, `clear_recents`, ⇒ `recents_paths` empty.
- `recording_persists_to_the_configured_store_file` — point persistence at a temp store, record
  ⇒ the JSON file exists and `RecentList::load` round-trips the canonical path (exercises the
  app→disk `persist_recents` wiring; parent-dir creation covered by the temp nested path).
- `load_recents_reads_the_configured_store` — pre-seed a store file at an injected temp path,
  point the app at it (`reset_recents_for_test(Some(path))`), assert the app is empty before the
  load and `load_recents` fills it from disk (the startup-load path, hermetic via the temp path).

**`freecell-app` menu tests (in `menus.rs` `#[cfg(test)]`, real temp files):**

- `menu_bar_has_the_three_specced_menus` — `build_menus(&default, now)` names ==
  `[FreeCell, File, Edit]` (extends the existing test to the new signature).
- `file_menu_places_open_recent_after_open` — the File menu's submenu named "Open Recent" sits
  immediately after the "Open…" item.
- `open_recent_lists_existing_files_capped_and_labelled` — a list of N existing temp files ⇒
  submenu has `min(N, MENU_LIMIT)` action items labelled by file name **and each carrying its
  own path**, then a separator, then "Clear Recent Files"; verify capping with N > 10.
- `open_recent_items_carry_the_exact_file_path` — each `OpenRecent` item downcasts to the exact
  `PathBuf` of its row, most-recent-first (guards the "carries path, not index" contract).
- `open_recent_empty_shows_disabled_placeholder` — empty list ⇒ the submenu is a single
  disabled "No Recent Files" item and no Clear item.
- `open_recent_omits_missing_files` — a recorded entry whose file is absent is dropped from the
  submenu (missing-file prune at display time).

**Render suite:** this phase touches no grid/chrome/titlebar render code (menu bar + app
wiring only), so the existing pixel baselines are unaffected; no render dispatch needed for
Phase 2 (`architecture.md §7`).
