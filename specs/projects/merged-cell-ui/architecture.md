---
status: draft
---

# Architecture: Merged Cell UI

Technical design for the merged-cell UI. Consumes the merged-cell API on the IronCalc
fork; makes merges render, select/edit as one unit, and be created/removed from the app.
All work lands in existing crates — **no new components warrant separate docs**, so this
is a **single architecture doc (1-phase)**.

## Crate touch-map

| Concern | Crate / file | Change |
|---|---|---|
| Engine pin | `app/Cargo.toml` / `Cargo.lock` | re-pin `freecell-fixes` to the merge-carrying tip |
| Engine wrapper | `freecell-engine/src/document.rs` | `merge_cells` / `unmerge_cells` / `merged_regions` |
| Command/event | `freecell-engine/src/worker/protocol.rs` | `Command::{MergeCells,UnmergeCells}`, `WorkerEvent::MergeNeedsConfirm` |
| Apply + guard | `freecell-engine/src/worker/run.rs` | apply arms; drop insert/delete guard; keep fill guard |
| Resident merge state | `freecell-engine/src/cache.rs`, `freecell-core/src/cache.rs` | build `MergeMap` from `get_merge_cells` |
| Merge logic (pure) | `freecell-core/src/merge.rs` (new) | region lookups + fixpoint snap + fill predicate |
| Selection | `freecell-core/src/selection.rs` | merge-aware `apply_motion`, `snap_cell`, `effective_range` |
| Render | `freecell-app/src/grid/view.rs` | region-draw pass, skip covered, span outlines/editor |
| Control | `freecell-app/src/chrome/view.rs` | action-row toggle + `toggle_merge` + data-loss dialog |
| Menu/shortcut | `freecell-app/src/shell/menus.rs`, `shell/window.rs` | Edit-menu item + ⌃⌘M + modal wiring |

## 1. Re-pin (Phase 1, first step)

FreeCell pins IronCalc by **branch** (`freecell-fixes`), so "newer branch version" is a
`Cargo.lock` bump, not a `Cargo.toml` edit. From `app/`:

```
cargo update -p ironcalc -p ironcalc_base   # advances the locked freecell-fixes rev to the merge tip (b922df5)
```

Verify the locked rev carries the merge API (`git grep merge_cells` in the vendored source
is unnecessary — a `cargo build -p freecell-engine` referencing `self.model.merge_cells`
compiles only against the new tip). This is the first commit of Phase 1.

## 2. Data model — the resident `MergeMap`

The UI must answer "is this cell in a region / which region / does this range hit a region"
**synchronously** on the render + input threads (no per-keystroke worker round-trip). The
authoritative merge list therefore lives in the **resident cache** (`Arc<RwLock<SheetCaches>>`),
refreshed on every publish — the same channel that already carries styles/geometry.

Today `freecell-core/src/cache.rs` stores `merges: Vec<CellRange>` (0-based, file-loaded, for
the guard). We keep the storage a `Vec<CellRange>` (merge counts are small — a few hundred;
no per-cell index, which would blow up on a whole-column merge) and add the query logic as
**pure free functions** in a new `freecell-core/src/merge.rs`, mirroring the existing
`merge_guard.rs` free-predicate pattern. `merge.rs` **absorbs** `blocks_fill` from
`merge_guard.rs`; `blocks_row_op`/`blocks_col_op` are **deleted** (§5).

```rust
// freecell-core/src/merge.rs  — all take the resident 0-based regions slice
/// The region covering `cell` (anchor start = region.start), or None.
pub fn region_at(merges: &[CellRange], cell: CellRef) -> Option<CellRange>;
/// region_at(cell).map(|r| r.start) — the anchor a covered cell edits/selects to.
pub fn anchor_of(merges: &[CellRange], cell: CellRef) -> Option<CellRef>;
/// Regions that intersect `range` (⩝ for toggle/unmerge/data-loss).
pub fn regions_intersecting(merges: &[CellRange], range: CellRange) -> Vec<CellRange>;
/// Fixpoint: grow `range` until it fully contains every region it touches (§7).
pub fn expand_to_regions(merges: &[CellRange], range: CellRange) -> CellRange;
/// Fill (⌘D/⌘R) target intersects any region → reject (moved from merge_guard).
pub fn blocks_fill(merges: &[CellRange], target: CellRange) -> bool;
```

