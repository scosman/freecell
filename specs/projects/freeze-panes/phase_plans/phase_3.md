---
status: complete
---

# Phase 3: Quadrant render + clamp rework

## Overview

The core, pixel-moving change (`components/viewport_split.md §1–§4`, `architecture.md §3`). The
custom grid today has a single content rect + single scroll pair per sheet. This phase splits the
body into up to four **quadrants** (Corner / TopBand / LeftBand / Body), makes the stored scroll
**body-relative**, and draws the freeze divider — while keeping `M = K = 0` byte-for-byte identical
to today (the hard requirement guarding the baseline suite + perf gate).

Cross-boundary **interactions** (mouse hit-test/cell_at_point wiring, reveal-into-body, edge
auto-scroll over the body sub-rect, per-region resize hotspots, selection drags across the divider)
are **Phase 4** — this phase adds the pure `PaneGeometry` helpers + tests but only wires the
`handle_scroll` clamp + scrollbar into the body-relative model. The mouse handlers keep the current
free-function path (identical when `M=K=0`).

## Steps

1. **`grid/mod.rs`** — add `FREEZE_DIVIDER` (`0x9E9E9E`) + `FREEZE_DIVIDER_PX` (`1.5`) consts. (done)

2. **`grid/layout.rs` — `PaneGeometry`** (pure, gpui-free):
   - Struct with `row_header_w, frozen_w, frozen_h, content_w, content_h, body_sx, body_sy`.
   - `body_area() -> (f64, f64)` = `((content_w-frozen_w).max(0), (content_h-frozen_h).max(0))`.
   - `clamp(total_w, total_h) -> (f64, f64)` — body-relative clamp against `total_* - frozen_*`
     and the body area (delegates to `clamp_scroll`).
   - `reveal(row, col, row_axis, col_axis) -> (body_sx, body_sy)` — no-op on a frozen axis
     (`offset_of(index) < frozen_*`), else reveal into the body sub-area via `reveal_axis` re-based
     by `frozen_*` (refactor `reveal_axis` to take a `base` offset; `scroll_to_reveal` passes 0).
   - `hit_test(...) -> GridHit` / `cell_at_point(...) -> CellRef` — region-routing forms
     (`§3.3`); reduce to the free-function results when `frozen_* = body_s* = 0`.
   - `edge_delta(local_x, local_y, step, hotzone) -> (f64, f64)` — `edge_autoscroll_delta` over the
     body sub-rect origin/size (`§3.4`).
   - `v_thumb()/h_thumb()` — `scrollbar_thumb` over the non-frozen extent against the body area
     (`§3.5`).

3. **`grid/view.rs` — `Quadrant` + `Frame`**:
   - `struct Quadrant { rows, cols, dest: (f32,f32,f32,f32), qorigin_x: f64, qorigin_y: f64 }` and
     a `QuadKind` index (Corner=0, TopBand=1, LeftBand=2, Body=3).
   - `Frame` gains `frozen_w: f64`, `frozen_h: f64`, `quadrants: [Option<Quadrant>; 4]`; keeps
     `scroll_x/scroll_y` (now body-relative) + `content_w/content_h` (full body area).

4. **`resolve_frame`** — read `M/K` off the cache; `frozen_w/frozen_h` = preview-aware
   `offset_of(K/M)`; body ranges via re-based scroll (`frozen_* + body_s*`), start floored at `M/K`;
   snapshot `visible_styles` over the **union** (`0..M ∪ body_rows` × `0..K ∪ body_cols`); build the
   ≤4 quadrants; keep `frame.rows/cols` = the body visible range.

5. **`cell_rect`/`span_rect`** take `&Quadrant` (subtract `qorigin_*`, not `frame.scroll_*`).

6. **`build_grid_layers`** — factor the per-cell/border/spill/selection/fill/editor loop into
   `build_quadrant(frame, quad, publication, covers_active, selection, host_overlays)`; `cell_index`
   built once over the quadrant union; one clipped content div per present quadrant at `quad.dest`;
   the in-cell editor + grabbable fill handle hosted in the quadrant containing the active cell
   (fallback body); ChartLayer clipped to the **body** dest (skipped if no body quadrant); freeze
   divider(s) at root after the scrollbar layer; scrollbars over the non-frozen extent.

7. **`handle_scroll`** — clamp the body-relative scroll against `total_* - frozen_*` + body area via
   a `PaneGeometry` build (mirror in `measure_frame`'s clamp).

## Tests

- `layout.rs` pure: `pane_geometry_zero_band_matches_freefns` (M=K=0 equivalence for
  clamp/reveal/hit_test/cell_at_point/edge_delta/thumb); `pane_geometry_rebased_clamp` (body top =
  row M at `body_sy=0`, last row reachable at max, cannot scroll a frozen row into the body);
  `pane_geometry_reveal_frozen_axis_noop` + body target never under the band;
  `pane_geometry_hit_test_regions` (corner/top/left/body + frozen-vs-body header split, scrolled +
  variable geometry); `pane_geometry_edge_delta_body_only`; `pane_geometry_thumb_over_nonfrozen`.
- gpui view (`freecell-app`): `resolve_frame` yields the expected four quadrant ranges for `(M,K)`;
  a body scroll leaves band quadrants at `qorigin=0`/pinned while the body quadrant moves; the
  divider element(s) present iff the axis is frozen; `M=K=0` resolves a single body quadrant.
- Render **subset** while iterating: `render_tests.sh test cell_` / `grid_` (confirm no regression;
  new `freeze_*` baselines are Phase 6).
