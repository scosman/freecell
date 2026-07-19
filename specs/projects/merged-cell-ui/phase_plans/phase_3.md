---
status: complete
---

# Phase 3: Merge-aware selection & editing

## Overview

Make selection + editing treat a merged region as one atomic unit (`functional_spec.md F4–F5`,
`architecture.md §7`). This is the delicate phase: all decision logic is written as **pure
functions in `freecell-core/src/selection.rs`** (unit-tested headless), and the grid input
call-sites (`grid/view.rs`) + the async ⌘-arrow reply (`shell/window.rs`) call into them.

Core invariant: `SelectionModel { anchor, active }` **never stores a covered cell** — any cell
entering `anchor`/`active` is snapped to its region's anchor (top-left). This keeps `active` a
valid edit target (so `SetCellInput { cell: active }` never hits a covered cell) and keeps the
Phase-2 span/outline logic simple. Because `snap_cell`/`effective_range` are identities on a
merge-free slice (`&[]`), every existing non-merge behavior and test is unchanged.

## Steps

1. **`freecell-core/src/selection.rs` — pure helpers.**
   - `pub fn snap_cell(merges: &[CellRange], cell: CellRef) -> CellRef` =
     `anchor_of(merges, cell).unwrap_or(cell)` (covered → region anchor; else identity).
   - `pub fn effective_range(merges: &[CellRange], sel: SelectionModel) -> CellRange` =
     `expand_to_regions(merges, sel.range())` (fixpoint growth to whole regions).
   - `fn exit_region(region, dir, dims) -> CellRef` — the plain-arrow exit corner per `§7`:
     Right→`(r0, c1+1)`, Left→`(r0, c0-1)`, Down→`(r1+1, c0)`, Up→`(r0-1, c0)`, clamped to `dims`.
   - `fn extend_merge_aware(sel, dir, dims, merges) -> SelectionModel` — the shift-extend rule:
     take the moving edge from `effective_range(sel)` on the active side (tie-broken by the motion
     direction when `active == anchor` so a single region grows the correct edge) and step it one
     line. A **grow** step (edge moving away from the anchor) lets `expand_to_regions` swallow a
     region whole; a **shrink** step (toward the anchor) instead moves the contracting edge **to the
     first cell past that region in the step direction** — just clear of the merge on the anchor
     side (`shrink_past_region`, mirroring `exit_region`), excluding the whole merge, clamped so the
     contracting edge never crosses the anchor — otherwise re-expansion re-pulls the region and the
     edge sticks. Then `active = snap_cell(far corner of expand_to_regions(bbox(anchor, stepped)))`,
     keep `anchor`. Reading the effective range each step prevents "sticking" on grow.

2. **`apply_motion` — add `merges: &[CellRange]` param, make Move/Extend merge-aware.**
   - `Motion::Move(d)`: if `region_at(merges, active)` → `exit_region`; else `step`; then
     `single(snap_cell(landing))` (so stepping *into* a region snaps to its anchor).
   - `Motion::Extend(d)` → `extend_merge_aware`.
   - Every other arm (`JumpEdge`/`ExtendEdge` sync fallback, `Page`/`ExtendPage`, `RowStart`,
     `DocumentStart`, …) wraps its landing/active in `snap_cell` to uphold the invariant.
   - Update the existing tests to pass `&[]` (behavior unchanged there).

3. **`freecell-core/src/lib.rs`** — export `snap_cell`, `effective_range`.

4. **`grid/view.rs` — call the pure helpers at the input sites.**
   - Add `fn sheet_merges(&self) -> Vec<CellRange>` (reads `cache.merges()` for the active sheet).
   - `mouse_down_cell`: snap the clicked cell — plain click → `single(snap_cell(cell))`,
     shift-click → `active = snap_cell(cell)`; drive the double-click `OpenInCellEditor` from the
     snapped `selection.active` (never the raw covered cell); arm the drag on `selection.anchor`.
   - `extend_drag_to_point` + the autoscroll drag extension: snap the mapped endpoint cell.
   - `open_cell_menu`: snap the collapse-to-single click (right-click a region selects it).
   - `move_active`: pass `&self.sheet_merges()` to `apply_motion`.
   - `handle_key_down` `ClearCells`: emit `effective_range(&merges, sel)` (clears whole regions →
     the anchor content) instead of the raw bbox.
   - Add `pub fn snap_selection(&self, sel) -> SelectionModel` (snaps both corners) for the window.

5. **`shell/window.rs` — ⌘-arrow reply.** In the `EdgeResolved` handler, snap the built `sel` via
   `self.grid.read(cx).snap_selection(sel)` before applying it to the grid + chrome (an
   edge-of-data jump can land on a covered cell at a sheet edge).

## Tests

Pure (`freecell-core/src/selection.rs`, the core deliverable):
- `snap_cell`: covered → anchor, anchor → itself, non-region → itself, empty slice → identity.
- `effective_range`: single covered cell → whole region; range clipping a region expands; chained
  pull-in; disjoint/contained unchanged.
- `apply_motion` Move **enters** a region from each of the 4 directions → lands on the anchor.
- `apply_motion` Move **exits** a region from each direction → first cell past the far edge.
- Move exit clamps at the grid edge (region flush to a boundary stays put).
- Move with no merges is byte-identical to the old behavior (`&[]`).
- Shift-extend **across** a tall region without sticking (repeat `Extend(Down)` advances past it).
- Shift-extend **shrink-back** past a region without sticking: grow across a tall/wide region then
  contract with the opposite motion, asserting the effective range jumps the whole merge each step
  (all four directions).
- Shift-extend from a single region grows the right edge in each of the 4 directions.
- Shift-extend chained-region pull-in.
- `JumpEdge`/`ExtendEdge` sync fallback snaps a covered landing to its anchor.
- Covered-cell invariant: after Move/Extend from inside a region, neither corner is covered.
- Existing motion/edge tests keep passing with the `&[]` argument.

Grid (`grid/view.rs` gpui view tests, merge-free helpers already exist):
- `mouse_down_cell` on a covered cell → selection is `single(region.anchor)`.
- Shift-click into a region → `active` snaps to the region anchor.
- Double-click a covered cell → `OpenInCellEditor` carries the anchor (edit routes to anchor).
- Keyboard `Delete` over a selection touching a region → `ClearCells` carries the effective range.
