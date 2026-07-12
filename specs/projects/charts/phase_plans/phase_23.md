---
status: complete
---

# Phase 23: Area

## Overview

P23 continues the **breadth batch** (implementation_plan §"New graph types") started by P22
(column & bar): the area type slots onto the already-hardened, editable pipeline (anchor/cull/clip,
live binding, save/source-patch, insert/move/resize/delete, chrome editing) proven on the line chart
through P21 and inherited by every breadth type. Like P22, the **net-new** work is a small slice on a
mostly-existing surface: the area **renderer + its type fidelity + its own regression baselines +
round-trip**, reusing everything else.

As with column/bar, most of the area surface already exists (PoC-lifted): the model already types
`ChartKind::Area { grouping }`, the load path already parses `c:areaChart` + `c:grouping` (defaulting
to **`standard`**, area's correct default — unlike bar's `clustered`), the write path already
serializes `c:areaChart` in `CT_AreaChart` schema order, the insert flow already knows
`ChartInsertKind::Area`, the fidelity accessor already treats an area chart as Faithful (and degrades
line-scoped features on it), and **`area.rs` already renders the hand-rolled filled-polygon fork** in
all three groupings (standard/stacked/100%). P22 also already upgraded the series-fill reader
(`parse_series_color` → theme-aware `parse_solid_fill`), so a themed **area** fill (`a:schemeClr`)
already resolves to its palette color.

So the **net-new** P23 work is the type's proof-of-production layer that the PoC skipped:

1. **Regression baselines** — the area renderer has never been exercised by a render scene (P22
   confirmed the chart baseline inventory is line + column/bar only). Add standalone
   `chart_area_*` scenes (standard / stacked / 100%-stacked / theme-fills) + one in-grid
   `grid_chart_area`, generate + **eyeball** each baseline, and commit them with the code. This is
   the real validation that `AreaPlot::paint` — never before pixel-tested — draws its polygons
   correctly.
2. **Round-trip** — prove area round-trips through the IronCalc `discover_and_parse` / `parse_chart_xml`
   reopen path in all three modes the brief names: **loaded** (parse `c:areaChart` + each grouping),
   **authored** (write path → reopen as `ChartKind::Area` with grouping preserved), and **edited**
   (`SetChartType(Area)` → save → reopen as area, live binding preserved).
3. **Fidelity honesty** — lock, with a focused test, that an area chart is **Faithful** while
   line-scoped features it does not honor (shown `c:dLbls`, axis `min/max`/reversed) keep their honest
   **Degraded** badge on area (mirroring P22's `p22_bar_gap_overlap_and_theme_fill_stay_faithful` +
   the line-scoped degradation tests). The accessor already does this (its loops include `areaChart`);
   P23 just pins area explicitly.

