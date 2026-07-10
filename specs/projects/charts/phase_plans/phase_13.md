---
status: complete
---

# Phase 13: Axis breadth & line styling (+ two owner fidelity observations)

## Overview

P13 hardens the **line** chart's OOXML fidelity toward Excel. It adds the axis/line OOXML
features the model dropped — axis `scaling` (min/max, reversed), major/minor **gridline
toggles**, `a:ln` stroke **width/color/alpha**, and honored **legend positions** (t/b/l/r/tr) —
and reconciles the derived `fidelity` accessor so the now-rendered features drop out of the
degrade set (scoped to the **line** renderer that honors them; other groups keep degrading).

On top of that it lands the two checkpoint fidelity observations as first-class goals:

- **(A) TRUE 90° rotated value-axis title.** Replace the P6 stacked-character fallback with a
  real rotated title. Achievable at the pinned gpui rev via `gpui::canvas()` + `Window::paint_svg`
  with an **inline SVG `<text transform="rotate(-90 …)">`**: `paint_svg` is the one public painter
  that accepts a rotation and gpui's SVG renderer shapes `<text>` through usvg/resvg + a font DB
  (system fonts — DejaVu/Liberation Sans are present in the render env and installed in CI). No
  gpui/gpui-component bump, no new deps. Typeface is DejaVu/Liberation (not Inter/Calibri) — an
  accepted GAP; the WEIGHT/SIZE/rotation match.
- **(B) Font & line weights toward Excel.** Ground truth from the reference workbook's
  `xl/charts/chart1.xml`: title `sz=1800 b=1` (**18pt bold**), axis titles `sz=1000 b=1` (**10pt
  bold**), tick/legend `sz=1000 b=0` (**10pt**), series line `a:ln w=28440` (**2.24pt**, ~3× the
  `w=9360`/0.74pt gridline). Tune the title to be bigger + bold, axis titles bold, and the default
  series line heavier (Excel ~2.25pt), honoring `a:ln@w` when present.

Scope: **line only.** No other chart type is implemented.

## Ground truth (reference `xl/charts/chart1.xml`)

- Title run: `<a:defRPr b="1" sz="1800">` → 18pt bold.
- Cat + Val axis title: `<a:defRPr b="1" sz="1000">` → 10pt bold. Val axis title `<a:bodyPr
  rot="-5400000"/>` → **-90°** (reads bottom-to-top).
- Tick / legend / dLbl text: `sz="1000" b="0"` → 10pt regular.
- Series line: `<a:ln w="28440"><a:solidFill><a:srgbClr val="4a7ebb"/></a:solidFill></a:ln>` →
  2.24pt (EMU/12700). Axis + gridline: `w="9360"` → 0.74pt.
- Val axis has `<c:majorGridlines>`, cat axis has none. Both `<c:scaling><c:orientation
  val="minMax"/></c:scaling>` (no explicit min/max). Legend `<c:legendPos val="r"/>`.

## Steps

### Model — `freecell-chart-model`

