---
status: complete
---

# Phase 7: Autofit column width (double-click the column resize divider)

## Overview

Double-clicking the column resize hotspot/divider (the same divider that drag-resizes a
column) auto-sizes that column to fit its content. Pairs with the shipped drag-resize.

Design anchors (functional_spec §7, architecture §7, decisions D7.1/D7.2/D7.3):

- **D7.1 — multi-column autofit: INCLUDE.** If the double-clicked divider's column is inside a
  multi-column header selection (the same condition `resize_run_for` uses for drag-resize),
  autofit **all** columns in that run — each to its own content; otherwise just the divider's
  column.
- **D7.2 — row-height autofit: DEFER** (column-only this phase).
- **D7.3 — measurement scope: published/overscan cells only (render-thread).** Measure the
  widest cell among the column's currently published cells via `measure_incell_text_width`,
  each at its own resolved font (family/size/bold/italic) from the resident cache. A value
  scrolled beyond the overscan is not measured — a documented limitation.
- **Reuse `SetColumnWidths`** via the existing `GridEvent::ResizeCommitted` — no new worker
  command. Single-column autofit is one undo step + xlsx round-trip, identical to a manual
  resize. Multi-column autofit emits one `ResizeCommitted` per column (each its own width), so
  it is one undo step **per column** — the direct consequence of reusing the per-width command
  with no new worker surface (architecture §7: "no new worker command").

All work is confined to `grid/view.rs` — no engine, protocol, or window changes.

## Steps

1. **Constants** (`grid/view.rs`, near the resize constants ~L55):
   - `const AUTOFIT_PADDING_PX: f32 = 2.0 * CELL_H_PAD + 3.0;` — the cell's left+right text
     padding plus a small buffer so content clears the gridline (keeps the widest published
     cell from spilling; `text_overflows_column` fits when `col_w >= text + 2·CELL_H_PAD`).
   - `const AUTOFIT_MIN_WIDTH_PX: f32 = 24.0;` — the configured floor (D7.3), wide enough to
     keep the column-letter header label readable; an empty column shrinks to it.
   - `const AUTOFIT_MAX_WIDTH_PX: f32 = 800.0;` — cap so one very long value can't run away.

2. **Pure width helper** (free fn, near `measure_incell_text_width`):
   ```rust
   fn autofit_width(max_text_px: f32) -> f32 {
       (max_text_px + AUTOFIT_PADDING_PX).clamp(AUTOFIT_MIN_WIDTH_PX, AUTOFIT_MAX_WIDTH_PX)
   }
   ```
   Extracted so the clamp is unit-testable without a `Window`.

3. **`autofit_width_for_column(&self, col, window) -> f32`** (`impl GridView`, near
   `commit_resize`): load `self.sources.publication`; if it doesn't cover the active sheet,
   return `autofit_width(0.0)`. While holding the caches read lock, snapshot each published
   cell in `col` (non-empty `display_text`) as `(text, font_px, family, bold, italic)` —
   `font_px` via `font_px_of`, `family` from `cache.font_families()` by the style's
   `font_family` index (default when `None`/empty). Drop the lock, then fold
   `measure_incell_text_width` over the snapshots for the max shaped width, and return
   `autofit_width(max)`. (Snapshot-then-measure mirrors `resolve_frame`'s "release the lock
   before shaping".)

4. **`autofit_column(&mut self, index, window, cx)`** (`impl GridView`): `let (start, end) =
   self.resize_run_for(RowOrCol::Col, index);` then for each `col in start..=end`, compute
   `autofit_width_for_column(col, window)` and `self.events.emit(GridEvent::ResizeCommitted {
   axis: Col, start: col, end: col, px })` — one existing command per column (→
   `Command::SetColumnWidths`, undoable, xlsx round-trip).

5. **Double-click wiring** in `resize_hotspots` (the **column** hotspot `on_mouse_down`): branch
   on `event.click_count` — `>= 2` calls `this.autofit_column(c, window, cx)` (no resize
   begins); `1` keeps `this.begin_resize(RowOrCol::Col, c, event, window, cx)`. Row hotspots are
   unchanged (D7.2). `cx.stop_propagation()` as today.

6. **No-op resize guard** in `commit_resize`: return early (no preview freeze, no emit) when
   `rd.current_px == rd.start_px`. A click on a divider with no drag today emits a redundant
   `SetColumnWidths` to the current width (a no-op undo step); on a double-click the first click
   would otherwise push that spurious step just before the autofit. Skipping an unchanged resize
   fixes both and keeps a double-click autofit exactly one undo step (single-column). Existing
   resize tests all change the width, so they are unaffected.

## Tests

- `autofit_width_*` (pure `#[test]`): empty column (`0.0` → `AUTOFIT_MIN_WIDTH_PX` floor); a
  normal measured width → `width + AUTOFIT_PADDING_PX`; a huge width clamps to
  `AUTOFIT_MAX_WIDTH_PX`; a tiny width clamps up to the floor.
- `autofit_column_emits_fit_width_for_published_cell` (`#[gpui::test]`): sources with a known
  text cell in a column; resolve a frame; `autofit_column(col)`; assert a single
  `ResizeCommitted { axis: Col, start==end==col, px }` where `px == autofit_width(measured)` and
  `px > AUTOFIT_MIN_WIDTH_PX` (content wider than the floor).
- `autofit_empty_column_shrinks_to_floor` (`#[gpui::test]`): autofit a column with no published
  cells → `px == AUTOFIT_MIN_WIDTH_PX`.
- `autofit_multi_column_selection_fits_each` (`#[gpui::test]`): select full columns 1..=3,
  `autofit_column(2)` → three `ResizeCommitted` events, one per column (D7.1).
- `commit_resize_noop_is_skipped` (`#[gpui::test]`): committing a resize whose `current_px ==
  start_px` emits no `ResizeCommitted` and freezes no preview.

## Render validation (subset only — Phase 8 owns the full suite + CI gate)

Autofit is interaction-triggered and must not move any static baseline. After coding, run the
resize/variable-geometry **subset** as a regression guard under a ~10-min watchdog
(`render_tests.sh test col_` / `row_resized` / `grid_variable`); expect green with no baseline
changes. If the render env can't be set up in this container, note it and defer to Phase 8. Do
not regenerate baselines; do not run the full suite.
