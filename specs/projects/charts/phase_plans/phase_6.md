---
status: complete
---

# Phase 6: Line P1 fidelity

## Overview

Phase 5 landed a production line renderer that draws multi-series lines on one shared nice-tick
scale with model-driven chrome (title, legend, axis-title captions). It is structurally correct
but visually "plain": every axis title is a horizontal caption, series colors are only explicit
sRGB or our palette, lines are always straight with a fixed round dot, and tick labels are raw
numbers.

Phase 6 closes the P1/P2 fidelity gap the coverage matrix flags for line charts
(`ooxml-coverage-matrix.md` §C/§D/§E) so a reviewer "sees a *real* line chart"
(implementation_plan P6). Five features, all driven here via **chart-model fixtures** (the
engine parse of these attributes from real `.xlsx` is P7, not this phase):

1. **Theme colors (`schemeClr` + tint)** — a series color can be a theme-slot reference
   (`a:schemeClr val="accent1"`) with optional `lumMod`/`lumOff` tint, resolved against a theme
   palette (default Office theme for the isolated component; P8 threads the workbook theme).
   (functional_spec §4 P1; matrix §C `a:schemeClr`.)
2. **Rotated vertical value-axis title** — the value-axis title moves from a horizontal caption
   above the plot to a **vertical** title down the left of the value axis, the Excel placement.
   gpui cannot rotate a text run at the pinned rev (only svg/img take a transform — verified),
   so it is rendered as stacked characters (the faithful-within-platform representation).
   (functional_spec §4 P1; matrix §D `c:catAx/c:valAx title`.)
3. **`numFmt` ticks** — the value axis carries an OOXML `formatCode`; tick labels are formatted
   through a bounded number-format applier (General, percent, thousands grouping, fixed
   decimals, currency prefix). (functional_spec §4 P2; matrix §D `c:numFmt`.)
4. **Markers (`c:marker`)** — a series can carry a marker symbol (circle/square/diamond/
   triangle/star/plus/x/dash/dot/none/auto); the line renderer paints the shape at each point.
   (functional_spec §4 P2; matrix §C `c:marker`.)
5. **`smooth`** — `ChartKind::Line { smooth }` already exists; honor it by drawing the curved
   (`StrokeStyle::Natural`) line instead of straight. (functional_spec §4 P2; matrix §E `c:smooth`.)

**Fidelity-accessor reconciliation.** Rendering `smooth` and all marker symbols faithfully makes
their prior `Degraded` classification wrong, so their detectors are removed from
`freecell-chart-model::fidelity`. `numFmt` support is *bounded* (a format code we don't parse
must still warn), so `custom_number_format` stays for P12 to complete — documented, per the
phase brief's "prefer leaving the accessor to P12 when ambiguous".

## Steps

### Model — `freecell-chart-model` (gpui-free, ironcalc-free)

1. **New `theme` module** (`src/theme.rs`):
   - `ThemeSlot` enum mirroring `a:schemeClr val`: `Dark1, Light1, Dark2, Light2, Accent1..6,
     Hyperlink, FollowedHyperlink`. `from_ooxml(&str) -> Option<ThemeSlot>` (accepts `dk1`/`tx1`,
     `lt1`/`bg1`, `dk2`/`tx2`, `lt2`/`bg2`, `accent1..6`, `hlink`, `folHlink`) for P7.
   - `ThemePalette { <slot>: Color, ... }` with `office_default()` (Office 2013+ theme RGBs:
     accent1 `4472C4`, accent2 `ED7D31`, accent3 `A5A5A5`, accent4 `FFC000`, accent5 `5B9BD5`,
     accent6 `70AD47`, dk1 `000000`, lt1 `FFFFFF`, dk2 `44546A`, lt2 `E7E6E6`, hlink `0563C1`,
     folHlink `954F72`) and `fn color(&self, ThemeSlot) -> Color`.
   - `ChartColor` enum `{ Rgb(Color), Theme { slot: ThemeSlot, lum_mod: Option<f32>, lum_off:
     Option<f32> } }` (derive Clone, Copy, Debug, PartialEq). `resolve(&self, &ThemePalette) ->
     Color` applies `lumMod`/`lumOff` as an HSL-luminance transform (`L' = clamp(L*lumMod +
     lumOff)`, documented approximation of the OOXML tint). `impl From<Color> for ChartColor`.
   - Private `rgb_to_hsl`/`hsl_to_rgb` helpers for the tint math.