`region_at` is a linear scan; render bounds its cost by scanning only `visible_merges` (§6).
The cache keeps `merges()`/`push_merge`/`merge` (fluent, used by fixtures) unchanged.

**Coordinate conversion.** Engine `MergeCell { row, column, width, height }` is **1-based**
`(row, column)` with `width`=cols, `height`=rows. FreeCell `CellRange` is **0-based**. The
one conversion site is `Document` (§3):

```
CellRange::new(
  CellRef::new((row-1) as u32, (column-1) as u32),
  CellRef::new((row-1+height-1) as u32, (column-1+width-1) as u32),
)
```

## 3. Engine wrapper + command/event plumbing

The UI never calls `Document` directly — it sends `Command`s to the worker, which mutates
`Document` in `apply_one` and republishes. Add:

**`document.rs`** (next to `merge_ranges`, using `record_engine_call()`):
```rust
pub(crate) fn merge_cells(&mut self, sheet: u32, area: CellRange) -> Result<(), String>;   // area 0-based; converts to 1-based (row,col,width,height) and calls self.model.merge_cells
pub(crate) fn unmerge_cells(&mut self, sheet: u32, anchor: CellRef) -> Result<(), String>; // calls self.model.unmerge_cells(sheet, row, col)
pub(crate) fn merged_regions(&self, sheet: u32) -> Result<Vec<CellRange>, String>;          // wraps self.model.get_merge_cells → 0-based CellRanges
pub(crate) fn merge_would_lose_data(&self, sheet: u32, area: CellRange) -> Result<bool, String>; // §8
```
`merged_regions` **replaces** the cache's raw-string `merge_ranges` parse — the resident map
now comes from the normalized engine API, so it reflects post-displacement truth.

**`worker/protocol.rs`:**
```rust
Command::MergeCells   { sheet: SheetId, area: CellRange, confirmed: bool }
Command::UnmergeCells { sheet: SheetId, anchor: CellRef }
WorkerEvent::MergeNeedsConfirm { sheet: SheetId, area: CellRange }   // data-loss round-trip (§8)
```
`EditRejectedReason::MergedCells` is **retained but re-scoped to fill only** (§5, §9).

**`worker/run.rs` — `apply_one`:**
- `Command::MergeCells { sheet, area, confirmed }`:
  - if `!confirmed && doc.merge_would_lose_data(idx, area)?` → emit `WorkerEvent::MergeNeedsConfirm { sheet, area }`, return `AppliedKind::NoOp` (no mutation).
  - else → `doc.merge_cells(idx, area)?` → `AppliedKind::Structure`.
- `Command::UnmergeCells { sheet, anchor }` → `doc.unmerge_cells(idx, anchor)?` → `AppliedKind::Structure`.
- `AppliedKind::Structure` already triggers eval + full republish, which **rebuilds the cache
  and re-reads `merged_regions`** — so the resident `MergeMap` and all cell values refresh in
  one frame (satisfies F7). Merge clears covered content → eval is correct via Structure.

**`cache.rs` (engine build loop, ~:408):** replace the `ws.merge_cells` string parse with
`doc.merged_regions(sheet)` → `push_merge` per region.

## 4. Undo/redo

No new UI code: merge/unmerge are single IronCalc history steps (engine-guaranteed).
`Command::Undo/Redo` already round-trip and republish → the `MergeMap` and discarded content
restore together. Add worker regression tests (§10), not new plumbing.

## 5. Retire the interim guard (structural edits now displace)

The engine displaces merges across insert/delete (grow/shrink/drop, never split), so:

