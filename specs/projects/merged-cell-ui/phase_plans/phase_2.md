---
status: complete
---

# Phase 2: Render merged regions as one box

## Overview

Make the resident `MergeMap` (built in Phase 1) *visible*: a merged region renders as a single
box spanning all its rows/columns — the anchor's content/fill drawn once across the whole span,
interior gridlines suppressed, outer gridlines/borders normal (`functional_spec.md F1`,
`ui_design.md §3`, `architecture.md §6`). The active-cell outline, the range overlay
(translucent fill + 2px border), and the in-cell editor base rect span the whole region box via
`span_rect`. No selection-*motion* change (that is Phase 3) — this phase only makes the overlays
*render* spanning a region when the active/selected cell already resolves into one.

Design safety: every new branch is keyed on `visible_merges`, which is empty for a merge-free
sheet, so every existing (non-merge) baseline is provably byte-identical — only new `merge_*`
scenes exercise the new paths.

## Steps

1. **`GridView::visible_merges` field (`grid/view.rs`).** Add `visible_merges: Vec<CellRange>`
   next to `visible_border_specs`; init `Vec::new()` in the constructor.

2. **Snapshot in `resolve_frame` (~:1095, mirroring `visible_border_specs`).** While the cache
   lock is held, after the visible-style snapshot:
   ```rust
   let visible_range = CellRange::new(
       CellRef::new(rows.start, cols.start),
       CellRef::new(rows.end.saturating_sub(1), cols.end.saturating_sub(1)),
   );
   self.visible_merges = regions_intersecting(cache.merges(), visible_range);
   for region in &self.visible_merges {           // anchor style for an off-screen anchor
       let a = region.start;
       if let Some(style) = cache.render_style(a.row, a.col) {
           self.visible_styles.entry((a.row, a.col)).or_insert(*style);
       }
   }
   ```
   Import `regions_intersecting`, `region_at`, `expand_to_regions` from `freecell_core`.

3. **`cell_index` also indexes off-screen anchors (`build_grid_layers` ~:2852).** So the region
   box can read an off-screen anchor's published value: index a cell when it is in-frame **or**
   it is a `visible_merges` anchor.

4. **Skip region cells in the per-cell loop (~:2877).** At the top of the `for c` body:
   `if self.visible_merges.iter().any(|m| m.contains(CellRef::new(r, c))) { continue; }` — drops
   covered content **and** the anchor's 1×1 draw (the box redraws it), so interior gridlines
   vanish. Also guard `same_fill` so a normal cell never suppresses its gridline against a merged
   neighbour (keeps the region's outer gridline).

5. **Region-box pass (new, right after the cell loop, before the border pass).** For each
   `region` in `visible_merges`: `span_rect(region.rows+1, region.cols+1, frame)` → one
   `cell_element` with the anchor's resolved fill/text/kind/style/font (via a new
   `resolve_cell_paint` helper mirroring the cell loop's resolution), `skip_*_gridline = false`
   (outer gridlines draw). Then draw the anchor's explicit border edges at the box outer
   perimeter (right/bottom always; left/top gated by `no_left_owner`/`no_top_owner` on the
   anchor) — per-cell stored styles, no unified-border synthesis (`architecture.md §6`).

6. **Border pass skips region cells (~:3030).** `continue` for any cell in a `visible_merges`
   region, so covered-cell interior explicit-border edges never draw (the box's own outer border
   is drawn in step 5).

7. **Text-spill: box bounds + no spill into a region (`neighbor_occupancy` ~:846).** A region
   cell reads `Blocked`, so no normal cell spills into a region; covered cells never originate
   spill (skipped in step 4); the anchor's text is bounded by the box (`cell_element` clips) —
   `functional_spec.md F1` "non-wrap clips to the box".

8. **Span the selection overlays (~:3094).**
   - `painted = expand_to_regions(&self.visible_merges, selection.range())` — snap to whole
     regions; use for the range fill rects, the 2px range border, the fill handle corner, and
     the header highlight. Gate the fill/border on the **raw** `selection.range().is_single()`
     (a lone selection on a region shows just the spanned active outline, no range fill).
   - active-cell outline: `region_or_cell_rect(selection.active, frame)` (new helper: the region
     box when `active` is in a region, else `cell_rect`).

9. **In-cell editor base rect (`in_cell_overlay_elements` ~:4963 + `measure_incell_geom`
   ~:4892).** Replace `cell_rect(cell…)` with `region_or_cell_rect(cell, frame)` so the editor
   box + its growth measurement span the region (`ui_design.md §5`).

10. **Scene `merge` support (`render-tests/src/scene.rs`).** Add `merges: Vec<CellRange>` +
    `Scene::merge(range)`; in `build_sources` send `Command::MergeCells { area, confirmed: true }`
    after inputs/styles, before `SetViewport`, so the real engine creates the region and the
    resident cache carries it.

11. **`merge_*` render cases (`render-tests/src/cases.rs`, `architecture.md §10`).** Add the
    baselines listed below; generate + eyeball; commit.

## Tests

- **`freecell-app` crate build + `cargo test -p freecell-app --lib`** stays green (the new
  helpers are pure geometry; existing view tests unchanged).
- **Render `merge_*` baselines** (per `architecture.md §10`; iterate with
  `render_tests.sh test merge_`, full suite deferred to Phase 5):
  - `merge_basic_box` — 2×3 merge, anchor text spanning, interior gridlines suppressed.
  - `merge_fill_center` — 2×2 merge, anchor fill + centered text across the box.
  - `merge_wide_header` — 1×4 header-style merge, bold centered (the file-loaded merged-title look).
  - `merge_active_outline` — single selection on a merge anchor → active outline spans the box.
  - `merge_range_selection` — a multi-cell range spanning a merge → fill + border snap to the region.
  - `merge_scroll_boundary` — a tall merge scrolled so the anchor is off-screen; the box still draws.