2. **New `marker` module** (`src/marker.rs`):
   - `MarkerSymbol` enum `{ None, Auto, Circle, Square, Diamond, Triangle, Star, X, Plus, Dash,
     Dot }` (mirrors `c:symbol val`), `from_ooxml(&str)`.
   - `Marker { symbol: MarkerSymbol, size: Option<f32> }` with `new(symbol)` + `with_size`.

3. **New `numfmt` module** (`src/numfmt.rs`):
   - `pub fn apply_number_format(code: &str, value: f64) -> String` — bounded applier: empty /
     `General` → general (delegates to `crate::format_number`); otherwise parse the first
     `;`-section for a currency/text prefix, thousands grouping (`,`), decimal count (digits
     after `.`), percent (`%` → ×100), and a trailing literal suffix; scientific/date/unparsable
     codes fall back to general. Documented as the P6 subset (P12 completes numFmt).

4. **`Series`** (`src/lib.rs`): change `color: Option<Color>` → `Option<ChartColor>`; add
   `marker: Option<Marker>`. Constructors set both `None`; `with_color(impl Into<ChartColor>)`
   (so existing `Color` call sites keep compiling via `From<Color>`); add `with_marker(Marker)`.

5. **`Axis`** (`src/lib.rs`): add `number_format: Option<String>` (the `c:numFmt` formatCode);
   update `untitled()`/`titled()` to set `None`; add `with_number_format(impl Into<String>)`.

6. **`lib.rs`** wiring: `mod theme/marker/numfmt`; re-export `ChartColor, ThemeSlot,
   ThemePalette, Marker, MarkerSymbol, apply_number_format`; make `format_number` `pub(crate)`.

7. **Fidelity accessor** (`src/fidelity.rs`): drop `smooth_enabled` + `markers_shown` from
   `has_render_affecting_unsupported_feature` and delete those helpers; keep
   `custom_number_format`. Update the module docs + the curated-set comment to record that P6
   now renders `smooth` and all marker symbols. Move the smooth/marker cases from
   `active_render_affecting_features_degrade` into a new `now_rendered_features_are_faithful`
   test; leave numFmt degrading.

### Renderer — `freecell-app::chart`

8. **`style.rs`**: add `resolve_series_color(color: Option<ChartColor>, index: usize) ->
   ModelColor` (resolves `ChartColor` against `ThemePalette::office_default()`, else the palette
   cycle at `index`) + `resolve_series_hsla(...) -> Hsla`. Document the office-default stand-in
   (P8 threads the workbook palette).

