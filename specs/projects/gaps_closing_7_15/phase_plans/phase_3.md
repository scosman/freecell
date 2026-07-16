---
status: complete
---

# Phase 3: Autofit row height (double-click a row divider)

## Overview

Mirror the shipped autofit-**column**-width gesture (gaps_closing_7_12 Phase 7, commit
e61684a) onto **rows**: double-clicking the divider below a row-number header resizes that
row to fit its tallest populated cell. Single-click-and-drag on the same hotspot still
resizes manually (unchanged) — only the double-click branch is new. Entirely in
`freecell-app/src/grid/view.rs`; reuses the existing `SetRowHeights` command (no engine/fork
work) via `GridEvent::ResizeCommitted { axis: Row, … }`, so autofit is one undo step per row
and marks the row manual (D5.1, resolved default).

Measurement mirrors the wrap auto-grow math (`measure_wrap_height`): fold over every
**populated** cell in the row, measuring each at **its own** column width — wrap-on cells
soft-wrap (gpui `LineWrapper`), wrap-off cells count only explicit `\n` segments — take the
max cell line-box height, clamp to `[DEFAULT_ROW_HEIGHT_PX (24), MAX_AUTO_ROW_HEIGHT_PX
(240)]`. Empty row → default.

## Steps

1. **`grid/view.rs` — extract a shared pure line-box helper.** Add a free function
   `cell_line_box_height(lines: u32, font_px: f32) -> f32` = `lines * round(phi·font_px) +
   vpad`, where `vpad = DEFAULT_ROW_HEIGHT_PX - round(phi·CELL_FONT_PX)` (the single-default-
   line slack). This is exactly the per-cell height `measure_wrap_height` computes today.
   Refactor `measure_wrap_height` (~3067-3101) to call it (drop its local `line_px`/`vpad`),
   so the two measurements never diverge. Pure/Window-free → unit-testable.

2. **`grid/view.rs` — `AutofitRowCell` snapshot struct.** A small private struct holding what
   the row measurement needs per populated cell: `text: SharedString, col_w: f32, font_px:
   f32, bold: bool, italic: bool, font_family: Option<SharedString>, wrap: bool`.

3. **`grid/view.rs` — `measure_row_height(cells: &[AutofitRowCell], window) -> f32`.** Fold
   over the snapshot: `lines` = soft-wrapped count (LineWrapper at `col_w - 2·CELL_H_PAD`,
   summed over `\n` segments) when `wrap`, else `text.split('\n').count()`; `needed =
   max(needed, cell_line_box_height(lines, font_px))` starting from `DEFAULT_ROW_HEIGHT_PX`;
   clamp to `[DEFAULT_ROW_HEIGHT_PX, MAX_AUTO_ROW_HEIGHT_PX]`. Reuses the wrap-on branch shape
   from `measure_wrap_height`.

4. **`grid/view.rs` — `autofit_height_for_row(&self, row: u32, window) -> f32`** mirroring
   `autofit_width_for_column` (~1721): load the publication (guard sheet mismatch → default);
   under the caches read lock, snapshot every populated (`!display_text.is_empty()`) cell
   whose `pc.row == row` into `AutofitRowCell` (resolve `render_style`, `font_px_of`, font
   family from the side table, `cache.col_width(pc.col)`, `wrap`); **drop the lock**; return
   `measure_row_height(&snapshots, window)`. Empty row → `DEFAULT_ROW_HEIGHT_PX`.

5. **`grid/view.rs` — `autofit_row(&mut self, index, window, cx)`** mirroring `autofit_column`
   (~1691): `(start, end) = resize_run_for(RowOrCol::Row, index)`; whole-sheet guard
   `spans_all_rows = start == 0 && end >= MAX_ROWS - 1` → collapse to `(index, index)`; for
   each `row in start..=end` emit `GridEvent::ResizeCommitted { axis: RowOrCol::Row, start:
   row, end: row, px: autofit_height_for_row(row, window) }`. One `SetRowHeights` (one undo
   step) per row.

6. **`grid/view.rs` — row hotspot double-click branch** (~3272-3288): replace the
   unconditional `begin_resize` with the same `match event.click_count { 1 =>
   begin_resize(Row,…), 2 => autofit_row(r,…), _ => {} }` the column hotspot uses (~3254). The
   existing `commit_resize` no-op guard (~1651) suppresses a spurious undo from the double-
   click's first click.

## Tests

- **Unit `cell_line_box_height_matches_default_and_scales`** (pure `#[test]`): `(1,
  CELL_FONT_PX) == DEFAULT_ROW_HEIGHT_PX`; more lines grow monotonically by `round(phi·px)`;
  a larger `font_px` yields a taller box.
- **gpui `autofit_row_single_line_is_default`**: a one-line populated row → default height
  (24) resize emitted for that row only.
- **gpui `autofit_row_explicit_newlines_grow`**: a wrap-off cell with two `\n` (3 visual
  lines) → ~`cell_line_box_height(3, CELL_FONT_PX)`, above default, below the cap.
- **gpui `autofit_row_wrap_on_counts_wrapped_lines`**: a wrap-on cell in a narrow column
  whose text exceeds the width → height reflects >1 wrapped line (taller than a single line).
- **gpui `autofit_row_clamps_at_max`**: a pathological many-line/tall cell → clamped at
  `MAX_AUTO_ROW_HEIGHT_PX` (240).
- **gpui `autofit_empty_row_is_default`**: a row with no published cells → 24.
- **gpui `autofit_multi_row_selection_fits_each`**: divider double-clicked inside a full-row
  multi-row selection → one `ResizeCommitted{Row}` per selected row.
- **gpui `autofit_row_under_select_all_fits_only_divider_row`**: select-all → exactly one
  row resize (the divider's row).
- **gpui `row_hotspot_double_click_autofits_single_click_resizes`** (if cheaply expressible):
  covered by the branch + existing `commit_resize_noop_is_skipped` guard.

## Notes

- Pixel suite: this changes row geometry but autofit is a **user gesture**, not passive
  render, so no existing baseline should move. Verify the relevant render subset stays green
  while iterating; defer full suite + CI gate to Phase 6. Report whether any baseline moved.
- Verify: `cargo build -p freecell-app`, `cargo test -p freecell-app --lib`, `cargo fmt --all
  --check`, run from `app/`. The standalone `freecell` binary can't link here (missing
  `-lxkbcommon`) — verify via lib build + tests. Two pre-existing
  `charts_roundtrip_libreoffice` failures are unrelated.
