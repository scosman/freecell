# Non-Adjacent (Disjoint) Cell Selection — Cmd/Ctrl+Click

**Status:** Future — deferred from `feature-gaps-7-11` planning (2026-07-12) as too large
for that "small gaps" batch.

## Goal

Cmd/Ctrl+click (macOS Cmd, Windows/Linux Ctrl) adds a cell/range to the current
selection as a **separate area**, so the user can select N non-adjacent cells/ranges and
then apply one operation to all of them — the classic use case is "select 8 scattered
cells and make them bold / clear them / delete." Matches Excel's multi-area selection.

## Why it's deferred (it's a core refactor, not a UI tweak)

FreeCell's selection is a **single contiguous rectangle** today:

- `SelectionModel { anchor: CellRef, active: CellRef }` — `freecell-core/src/selection.rs:64`.
  One rect via `range() -> CellRange::new(anchor, active)`; full rows/cols are just a rect
  that spans the sheet. There is **no representation for a second area**.
- Stored per sheet in the grid: `selection: HashMap<SheetId, SelectionModel>`
  (`grid/view.rs:196`); mirrored on `ChromeView` and cached on the window.

Adding disjoint areas means introducing something like `Vec<CellRange>` + a designated
**active** area/cell, and it ripples into essentially every selection consumer:

- **Rendering** — selection overlay painting assumes one rect (`build_grid_layers` /
  selection layer, `grid/view.rs`); must paint N rects + one active-cell outline.
- **Keyboard motion** — `apply_motion` (`selection.rs:193`) is pure over the single
  `(anchor, active)` pair; arrow/extend/jump semantics on a multi-area selection need a
  defined model (Excel collapses to the active area on a plain arrow).
- **Mouse** — `mouse_down_cell` (`grid/view.rs:1191`) currently reads only `.shift`; the
  disjoint path branches on `event.modifiers.secondary()` and must append an area + set a
  new drag anchor. Header multi-select (`select_column`/`select_row`) is single-run too.
- **Clipboard** — copy/cut/paste operate on `selection().range()`; Excel **refuses** copy
  of a non-contiguous selection ("that command cannot be used on multiple selections")
  unless the areas share a full row/col band. Needs an explicit guard.
- **Formatting / clear / delete** — every `SetStyleAttr`/clear op takes the single range;
  must fan out over all areas (this is the actual payoff of the feature).
- **Formula/data row** — `DataRowEvent::SelectionChanged { single: bool }` only knows
  single-vs-range; multi-area editing targets the **active cell only** (Excel).
- **Ref box** — `format_selection_ref` (`selection.rs:132`) formats one A1 range.

## Sketch (when picked up)

1. **Core:** extend `freecell-core` selection to `{ areas: Vec<CellRange>, active: CellRef }`
   (keep a `single`/`is_single` fast path; preserve `Copy`-ish ergonomics or move to a
   cheap clone). Define motion semantics (plain arrow collapses to active area).
2. **Grid render + input:** paint N rects; Cmd/Ctrl+click appends/toggles an area;
   Cmd/Ctrl+drag adds a new rectangular area.
3. **Ops:** fan formatting/clear/delete over all areas (one undo step); target editing at
   the active cell; **block** copy/cut on non-contiguous selections with a friendly message.
4. **Tests:** selection model unit tests, multi-area formatting → single undo, copy-guard,
   render cases for multi-rect overlay.

**Size:** L (core type change with wide blast radius). **Risk:** input-code regressions;
undo granularity; interaction with the just-shipped spill/auto-grow render paths.
No engine/IronCalc change required — this is entirely FreeCell (`freecell-core` + app).