**Stacking / baseline math (already in `area.rs`, unit-pinned).** Area is drawn as **filled polygons**
(baseline → upper boundary forward → lower boundary back → close), not strokes — the hand-rolled fork
(`research/compare-area.md`; gpui-component's `Area` closes on a single flat `y0` and cannot draw a wavy
stacked baseline). The per-series cumulative `(lo, hi)` segments come from the shared `super::stacking`
helpers, so each grouping's math is:
- **standard** (area's default, *overlapping* filled regions): every series rises from the zero
  baseline (`lo = 0`, `hi = v`); bands are painted **back-to-front in series order** (series 0 backmost)
  with a semi-transparent fill (`FILL_ALPHA`) so earlier bands show through later ones — the "with
  alpha" option the plan allows.
- **stacked**: each series' polygon sits on the cumulative top of the ones below it
  (`stacked_segments`), so `band k`'s `lo` == `band k-1`'s `hi`.
- **100%-stacked**: the stack is normalized so each category sums to 100 (`percent_segments`), the value
  axis pinned to `0..100`.
The value axis includes the **zero baseline** in every non-percent grouping (`NiceScale::for_values`
over the raw values / the per-category stack totals), which is exactly what an area fill needs. These
are already unit-tested in `area.rs` (`stacked_baselines_are_cumulative`,
`percent_sums_to_100_and_axis_is_0_100`, `standard_bands_all_start_at_zero`); P23 keeps them and adds
the scene-level `p23_scenes_carry_their_area_kind` guard.

**Out of P23 (kept honest, not silently dropped):** the area renderer does **not** draw data labels or
honor axis `scaling` (min/max/reversed) — those stay **line-scoped** in the fidelity accessor, so an
area chart carrying them keeps its Degraded badge (a later phase, exactly like bar). Pie/doughnut,
scatter, and bubble are P24–P26.

## Steps

### Model / load / write / authoring / fidelity — already present (verify, don't rebuild)

The area surface is already in place from the PoC lift + P22's shared upgrades; P23 changes **no**
model, load, write, or authoring code. Verified during planning:
- **Model** (`lib.rs`) — `ChartKind::Area { grouping }` (no bar-style `layout`; area has no
  gap/overlap). No change.
- **Load** (`load.rs`) — `parse_kind` `"areaChart"` reads `c:grouping` defaulting to
  `Grouping::Standard` (area's correct default). `parse_series_color` (P22) already resolves a
  `schemeClr` area fill to `ChartColor::Theme`. No change.
- **Write** (`write.rs`) — `group_element` `ChartKind::Area` emits
  `<c:areaChart><c:grouping/><c:varyColors/>{series}<c:axId/><c:axId/></c:areaChart>` (the
  `CT_AreaChart` child order), which already round-trips through `parse_chart_xml`. No change.
- **Renderer** (`area.rs`) — the hand-rolled filled-polygon fork over the shared `stacking` helpers +
  `NiceScale`; dispatched from `chart::mod::chart_element` (`ChartKind::Area { .. } => area_element`).
  No change (net-new is scenes + baselines that exercise its `paint`).
- **Authoring** (`authoring.rs`) — `ChartInsertKind::Area → ChartKind::Area { grouping: Standard }`
  and the inverse. No change.
- **Fidelity** (`fidelity.rs`) — area is Faithful; the line-scoped detectors (`unsupported_data_labels`,
  `unsupported_axis_scaling`, …) already degrade `areaChart` (their loops list it). No code change; a
  focused test is added below.

### Render scenes — `chart_scene.rs`

1. `chart_scene.rs`: add four standalone area scenes (+ `all()` entries + a `p23_scenes_*` unit test),
   built from a shared 3-series `months()` shape so they mirror the P22 column trio + theme scene:
   - `chart_area_standard` — 3-series **standard** (overlapping) area, explicit sRGB fills, title /
     axis-titles / legend. Series authored tallest-first so the back-to-front alpha layering reads as
     three overlapping translucent regions (not a stack).
   - `chart_area_stacked` — the same 3 series **stacked** (cumulative bands, value axis to the stack
     total).
   - `chart_area_percent` — the same 3 series **100%-stacked** (0–100 % axis, `%` ticks).
   - `chart_area_theme_fills` — a standard area whose series carry **theme `schemeClr`** fills
     (accent1/2/3), proving theme resolution for an **area** fill (the area analogue of
     `chart_column_theme_fills`).

### Render suite registration — `render_suite.rs`

2. `tests/render_suite.rs`: add the four to `chart_render_cases!` (the `chart_scene_names_match_table`
   drift guard keeps them in lockstep with `chart_scene::all()`).

### Grid case — `cases.rs` + `render_suite.rs`

3. `cases.rs`: add `grid_chart_area` — a **loaded** standard-area `ChartSpec` at the shared
   `chart_anchor` over the backing table (a new `in_grid_area_chart` fixture + `in_grid_area_spec` with
   a `<c:areaChart>` source so it classifies Faithful), proving the ChartLayer → `area_element` in-grid
   path (the area analogue of `grid_chart_column`). Add it to `render_cases!` + `CASE_NAMES` (the
   `case_names_match_table` guard enforces lockstep).

### Baselines (own phase item — generate + eyeball, then commit with the code)

4. `render_tests.sh generate --only chart_area_` / `--only grid_chart_area`; **eyeball** each PNG (the
   standard bands overlapping translucently; the stacked bands cumulative and non-overlapping; the
   percent bands filling 0–100 with `%` ticks; the theme-fill area in Office accent colors; the in-grid
   area over the table). Commit them with the code.

### Round-trip (local `discover_and_parse` / `parse_chart_xml` reopen)

5. `write.rs`: extend the area serialize round-trip to **all three groupings** (standard / stacked /
   percentStacked) and add `write_authored_area_reopens_as_area_with_grouping` — an **authored** stacked
   area written via `write_authored_charts`, reopened through `discover_and_parse` as
   `ChartKind::Area { grouping: Stacked }` (the area twin of
   `write_authored_bar_reopens_as_horizontal_bar_with_layout`).
6. `load.rs`: add `parses_area_grouping_and_theme_fill` — `c:areaChart` with `c:grouping` = standard /
   stacked / percentStacked parses into the right `Grouping` (and an **absent** grouping defaults to
   `Standard`); a `schemeClr` area series fill parses to `ChartColor::Theme`.
7. `worker_seam.rs`: add `retyped_to_area_chart_roundtrips` — `SetChartType(Area)` on a ranged chart →
   save → `discover_and_parse` reopens as `ChartKind::Area` with the `c:f` range binding preserved (the
   **edited** path, mirroring `retyped_authored_chart_roundtrips`'s Column switch).

### Fidelity — `fidelity.rs`

8. `fidelity.rs`: add `p23_area_stays_faithful_but_line_scoped_features_degrade` — a plain `c:areaChart`
   (with a `schemeClr` fill) is **Faithful**; the same area carrying a shown `c:dLbls` or an axis
   `min/max` **Degrades** (line-scoped, unchanged). Locks area's honesty explicitly, like P22's bar
   test. No production code change.

## Tests

- **Renderer (`area.rs`)** — existing `stacked_baselines_are_cumulative`,
  `percent_sums_to_100_and_axis_is_0_100`, `standard_bands_all_start_at_zero`, `rejects_non_area` stay
  green (they already unit-pin the stacking/baseline math).
- **Scenes (`chart_scene.rs`)** — `p23_scenes_carry_their_area_kind`: every new scene is lookupable,
  `chart_`-prefixed, a `ChartKind::Area` of the intended `grouping`, and the theme-fills scene resolves
  every series to a `ChartColor::Theme`.
- **Load (`load.rs`)** — `parses_area_grouping_and_theme_fill`: standard / stacked / percentStacked
  grouping parse correctly; an absent grouping defaults to `Standard`; a `schemeClr` area fill →
  `ChartColor::Theme`.
- **Write (`write.rs`)** — `serialize_roundtrips_area_all_groupings` (serialize→parse reconstructs the
  model for each grouping); `write_authored_area_reopens_as_area_with_grouping` (full write→discover
  reopen as `Area{Stacked}`).
- **Edited round-trip (`worker_seam.rs`)** — `retyped_to_area_chart_roundtrips`: retype to Area →
  save → reopen as `ChartKind::Area`, range binding preserved.
- **Fidelity (`fidelity.rs`)** — `p23_area_stays_faithful_but_line_scoped_features_degrade`: area is
  Faithful; a shown `c:dLbls` / axis `min/max` on area still Degrades. (The existing
  `shown_data_labels_are_line_scoped` / `axis_scaling_is_line_scoped` loops already include
  `areaChart`; this pins it under a P23-named test.)
- **Pixel (new baselines)** — `chart_area_standard`, `chart_area_stacked`, `chart_area_percent`,
  `chart_area_theme_fills`, `grid_chart_area` each render == their eyeballed baseline.

## Render validation

Area rendering **is** in-scope for the pixel suite (grid/cell/sheet **and** chart scenes, CLAUDE.md
render-tests §Scope). Per the brief, P23 does **not** run the full suite (deferred to P26, the final
type's cross-type sweep). During coding, iterate with the **subset only**, foreground under a `timeout`
(never background a render job):
- `render_tests.sh test chart_area_` — the four new standalone area scenes.
- `render_tests.sh test grid_chart_` — the new in-grid area (+ confirms the other `grid_chart_*` are
  untouched).
- `render_tests.sh test chart_line_` / `test chart_column_` — confirm the line + column/bar baselines
  did **not** move (the additions are new scenes only; no existing chart pixel should shift).

New baselines are generated + **eyeballed** and committed with the code. No existing baseline is intended
to move (every prior chart case is a line/column/bar chart; no area baseline existed before). The CI
`render` gate is dispatched by the manager after commit.

## Results

All green. Area is a production type on the hardened pipeline: it loads, renders (standard / stacked /
100%-stacked filled polygons + theme/sRGB fills over a zero-baseline value axis), authors, edits,
live-binds, and round-trips — reusing the P1–P21 machinery, with the type's regression baselines +
round-trip proofs + a focused fidelity test added.

### What shipped (net-new — the rest was PoC-lifted + inherited from P22)
- **No model/load/write/authoring/fidelity code change.** Verified they already handle area:
  `ChartKind::Area { grouping }` (no gap/overlap analog); `parse_kind` reads `c:grouping` defaulting to
  `Standard` (area's default, unlike bar's `Clustered`); `group_element` emits `c:areaChart` in
  `CT_AreaChart` order; `ChartInsertKind::Area` ↔ `ChartKind::Area`; the fidelity accessor treats area
  as Faithful and its line-scoped detectors already degrade `areaChart`. P22's theme-aware
  `parse_series_color` already resolves a `schemeClr` **area** fill.
- **Renderer (`area.rs`)** — the existing hand-rolled filled-polygon fork (baseline → upper forward →
  lower back → close) over the shared `stacking` helpers + `NiceScale` (zero-baseline `for_values` for
  standard/stacked, `0..100` for percent). Never before pixel-tested; P23's baselines are its first
  real validation. No code change.
- **Render scenes + baselines (5, generated + eyeballed, committed with the code)** —
  `chart_area_standard` (3 overlapping semi-transparent bands from zero, axis→80), `chart_area_stacked`
  (cumulative bands, axis→150), `chart_area_percent` (0–100% normalized, `%` ticks),
  `chart_area_theme_fills` (`schemeClr` accent1/2/3 → Office blue/orange/grey), and `grid_chart_area`
  (a loaded standard area painted in-grid over the backing table via ChartLayer → `area_element`).
- **Round-trip proofs (loaded / authored / edited, grouping preserved)** —
  `load.rs::parses_area_grouping_and_theme_fill` (standard/stacked/percentStacked parse; absent→Standard;
  `schemeClr` area fill→Theme); `write.rs::serialize_roundtrips_area_all_groupings` +
  `write_authored_area_reopens_as_area_with_grouping` (authored stacked area → write → `discover_and_parse`
  reopens as `Area{Stacked}`); `worker_seam.rs::retyped_to_area_chart_roundtrips` (`SetChartType(Area)` →
  save → reopen as area, `c:f` binding preserved).
- **Fidelity** — `fidelity.rs::p23_area_stays_faithful_but_line_scoped_features_degrade` pins that a plain
  area (with a resolved theme fill) is Faithful while a shown `c:dLbls` / axis `min/max` on area still
  Degrades (line-scoped, unchanged).

### Render subset — PASS (all foreground, each under a `timeout 600`)
- `render_tests.sh test chart_area_` → **4 passed** (124s): the four new standalone area scenes render
  == their eyeballed baselines.
- `render_tests.sh test grid_chart_` → **8 passed** (418s): the new `grid_chart_area` + all 7 existing
  `grid_chart_*` cases — existing grid baselines unchanged.
- `render_tests.sh test chart_line_` → **15 passed** (124s), `test chart_column_` → **5 passed** (124s),
  `test chart_bar_` → **1 passed** (124s): no existing line/column/bar baseline moved (the change is new
  scenes only; no shared render code touched).

The `chart_scene_names_match_table` + `case_names_match_table` drift guards are green. (Full cross-type
suite deferred to P26 per the brief; the CI `render` gate is manager-dispatched after commit.)

### Round-trip — PASS (local `discover_and_parse` / `parse_chart_xml` reopen)
- Engine chart unit: **110 passed** — incl. `serialize_roundtrips_area_all_groupings`,
  `write_authored_area_reopens_as_area_with_grouping`, `parses_area_grouping_and_theme_fill`.
- Engine integration (`worker_seam`): `retyped_to_area_chart_roundtrips` + `retyped_authored_chart_roundtrips`
  green. (The two `charts_roundtrip_libreoffice` tests are the documented env-broken `soffice` caveat, out
  of scope; external round-trip rides CI.)
- Model **86** (incl. the new fidelity test) + render-tests lib **8** chart-scene unit tests green.

### Checks
`cargo fmt --all --check` clean; `cargo clippy` clean across freecell-chart-model / -engine / -app /
render-tests (`--all-targets`).
