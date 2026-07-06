# gpui-component Chart Module — Capabilities & API

**Scope of this note:** exactly what `gpui-component`'s chart module can render, so FreeCell
can decide "at most what gpui-component already renders" without building a charting engine.

**Pinned rev studied:** `a9a7341c35b62f27ff512371c62419342264710c` (the rev FreeCell uses — not
latest main). All citations below are `path:line` at this rev. Browse base:
`https://github.com/longbridge/gpui-component/blob/a9a7341c35b62f27ff512371c62419342264710c/`

**Files read (full):** `crates/ui/src/chart/{mod,line_chart,area_chart,bar_chart,candlestick_chart,pie_chart}.rs`;
`crates/ui/src/plot/{mod,axis,grid,label,tooltip}.rs`;
`crates/ui/src/plot/scale/{linear,ordinal,sealed}.rs`;
`crates/ui/src/plot/shape/{bar,arc,stack}.rs`;
theme colors `crates/ui/src/theme/theme_color.rs`;
usage example `crates/story/src/stories/chart_story/{chart_story,stacked_bar_chart}.rs`.

---

## 1. Architecture overview

The chart module is a thin, opinionated layer over a small D3-inspired plotting toolkit in
`crates/ui/src/plot/`. It is **not** a configurable charting library — each chart type is a
purpose-built struct with a fixed layout algorithm.

- **`Plot` trait** (`plot/mod.rs:23`): every chart implements `paint(bounds, window, cx)` plus
  three optional tooltip hooks (`id`, `tooltip_state`, `tooltip`). The trait requires
  `IntoElement`, and `#[derive(IntoPlot)]` (`plot/mod.rs:8`, macro from `gpui_component_macros`)
  generates the GPUI element wiring so a chart struct can be dropped into any `div().child(...)`.
  Returning `Some(id)` from `id()` opts into hover tooltips; `None` (default) = a static,
  non-interactive painted element (`plot/mod.rs:26-33`).
- **Primitives** (`plot/shape/`): `Line`, `Area`, `Bar`(+`BarAlignment`), `Arc`(+`ArcData`),
  `Pie`, and `Stack`(+`StackSeries`,`StackPoint`) — d3-shape ports (`plot/shape.rs:1-13`).
- **Scales** (`plot/scale/`): `ScaleLinear`, `ScaleBand`, `ScalePoint`, `ScaleOrdinal`
  (`plot/scale.rs:7-11`). Linear scale is raw min/max → pixel range with **no "nice" rounding
  and no tick generation** (`plot/scale/linear.rs:21-51`).
- **Axis/grid/label/tooltip** (`plot/axis.rs`, `grid.rs`, `label.rs`, `tooltip.rs`): low-level
  drawing helpers the charts compose.
- **Palette** (`theme/theme_color.rs:134-147`): the theme exposes exactly `chart_1..chart_5`
  (5 categorical colors) plus `chart_bullish` / `chart_bearish` (candlestick up/down). There is
  **no auto-cycling** through this palette inside the chart types — every chart defaults to a
  single color, `chart_2` (e.g. `line_chart.rs:171`, `bar_chart.rs:394`, `area_chart.rs:212`).
  The 5-color palette is only applied automatically if *you* wire a `ScaleOrdinal` yourself
  (as the hand-rolled stacked-bar example does, below).

### The numeric value type is effectively locked to `f64`
Every X/Y-generic chart bounds its numeric value type `V`/`Y` on a **private sealed trait**
`Sealed` (`plot/scale/linear.rs`, bound repeated on each chart). `Sealed` is implemented for
**only `f64`** — and `rust_decimal::Decimal` behind a `decimal` cargo feature
(`plot/scale/sealed.rs:1-7`). So integers (`i32`, `u32`, …) and `f32` are **not** accepted as
the value type on Line/Area/Bar/Candlestick. `PieChart` is the exception: its value accessor is
hard-typed `Fn(&T) -> f32` (`pie_chart.rs:109`). So the phase-1 hunch "bar_chart only accepts
f64" is essentially **correct** for the value axis (f64 or Decimal only), though the *item* type
`T` and the *category* type `X` are fully generic.

