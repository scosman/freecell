---
status: complete
---

# Phase 22: Column & bar

## Overview

P22 opens the **breadth batch** (implementation_plan §"New graph types"): the first non-line type
slots onto the already-hardened, editable pipeline (anchor/cull/clip, live binding,
save/source-patch, insert/move/resize/delete, chrome editing) built + proven on the line chart
through P21. It sets the pattern the remaining types (area/pie/scatter/bubble, P23–P26) follow: a
**renderer + type fidelity + its own regression baselines + round-trip**, reusing everything else.

A lot of the column/bar surface already exists (PoC-lifted): the model already types
`ChartKind::Bar { dir, grouping }` (`BarDir::Col`/`Bar`, all four `Grouping`s), the load path already
parses `c:barChart`/`c:barDir`/`c:grouping`, the write path already serializes `c:barChart`, the
insert flow already knows `ChartInsertKind::Column`/`Bar`, the fidelity accessor already treats a bar
chart as Faithful, and `bar.rs` already renders columns/bars in all three groupings
(clustered/stacked/100%). So the **net-new** P22 work is the four things production column/bar needs
that the PoC skipped:

1. **`gapWidth` / `overlap` geometry** (`ooxml-coverage-matrix.md` §E). Not modeled, not parsed, not
   written, not honored — the PoC hard-codes the bar-slot padding. Add them to the model
   (`BarLayout`), parse them (`c:gapWidth` / `c:overlap`), write them, and drive the clustered
   bar-slot geometry from them (replacing the hard-coded `GROUP_FILL`/`SUB_BAR_FILL`).
2. **Excel horizontal-bar category order** (`ooxml-coverage-matrix.md` §B, `SYNTHESIS §4.3` — the
   classic bar-chart gotcha). Excel renders the **first** category at the **bottom** of a horizontal
   bar chart; the PoC renders it top-down (data order). Fix the horizontal orientation to reverse the
   category axis order (labels follow their bars).
3. **Per-type fills** (`ooxml-coverage-matrix.md` §C). A bar series' `c:spPr`/`a:solidFill` currently
   parses only `a:srgbClr`; extend it to **theme `a:schemeClr`** (+ `lumMod`/`lumOff` tint) too, so a
   themed bar fill resolves to its palette color — consistent with how the line type already reads its
   `a:ln` fill (`parse_solid_fill`). The renderer already resolves both via `resolve_series_hsla`; this
   closes the load gap.
4. **Regression baselines + round-trip** for the type: new `chart_column_*` / `chart_bar_*` standalone
   render scenes + one in-grid `grid_chart_column`, and column/bar round-trip proofs through
   `discover_and_parse` / `parse_chart_xml`.

**Out of P22 (kept honest, not silently dropped):** the bar renderer still does **not** draw data
labels or honor axis `scaling` (min/max/reversed) — those stay **line-scoped** in the fidelity
accessor, so a bar chart carrying them keeps its Degraded badge (a later phase, exactly like the model
comments already say). Area/pie/scatter/bubble are P23–P26.

## Steps

### Model — `freecell-chart-model`

1. `spec.rs`? No — add to `lib.rs`: a new `BarLayout { gap_width: u16, overlap: i16 }` struct
   (`Clone, Copy, Debug, PartialEq, Eq`) with `Default` = `{ gap_width: 150, overlap: 0 }` (the OOXML
   `ST_GapAmount` / `ST_Overlap` defaults), doc-commented to the `c:gapWidth` / `c:overlap` percentages
   (gap 0..=500 % of bar width; overlap -100..=100 %). Export it from `lib.rs`.
2. `lib.rs`: add `layout: BarLayout` to `ChartKind::Bar`:
   ```rust
   Bar { dir: BarDir, grouping: Grouping, layout: BarLayout },
   ```
   Update the two chart-model construction sites (`authoring.rs` `Column`/`Bar` → `BarLayout::default()`;
   the `lib.rs` round-trip test → `BarLayout::default()`). Matches that only want `dir`/`grouping`
   (`authoring::from_chart_kind`) already use `..`.

### Engine load — `load.rs`

3. `parse_kind` `"barChart"` arm: read `c:gapWidth@val` (default 150, clamp 0..=500) and `c:overlap@val`
   (default 0, clamp -100..=100) into a `BarLayout`, via a small `bar_layout(group)` helper. An absent
   element takes the default (Excel omits them at default) so a real file round-trips.