1. **`Axis` widening** (`lib.rs`): add `min: Option<f64>`, `max: Option<f64>`, `reversed: bool`,
   `major_gridlines: bool`, `minor_gridlines: bool`. `Default`/`titled`/`untitled` keep
   `major_gridlines: true` (Excel's value-axis default = the current always-on behavior),
   `minor_gridlines: false`, `min/max None`, `reversed false` — so existing fixtures are
   unchanged. Builders: `with_bounds(min,max)`, `reversed()`, `without_major_gridlines()`,
   `with_minor_gridlines()`. Doc that the line renderer reads **value-axis** gridlines (horizontal)
   + both axes' scaling.

2. **New `stroke.rs`** — `LineStroke { width_pt: Option<f32>, color: Option<ChartColor>, alpha:
   Option<f32> }` mirroring `a:ln` (`@w` EMU→pt via `width_pt_from_emu`, `a:ln/a:solidFill` color,
   `a:alpha`). Add `stroke: Option<LineStroke>` to `Series` (+ `with_stroke` builder;
   constructors default `None`). Export from `lib.rs`.

3. **`ChartColor::resolve_with_alpha`** is not needed — alpha rides `LineStroke.alpha` and is applied
   by the renderer to the resolved `Hsla`.

### Engine — `freecell-engine/src/chart/load.rs`

4. Refactor `parse_axes` to build a full `Axis` per axis node via a new `axis_from_node(ax) -> Axis`
   that reads: title (existing `axis_title`), **numFmt** `formatCode` (currently unparsed for loaded
   charts — add it; benign `General`/empty → `None`), **scaling** (`c:scaling/c:min|c:max` → f64,
   `c:orientation val="maxMin"` → `reversed`), and **gridlines** (`c:majorGridlines` /
   `c:minorGridlines` child presence). Scatter path maps the two `valAx` the same way.

5. `parse_series`: parse the series `a:ln` into `LineStroke` — `a:ln@w` (EMU→pt), `a:ln/a:solidFill`
   color (srgb **or** schemeClr+lumMod/lumOff → `ChartColor`), `a:alpha` (per-mille→fraction). Add
   `parse_solid_fill(node) -> Option<(ChartColor, Option<f32> /*alpha*/)>` helper. Set
   `series.stroke` when an `a:ln` is present.

### Renderer — `freecell-app::chart`

6. `line.rs`:
   - Honor `val_axis.min/max` (clamp/override the shared `NiceScale` domain) and `reversed` (invert
     the value pixel range); honor `cat_axis.reversed` (reverse category x order).
   - Honor `val_axis.major_gridlines` (draw horizontal gridlines only when true).
   - Per-series line width from `LineStroke.width_pt` → px (`pt * PT_TO_PX`, clamped), heavier
     Excel-like default (`DEFAULT_LINE_WIDTH_PT = 2.25`, `PT_TO_PX = 1.2`); stroke color from
     `LineStroke.color` (else series color / palette); `LineStroke.alpha` applied to the `Hsla`.
   - Remove `const LINE_WIDTH`.

7. `style.rs`: add font constants (`TITLE_FONT_SIZE = 18`, `AXIS_TITLE_FONT_SIZE = 12`,
   `LEGEND_FONT_SIZE = 11`) and `with_alpha(Hsla, f32)` helper.

8. `chrome.rs`:
   - **True rotated vertical axis title**: `vertical_axis_title` returns a fixed-width `div`
     hosting a `canvas()` whose paint closure builds an inline SVG (`<text …
     transform="rotate(-90 …)" font-weight="bold">`, XML-escaped, viewBox width = column width so it
     renders 1:1, height = estimated text length) and calls `window.paint_svg(bounds, key,
     Some(bytes), TransformationMatrix::unit(), AXIS_TITLE color, cx)` with a content-hashed cache
     key. Guard zero-size bounds.
   - Chart title → `TITLE_FONT_SIZE` bold; bottom caption + legend text weights/sizes tuned
     (axis-title captions **bold**).
   - **Legend position**: place the legend column/row per `chart.legend.position`
     (Right/Left as side columns, Top/Bottom as rows, TopRight as a right column) instead of always
     right. Keep swatch↔mark mapping.

### Fidelity — `freecell-chart-model/src/fidelity.rs`

9. Reconcile:
   - Drop `"min"`,`"max"` from `RENDER_AFFECTING_PRESENCE_MARKERS`.
   - Replace `axis_reversed` gating with `unsupported_axis_scaling(xml)` = `!is_line_chart(xml) &&
     (axis_reversed || contains min || contains max)` — scaling now renders on **line** (Faithful),
     still degrades on non-line groups.
   - Gridline toggles remain excluded (now correctly honored, not a false-Faithful) — update the
     module doc note. `a:ln` solid stroke stays Faithful (unchanged).
   - Update module docs + tests accordingly.

### Render scenes — `render-tests`

10. Add scenes to `chart_scene.rs` `all()` + the `chart_render_cases!` list in `render_suite.rs`
    (drift-guard): `chart_line_reversed` (reversed cat axis), `chart_line_scaled` (explicit
    val-axis min/max), `chart_line_no_gridlines` (`without_major_gridlines`), `chart_line_styled`
    (`a:ln` heavy width + custom color + alpha per series), `chart_line_legend_bottom`
    (legend Bottom). Regenerate + eyeball baselines for the changed existing scenes
    (heavier lines, bold/bigger title, rotated title) and the new ones.

## Tests

Model:
- `axis_scaling_builder_and_defaults` — bounds/reversed/gridline builders + `major_gridlines`
  default true, `minor` false.
- `line_stroke_width_pt_from_emu` — 28440 EMU → 2.24pt; builder round-trips.
- `series_carries_stroke` — `with_stroke` sets it; constructors default None.

Engine (`load.rs`):
- `parses_axis_scaling_gridlines_and_numfmt` — a valAx with min/max + majorGridlines + numFmt and a
  reversed catAx parse into the model.
- `parses_series_line_stroke` — `a:ln w=28440` + solidFill + alpha → `LineStroke`.
- `absent_scaling_leaves_axis_defaults` — a plain axis → None bounds, not reversed, major on.

Renderer:
- `line.rs`: `explicit_bounds_override_the_nice_scale`; `reversed_value_axis_inverts_range`;
  `reversed_category_axis_reverses_x`; `major_gridlines_toggle_respected`;
  `stroke_width_honors_a_ln_and_defaults_heavier`; `stroke_alpha_applies_to_color`.
- `chrome.rs`: `legend_layout_follows_position` (left/right = column, top/bottom = row);
  existing legend/caption tests updated.

Fidelity (`fidelity.rs`):
- `axis_scaling_is_line_scoped` — min/max/reversed on `lineChart` → Faithful; on
  bar/area/pie/scatter → Degraded.
- Update `active_render_affecting_features_degrade` (move min/max/reversed out of the line frame).
- `gridline_toggle_is_faithful` — `<c:majorGridlines>`/absent on a line → Faithful.

Render: subset `render_tests.sh test chart_` while iterating; a dedicated late phase runs the
chart subset, eyeballs every changed/new baseline, and dispatches the CI `render` gate.