### How you feed data (common shape)
All charts are generic over an arbitrary item type `T` and take `Vec<T>` via
`new(impl IntoIterator<Item = T>)`. You then attach **accessor closures** mapping each item to
axis values — e.g. `.x(|d| d.month.clone())`, `.y(|d| d.desktop)`. This is the same
closure-accessor pattern across the module. Category/x types just need `Into<SharedString>`.

---

## 2. Per-chart-type reference

### LineChart — `chart/line_chart.rs`
```rust
pub struct LineChart<T, X, Y>            // X: Into<SharedString>; Y: Num + ToPrimitive + Sealed (=f64)
LineChart::new(data: impl IntoIterator<Item=T>)
  .x(|&T| -> X) .y(|&T| -> Y)            // SINGLE x, SINGLE y (Option, replaced on re-call)
  .stroke(impl Into<Hsla>)              // one line color
  .natural() | .linear() | .step_after()// curve interpolation (default Natural)
  .dot()                                // draw a dot at each point
  .grid(bool) .x_axis(bool) .tick_margin(usize)
  .id(..) .name(..)                     // enable hover tooltip + series name
```
- **Multiple series:** **No.** `y: Option<Rc<dyn Fn>>` (`line_chart.rs`, struct); `.y()` replaces.
  One line per `LineChart`. (Overlaying several `LineChart` elements does not share a y-scale —
  each normalizes its own domain independently — so true multi-line requires the raw `Line`
  primitive in a custom `Plot`.)
- **Coloring:** single `.stroke()` color; default `chart_2` (`line_chart.rs:171`). No per-point.
- **Config:** curve style, dots, grid on/off, x-axis on/off, tick label density. Y-domain is
  forced to include 0 (`line_chart.rs`, `.chain(Some(Y::zero()))`).
- **Tooltip:** yes (crosshair + dot + one row), opt-in via `.id()` (`line_chart.rs`, Plot impl).
- **Data labels:** none.

### AreaChart — `chart/area_chart.rs`  *(the only natively multi-series chart)*
```rust
pub struct AreaChart<T, X, Y>
AreaChart::new(data)
  .x(|&T| -> X)
  .y(|&T| -> Y)                         // PUSHES a series — call repeatedly for multiple areas
  .stroke(Hsla) .fill(impl Into<Background>)   // PUSH per-series (Vec-indexed)
  .name(..)                             // PUSH per-series tooltip label
  .natural()|.linear()|.step_after()    // PUSH per-series curve style
  .grid(bool) .x_axis(bool) .tick_margin(usize) .id(..)
```
- **Multiple series:** **Yes** — `y`, `strokes`, `fills`, `names`, `stroke_styles` are all `Vec`
  (`area_chart.rs:30-40`); each `.y()`/`.stroke()`/`.fill()`/`.name()` **pushes**. Paint loops
  over series (`area_chart.rs:203-230`).
- **⚠️ "Stacked" is a misnomer — areas OVERLAY, they do NOT stack.** Each series is drawn from
  the same baseline `y0 = height` up to its own value (`area_chart.rs:224-225`), so series
  overlap rather than accumulate. The story pane literally titled *"Area Chart - Stacked"*
  (`chart_story.rs:163-185`) is not cumulatively stacked. There is no stacking helper in
  AreaChart.
- **Coloring:** per-series `.stroke()` + `.fill()` (fill accepts solid or `linear_gradient`).
  Defaults to `chart_2` stroke / `chart_2 @ 0.4` fill when a series index is unset
  (`area_chart.rs:212-214`). **Awkward:** the builder is positional/index-aligned — if series
  Vecs get out of sync (e.g. stroke set for series 0 but not 1) it silently falls back to the
  default color.
- **Tooltip:** yes, **multi-row** — one dot + one row per series (`area_chart.rs:257-308`).
- **Data labels:** none.

