---
status: complete
---

# Phase 4: Cross-boundary interactions + band publishing

## Overview

Phase 3 built the four-quadrant render + the pure `PaneGeometry` hit-test/reveal/edge-delta/clamp
methods (`grid/layout.rs`), but the mouse handlers still call the free `layout::*` functions —
correct only at `M=K=0`. This phase wires the frozen-aware `PaneGeometry` methods into every
interaction path (`components/viewport_split.md §3.2–§3.4, §5`) so hit-test, drag-extend, reveal,
edge auto-scroll, and resize hotspots are all region-correct on a frozen sheet, and selection/fill
drags extend continuously across the divider.

It also lands the **band-publishing fix** flagged by Phase 3's review: the worker now **always
publishes the leading `0..M` / `0..K` bands** alongside the body window, so a frozen band shows its
cell VALUES (not just fills/borders) even when the body is scrolled deep past it. This must land
before the Phase 6 `freeze_scrolled_body` baseline (else that baseline would enshrine blank bands).

## Steps

1. **`grid/view.rs` — one input-geometry helper.** Add `fn input_pane_geometry(cache, viewport_w,
   content_h, scroll_x, scroll_y) -> layout::PaneGeometry` (associated fn) that reproduces
   `resolve_frame`'s non-preview band computation: `M/K` off the cache, `frozen_w/frozen_h =
   offset_of(K/M)`, the gutter sized to the deepest visible **body** row, body-relative scroll. Every
   mouse handler builds the pane through this so a hit-test agrees byte-for-byte with the rendered
   frame; `M=K=0` reduces to the pre-freeze gutter/geometry.

2. **Wire the frozen-aware hit-test / cell_at_point into the handlers** (`§3.3`):
   - `handle_mouse_down` (`hit_test`) — build the pane, use `pane.hit_test`; reuse `pane.row_header_w`
     / `pane.content_w` for the chart/fill content-offset math.
   - `handle_right_mouse_down` (`hit_test`) — same.
   - `extend_header_drag`, `extend_drag_to_point`, `update_fill_drag` (`cell_at_point`) — use
     `pane.cell_at_point` so a drag into a band selects that band's cell (continuous drag across the
     divider; the overlay already tiles per-quadrant from Phase 3).

3. **Frozen-aware reveal** (`§3.2`): replace `layout::scroll_to_reveal` with `pane.reveal` in
   `resolve_frame`'s pending-reveal path and in `reveal_and_announce` — no-op on a frozen axis, body
   target aligned below/right of the divider.

4. **Edge auto-scroll over the body sub-rect** (`§3.4`): `current_edge_delta` uses `pane.edge_delta`;
   `autoscroll_tick` uses `pane.edge_delta` + `pane.clamp` (body-relative), and computes the hovered
   cell via `pane.cell_at_point` at the clamped body scroll. A drag into a frozen band no longer
   auto-scrolls; the body's live edges still fire.

5. **Per-region resize hotspots** (`§5`): `resize_hotspots` iterates the union `0..K ∪ body_cols` and
   `0..M ∪ body_rows`, placing frozen-track dividers at their **unscrolled** band position and body
   dividers past the freeze line (clip the body dividers at `row_header_w + frozen_w` /
   `COL_HEADER_H + frozen_h`). Factor the rect+listener into `col_resize_hotspot` /
   `row_resize_hotspot` helpers. `begin_resize`/`autofit_*` are unchanged (index-based). The band
   grows with a frozen-track resize because `resolve_frame`'s `frozen_w/frozen_h` are preview-aware
   (Phase 3).

6. **Band-publishing fix (worker "always publish the leading bands").**
   - `freecell-core/publication.rs`: add `frozen_rows: u32` / `frozen_cols: u32` to `Publication`;
     make `covers()` union-aware (`(rows.contains(r) || r < M) && (cols.contains(c) || c < K)`);
     `empty()` sets them 0. Update the literal test/fixture constructions (all frozen-free → 0).
   - `freecell-engine/worker/run.rs::build_publication`: read `M/K` off the resident cache (0 when
     absent), probe the union of the ≤4 quadrant rectangles via chained ranges (`(0..M).chain(body
     rows floored at M)` × cols), set `Publication.frozen_rows/cols`. O(visible): the bands are a few
     leading tracks. No protocol/event change — the worker is the publication authority and already
     tracks `M/K`.
   - Update the Phase-3 pointer comment at `build_grid_layers`'s `cell_index` union filter to note the
     fix is in.

## Tests

- **core** (`publication.rs`): `covers_includes_frozen_bands` — a band row/col reads covered while a
  scrolled-out non-band track does not; `frozen=0` reduces to the old contains-only membership.
- **engine** (`run.rs`): `publication_always_publishes_frozen_bands_when_body_scrolled_deep` — freeze
  M=2, write a band cell + a deep body cell, scroll the body past the band, assert the band cell is
  published + `covers()` reports it (fails pre-fix).
- **gpui** (`view.rs`):
  - `click_in_top_band_selects_frozen_row_and_scrolled_col` — region routing through `pane.hit_test`.
  - `drag_extends_selection_across_divider_into_band` — `pane.cell_at_point` lands a body-anchored
    drag on a frozen-column cell.
  - `reveal_frozen_row_is_noop_and_body_row_lands_below_divider` — `pane.reveal` no-op on the frozen
    axis; a deep body row reveals into the body quadrant.
  - `frozen_col_resize_grows_band_and_hotspots_build` — a live frozen-col resize grows `frozen_w`;
    `resize_hotspots` builds without panic on a frozen frame.
- Render **subset** while iterating (`render_tests.sh test cell_` / `grid_` / `selection_`) — no
  `freeze_*` baselines (Phase 6).

Checks: `-p freecell-core -p freecell-engine -p freecell-app`, `cargo fmt --all --check`.
