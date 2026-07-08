---
status: complete
---

# Phase 1: Recent-files core (`freecell-core::recent`)

## Overview

Phase 1 builds the pure, GPUI-free, IronCalc-free recent-files model and all of its logic
in `freecell-core`. This is the foundation both later phases consume (`freecell-app` store
ownership in Phase 2, welcome-view rows in Phase 3). Per the codebase's pure-vs-GPUI split
(`architecture.md §1`), everything time- or size-dependent is a pure function of injected
inputs — `now` is always passed in as Unix seconds, never read from the wall clock — so the
whole module is deterministically unit-tested headless.

Scope is exactly `architecture.md §2` + the `freecell-core` half of the testing strategy in
`architecture.md §7`:

- `RecentEntry` / `RecentList` data model + serde (de)serialize (`from_json`/`to_json`,
  `load`/`save`).
- `record` — front-insert + dedupe-by-path + cap at `STORE_CAP=10` + prune-missing.
- `clear` — empty the list.
- `display_entries` — stat each file for size, drop missing (silent prune,
  `functional_spec.md §1.4`), build `DisplayEntry` rows most-recent-first, honor a limit.
- Pure formatters `format_size`, `parent_folder_label`, `format_relative_time`
  (`functional_spec.md §1.6–1.7`), backed by a dependency-free civil-date helper
  (Howard Hinnant `civil_from_days`) so no `chrono`/`time` dependency is added.

No `freecell-app` changes (Phase 2/3). No `dirs` (that lives in the app layer, Phase 2).

## Steps

1. **`freecell-core/Cargo.toml`** — add `serde` (workspace, has the `derive` feature) and
   `serde_json` (workspace) to `[dependencies]`; add a `[dev-dependencies]` table with
   `tempfile.workspace = true` (real temp files back the `record`/`display_entries`/`save`
   tests). All three are already workspace deps + in-tree (engine uses them), so
   `cargo deny` stays green and `tests/dependency_rule.rs` is unaffected (it only forbids
   `gpui*`/`ironcalc*`).

2. **`freecell-core/src/lib.rs`** — add `pub mod recent;` (alphabetical, after `publication`)
   and a crate-root re-export line
   `pub use recent::{DisplayEntry, RecentEntry, RecentList};`. Mention the module in the
   crate-level `//!` doc list to match house style.

3. **`freecell-core/src/recent.rs`** — new module. Contents:

   - Module `//!` doc referencing `functional_spec.md §1` + `architecture.md §2`, stating
     the no-wall-clock rule.

   - Constants: `pub const STORE_CAP: usize = 10;`, `pub const WELCOME_LIMIT: usize = 5;`,
     `pub const MENU_LIMIT: usize = 10;`.

   - Types (exact shapes from `architecture.md §2`):
     ```rust
     #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
     pub struct RecentEntry { pub path: PathBuf, pub last_opened: i64 }

     #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
     pub struct RecentList { pub entries: Vec<RecentEntry> }

     #[derive(Debug, Clone, PartialEq, Eq)]
     pub struct DisplayEntry { pub path: PathBuf, pub name: String, pub subtitle: String, pub relative_time: String }
     ```

   - `impl RecentList`:
     - `pub fn from_json(bytes: &[u8]) -> Self` — `serde_json::from_slice(bytes).unwrap_or_default()` (never panics).
     - `pub fn to_json(&self) -> Vec<u8>` — `serde_json::to_vec_pretty(self).unwrap_or_default()` (infallible for UTF-8 paths, documented).
     - `pub fn load(path: &Path) -> Self` — `fs::read` then `from_json`; any error ⇒ default empty.
     - `pub fn save(&self, path: &Path) -> io::Result<()>` — `create_dir_all` the (non-empty) parent, `fs::write` the JSON.
     - `pub fn record(&mut self, path: PathBuf, now: i64)` — retain != path (dedupe), `insert(0, …)` (front), `retain(exists)` (prune missing), `truncate(STORE_CAP)`.
     - `pub fn clear(&mut self)` — `self.entries.clear()`.
     - `pub fn display_entries(&self, now: i64, limit: usize) -> Vec<DisplayEntry>` — lazy `filter_map` (stat via `fs::metadata`; drop on error) building each row (`format_size`, `subtitle`, `file_name_label`, `format_relative_time`), `.take(limit)`.

   - Pure formatters:
     - `pub fn format_size(bytes: u64) -> String` — `<1KB`→`"{n} B"`; `<1MB`→`"{n} KB"` (round to nearest KB); `<1GB`→`"{:.1} MB"`; else `"{:.1} GB"` (binary 1024 units). A value that rounds up to a full 1024 of its unit is promoted to the next unit (never `1024 KB`/`1024.0 MB`).
     - `pub fn parent_folder_label(path: &Path) -> String` — `path.parent().and_then(file_name)`; `""` if none.
     - `pub fn format_relative_time(now: i64, then: i64) -> String` — buckets per `functional_spec.md §1.7`, future clamps to `Just now`.

   - Private helpers:
     - `fn file_name_label(path: &Path) -> String` — lossy file name, `""` if none.
     - `fn subtitle(size_label: &str, folder: &str) -> String` — `"{size} · {folder}"`, or just the size when `folder` is empty (directly unit-testable folder-absent branch).
     - `fn civil_from_days(days: i64) -> (i64, u32, u32)` — Hinnant `civil_from_days` (Unix-epoch days → (year, month, day)).
     - `fn weekday_name(days: i64) -> &'static str` — index `(days.rem_euclid(7) + 4) % 7` into `["Sun",…,"Sat"]` (day 0 = 1970-01-01 = Thursday = index 4).
     - `const MONTHS: [&str; 12]`, `const SECONDS_PER_DAY: i64 = 86_400`.