### BarChart — `chart/bar_chart.rs`  *(the richest / most polished type)*
```rust
pub struct BarChart<T, B, V>            // B: category (Into<SharedString>); V: value (=f64)
BarChart::new(data)
  .band(|&T| -> B) .value(|&T| -> V)    // SINGLE value series
  .fill(|&T, bar_bounds, chart_bounds, BarAlignment| -> impl Into<Background>)  // per-BAR fill
  .fill_gradient(|&T, chart_range, chart_to_bar| -> [LinearColorStop;2])        // auto-oriented
  .alignment(BarAlignment)              // Bottom|Top (vertical) | Left|Right (horizontal)
  .corner_radii(impl Into<Corners<Pixels>>)
  .label(|&T| -> impl Into<SharedString>)   // DATA LABEL at bar end
  .label_axis(bool) .grid(bool) .tick_margin(usize) .id(..) .name(..)
```
- **Multiple series:** **No** — single `value` accessor (`bar_chart.rs:32`). No grouped or
  stacked bars built in. (Stacking is DIY — see §3.)
- **Orientation:** **4 alignments** — `Bottom`/`Top` = vertical, `Left`/`Right` = horizontal
  (`shape/bar.rs:12-23`). So horizontal bars ARE supported.
- **Coloring:** most flexible of all types. `.fill()` closure runs **per bar** and receives the
  bar's pixel frame + full chart bounds + alignment (`bar_chart.rs:120-132`), enabling per-point
  colors, chart-wide gradients, patterns, sampled colormaps. `.fill_gradient()` gives an
  auto-oriented base→tip 2-stop gradient with clip-to-bar interpolation
  (`bar_chart.rs:169-176`, `clip_stops_to_bar` at `bar_chart.rs:577`). Default fill `chart_2`.
- **Data labels:** **yes** — `.label()` draws value text at each bar end, alignment auto-picked
  per orientation (`bar_chart.rs:452-461`, `shape/bar.rs:245-291`).
- **Config:** rounded corners, category-axis on/off, grid on/off, bar padding is **hard-coded**
  (`padding_inner(0.4)`, `padding_outer(0.2)` — `bar_chart.rs:235-236`, not user-settable).
- **Tooltip:** yes (highlight band + row), works for all 4 alignments (`bar_chart.rs:470-564`).

### CandlestickChart — `chart/candlestick_chart.rs`  *(weakest / least featured)*
```rust
pub struct CandlestickChart<T, X, Y>
CandlestickChart::new(data)
  .x(|&T| -> X) .open(..).high(..).low(..).close(..)   // 4 OHLC accessors, all -> Y(=f64)
  .body_width_ratio(f32)                // default 0.8
  .grid(bool) .x_axis(bool) .tick_margin(usize)
```
- **Multiple series:** N/A (one OHLC series).
- **Coloring:** **hard-coded, not customizable** — bullish `chart_bullish`, bearish
  `chart_bearish` from theme, chosen by `close > open` (`candlestick_chart.rs:199-204`). No color
  setter, no per-candle override.
- **Config:** only body-width ratio, grid, x-axis, tick margin. Y-domain from raw OHLC min/max
  (does **not** force 0).
- **Tooltip / interactivity:** **NONE.** The struct has no `id` field and the `Plot` impl only
  implements `paint` (`candlestick_chart.rs:108-238`) — no `id()`/`tooltip_state()`/`tooltip()`.
  So candlesticks are completely non-interactive.
- **Data labels / legend:** none.

### PieChart — `chart/pie_chart.rs`  *(donut-capable; also non-interactive)*
```rust
pub struct PieChart<T>                  // NOT generic over value type
PieChart::new(data)
  .value(|&T| -> f32)                   // NOTE: f32, not f64
  .inner_radius(f32) | .inner_radius_fn(|&ArcData<T>| -> f32)   // donut hole
  .outer_radius(f32) | .outer_radius_fn(|&ArcData<T>| -> f32)   // per-slice radius possible
  .pad_angle(f32)                       // gaps between slices
  .color(|&T| -> impl Into<Hsla>)       // per-slice color
  .label(|&T| -> SharedString)          // leader-line labels outside the ring
  .label_line_color(..) .label_color(..) .label_gap(..)
```
- **Multiple series:** N/A (single ring of slices).
- **Coloring:** **per-slice** via `.color()` closure (`pie_chart.rs:115-121`); default `chart_2`
  for every slice if unset (`pie_chart.rs:180`) — i.e. no auto-palette, all slices identical
  unless you supply colors.