- **`worker/run.rs` `pre_validate`:** remove the `merge_guard` arms for `InsertRows`,
  `DeleteRows`, `InsertColumns`, `DeleteColumns`. **Keep** the arms for `FillDown`,
  `FillRight`, `FillDrag` (fill into a merge stays rejected — documented limitation).
- **`freecell-core`:** delete `blocks_row_op` / `blocks_col_op` (and their tests); `blocks_fill`
  moves into `merge.rs`. `merge_guard.rs` is removed (or reduced to a re-export shim).
- **UI menus (`grid/view.rs`):** delete `merge_block_flags` and the insert/delete disable flags +
  the "Sheet has merged cells — not yet supported here." footnote in `header_menu_items` /
  `cell_menu_items` (insert/delete items now always enabled). The `cache.merges()` read on
  right-click that fed the flags is removed.
- **Move rows/columns:** N/A — FreeCell has no such gesture (only sheet-tab reorder), so the
  engine's split-move `Err` is unreachable.

## 6. Rendering (grid/view.rs)

**Snapshot (in `resolve_frame`, mirroring `visible_border_specs` at ~:1103):**
```rust
self.visible_merges = merge::regions_intersecting(cache.merges(), visible_range);  // Vec<CellRange>
```
So the per-frame region set is small (regions touching the viewport, incl. anchors scrolled
off-screen).

**Cell loop (`build_grid_layers`, ~:2906):** skip any `(r,c)` that belongs to a region:
`if visible_merges.iter().any(|m| m.contains((r,c))) { continue; }`. This removes covered
content **and** the covered cells' right/bottom gridline edges → interior gridlines vanish.

**Region pass (new, after the cell loop):** for each `region` in `visible_merges`, emit **one**
`cell_element`-equivalent at `span_rect(region.rows, region.cols, frame)`:
- fill = anchor's `RenderStyle` fill; content = anchor's publication value, formatted per anchor
  style; h/v-align from the anchor (no centering added); font/wrap from anchor.
- draws the region box's **outer** right/bottom gridline (the box element's own borders);
  the box's left/top gridlines are drawn by the normal neighbor cells above/left, as today.
- `span_rect` handles an off-screen anchor (negative offset) with normal viewport clipping.

**Explicit borders (second pass, `border_spec_at`):** skip interior covered edges; draw the
anchor's explicit border edges at the box outer edge. (Per-cell stored styles only — no
unified-border synthesis; F1/UI §3 scope.)

**Text-spill pass (~:3105):** a region anchor's available width is the **box** width; covered
cells never originate spill.

**Selection overlays (~:3124–3161):**
- active-cell outline: `let r = merge::region_at(cache.merges(), sel.active).unwrap_or(single(sel.active)); span_rect(r)`.
- range fill + 2px range border: driven by `effective_range(sel, merges)` (§7) — already whole
  regions, so no partial slivers.

**In-cell editor (`in_cell_overlay_elements`, ~:5022):** base rect = `span_rect(region_at(cell))`
when `cell` is in a region (it will be the anchor — §7), else `cell_rect(cell)`; `incell_geom`
growth seeds from that base.

## 7. Selection & editing (freecell-core/selection.rs + grid input)

**Invariant:** `SelectionModel { anchor, active }` never stores a **covered** cell — any cell
entering `anchor`/`active` is snapped to its region anchor. This keeps `active` a valid edit
target and the outline/box logic simple.

**New pure helpers (take `merges: &[CellRange]`):**
```rust
fn snap_cell(merges, cell) -> CellRef            // region_at → region.start, else cell
fn effective_range(merges, sel) -> CellRange     // expand_to_regions(bounding_box(anchor, active))
```

**`expand_to_regions` (fixpoint):**
```
loop { changed=false;
  for m in merges { if m intersects range && !range.contains(m) { range = bbox(range ∪ m); changed=true } }
  if !changed break }
```
Grows monotonically, bounded by the sheet → terminates; O(n²) worst, n small. Chained pull-in
(a region added at a new edge pulls the next) is covered by the outer loop.