4. Run the gates (`cargo fmt`, `clippy -D warnings`, `test --workspace`, `build`,
   `cargo deny check`) from `/home/user/freecell/app`; iterate until clean.

## Tests (in `recent.rs` `#[cfg(test)]`, real temp files via `tempfile`)

- `record_front_inserts` — record A then B ⇒ order `[B, A]`.
- `record_dedupes_and_moves_to_front_updating_time` — record A(t1), B(t2), A(t3) ⇒ `[A(t3), B(t2)]`.
- `record_caps_at_store_cap` — record 11 distinct existing files ⇒ len 10, oldest dropped, newest at front.
- `record_prunes_missing_files` — a prior entry whose file was deleted is dropped on the next record.
- `clear_empties` — after `clear`, `entries` is empty.
- `json_round_trips` — `from_json(to_json(list)) == list` for a populated list.
- `from_json_on_garbage_is_empty` — garbage/empty/`{}` bytes ⇒ empty, no panic.
- `load_absent_is_empty` — `load` of a nonexistent path ⇒ empty.
- `save_then_load_round_trips` — `save` to a temp path (with a created parent) then `load` ⇒ equal list.
- `display_entries_builds_rows` — subtitle `"{size} · {folder}"`, `name` = file name, ordering.
- `display_entries_drops_missing` — delete one recorded file ⇒ it is absent from display rows.
- `display_entries_honors_limit` — `limit` caps the row count.
- `format_size_boundaries` — B / KB / MB / GB thresholds and 1-decimal MB/GB.
- `format_size_rolls_over_at_unit_boundaries` — `1 MiB − 1` ⇒ `1.0 MB`, `1 GiB − 1` ⇒ `1.0 GB` (never `1024 KB`/`1024.0 MB`).
- `subtitle_omits_folder_when_absent` — the pure `subtitle` helper: folder present ⇒ `"{size} · {folder}"`, folder empty ⇒ just the size (covers the folder-absent branch of `display_entries`, unreachable via a sandboxed temp dir).
- `parent_folder_label_cases` — nested path, root, no-parent.
- `civil_from_days_known_dates` — day 0 = 1970-01-01, plus a mid-year and a leap-year anchor and a negative day.
- `weekday_name_known_days` — known epochs spot-checked (1970-01-01 = Thu, plus recent days).
- `format_relative_time_buckets` — every `§1.7` bucket via injected `now`/`then`: future-clamp, just-now, `{n}m ago`, `{n}h ago` (same day), Yesterday, weekday range (2–6 days), same-year `{Mon} {D}`, earlier-year `{Mon} {D}, {YYYY}`.