4. `parse_series_color`: rewrite to reuse the theme-aware `parse_solid_fill` (already used by the line
   stroke) instead of the `srgbClr`-only reader — return `Option<ChartColor>` (drop the fill alpha; the
   `Series` color model carries none). A `srgbClr` fill still yields `ChartColor::Rgb` (existing tests
   unchanged); a `schemeClr` fill now yields `ChartColor::Theme { slot, lum_mod, lum_off }`. `parse_series`
   passes the `ChartColor` straight to `with_color`.

### Engine write — `write.rs`

5. `group_element` `ChartKind::Bar` arm: destructure `layout` and emit `<c:gapWidth val=…/>` +
   `<c:overlap val=…/>` **after** the series (and any `dLbls`) and **before** the `<c:axId>` pair — the
   `CT_BarChart` child order — so the output round-trips through `parse_chart_xml`. Update the two write
   test construction sites to `BarLayout::default()` (and add a non-default-layout round-trip assertion,
   see Tests).

### Renderer — `bar.rs`

6. Carry the layout on `BarPlot` (`gap_width: u16`, `overlap: i16`), populated from `chart.kind`'s
   `BarLayout` in `from_chart` (destructure `ChartKind::Bar { dir, grouping, layout }`).
7. Replace the hard-coded `GROUP_FILL` / `SUB_BAR_FILL` clustered geometry with pure, unit-testable
   helpers driven by gap/overlap:
   - `clustered_metrics(slot, n, gap_width, overlap) -> (bar_w, advance)` where
     `bar_w = slot / (1 + (n-1)·f + gap_width/100)`, `advance = bar_w·f`, `f = 1 - overlap/100` (the
     center-to-center step; `overlap=0` ⇒ contiguous bars, `overlap>0` ⇒ they overlap). The cluster span
     is `bar_w + (n-1)·advance`, centered in the slot; series `j`'s near edge sits at
     `-span/2 + j·advance` from the slot center, band width `bar_w`.
   - `stacked_bar_width(slot, gap_width) -> slot / (1 + gap_width/100)` (one column per category; overlap
     is inapplicable to a single stacked column). Center the stacked band in the slot.
   Clamp gap/overlap to their OOXML ranges inside the helpers.
8. **Reverse the horizontal (`BarDir::Bar`) category order**: compute the category-slot `centers`
   bottom-up (`plot_bottom - slot·(i+0.5)`) so category 0 is at the bottom (Excel order). Columns
   (`BarDir::Col`) stay left-to-right (category 0 at left). Category labels use `geo.centers[i]`, so they
   follow their bars automatically.
9. Update the `bar.rs` geometry unit tests to the new gap/overlap math (partition/disjoint within a
   cluster at default gap, a wider bar + real overlap at a non-default overlap, stacked width shrinks as
   gap grows) and add a `reversed horizontal order` test (category 0's center is below category n-1's).

### Fidelity note — `fidelity.rs`

10. Update the module-doc line that excludes `gapWidth`/`overlap` as "written at defaults" to note they
    are now **honored by the P22 bar renderer** (so any value renders as authored — still Faithful, no
    classification change; the exclusion stands, its *reason* is upgraded). No code change.

### Render scenes — `chart_scene.rs` + `render_suite.rs`

11. `chart_scene.rs`: add six standalone scenes (+ `all()` entries + a `p22_scenes_*` unit test):
    - `chart_column_clustered` — 3-series clustered column, explicit sRGB fills, title/axis-titles/legend.
    - `chart_column_stacked` — 3-series **stacked** column (cumulative segments, axis to the stack total).
    - `chart_column_percent` — 3-series **100%-stacked** column (0–100 % axis).
    - `chart_bar_clustered` — 2-series **horizontal** clustered bar over distinct categories (A/B/C/D) so
      the **reversed** order is visually unambiguous (A at the bottom).
    - `chart_column_gap_overlap` — 2-series clustered column with a **non-default** `gapWidth`+`overlap`
      (narrow gap, bars overlap) — visibly different geometry from the default.
    - `chart_column_theme_fills` — clustered column whose series carry **theme `schemeClr`** fills
      (accent1/2/3), proving theme resolution for a bar fill.
12. `render_suite.rs`: add the six to `chart_render_cases!` (the `chart_scene_names_match_table` drift
    guard keeps them in lockstep with `chart_scene::all()`).

### Grid case — `cases.rs` + `render_suite.rs`

13. `cases.rs`: add `grid_chart_column` — a **loaded** clustered-column `ChartSpec` at the shared
    `chart_anchor` over the backing table (a new `in_grid_column_chart` fixture + a `<c:barChart>` source
    so it classifies Faithful), proving the ChartLayer → `bar_element` in-grid path. Add it to
    `render_cases!` (the `case_names_match_table` guard enforces lockstep).

