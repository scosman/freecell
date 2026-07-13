---
status: complete
---

# Phase 2: Fill down / right (⌘D / ⌘R)

## Overview

Add keyboard **Fill Down (⌘D)** and **Fill Right (⌘R)** — the signature spreadsheet
affordance — as **COPY**-fills (not series fills). ⌘D copies the selection's **top row**
down over the rest of the selection; ⌘R copies the selection's **left column** right over
the rest. The drag-fill handle and Fill Series stay deferred.

The op rides the fork's existing `UserModel::auto_fill_rows` / `auto_fill_columns`. Copy
(not series) semantics fall out of seeding from a **single line** (one row for ⌘D, one
column for ⌘R): the fork's `detect_progression` requires ≥2 seed values, so a single-line
seed always falls through to `extend_to` — a copy with relative-reference adjustment for
formulas. Confirmed by reading `/workspace/ironcalc/base/src/user_model/autofill.rs` +
`sequence_detector.rs` (every detector short-circuits on `< 2` values). **No fork change.**

Because the op is a single `auto_fill_*` call, it rides IronCalc's undo history → **one
fill = one undo step**, for free.

Decision **D3.1**: include the single-cell "pull from neighbor" behavior (Excel-expected,
cheap) — ⌘D on a lone cell copies the cell **above**; ⌘R the cell **to the left**; no-op at
row 0 / col 0.

## Pipeline (mirrors the Copy/Cut/ClearCells path, architecture.md §0/§3)

`command_for_key` (`grid/input.rs`) → `GridKeyCommand::FillDown/FillRight` → dispatch in
`GridView::handle_key` (`grid/view.rs`) → `GridEvent::FillDown/FillRight(CellRange)` →
window sink (`shell/window.rs`) → `Command::FillDown/FillRight { sheet, range }`
(`protocol.rs`) → `process_batch` edit bucket + `apply_one` (`worker/run.rs`) →
`WorkbookDocument::fill_down/fill_right` (`document.rs`) → `UserModel::auto_fill_*`.

## Steps

1. **`grid/input.rs`** — add `GridKeyCommand::FillDown` + `FillRight`. In the
   `secondary && !shift` chord block (next to `c`/`x`/`v`/`a`), map `"d" => FillDown`,
   `"r" => FillRight`. (Bare `d`/`r` stay printable; `Shift`+chord stays reserved/unbound.)

2. **`grid/mod.rs`** — add `GridEvent::FillDown(CellRange)` + `FillRight(CellRange)` with a
   doc comment (`functional_spec.md §3`).

3. **`grid/view.rs`** — in `handle_key`'s `match command`, add
   `GridKeyCommand::FillDown => emit GridEvent::FillDown(self.selection().range())` and the
   `FillRight` analog (mirror the `ClearCells` arm).

4. **`shell/window.rs`** — in the `GridEventSink` match, add `GridEvent::FillDown(range)` /
   `FillRight(range)` arms that `client.send(Command::FillDown/FillRight { sheet:
   shared.active_sheet.get(), range: *range })` (mirror `ClearCells`).

5. **`protocol.rs`** — add `Command::FillDown { sheet: SheetId, range: CellRange }` +
   `FillRight { ... }` with doc comments.

6. **`worker/run.rs`**:
   - Add both to the `edit @ (...)` bucket in `process_batch` (~L527).
   - `apply_one`: add arms → `doc.fill_down/fill_right(idx, *range)?` → `AppliedKind::Cell`.
   - `op_of`: add arms → `AppliedOp::Cells { sheet, range }` (fill writes within the
     selection rectangle; **required** — `op_of`'s fallthrough is `unreachable!`).
   - `pre_validate`: add a **merge guard** (`functional_spec.md §3` edge case) — reject with
     `EditRejectedReason::MergedCells` when the selection range intersects a file-loaded
     merge, via a new `blocks_fill` predicate (reuses `merge_guard()` + the existing dialog).

7. **`freecell-core/src/merge_guard.rs`** — add `blocks_fill(merges, target) -> bool`
   (rectangle intersection), with a unit test.

8. **`document.rs`** — add `fill_down(sheet_idx, range)` + `fill_right(sheet_idx, range)`:
   - Compute 0-based `top/bottom/left/right` from `range`.
   - **fill_down:** multi-row (`bottom > top`) → seed = top row (single-row `Area`,
     `height=1`, full width), `to_row = bottom`. Single cell (`bottom==top && left==right`)
     → pull from above: no-op if `top==0`, else seed = the cell above, `to_row = top`.
     Single row with width>1 (`bottom==top && left<right`) → **no-op**.
   - **fill_right:** column analog (seed = left column, `to_column = right`; single cell →
     pull from left, no-op at col 0; single column height>1 → no-op).
   - Convert to IronCalc 1-based coords, build the `Area`, call `auto_fill_rows/columns`.
   - `record_engine_call()` at entry (the file's convention for engine-touching methods).

## Tests

**`document.rs` (engine, `-p freecell-engine`)**
- `fill_down_copies_top_row_not_series`: `A1=1`, select `A1:A5`, `fill_down` → `A2..A5 == 1`
  (copy, **not** `2,3,4,5`).
- `fill_down_adjusts_relative_formula`: `A1="=B1"`, `B1..B5 = 10,20,30,40,50`, select
  `A1:A5`, `fill_down` → `A2 == 20 … A5 == 50` (relative adjust).
- `fill_right_copies_left_column_not_series`: `A1=7`, select `A1:E1`, `fill_right` →
  `B1..E1 == 7`.
- `fill_down_single_cell_pulls_from_above`: `A1=9`, select single `A2`, `fill_down` →
  `A2 == 9`.
- `fill_right_single_cell_pulls_from_left`: `A1=9`, select single `B1`, `fill_right` →
  `B1 == 9`.
- `fill_down_single_cell_top_row_is_noop`: select `A1`, `fill_down` → `A1` unchanged.
- `fill_right_single_cell_first_col_is_noop`: select `A1`, `fill_right` → `A1` unchanged.
- `fill_down_single_row_multi_col_is_noop`: `A1=1,B1=2`, select `A1:B1`, `fill_down` →
  unchanged.
- `fill_down_multi_col_block`: `A1=1,B1=2`, select `A1:B3`, `fill_down` → column A all 1,
  column B all 2.

**`worker/run.rs` (engine)**
- `fill_down_is_one_undo_step`: fill `A1:A3` (`A1=1`) then `Undo` → `A2/A3` empty again
  (one op, single undo restores).
- `fill_over_merge_is_rejected`: over the merged fixture, `FillDown`/`FillRight` whose range
  intersects the merge → `EditRejected { MergedCells }`, `ops_seen` unchanged.

**`grid/input.rs`**
- `fill_chords_map_on_secondary_only`: `⌘D → FillDown`, `⌘R → FillRight`; bare `d`/`r` →
  `None`; `⌘⇧D`/`⌘⇧R` → `None`.

**`merge_guard.rs`**
- extend `merge_guard_predicate` (or a new test) for `blocks_fill` (intersect vs disjoint).

No pixel suite (data op — `functional_spec.md` render-scope table).