- **Donut / inner radius:** **yes, well-supported** — `inner_radius`, `outer_radius`, plus
  `*_fn` variants for per-slice radii, and `pad_angle` for slice gaps. Pie computes its own
  layout: `Pie::arcs()` assigns angles, `Arc` draws the paths (`pie_chart.rs:164-187`,
  `shape/arc.rs`). Outer radius auto-defaults to `0.4 * height` if unset (`pie_chart.rs:158-162`).
- **Data labels:** **yes** — leader-line + text labels with a real overlap-avoidance layout pass
  (`spread_labels`, `pie_chart.rs:210-329`). Skips slices < 0.5°.
- **Tooltip / interactivity:** **NONE.** No `id` field; `Plot` impl is `paint`-only
  (`pie_chart.rs:152-276`). No hover.

---

## 3. Cross-cutting capabilities — EXISTS vs MISSING

| Feature | Status | Evidence |
|---|---|---|
| **Category / X-axis labels** | ✅ line, area, bar, candlestick (band or point labels) | `line_chart.rs` `axis.x(height).x_label(..)`; `chart/mod.rs:24-85` label builders |
| **Numeric value / Y-axis labels** | ❌ **MISSING on every chart type** | No chart calls `y_label(..)` with numbers. Bar's `y_label` is used only for *category* names on horizontal bars (`bar_chart.rs:355-361`). The value axis has only unlabeled gridlines. |
| **Gridlines** | ✅ but **fixed at ~4 unlabeled lines** | Hard-coded `(0..=3)`/`(0..4)` (`line_chart.rs`, `area_chart.rs:194-200`, `bar_chart.rs:376-388`); count not configurable |
| **Axis line toggles** | ✅ `.x_axis(bool)` / `.grid(bool)` | e.g. `line_chart.rs` |
| **Legend** | ❌ **MISSING entirely** | `grep legend` across chart+plot+story = 0 hits. The on-hover tooltip rows (swatch+name+value, `tooltip.rs:373-390`) are the only legend-like element, and only appear on hover. |
| **Chart title / subtitle** | ❌ not in the chart module | Story wraps charts in its own `div` text (`chart_story.rs:110-151`). The only "title" is the tooltip's hovered-x header (`tooltip.rs:293-295`). |
| **Tooltips (hover)** | ✅ line, area (multi-row), bar; ❌ candlestick, pie | Plot impls; candlestick/pie implement `paint` only |
| **Stacking** | ❌ **not built into any chart type** | Achievable only by hand-rolling a custom `Plot` with the `Stack`/`StackSeries` primitive (`shape/stack.rs`) + `Bar` + scales. The working example (`chart_story/stacked_bar_chart.rs`) lives in the **story crate, not the chart module**, and its header comment says *"You can draw any chart you want by using the `Plot`."* Areas overlay, they don't stack (§2 AreaChart). |
| **Orientation (horizontal bars)** | ✅ bar only (4 `BarAlignment`s) | `shape/bar.rs:12-47` |
| **Area fill / gradients** | ✅ area (`.fill`), bar (`.fill`/`.fill_gradient`) | §2 |
| **Donut / inner radius** | ✅ pie | `pie_chart.rs:57-107` |
| **Custom numeric formatting on ticks/labels** | ❌ label text is whatever your closure returns; no number formatter | closures return `SharedString`/`Into<SharedString>` |
| **Zoom / pan / brush / animation** | ❌ none | no such code in module |