**`apply_motion(sel, motion, dims, merges)` — merge-aware:**
- *Plain arrow* from `active` (a region anchor of region R, or a lone cell):
  - if in region R spanning rows[r0..r1]×cols[c0..c1]: exit past the far edge —
    Right→(r0,c1+1), Left→(r0,c0−1), Down→(r1+1,c0), Up→(r0−1,c0); else normal single step.
  - clamp to `dims`; then `active = snap_cell(landing)`; `anchor = active` (collapse).
- *Shift-extend* (grow the selection): compute the moving edge from `effective_range(sel)`
  on the `active` side; step it one line in `motion` (clamped); set `active` to the snapped
  far corner (`snap_cell`) of `expand_to_regions(bbox(anchor, stepped_corner))`; keep `anchor`.
  Re-snapping each step prevents "sticking" inside a tall region (the edge advances off the
  effective range, not off the anchor).
- ⌘+arrow (edge-of-data) keeps its async `ResolveEdge` path, then snaps the resolved cell.

**Click / drag (`grid/view.rs`):**
- plain click → `single(snap_cell(clicked))`; shift-click / drag → `anchor` kept,
  `active = snap_cell(cell_at_point)`. `cell_at_point` (layout) is unchanged; snapping is
  applied at the call sites in `mouse_down_cell` / `extend_drag_to_point`.

**Editing commit:** `active` is already the anchor, so `Command::SetCellInput { cell: active }`
never targets a covered cell. Type-to-edit / F2 / double-click all begin on `active`.

**Clear (Delete):** `ClearCells` over `effective_range(sel)` → engine `range_clear_contents`
(clears anchor content, does **not** unmerge; covered cells already empty). No special-casing.

**Ref/formula box:** shows `active` (the anchor) A1 via existing `format_selection_ref`.

## 8. Merge/Unmerge control + data-loss flow (chrome/view.rs)

**Action-row button** (in the alignment/wrap group, after wrap + an `action_divider()`), built
with the existing `toggle` closure; `on_click → this.toggle_merge(window, cx)`.
- `merge_active()` (mirrors `bold_active()`): `!regions_intersecting(merges, effective_range(sel)).is_empty()` → pressed.
- `disabled = self.degraded || (sel is 1×1 && region_at(active).is_none())`.
- tooltip: pressed → "Unmerge cells", else "Merge cells".

**`toggle_merge`:**
```
commit pending edit;
let range = effective_range(sel, merges);
let hit = regions_intersecting(merges, range);
if !hit.is_empty() {                      // UNMERGE all intersecting
    for r in hit { client.send(Command::UnmergeCells { sheet, anchor: r.start }); }
} else if range not 1x1 {                  // MERGE
    client.send(Command::MergeCells { sheet, area: range, confirmed: false });
}
```
**Data-loss round-trip:** the worker answers `MergeCells{confirmed:false}` with either the
merge (no loss) or `WorkerEvent::MergeNeedsConfirm { sheet, area }`. `window.rs` handles that
event by opening the new **`ActiveModal::Confirm`** (title/body/buttons per UI §6); **Merge**
resends `Command::MergeCells { area, confirmed: true }`; **Cancel** dismisses.