### Baselines (own phase item — generate + eyeball, then commit with the code)

14. `render_tests.sh generate --only chart_column_` / `--only chart_bar_` / `--only grid_chart_column`;
    **eyeball** each PNG (clustered columns side-by-side; stacked segments cumulative; percent fills
    0–100; the horizontal bar with A at the **bottom**; the gap/overlap geometry visibly tighter/overlapping;
    the theme-fill columns in accent colors; the in-grid column over the table). Commit them with the code.

### Round-trip (local `discover_and_parse` / `parse_chart_xml` reopen)

15. Extend `write.rs` round-trip tests: a **non-default** `BarLayout` (gap+overlap) round-trips
    serialize→parse for both orientations; a `schemeClr` bar fill round-trips (parse→model). Add a
    `load.rs` parse test: `c:gapWidth`/`c:overlap`/`schemeClr` fill parse into the model; an absent
    gap/overlap takes the defaults. Add a `worker_seam` / engine round-trip proof that an **authored**
    column and bar (write path) reopen through `discover_and_parse` as `ChartKind::Bar` with the right
    `dir` (the authored-column case already exists in `worker_seam`; add the bar `dir`).

## Tests

- **Model (`lib.rs`)** — `BarLayout::default()` is `{150, 0}`; the `ChartKind::Bar` round-trip test
  carries a layout.
- **Load (`load.rs`)** — `parse_chart_xml` reads `c:gapWidth`/`c:overlap` into `BarLayout` (and defaults
  them when absent); a series `schemeClr` fill parses to `ChartColor::Theme`; the existing `srgbClr`
  column test still parses to `ChartColor::Rgb`.
- **Write (`write.rs`)** — `serialize_chart_xml` emits `c:gapWidth`/`c:overlap` in schema order; a
  non-default `BarLayout` round-trips for `Col` **and** `Bar`; `near_empty_insert_templates_round_trip`
  (already exercises Column/Bar) stays green with the new field.
- **Renderer (`bar.rs`)** — `clustered_metrics`: bars are disjoint within a cluster at default gap and
  overlap ≥ their `advance` at a positive overlap; the cluster is centered and fits the slot;
  `stacked_bar_width` shrinks as gap grows; **horizontal category order is reversed** (center of category
  0 is below category n-1); stacked baselines stay cumulative; percent stacks sum to 100 on a 0–100 axis
  (existing tests, re-pointed at the new geometry).
