---
status: complete
---

# Phase 3: Text spill / overflow (horizontal)

## Overview

Excel-style horizontal text spill: a wrap-off **text** cell whose rendered text is wider than
its column visually overflows into adjacent **empty** cells instead of clipping at the column
boundary (`functional_spec.md §2`, `architecture.md §2`). Direction follows effective
alignment (general/left → right; right → left; center → both). Spill stops at the first cell
with content (fill/border does NOT stop it), and never spills past the covered region
(`functional_spec.md §2.5`).

The neighbor-scan + direction + width-gate logic is extracted **gpui-free** into
`grid/layout.rs` and unit-tested (no `Window`). The render integration lives in
`build_grid_layers` / the cell loop in `grid/view.rs`: a spilling cell suppresses its own
clipped text and a **separate positioned text element** paints the full text over the spill
rect, `overflow_hidden` to that rect, `whitespace_nowrap`, using the origin cell's
font/color/alignment. Non-spilling cells take the **identical** existing code path so their
pixels are byte-unchanged.

Both rightward (must-have) and left/center (bidirectional) are implemented — the spill-rect
math is symmetric (`span_rect`), so all three ride the same code path.

## Steps

1. **`grid/layout.rs` — pure spill helpers (gpui-free, unit-tested).**
   - `SpillDirection { Right, Left, Both }` + `pub fn spill_direction(align: Align) ->
     SpillDirection` (Left→Right, Right→Left, Center→Both).
   - `Occupancy { Empty, Blocked }` — a neighbor column is `Empty` only when content-free AND
     coverage is known; `Blocked` otherwise (content, being-edited, or coverage unknown).
   - `SpillSpan { left: u32, right: u32 }` (inclusive, always contains origin) + `spills(origin)`.
   - `pub fn spill_span(origin, direction, min_col, max_col, occupancy: impl FnMut(u32) ->
     Occupancy) -> SpillSpan` — scans outward across `Empty` columns, stops at the first
     `Blocked` column or the inclusive `[min_col, max_col]` bound.
   - Width gate: `pub fn estimated_text_width(text, font_px) -> f32` (conservative UNDER-estimate,
     ~0.5em avg glyph advance, so comfortably-fitting text never spills → keeps non-spill pixels
     untouched) and `pub fn text_overflows_column(text, font_px, col_w, h_pad) -> bool`.

2. **`grid/view.rs` — spill decision in the cell loop (`build_grid_layers`).**
   - Add `fn neighbor_occupancy(&self, r, c, publication) -> layout::Occupancy`: `Blocked` if a
     mirror is at `(r,c)`, or `!publication.covers(r,c)`, or `cell_index` has it; else `Empty`.
   - Add `fn font_px_of(style) -> f32` (default `CELL_FONT_PX`; `q/4 * 96/72` for non-zero size)
     mirroring `cell_element`.
   - In the per-cell loop, after resolving `(text, text_color, kind, attr_style, font_family)`,
     compute a spill decision when: `covers_active`, `kind == Text`, `!text.is_empty()`, wrap
     off, cell not mirrored, and `text_overflows_column`. Direction from effective align; scan
     bounded by `[frame.cols.start, frame.cols.end - 1]` via `neighbor_occupancy`. Keep only if
     `span.spills(c)`.
   - When spilling: pass **empty** text to `cell_element` (suppresses its clipped text — no
     double-paint; the origin div still paints fill/border/gridline) and record a `SpillPlan`.
     Non-spilling cells call `cell_element` unchanged.

3. **`grid/view.rs` — paint the spill elements.** After the cell-fill loop and the border loop
   (before the selection overlay), for each `SpillPlan` build a `spill_element` over
   `span_rect(row..row+1, span.left..span.right+1, frame)` and push into `content_children` (so
   it sits above cell fills/gridlines/borders, inside the content-clip wrapper — never escapes
   into headers). `spill_element` mirrors `cell_element`'s text styling (size/color/family/
   bold/italic/underline/strike/v_align/padding + justify per direction) but has **no** fill,
   border, or gridline.

4. **Render cases (`render-tests/src/cases.rs` + macro in `tests/render_suite.rs`).** Add the
   `spill_` cases (see Tests). Leave existing scenes as-is; regenerate + eyeball baselines.

## Tests

Pure unit tests (in `grid/layout.rs`, no `Window`):
- `spill_direction_follows_alignment` — Left→Right, Right→Left, Center→Both.
- `spill_span_extends_right_over_empties_stops_at_content` — rightward run stops at first Blocked.
- `spill_span_extends_left_for_right_aligned`.
- `spill_span_center_extends_both_bounded_each_side`.
- `spill_span_stops_at_scan_bound` — bounded by `max_col`/`min_col` (frame/coverage edge).
- `spill_span_no_empty_neighbor_is_no_spill` — `spills(origin)` false when the adjacent cell is Blocked.
- `estimated_width_fits_short_text_overflows_long` + `text_overflows_column` boundary (a snug
  short label does NOT overflow; long text does).

Render cases (`spill_` prefix — eyeballed to show continuous text crossing gridlines):
- `spill_right_over_empties` — long general/left text spills right over empties.
- `spill_left_right_aligned` — right-aligned long text spills left.
- `spill_center_both` — center-aligned long text spills both ways (blockers each side).
- `spill_stop_at_nonempty` — spill stops at an occupied neighbor.
- `spill_over_fill_only_neighbor` — a filled but content-less neighbor does NOT stop the spill.
- `spill_wrap_on_no_spill` — wrap-on long text wraps in cell, no spill.
- `spill_number_no_spill` — a long number clips, no spill.
- `spill_stop_at_coverage_edge` — `.publish(.., 0..3)` so spill stops at the coverage edge
  (uncovered empty-looking cells are NOT spilled over — `functional_spec.md §2.5`).
