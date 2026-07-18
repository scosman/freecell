---
status: complete
---

# Freeze Panes

Pin frozen row/column bands so header rows and label columns stay visible while the rest
of the sheet scrolls underneath — Excel/Sheets' "Freeze Panes." A v0.5 table-stakes gap:
frozen header rows are near-universal in real sheets, and a file that has them today
renders as an ordinary scrolling sheet.

## What to build

- **Interaction:** header-driven freeze. Right-click a row or column header → **Freeze**
  pins that track and everything above/left of it; if the clicked track is already the
  current freeze boundary, the item reads **Unfreeze**. (Header-driven, not Excel's
  "freeze at the active cell" variant.) Header context menu only for v0.5 — no View-menu
  presets.
- **Engine wiring (no fork change):** IronCalc already models frozen panes end-to-end and
  round-trips them through `<pane>` on open/save (`UserModel::set_frozen_rows_count` /
  `set_frozen_columns_count`, undoable — round-3 API audit). Work is thin: a `SetFrozen`
  worker command + `document.rs` wrappers, and surfacing the current counts through the
  publication/`SheetCache` so the grid reads them per frame.
- **Grid render (the real work):** split the custom grid's single viewport/scroll into up
  to four quadrants (frozen corner, frozen top band, frozen left band, scrolling body),
  each with its own visible-range + offset math, with scroll clamped so the frozen bands
  never move. Draw the freeze divider line(s). Update header/cell hit-testing,
  scroll-to-reveal, and resize/selection drags to work across the boundary.

## Why it's non-trivial

The grid is a custom GPU viewport with a single content rect + single scroll pair per
sheet; nothing in the render layer knows about frozen regions today. This touches the
grid's most performance-sensitive and most-tested code (render hot path + `layout.rs`
geometry + the render-baseline pixel suite). Self-contained but sizeable (L); engine risk
is low (API exists + round-trips).

## Scope boundaries

- In: render + interaction + persistence for frozen rows and/or columns; one freeze
  boundary per axis (Excel's model).
- Out (later): split panes independent of freeze (v2.0), which will reuse this
  viewport-split machinery.

## Source

Design note: [`projects/freeze-panes.md`](../../../projects/freeze-panes.md);
GAPS.md v0.5 tier row "Freeze panes".
