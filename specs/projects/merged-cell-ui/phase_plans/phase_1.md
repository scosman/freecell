---
status: complete
---

# Phase 1: Re-pin + engine wrapper + resident MergeMap + retire interim guard

## Overview

Wire the merged-cell engine API into FreeCell's worker/cache seam and retire the interim
insert/delete guard, with **no visible UI change**. After this phase the engine can create /
remove / query merges via `Command`s, the resident cache carries the live merge list from the
normalized engine API, the pure merge-query module (`freecell-core/src/merge.rs`) exists for
Phases 2–3, and insert/delete near a merge now **displaces** (no rejection) while fill into a
merge stays rejected.

## Steps

1. **Re-pin (arch §1).** From `app/`: `cargo update -p ironcalc -p ironcalc_base` — advances the
   locked `freecell-fixes` rev `81feec4` → the merge tip `b922df5` (carries
   `UserModel::{merge_cells,unmerge_cells,get_merge_cells,get_merge_cell}`). `Cargo.lock` only.

2. **`freecell-core/src/merge.rs` (new, pure; arch §2).** Free functions over the resident
   0-based regions slice:
   - `region_at(merges, cell) -> Option<CellRange>` — linear scan, region covering `cell`.
   - `anchor_of(merges, cell) -> Option<CellRef>` — `region_at(cell).map(|r| r.start)`.
   - `regions_intersecting(merges, range) -> Vec<CellRange>` — regions intersecting `range`.
   - `expand_to_regions(merges, range) -> CellRange` — fixpoint grow until `range` contains
     every region it touches (chained pull-in).
   - `blocks_fill(merges, target) -> bool` — moved verbatim from `merge_guard.rs`.
   Delete `merge_guard.rs`; drop `blocks_row_op`/`blocks_col_op` (and their tests). Update
   `lib.rs`: `pub mod merge;` (was `merge_guard`) + re-export the query fns (was
   `blocks_col_op`/`blocks_row_op`).

3. **`document.rs` (arch §2/§3/§8).** Add, next to `merge_ranges` (which is **deleted**):
   - `merge_cells(&mut self, sheet, area: CellRange) -> Result<(),String>` — 0-based `area` →
     1-based `(row,column,width,height)` → `self.model.merge_cells`.
   - `unmerge_cells(&mut self, sheet, anchor: CellRef) -> Result<(),String>` →
     `self.model.unmerge_cells(sheet,row,col)`.
   - `merged_regions(&self, sheet) -> Result<Vec<CellRange>,String>` — wraps
     `self.model.get_merge_cells` → 0-based `CellRange`s (the `MergeMap` source; replaces the
     `merge_ranges` A1-string parse).
   - `merge_would_lose_data(&self, sheet, area) -> Result<bool,String>` — sparse scan of the
     worksheet cell map (never `width*height`); `true` iff any non-anchor populated cell inside
     `area` is a non-`EmptyCell`. Import `Cell` from `ironcalc_base::types`.

4. **`worker/protocol.rs` (arch §3).** Add `Command::MergeCells { sheet, area, confirmed }`,
   `Command::UnmergeCells { sheet, anchor }`, `WorkerEvent::MergeNeedsConfirm { sheet, area }`.
   Re-scope the `EditRejectedReason::MergedCells` doc to **fill-only** (variant retained).

5. **`worker/run.rs` (arch §3/§5).**
   - Import `blocks_fill` from `freecell_core::merge` (drop `blocks_row_op`/`blocks_col_op`).
   - Bucket `MergeCells`/`UnmergeCells` into `edits`.
   - `pre_validate`: **remove** the Insert/Delete-rows/cols merge-guard arms; **keep** the
     Fill{Down,Right,Drag} arms. Point the `merge_guard` reader at `doc.merged_regions`.
   - `apply_edit_batch` pre-validate loop: an unconfirmed `MergeCells` whose
     `merge_would_lose_data` is true → emit `MergeNeedsConfirm` and drop it (no mutation, no undo
     step). (Realized here because `apply_one` is emit-free; behavior == arch §3.)
   - `apply_one`: `MergeCells` → `doc.merge_cells` → `Structure`; `UnmergeCells` →
     `doc.unmerge_cells` → `Structure`.
   - `op_of`: both → `AppliedOp::Rebuild { sheet }` (full active-sheet cache rebuild re-reads
     `merged_regions`).

6. **`cache.rs` build loop (arch §3).** Replace the `ws.merge_cells` A1-string parse with
   `doc.merged_regions(sheet_idx)` → `push_merge` per region.

7. **`grid/view.rs` menus (arch §5).** Delete `merge_block_flags` (+ its test); drop the
   `insert_*_blocked`/`delete_*_blocked` fields from `HeaderMenu`/`CellMenu`; remove the
   `cache.merges()` read on right-click + the `merges` param of `open_cell_menu`; make the
   insert/delete items always enabled; delete the "Sheet has merged cells — not yet supported
   here." footnote. Remove `blocks_col_op`/`blocks_row_op` from the imports.

## Tests

Pure (`freecell-core`, `merge.rs`):
- `region_at` / `anchor_of`: covered cell, anchor, outside → region / anchor / None.
- `regions_intersecting`: edge-touch, disjoint, multiple.
- `expand_to_regions`: single region, already-contained no-op, chained pull-in, no-merge identity.
- `blocks_fill`: intersection blocks, disjoint doesn't, empty list never blocks (moved test).

Worker (`freecell-engine`):
- `merge_cells` create → cache `merges()` reflects the new region; covered content cleared.
- `unmerge_cells` → region gone from cache.
- `MergeCells{confirmed:false}` over covered **non-empty** → `MergeNeedsConfirm`, no mutation;
  `confirmed:true` performs it; all-empty / single-value merges silently (no confirm).
- Insert/delete near a file-loaded merge **displaces** (no `MergedCells` rejection) and the cache
  reflects the new region (rewrite of `merge_guard_blocks_and_allows_on_fixture`).
- Fill into a merge still rejected (`fill_into_merge_is_rejected_disjoint_fill_applies` kept;
  `merge_ranges` assertion → `merged_regions`).
- Undo/redo of a merge restores the region + discarded content.
- `merged_regions` 0-based conversion round-trips the fixture `K7:L10`.

Chrome (`freecell-app`): existing menu tests updated (no blocked fields / footnote); insert/delete
items assert always-enabled.
