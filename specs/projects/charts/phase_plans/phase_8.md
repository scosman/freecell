---
status: complete
---

# Phase 8: Render line chart in the spreadsheet (`ChartLayer`)

## Overview

Wire the chart pipeline's read side (`freecell-engine::chart::discover_and_parse`) into the app
and paint the discovered charts **in the grid at their anchored position**, floating above the
cells and clipped to the viewport — the make-or-break app-integration slice (architecture §4.2, §5
challenge 1; functional_spec §1; ui_design §1–2).

Concretely, this phase adds a **ChartLayer** to the `GridView`, painted **after cells, before the
header/chrome layers**. For each chart on the active sheet it maps the `twoCellAnchor` (from/to cell
+ EMU offsets) to a **content-local pixel rect** through the grid's existing geometry (so scroll and
variable-geometry "zoom" come free), **culls** off-screen charts, and **dispatches on fidelity**
(`ChartSpec::display_fidelity()`): Faithful/Degraded → the real `chart_element(&Chart)` (Degraded
adds the corner "⚠ May not display as intended" badge, ui_design §2.2); Unsupported → a bordered
**placeholder** with the title + "Unsupported chart type" (ui_design §2.3). Chart **values are
static** this phase — live rebinding is P9, save is P10, perf/lazy-parse is P11.

Exit: opening a real `.xlsx` shows its line chart in place (validated by new in-grid render
baselines: a line chart over cells, a Degraded-badge case, an Unsupported-placeholder case, and a
scrolled-clip case).

**Scope guard (deferred):**
- **Multi-sheet chart placement.** `discover_and_parse` returns charts in worksheet order; this
  phase attaches the discovered charts to the workbook's **active (first) sheet** — correct for the
  single-sheet line fixtures the exit targets. Per-sheet mapping (correlating `xl/worksheets/sheetN`
  → `SheetId`) rides with live binding (P9), which builds the sheet+range resolution anyway.
- **Unsupported charts from a real file.** The P7 load path currently **drops** an unparseable
  (surface/radar/…) chart (documented in `load.rs`), so the real app won't yet surface a placeholder
  for one; the placeholder **render** is built + validated here via a directly-constructed
  Unsupported spec, ready for the P14 load-path upgrade (retain-source + emit Unsupported spec).
  Degraded (3-D→2-D) charts *do* flow through the real load path and render + badge.
- Interaction (select/move/resize) is authoring (P22+); charts here are read-only. The plain chart
  divs register no listeners, so the grid's coordinate-based hit-testing is unaffected.

## Steps

1. **`app/crates/freecell-app/src/grid/chart_layer.rs` (NEW)** — the gpui-free anchor→pixel + cull
   math + the resident render shape:
   - `pub const EMU_PER_PX: f64 = 9525.0` (914 400 EMU/inch ÷ 96 px/inch) and
     `pub fn emu_to_px(emu: i64) -> f64`.
   - `pub trait GridGeometry { fn col_start(&self, col: u32) -> f64; fn row_start(&self, row: u32) -> f64; }`
     — the content-space (pre-scroll) column/row start offsets, a tiny seam so anchor→pixel is
     unit-tested without a `Frame`/gpui.
   - `pub struct ChartRect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }` with
     `pub fn is_offscreen(&self, content_w: f64, content_h: f64) -> bool` (cull if degenerate or
     wholly outside `[0,content_w)×[0,content_h)`).
   - `pub fn anchor_rect(anchor: &Anchor, geom: &impl GridGeometry, scroll_x: f64, scroll_y: f64) -> ChartRect`
     — `x0 = col_start(from.col) + emu_to_px(from.col_off) - scroll_x`, etc.; `w/h` clamped ≥ 0.
   - `pub struct RenderedChart { pub chart: Chart, pub anchor: Anchor, pub fidelity: Fidelity }` +
     `pub fn from_spec(spec: &ChartSpec) -> Self` (clone chart, copy anchor, derive fidelity once —
     "resident Vec<RenderedChart>", architecture §4.2; values static this phase).

2. **`app/crates/freecell-app/src/chart/in_grid.rs` (NEW)** — fidelity dispatch + the two
   non-plot affordances:
   - `pub const COMPAT_WARNING_TEXT: &str = "⚠ May not display as intended"` (ui_design §2.2),
     `pub const UNSUPPORTED_TEXT: &str = "Unsupported chart type"` (ui_design §2.3).
   - `pub enum RenderMode { Chart, ChartWithBadge, Placeholder }` +
     `pub fn render_mode(fidelity: Fidelity) -> RenderMode` (pure, testable dispatch).
   - `pub fn in_grid_chart_element(chart: &Chart, fidelity: Fidelity) -> gpui::AnyElement` —
     Placeholder → `placeholder_element(title)`; Chart/ChartWithBadge → `chart_element(chart)`
     wrapped `.relative().size_full()`, with the corner badge overlaid when Degraded; falls back to
     the placeholder if `chart_element` returns `None` (never a blank hole, functional_spec §1).
   - `placeholder_element(Option<&str>)`: bordered white rect, optional title on top, centered muted
     "Unsupported chart type". `badge`: bottom-right small grey label.

3. **`app/crates/freecell-app/src/chart/mod.rs`** — `pub mod in_grid;` +
   `pub use in_grid::{in_grid_chart_element, render_mode, RenderMode};`.

