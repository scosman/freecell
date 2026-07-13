---
status: complete
---

# Phase 3: ⌘+arrow → edge-of-data

## Overview

Change `Motion::JumpEdge` (⌘+arrow) and `Motion::ExtendEdge` (⌘⇧+arrow) from jumping to the
**sheet edge** to Excel/Sheets **edge-of-data** semantics. The exact Excel Ctrl+Arrow algorithm is
implemented as a **pure, exhaustively-tested function in `freecell-core`**, but resolved
**worker-side** (architecture D4.1 Option A): the published viewport lacks off-viewport occupancy,
so the grid routes **only** these two motions to an async worker query (`Command::ResolveEdge` →
`WorkerEvent::EdgeResolved`) that feeds real `sheet_data` occupancy to the pure algorithm and
returns the target cell. All other motions stay synchronous and unchanged.

## Steps

1. **`freecell-core/src/selection.rs` — pure algorithm.** Add:
   - `fn edge_of_data_index(pos: u32, forward: bool, len: u32, occupied: impl Fn(u32) -> bool) -> u32`
     — the 1-D Excel Ctrl+Arrow rule on one line of travel. If `pos` is at the boundary edge in the
     direction → stay. If the active cell **and** its neighbour are occupied → advance to the last
     occupied cell of the contiguous run (or the boundary). Otherwise (active empty, or neighbour
     empty) → skip to the next occupied cell, or the boundary edge if none.
   - `pub fn resolve_edge(from: CellRef, dir: Direction, dims: SheetDims, occupied: impl Fn(CellRef) -> bool) -> CellRef`
     — maps a `CellRef`+`Direction` onto `edge_of_data_index` over the active cell's column (Up/Down)
     or row (Left/Right).
   - Update the doc comments on `Motion::JumpEdge`/`ExtendEdge` and the private `edge()` to record
     that edge-of-data is resolved worker-side via `resolve_edge`; `edge()`/`apply_motion` keep the
     sheet-edge behavior as the synchronous fallback (existing pure tests unchanged).
2. **`freecell-core/src/lib.rs`** — add `resolve_edge` to `pub use selection::{…}`.
3. **`protocol.rs`** — import `Direction`; add `Command::ResolveEdge { sheet, from: CellRef, dir:
   Direction, req_id: u64 }` (a pure read; `extend`/anchor stay UI-side) and
   `WorkerEvent::EdgeResolved { req_id: u64, target: CellRef }`.
4. **`worker/run.rs`** — import `Direction`; classify `Command::ResolveEdge` into a new `edge_ops`
   bucket (pure read, alongside `reads`/`stats_ops`); after the edit batch, resolve each via
   `self.doc.resolve_edge(idx, from, dir)` (unresolvable sheet → reply `from`, i.e. no move) and
   emit `WorkerEvent::EdgeResolved`.
5. **`document.rs`** — add `use std::collections::HashSet`; add
   `resolve_edge(&self, sheet_idx: u32, from: CellRef, dir: Direction) -> CellRef`: collect the
   populated indices on the active cell's line from `sheet_data` (like `find_matches`/
   `selection_stats` — O(populated), correct past the viewport), build a `HashSet<u32>` of occupied
   indices (a cell is occupied iff `cell_content` is non-empty), and call
   `freecell_core::resolve_edge` with full-sheet `SheetDims`.
6. **`grid/mod.rs`** — add `GridEvent::ResolveEdge { from: CellRef, anchor: CellRef, dir: Direction,
   extend: bool }`.
7. **`grid/view.rs`** — in `move_active`, intercept `Motion::JumpEdge`/`ExtendEdge` and emit
   `GridEvent::ResolveEdge` (carrying the current selection's `active` as `from`, its `anchor`, and
   `extend`), returning before the synchronous `apply_motion`. Add
   `pub fn set_selection_and_reveal(sel, window, cx)` = `set_selection` + `reveal_and_announce` of
   the active cell (no `SelectionChanged` emit — the window folds the chrome directly, mirroring the
   paste path).
8. **`shell/window.rs`** — add `PendingEdge { req_id, anchor, extend }` (Copy) and
   `edge_seq: Cell<u64>` + `pending_edge: Cell<Option<PendingEdge>>` to `SinkShared`. In
   `make_grid_sink`, handle `GridEvent::ResolveEdge`: allocate a `req_id`, store `pending_edge`, send
   `Command::ResolveEdge`. In `on_worker_event`, add the exhaustive `WorkerEvent::EdgeResolved` arm:
   if `pending_edge.req_id` matches, build the `SelectionModel` (extend → keep anchor; else
   `single(target)`), apply via `grid.set_selection_and_reveal` + `chrome.on_selection_changed` +
   `last_selection`, and clear `pending_edge`. In `route_selection_changed`, clear `pending_edge` so
   a genuine user selection change cancels a superseded in-flight edge jump.

## Tests

- **`selection.rs` (pure, exhaustive)** — `edge_of_data_*`/`resolve_edge_*`:
  - active empty → next non-empty ahead; active empty → boundary when none ahead
  - active occupied + adjacent occupied → last cell of the run; run reaching the boundary → boundary
  - active occupied + adjacent empty → across the gap to the next occupied; none ahead → boundary
  - already at the boundary edge in the direction → no move
  - all four directions map to the right axis (row for Up/Down, col for Left/Right)
  - empty sheet (no occupancy) → boundary edge
- **`document.rs` engine test** — `resolve_edge` over a seeded sheet: from a data cell into a run
  lands on the run's last cell; across a gap lands on the next block; off the end lands on the sheet
  edge; a horizontal case; an empty-column jump → last row.
- **`worker/run.rs` test** — a `Command::ResolveEdge` batch replies `EdgeResolved` with the right
  `req_id` + target and **publishes nothing** (pure read).