### 3D
**None. Confirmed.** There is no z-axis, perspective, or 3D geometry anywhere. All rendering is
2D via gpui `PathBuilder`/`paint_quad`/`paint_path` (e.g. `candlestick_chart.rs:213-235`,
`shape/bar.rs`, `shape/arc.rs`). No 3D types, no depth, nothing. A 3D chart is entirely out of
scope for this library.

---

## 4. Capability matrix

| Chart | Multiple series | Per-series/point color | Legend | Category axis labels | Numeric axis labels | Tooltip (hover) | Stacking | Data labels | Notable limitations |
|---|---|---|---|---|---|---|---|---|---|
| **Line** | ❌ single | stroke only (1 color) | ❌ | ✅ x | ❌ | ✅ (opt-in) | ❌ | ❌ | one line per element; multi-line needs raw primitive; no y-axis numbers |
| **Area** | ✅ (overlay, Vec of `.y()`) | ✅ per-series stroke+fill | ❌ | ✅ x | ❌ | ✅ multi-row | ❌ (overlays only; "stacked" story is a misnomer) | ❌ | positional/index-aligned builder is fragile; no true stacking; no y-axis numbers |
| **Bar** | ❌ single value | ✅ per-bar `.fill`/`.fill_gradient` | ❌ | ✅ band | ❌ | ✅ | ❌ (DIY via `Stack`) | ✅ `.label()` | no grouped/stacked built-in; bar padding hard-coded; no y-axis numbers |
| **Candlestick** | N/A (OHLC) | ❌ hard-coded bull/bear only | ❌ | ✅ x | ❌ | ❌ none | N/A | ❌ | no interactivity at all; no color control; sparsest API |
| **Pie/Donut** | N/A | ✅ per-slice `.color` | ❌ | N/A | N/A | ❌ none | N/A | ✅ leader-line `.label()` | no hover/interactivity; default = all slices same color (no auto-palette) |

---

## 5. Maturity assessment — does "has charts, but prob not good enough" hold?

**Largely yes, with nuance.** What exists is **cleanly written, D3-derived, and genuinely
pretty** (nice curves, gradients, rounded bars, donut layout with real label de-overlap, polished
hover tooltips on 3 of 5 types). For simple, single-series, category-vs-value visuals it is solid
and would look at home in a modern app.

But as a general spreadsheet-charting backend it has hard gaps:

- **No numeric value-axis labels on ANY chart type.** You get 4 unlabeled gridlines and a
  category axis — you cannot read magnitudes off the axis. For a spreadsheet this is a major
  shortfall.
- **No legend anywhere** (only per-hover tooltip rows).
- **No chart titles/subtitles** in the module (the demo fakes them with surrounding divs).
- **Multiple series is essentially Area-only, and even that only overlays** (no real stacking).
  Line and Bar are single-series; grouped/stacked bars and multi-line require hand-writing a
  custom `Plot` against the raw `Line`/`Bar`/`Stack` primitives (the library's own stacked-bar
  demo does exactly this — it is **not** part of the chart module).
- **Candlestick and Pie have no interactivity** (no tooltips/hover) and Candlestick has no color
  control.
- **Value type is locked to `f64`** (or `Decimal` via feature); Pie inconsistently uses `f32`.
- **Layout knobs are hard-coded** (gridline count, bar padding, tick spacing heuristics).

**Bottom line for FreeCell:** the primitives layer (`plot/` — `Line`/`Bar`/`Area`/`Arc`/`Pie`/
`Stack` + scales + axis/grid/tooltip) is the real asset and is reusable; the five chart *structs*
are convenience wrappers that cover Excel's simplest chart set (column/bar, line, area, pie/donut)
but omit axes labeling, legends, titles, grouping, and stacking. Matching "at most what
gpui-component renders" means: single-series column/line/area/pie/donut with category-axis labels,
per-bar/per-slice colors, optional data labels, and hover tooltips — and accepting that anything
beyond that (numeric axes, legends, grouped/stacked, second axis, scatter, combo) is either DIY on
the primitives or out of scope.