4. **`app/crates/freecell-app/src/grid/mod.rs`** — `pub mod chart_layer;`.

5. **`app/crates/freecell-app/src/grid/view.rs`** — install + paint the layer:
   - New field `charts: HashMap<SheetId, Vec<RenderedChart>>`; init empty in `new`.
   - `impl chart_layer::GridGeometry for Frame` (`col_start = col_offset`, `row_start = row_offset`).
   - `pub fn set_sheet_charts(&mut self, sheet: SheetId, specs: Vec<ChartSpec>, cx)` — precompute
     `RenderedChart::from_spec` for each, insert (or remove when empty), `cx.notify()`.
   - In `build_grid_layers`, **immediately after the content-layer push** (before the header layer):
     if `self.charts.get(&self.active_sheet)` is `Some`, for each chart map `anchor_rect`, skip when
     `is_offscreen`, build a positioned `div` (content-local x/y/w/h) hosting
     `chart::in_grid_chart_element(&rc.chart, rc.fidelity)`, and push a single clipped layer div
     (`.left(row_header_w).top(COL_HEADER_H).w(content_w).h(content_h).overflow_hidden()`), matching
     the content layer's geometry. When the sheet has no charts, push nothing (zero baseline
     perturbation for existing grid scenes).
   - `#[cfg(test)] pub(crate) fn sheet_chart_fidelities(&self, sheet) -> Vec<Fidelity>` for the view
     test.

6. **`app/crates/freecell-app/src/shell/window.rs`** — wire the load path:
   - New `fn load_charts(&mut self, window, cx)`: return early unless `self.opened_from` is `Some`;
     capture the grid's `active_sheet()`; `cx.spawn_in` → `background_executor().spawn` running
     `freecell_engine::chart::discover_and_parse(&path)` off the UI thread; on success (non-empty)
     `this.update_in` → `grid.set_sheet_charts(sheet, specs, cx)`; log + drop on error (never breaks
     the window, architecture §6).
   - Call `self.load_charts(window, cx)` in the `WorkerEvent::Loaded` arm, after `reconcile_sheets`.

7. **`app/render-tests/src/cases.rs`** — extend the harness for in-grid charts:
   - `RenderCase` gets `pub charts: Vec<ChartSpec>` (default empty) + a `.charts(Vec<ChartSpec>)`
     builder; helper constructors for a Faithful line, a Degraded line (source classifies Degraded),
     and an Unsupported spec (source `surfaceChart`, title-bearing chart).
   - Add grid-chart cases: `grid_chart_line`, `grid_chart_degraded_badge`,
     `grid_chart_unsupported_placeholder`, `grid_chart_scrolled_clipped` (same chart, revealed deep
     so it clips at the content top-left).

8. **`app/render-tests/src/render.rs`** — in `run_render_scene`, inside the grid builder closure,
   compute `active_sheet` before moving `sources` into `GridView::new`, and when `case.charts` is
   non-empty call `view.set_sheet_charts(active_sheet, charts, cx)`.

9. **`app/render-tests/tests/render_suite.rs`** — add the four `grid_chart_*` names to the
   `render_cases!` list (keeps `case_names_match_table` green).

## Tests

- **`chart_layer` (pure, gpui-free):**
  - `emu_to_px_converts_at_96_dpi` — 9525→1, 19050→2, 0→0, negative.
  - `anchor_rect_maps_corners_with_offsets_and_scroll` — uniform mock geometry (100 px cols / 24 px
    rows) + EMU offsets + a scroll offset ⇒ expected x/y/w/h.
  - `is_offscreen_culls_each_side_and_degenerate` — left/right/above/below/zero-area → true;
    overlapping/partially-visible → false.
  - `rendered_chart_from_spec_derives_fidelity` — loaded `<c:lineChart/>` → Faithful, `<c:bar3DChart/>`
    → Degraded, `<c:surfaceChart/>` → Unsupported; anchor/chart copied through.
- **`in_grid` (pure):**
  - `render_mode_maps_each_fidelity` — Faithful→Chart, Degraded→ChartWithBadge, Unsupported→Placeholder.
  - `render_mode_agrees_with_fidelity_predicates` — cross-checks `renders_as_chart` /
    `shows_compatibility_warning`.
  - `warning_and_placeholder_text_match_ui_design`.
- **`GridView` (`#[gpui::test]`):**
  - `set_sheet_charts_stores_and_derives_fidelity` — install a Faithful line + a Degraded line + an
    Unsupported spec on the active sheet; assert `sheet_chart_fidelities` == `[Faithful, Degraded,
    Unsupported]`; setting an empty vec clears them.
- **Render (`render-tests`, new baselines — eyeballed):** `grid_chart_line`,
  `grid_chart_degraded_badge`, `grid_chart_unsupported_placeholder`, `grid_chart_scrolled_clipped`.
  Confirm existing grid baselines (`grid_empty_origin`, `grid_mixed_content`) are **unchanged**.

## Render validation (in-scope: grid/cell/sheet)

This phase paints inside the grid, so it is IN-SCOPE for the pixel suite. During coding, iterate with
the **subset** only (`render_tests.sh test grid_chart_` for the new scenes; `test grid_` to confirm
no base-grid drift). Generate + **eyeball** the four new baselines (`generate --only grid_chart_`),
commit them with the code. The manager owns the deferred full-suite run + CI `render` dispatch.
