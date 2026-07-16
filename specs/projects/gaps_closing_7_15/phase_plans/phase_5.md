---
status: complete
---

# Phase 5: Hide / Unhide rows & columns

## Overview

Add hide/unhide of rows and columns via the header context menu. A hidden track collapses to
zero visible size (no cell/header/gridline, neighbors abut, can't click into it, but a range
spanning it still includes it), preserves its data + prior size, and **round-trips to `.xlsx`**
(owner decision D4.1). Hide/Unhide are each one undo step.

## MAJOR FINDING — Stage A & B are already satisfied upstream (fork work is a no-op)

The task specified two fork `fix/` branches (`fix/row-hidden-setter`, `fix/column-hidden`) and a
re-pin. **On inspection this is unnecessary: the currently-pinned `freecell-fixes` (rev
`81feec4`) already contains the complete, undoable hide/show API.** Upstream `main` (which the
fork's `main`/`freecell-fixes` track) landed commit `a520f48f` "UPDATE: Adds hide/show
row/column to the API" (Nicolás Hatcher, 2026‑02) **before** this project's research was written.
Verified at rev `81feec4` (`git merge-base --is-ancestor a520f48f origin/freecell-fixes` = YES):

- `types.rs`: **both** `Row.hidden` **and** `Col.hidden` fields exist.
- xlsx import parses `hidden="1"` for **rows** (`worksheets.rs:827`) **and cols** (`:151`).
- xlsx export emits `hidden="1"` for **rows** (`:440`) **and cols** (`:86`).
- `UserModel::set_rows_hidden(sheet, row_start, row_end, hidden)` (`common.rs:1408`) and
  `UserModel::set_columns_hidden(sheet, col_start, col_end, hidden)` (`common.rs:1340`) — each a
  single undoable diff-list (`Diff::SetRowHidden` / `SetColumnHidden`), one undo step.
- `model`/`worksheet`: `set_row_hidden`, `set_column_hidden`, `is_row_hidden`, `is_column_hidden`.

**Consequence:** creating the two `fix/` branches would duplicate code already upstream (nothing
to add; nothing to PR). Per the fork policy the whole point is to get fixes *into* the engine and
upstream — these already are. So **no fork branches are created, `freecell-fixes` HEAD stays
`81feec4`, and the FreeCell pin is unchanged** (Stage B `cargo update` is a no-op). This is a
material deviation from the task, surfaced in the return summary for the manager/owner. All
FreeCell-side work (Stages C+D — the actual user-facing feature) proceeds against the existing
API and is the real deliverable.

## Steps

### Stage C — engine (freecell-engine)

1. **`freecell-core/src/cache.rs`** — teach `SheetCache` the hidden set + zero-size geometry:
   - Add `hidden_rows: BTreeSet<u32>`, `hidden_cols: BTreeSet<u32>` to `SheetCache` and
     `SheetCacheBuilder`; init empty in `new()`.
   - Builder: `push_hidden_row/col`, consuming `hidden_row/col` (fixtures/tests).
   - Accessors: `hidden_rows() -> &BTreeSet<u32>`, `hidden_cols()`, `is_row_hidden(row)`,
     `is_col_hidden(col)`.
   - **Chokepoint:** a free fn `axis_from(count, default, overrides, hidden)` builds the axis from
     the real overrides with every hidden index forced to `0.0`. `build()` + `rebuild_row_axis` +
     `rebuild_col_axis` all route through it, so *every* axis rebuild renders hidden tracks as 0.
     `row_overrides`/`col_overrides` keep the **real** (non-hidden) sizes (so the worker's
     auto-grow/resize mirror stays truthful and D4.3 "hidden ≠ 0px resize" holds — hidden is its
     own set). Unhide restore comes from the engine re-emitting the real size on rebuild.
2. **`freecell-engine/src/cache.rs`** (`build_sheet_cache`): in the `ws.cols` loop push
   `push_hidden_col(c)` for each `c in min..=max` when `col.hidden`; in the `ws.rows` loop push
   `push_hidden_row(r0)` when `r.hidden`. (Independent of the existing `custom_width`/
   `custom_height` branches.)
3. **`freecell-engine/src/document.rs`**: `set_rows_hidden(sheet_idx, row_start, row_end, hidden)`
   and `set_columns_hidden(...)` — thin wrappers over the fork setters (0-based → 1-based `+1`),
   `record_engine_call()`.
4. **`freecell-engine/src/worker/protocol.rs`**: `Command::SetRowsHidden { sheet, start, end,
   hidden }` and `SetColumnsHidden { sheet, start, end, hidden }` (0-based inclusive run).
5. **`freecell-engine/src/worker/run.rs`**: bucket both into the `edit @ (…)` arm; `apply_one` →
   `doc.set_rows_hidden/set_columns_hidden` → `AppliedKind::GeometryOnly` (no eval — hiding never
   changes values); `op_of` → `AppliedOp::Rebuild { sheet }` (full cache rebuild re-reads hidden).

### Stage D — grid (freecell-app/src/grid)

6. **`grid/mod.rs`**: `GridEvent::HideRows { at, count }`, `HideColumns { at, count }`,
   `UnhideRows { at, count }`, `UnhideColumns { at, count }`.
7. **`grid/view.rs`**:
   - `HeaderMenu`: add `hide_blocked: bool` and `unhide_run: Option<(u32, u32)>` (the minimal
     `[first_hidden, last_hidden]` span within the selected run, or `None` when the run has no
     hidden track → Unhide disabled).
   - `open_header_menu` (the right-click handler): under the caches lock also clone
     `hidden_rows`/`hidden_cols` + `dims()`; after resolving `run`, compute:
     - `hidden_in_run` = hidden entries in `run.0..=run.1` (BTreeSet range).
     - `hide_blocked` = `total − hidden_count − run_len + hidden_in_run == 0` (would hide every
       remaining visible track — reachable only via Select-All → Hide, since dims are Excel-max).
     - `unhide_run` = `(first_hidden, last_hidden)` in the run, else `None`. Unhide targets this
       minimal span (not the whole run) so **Select-All → Unhide bounds the engine work to the
       hidden span**, not 1M rows, while staying one undo step.
   - Extract a pure `header_menu_items(&HeaderMenu) -> Vec<(String, bool, GridEvent)>` (mirrors
     `cell_menu_items`) so the item mapping is unit-testable; `header_menu_elements` renders it.
     Append **Hide** ("Hide N rows/columns", disabled = `hide_blocked`, event over the selected
     run) and **Unhide** ("Unhide N rows/columns", enabled iff `unhide_run.is_some()`, event over
     the hidden span). Keep the "Sheet has merged cells" footnote gated on the insert/delete
     block flags only (not hide/unhide).
   - Zero-size rendering needs **no** grid change: the frame clones `cache.axes()`, which already
     carry hidden→0 (step 1), so headers/cells/gridlines/hit-test/scroll all collapse the track.
8. **`shell/window.rs`**: route the 4 new `GridEvent`s → `Command::SetRowsHidden/SetColumnsHidden`
   for the active sheet (Hide → `hidden: true`, Unhide → `hidden: false`).

## Tests

- **core (`freecell-core/src/cache.rs`)**: a builder with `hidden_row`/`hidden_col` reports the
  track size as `0.0`, excludes it from `total_*`, `index_at` never lands on it, and
  `hidden_rows()`/`is_row_hidden` reflect it; a hidden track with a custom override still renders 0
  but keeps its override in `row_overrides()` (restore-truth / D4.3).
- **engine (`freecell-engine`)**: `set_rows_hidden`/`set_columns_hidden` toggle the flag with one
  undo entry each; open a fixture (built via `set_*_hidden` then save→reload) → `build_sheet_cache`
  populates `hidden_rows`/`hidden_cols`; `SetRowsHidden`/`SetColumnsHidden` command → `GeometryOnly`
  + full rebuild + published cache reflects hidden; undo restores. Round-trip: hide → save → reload
  keeps hidden.
- **grid view (`freecell-app`)**: `header_menu_items` — Hide present + enabled for a normal run;
  Hide disabled for a Select-All run; Unhide disabled when no hidden in run, enabled + scoped to
  the hidden span when the run contains a hidden track; right-click routing sets the new
  `HeaderMenu` fields. A hidden track renders zero-size (frame axis size 0) and can't be clicked
  into.
- **Render subset** while iterating (`render_tests.sh test <prefix>`). The hidden-track zero-size
  geometry **will move baselines** — those are **noted for Phase 6** (regen/eyeball/full-suite/CI
  gate deferred there per the spec). Do NOT regen all baselines here.

## Render baselines affected (for Phase 6)

**No existing baseline moves.** The zero-size geometry only changes an axis when the hidden set is
non-empty: `axis_from` **early-returns the byte-identical `Axis::from_overrides`** when `hidden`
is empty, and no existing render fixture hides any track — so every current baseline renders
exactly as before (verified by construction, not just left unrun).

**New case Phase 6 must author + eyeball:** a sheet with a **hidden row + hidden col** (neighbors
abutting; no cell/header/gridline for the hidden tracks), plus optionally a range selection
spanning a hidden track. Listed here so the dedicated Phase-6 render pass (full suite + CI `render`
gate) picks it up and it isn't lost.
