---
status: complete
---

# Phase 1: Engine wiring + read model

## Overview

Plumb the frozen pair `(M, K)` end to end **without any geometry/render effect**: the
IronCalc worksheet fields (`frozen_rows` / `frozen_columns`, already round-tripping xlsx
`<pane>`) flow into the `SheetCache` read model at build time, and a new
`Command::SetFrozen` worker command (mirroring `SetRowsHidden`) drives the fork's undoable
`set_frozen_rows_count` / `set_frozen_columns_count` setters. The app side gains the
`GridEvent::SetFrozen` event + window routing so Phase 2's header menu has a seam to emit
into. No fork change, no pixel change (`architecture.md §2`, implementation plan Phase 1).

## Steps

1. **`freecell-core/src/cache.rs` — read-model fields.**
   - `SheetCache`: add `frozen_rows: u32`, `frozen_cols: u32` (beside `hidden_rows`),
     doc-commented as track-index counts with **no geometry effect** (not fed to
     `axis_from`; the pixel extent is Phase 3's `axis.offset_of(M)`).
   - Accessors `pub fn frozen_rows(&self) -> u32` / `frozen_cols(&self) -> u32` beside
     `hidden_rows()`.
   - `SheetCacheBuilder`: matching fields (default `0`), non-consuming setters
     `set_frozen_rows(u32)` / `set_frozen_cols(u32)` (engine build loop), fluent
     `frozen_rows(u32)` / `frozen_cols(u32)` (fixtures), and `build()` copies both counts
     through unchanged.

2. **`freecell-engine/src/worker/protocol.rs` — command.** New variant after
   `SetColumnsHidden`:
   ```rust
   SetFrozen { sheet: SheetId, rows: Option<u32>, cols: Option<u32> }
   ```
   Doc: each `Some` axis rides the fork's undoable setter; the UI sends exactly ONE axis
   per action so it is one undo step; geometry-only (no evaluation); cache rebuilt (it
   re-reads the worksheet counts).

3. **`freecell-engine/src/document.rs` — wrappers.** Beside `set_columns_hidden`:
   ```rust
   pub(crate) fn set_frozen_rows(&mut self, sheet_idx: u32, count: u32) -> Result<(), String>;
   pub(crate) fn set_frozen_columns(&mut self, sheet_idx: u32, count: u32) -> Result<(), String>;
   ```
   Each records `instrument::record_engine_call()` then calls
   `self.model.set_frozen_{rows,columns}_count(sheet_idx, n as i32)`. Defensive clamp: the
   fork's `Model::set_frozen_rows` **errors** at `count >= LAST_ROW` (1,048,576) — i.e. its
   max accepted count is all-but-one track — so clamp to
   `[0, limits::MAX_ROWS - 1]` / `[0, limits::MAX_COLS - 1]` (comment citing the fork
   guard) so a "freeze at the very last track" action degrades to the engine max instead of
   erroring (`functional_spec.md §5.2` tolerates, never blocks).

4. **`freecell-engine/src/worker/run.rs` — dispatch + classification.**
   - `apply_one` (beside `SetColumnsHidden`): resolve idx, call `doc.set_frozen_rows` /
     `set_frozen_columns` for whichever field is `Some`, return
     `AppliedKind::GeometryOnly`.
   - Edit-bucket match (`process_batch`, ~line 596): add `Command::SetFrozen { .. }` to the
     `edits` arm.
   - `op_of` (~line 3750): `Command::SetFrozen { sheet, .. }` → `AppliedOp::Rebuild`
     (rebuild re-reads the worksheet counts, so undo/redo needs no special handling).

5. **`freecell-engine/src/cache.rs` — `build_sheet_cache` reads the counts.** After the
   row/col loops:
   ```rust
   builder.set_frozen_rows(ws.frozen_rows.max(0) as u32);
   builder.set_frozen_cols(ws.frozen_columns.max(0) as u32);
   ```
   (fork fields are `i32`; negative is defensively floored to 0). This is the "open a file
   with `<pane>` shows the bands immediately" path.

6. **`freecell-app` — event + routing.**
   - `grid/mod.rs` `GridEvent`: `SetFrozen { rows: Option<u32>, cols: Option<u32> }` after
     `UnhideColumns` (grid-relative; the window supplies the active sheet).
   - `shell/window.rs` grid-event handler (beside the `HideRows` arm): forward as
     `Command::SetFrozen { sheet: shared.active_sheet.get(), rows: *rows, cols: *cols }`.

## Tests

- **core (`cache.rs`):** `frozen_counts_default_zero_and_round_trip_through_builder` — a
  fresh builder builds with `(0, 0)`; fluent `.frozen_rows(3).frozen_cols(2)` and the
  non-consuming setters both surface through the built cache's accessors; the counts have
  **no geometry effect** (axis totals identical with and without them, and unrelated to the
  hidden mechanism).
- **engine (`worker/run.rs` tests):**
  - `set_frozen_toggles_counts_and_one_undo_restores` — `SetFrozen { rows: Some(3) }` then
    `{ cols: Some(2) }`: the resident cache reports `(3, 2)`; the batch is geometry-only
    (a `StyleCacheUpdated` publish, like the hidden test); **one** `Undo` restores cols to
    0 (leaving rows 3), a second restores rows to 0; `Redo` re-applies.
  - `set_frozen_clamps_to_engine_max` — `rows: Some(u32::MAX)` doesn't error; count clamps
    to `MAX_ROWS - 1`.
  - `pane_fixture_populates_cache_counts_on_open` — inject
    `<pane xSplit="1" ySplit="2" … state="frozen"/>` into a saved empty workbook's
    sheet1.xml (same zip-rewrite pattern as `merged_fixture`), open it via `worker_over` →
    cache reports `(2, 1)` with no user action.
  - `set_frozen_save_reopen_round_trips` — `SetFrozen` both axes, `Command::Save` to a
    temp path, reopen via `WorkbookDocument::open` + `worker_over` → cache counts
    preserved.

## Checks

`cargo build -p freecell-core -p freecell-engine -p freecell-app`;
`cargo test -p freecell-core --lib`, `cargo test -p freecell-engine --lib`;
`cargo fmt --all --check` (whole workspace). Run from `app/`.