`Document::merge_would_lose_data(sheet, area)`: scans the sheet's **populated** cells within
`area` (sparse — via the engine's cell storage, not a dense address walk) and returns `true`
if any non-anchor cell is non-empty. (If `Document` lacks a sparse-in-range iterator, add a
small helper over the worksheet's cell map; never iterate `width*height` addresses — a
whole-column merge would be pathological.)

**Menu + shortcut (`shell/menus.rs`):** `MergeCells` action bound to **⌃⌘M**
(`KeyBinding::new(&key("ctrl-cmd-m"...))` per the repo's key helper) + an Edit-menu
`MergeItem`; the app-level `on_action` calls the same chrome `toggle_merge` (mirrors how
`ToggleBold` routes through `window.rs`).

## 9. Error handling

| Situation | Surface |
|---|---|
| Merge would discard data | `ActiveModal::Confirm` (Merge / Cancel) via `MergeNeedsConfirm` |
| Merge validation `Err` (array/spill collision) | `ActiveModal::Error` (OK), engine reason, no change |
| Fill into a merge | `ActiveModal::Error` (OK), re-worded "Can't fill merged cells" (`MergedCells` reason) |
| Insert/delete near merge | **no dialog** — succeeds via displacement (guard removed) |
| Unmerge on non-merge | engine no-op; toggle never issues it (button inactive) |

The toggle structurally avoids the engine's overlap-reject (it unmerges whenever the selection
hits any region), so `merge_cells` is only ever called on a merge-free rectangle.

## 10. Testing strategy

Match checks to scope (per repo CLAUDE.md — crate-scoped builds; pixel suite is **in scope**
for the region rendering but **not** for the action-row button).

- **Pure unit (`freecell-core`, cheap, no GPUI):** `merge.rs` — `region_at`, `anchor_of`,
  `regions_intersecting`, `expand_to_regions` (single region, chained pull-in, already-contained
  no-op, edge-touch); `selection.rs` — `snap_cell`, `effective_range`, `apply_motion`
  enter/exit a region each direction, shift-extend across a region without sticking, chained-region
  extension, ⌘-arrow snap, grid-boundary clamps.
- **Worker (`freecell-engine`):** MergeCells/UnmergeCells apply + republish refreshes `MergeMap`;
  `MergeNeedsConfirm` fires iff covered non-empty (empty & single-value merge silently);
  `confirmed:true` performs it; insert/delete **displaces** (grow/shrink/drop) with no rejection
  and the cache reflects the new region; fill still rejected; undo/redo restores region + discarded
  content; sheet switch shows per-sheet merges; xlsx round-trip of an **in-app-created** merge
  (create → save → reopen).
- **Render pixel baselines (in scope — grid/cell/sheet):** new `merge_*` cases — basic box
  (content across span, interior gridlines suppressed), box with fill + alignment, active-cell
  outline spanning a region, range selection spanning a region, region at a scroll boundary
  (anchor off-screen), file-loaded merge. Iterate with `render_tests.sh test merge_`; **full**
  suite + CI `render` gate deferred to the dedicated late phase; regenerate + **eyeball**
  baselines for the intentional new rendering.
- **Chrome (`freecell-app` gpui view tests + Xvfb smoke — NOT pixel suite):** button
  enabled/disabled/pressed states across selection kinds; `toggle_merge` decision (merge vs
  unmerge-all vs no-op); ⌃⌘M dispatch; `Confirm` modal copy + Merge/Cancel outcomes.

## 11. Risks & mitigations

- **Merge-aware selection (§7) is the delicate area** the backlog flagged (fixpoint + input
  code). Mitigation: all logic is in pure, unit-tested `merge.rs`/`selection.rs` functions with
  the covered-cell invariant; the grid only calls them. Built + tested as its own phase.
- **Data-loss scan cost** on a huge selection: sparse-in-range iteration only (§8).
- **Off-screen anchor rendering:** the dedicated region pass keyed on `visible_merges` (not the
  cell loop) guarantees a region draws whenever any part is visible.
- **Guard removal correctness:** covered by worker tests asserting displacement + cache refresh;
  the fill guard stays to avoid covered-cell writes.

## 12. Phasing (feeds implementation_plan)

1. **Re-pin + engine/plumbing + resident MergeMap + guard retire** (worker-tested; no visible UI).
2. **Render merges as one box** (region pass, skip covered, span overlays/editor; render subset).
3. **Merge-aware selection & editing** (pure helpers + input call-sites; unit-tested).
4. **Merge/Unmerge control + data-loss dialog** (action row + Edit menu + ⌃⌘M; chrome tests + smoke).
5. **Render validation (dedicated late phase)** — full pixel suite (watchdog) + eyeball/refresh
   `merge_*` baselines + dispatch CI `render` gate green; xlsx round-trip test.