9. **`line.rs`**:
   - Resolve each series color via `resolve_series_hsla(s.color, i)`; carry the series' optional
     `Marker` into `LineSeries`.
   - Honor `smooth`: read the `ChartKind::Line { smooth }` flag; `StrokeStyle::Natural` when
     true, else `Linear`. Stop calling the primitive's `.dot()`.
   - Paint markers manually after each series' line: `paint_marker(window, center, symbol, size,
     fill, stroke)` where the default (series `marker == None`) is a `Circle` at the current
     `DOT_SIZE` with white stroke — **pixel-identical to the P5 dot** so untitled/plain scenes
     don't regress. Filled shapes (circle/square/diamond/triangle/star/dot/auto) via quad/fill
     path; stroked shapes (plus/x/dash) via `PathBuilder::stroke`; `None` paints nothing.
   - Value-axis ticks: format via `apply_number_format(code, t)` when
     `chart.val_axis.number_format` is set, else `format_tick(t)`.

10. **`chrome.rs`**:
    - `captions(chart)` → `(vertical_title, horizontal_title)`: the **vertical-axis** title (value
      normally; category for horizontal bar) and the **horizontal-axis** title (category normally;
      value for horizontal bar).
    - Restructure `chart_frame`: drop the top caption; add a narrow left **vertical title column**
      (stacked characters, `justify_center`) before the plot when the vertical title is non-empty;
      keep the bottom horizontal caption.
    - `legend_entries`: resolve `s.color` via `resolve_series_color(s.color, i).to_hex()`.

11. **`bar.rs` / `area.rs` / `scatter.rs`**: update the model-color read at plot-build time to
    `resolve_series_hsla(s.color, i)` (mechanical; keeps the shared chrome/type builds green —
    only line renders markers/smooth this phase).

### Engine — `freecell-engine::chart`

12. **`load.rs`** test only: the existing `parse_series` uses `with_color(Color)` (still compiles
    via `From<Color>`); update the one assertion `s.color == Some(Color::…)` to
    `Some(ChartColor::Rgb(Color::…))`.

### Render scenes — `render-tests/src/chart_scene.rs`

13. Add two scenes showcasing the new fidelity, and regenerate every `chart_` baseline (the four
    titled line scenes change because the value-axis title becomes the left vertical title):
    - `chart_line_markers`: multi-series straight line, each series a **theme color**
      (accent1/2/3) and a distinct **marker** (circle/square/diamond), value axis `numFmt`
      `"$#,##0"` (currency ticks).
    - `chart_line_smooth`: multi-series **smooth** (curved) line, **theme** colors, value axis
      `numFmt` `"0%"` (percent ticks over fractional values).
    - Extend the scene-table unit tests to assert the new scenes carry their features.

## Tests

Model (`freecell-chart-model`):
- `theme`: `office_default` returns the known accent RGBs; `ChartColor::Rgb` resolves to itself;
  `Theme{accent1}` resolves to `4472C4`; `From<Color>`; `lumMod`/`lumOff` tint lightens/darkens
  luminance within tolerance; `ThemeSlot::from_ooxml` maps `dk1/tx1`, `accent3`, `folHlink`, and
  rejects junk.
- `marker`: `MarkerSymbol::from_ooxml` maps `circle/square/diamond/none/auto` and rejects junk;
  `Marker::new`/`with_size`.
- `numfmt`: `apply_number_format` for `""`/`General`, `0%`, `0.0%`, `#,##0`, `#,##0.00`, `0.00`,
  `$#,##0`, a negative (`-1,500`), and a graceful fallback for an unsupported code (date/scientific).
- `Series`: `with_color(Color)` and `with_color(ChartColor::Theme{..})` both set `color`;
  `with_marker`; constructors default `color`/`marker` to `None`.
- `Axis`: `with_number_format` sets it; `titled`/`untitled` default it to `None`.
- `fidelity`: `now_rendered_features_are_faithful` — `smooth val="1"` and a non-circle marker are
  `Faithful`; `custom_number_format` still `Degraded`; existing benign/unsupported tests stay green.

Renderer (`freecell-app::chart`):
- `line.rs`: a smooth chart selects `StrokeStyle::Natural`, a non-smooth `Linear` (assert via a
  small exposed hook / the plot's stored flag); a series carries its `Marker` into `LineSeries`;
  a theme-colored series resolves to the office accent color.
- `style.rs`: `resolve_series_color` returns the explicit sRGB, the resolved theme color, and the
  palette fallback for the three input cases.
- `chrome.rs`: `captions` returns `(value, category)` normally and `(category, value)` for
  horizontal bar; a legend entry resolves a theme-colored series to its office accent hex; the
  vertical/bottom title collapse when empty.

Render (`render-tests`, subset `chart_` only while iterating; full suite deferred to the
manager's late phase):
- Regenerate + eyeball all seven `chart_` baselines; confirm the four titled line scenes changed
  only by the rotated vertical value-axis title, `chart_line_no_titles` is unchanged, and the two
  new scenes show theme colors + markers + smooth + numFmt ticks correctly.