- **Fidelity (`fidelity.rs`)** — a bar chart with a non-default `c:gapWidth`/`c:overlap` (and a
  `schemeClr` fill) stays **Faithful** (guards that honoring them didn't accidentally start badging them);
  a bar chart carrying a shown `c:dLbls` or axis `min/max` still **Degrades** (line-scoped, unchanged).
- **Scenes (`chart_scene.rs`)** — every new scene is lookupable, `chart_`-prefixed, a `ChartKind::Bar`
  of the intended `dir`/`grouping`, and the gap/overlap + theme-fill scenes carry the intended layout/color.
- **Pixel (new baselines)** — `chart_column_clustered`, `chart_column_stacked`, `chart_column_percent`,
  `chart_bar_clustered`, `chart_column_gap_overlap`, `chart_column_theme_fills`, `grid_chart_column` each
  render == their eyeballed baseline.
- **Round-trip (`write.rs` / `load.rs` / `worker_seam.rs`)** — authored column + bar reopen as
  `ChartKind::Bar` with the right `dir`; gap/overlap + theme fill survive.

## Render validation

Column/bar rendering **is** in-scope for the pixel suite (grid/cell/sheet **and** chart scenes,
CLAUDE.md render-tests §Scope). Per the brief, P22 does **not** run the full suite (deferred to P26, the
final type's cross-type sweep). During coding, iterate with the **subset only**, foreground under a
`timeout` (never background a render job):
- `render_tests.sh test chart_column_` / `test chart_bar_` — the new standalone scenes.
- `render_tests.sh test grid_chart_` — the new in-grid column (+ confirms the other `grid_chart_*` are
  untouched).
- `render_tests.sh test chart_line_` — confirm the line baselines did **not** move (the shared
  chrome/style changes are additive; no line pixel should shift).

New baselines are generated + **eyeballed** and committed with the code. No existing baseline is intended
to move (every existing chart case is a line/surface chart; no bar baseline existed before). The CI
`render` gate is dispatched by the manager after commit.

## Results

All green. Column & bar is a production type on the hardened pipeline: it loads, renders (both
orientations, all three groupings, gap/overlap, Excel reversed bar order, theme+sRGB fills), authors,
edits, live-binds, and round-trips — reusing the P1–P21 machinery, with the renderer + type fidelity +
regression baselines + round-trip added.

### What shipped
- **Model** — `BarLayout { gap_width, overlap }` (OOXML defaults 150/0) added to `ChartKind::Bar`;
  transparent to live-binding (`resolve_chart` preserves `kind`).
- **Load** — `parse_kind` reads `c:gapWidth`/`c:overlap` (clamped to ST ranges, defaulted when absent);
  `parse_series_color` upgraded to the theme-aware `parse_solid_fill` so a bar/area/pie series fill honors
  `a:schemeClr` (+ tint), not just `a:srgbClr`.
- **Write** — `group_element` emits `c:gapWidth`/`c:overlap` in `CT_BarChart` schema order; round-trips.
- **Renderer (`bar.rs`)** — the hard-coded `GROUP_FILL`/`SUB_BAR_FILL` slot padding is replaced by pure
  gap/overlap geometry (`clustered_metrics`/`clustered_bar_offset`/`stacked_bar_width`), and the horizontal
  (`BarDir::Bar`) category order is **reversed** (`category_centers`) so the first category renders at the
  **bottom** — the Excel gotcha. Per-type fills already resolve via `resolve_series_hsla` (theme + sRGB).
- **Fidelity** — `gapWidth`/`overlap` stay Faithful (now honored, not "benign default"); data labels /
  axis scaling on bar remain line-scoped (Degraded), unchanged.
- **Reuse (unchanged, verified)** — insert already maps `ChartInsertKind::Column`/`Bar`; anchor/cull/clip,
  live binding, save/source-patch, move/resize/delete, chrome editing all work for the type via the shared
  pipeline.

### New render scenes + baselines (7, generated + eyeballed, committed with the code)
- `chart_column_clustered` — 3-series clustered column, sRGB fills, legend, rotated value-axis title.
- `chart_column_stacked` — cumulative segments, axis to the stack total.
- `chart_column_percent` — 0–100% normalized, `%` ticks.
- `chart_bar_clustered` — horizontal bar; **Alpha (first category) renders at the BOTTOM**, Delta at the
  top — the reversed Excel order, eyeball-confirmed.
- `chart_column_gap_overlap` — `BarLayout::new(40, 50)`; visibly wider bars with ~50% overlap.
- `chart_column_theme_fills` — `schemeClr` accent1/2/3 resolving to the Office blue/orange/grey.
- `grid_chart_column` — a loaded clustered column painted in-grid over the backing table (ChartLayer →
  `bar_element`).

### Render subset — PASS (foreground, under a `timeout` watchdog)
`render_tests.sh test chart_` → **29 passed, 0 failed** (517.58s): all 21 chart scenes (the 15 `chart_line_*`
baselines **unchanged** + the 6 new column/bar), all 7 `grid_chart_*` cases (incl. the new
`grid_chart_column`), and the `chart_scene_names_match_table` drift guard. The `case_names_match_table`
guard is green in the unit run. No existing baseline moved. (Full cross-type suite deferred to P26 per the
brief; the CI `render` gate is manager-dispatched after commit.)

### Round-trip — PASS (local `discover_and_parse` / `parse_chart_xml` reopen)
- Engine chart unit: **107 passed** — incl. `serialize_roundtrips_bar_both_orientations` (default **and**
  non-default gap/overlap × Col/Bar), `serialize_emits_gap_width_and_overlap`,
  `write_authored_bar_reopens_as_horizontal_bar_with_layout` (full write→discover reopen as
  `Bar{dir:Bar}` with layout), `parses_bar_gap_overlap_and_theme_fill`, `bar_layout_defaults_and_clamps`.
- Engine integration: `worker_seam` **50** + `roundtrip` **19** + `charts_corpus` **8** = **77 passed** —
  incl. the loaded-column corpus + the `SetChartType(Column) → Save → discover_and_parse` reopen-as-Bar
  proof. (The two `charts_roundtrip_libreoffice` tests are the documented env-broken `soffice` caveat,
  out of scope; external round-trip rides CI.)
- Model **85** + app chart **63** + render-tests lib **12** unit tests green.

### Checks
`cargo fmt --all --check` clean; `cargo clippy` clean across freecell-chart-model / -engine / -app /
render-tests (`--all-targets`).
