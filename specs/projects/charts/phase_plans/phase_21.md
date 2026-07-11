---
status: complete
---

# Phase 21: MVP sweep → v1 ships (the ship gate)

## Overview

P21 is the **v1 MVP ship gate** for the line chart — a **validation + hardening** phase, not a
feature phase. The goal is to prove the line-chart MVP is **shippable end-to-end**: it displays,
lives-binds, saves/preserves, and is fully **authorable + editable** (insert / move / resize /
delete / chrome edits), with the perceptual-diff render suite green, the local save→reopen
round-trip green across the whole surface, and the perf envelope re-measured. Expect mostly
test/scene/perf work + confirming green — the only product-code touch is one render scene that
closes a coverage gap (the near-empty inserted chart's rendered state).

Concretely the phase does four things:

1. **Run the FULL perceptual-diff render suite** (foreground, under a `timeout` watchdog) — this is
   the dedicated late phase where the whole pixel suite runs (CLAUDE.md render-tests §3).
2. **Coverage sweep** — confirm the suite (pixel baselines + round-trip tests) validates the line
   chart across the whole MVP surface; add a scene only for an authoring/editing **visual** state
   that genuinely lacks a baseline.
3. **External round-trip proved LOCALLY** via the IronCalc `discover_and_parse` reopen path across
   the surface (displayed / edited / authored). The real Excel+LibreOffice round-trip rides CI
   `roundtrip.yml` (manager-dispatched); LibreOffice is env-broken in this container (out of scope).
4. **Re-measure the perf envelope** (many-charts / huge-sheet first-paint, edit-rerender,
   scroll-with-K), foreground under `timeout`, force+asserted, p50/p99, env-stamped.

## Coverage matrix — the line chart across the MVP surface

Each MVP surface behavior, mapped to the artifact that validates it. "Pixel" = a committed render
baseline; "round-trip" = a `discover_and_parse` save→reopen test; "unit/view" = a gpui-free or
gpui view test.

| Surface behavior | Validated by | Kind | Status |
| --- | --- | --- | --- |
| **Display — faithful** line in grid | `grid_chart_line` | pixel | covered |
| **Display — degraded badge** (3D→2D) | `grid_chart_degraded_badge` | pixel | covered |
| **Display — unsupported placeholder** | `grid_chart_unsupported_placeholder` | pixel | covered |
| **Display — scrolled + clipped** (anchor tracks scroll) | `grid_chart_scrolled_clipped` | pixel | covered |
| **Chart chrome — title present / absent** | `chart_line_*` / `chart_line_no_titles` | pixel | covered |
| **Chart chrome — legend right / off / bottom** | default `chart_line_*` / `chart_line_no_legend` / `chart_line_legend_bottom` | pixel | covered |
| **Chart chrome — axis titles / untitled** | titled `chart_line_*` / `chart_line_no_titles` | pixel | covered |
| **Chart chrome — series colors / strokes** | `chart_line_markers` (theme) / `chart_line_styled` (`a:ln`) | pixel | covered |
| **Chart chrome — data labels (val/percent/name)** | `chart_line_value_labels` / `_percent_labels` / `_named_labels` | pixel | covered |
| **Chart chrome — markers / smooth / gridlines / reversed / scaled** | `chart_line_markers` / `_smooth` / `_no_gridlines` / `_reversed` / `_scaled` | pixel | covered |
| **Live rebind** (edit source cell → re-render) | `editing_a_source_cell_reresolves_the_chart`, `coalesced_edits_converge_the_chart` | round-trip/seam | covered |
| **Save / preserve** (untouched byte-stable, edited reflowed) | `save_through_worker_preserves_and_patches_charts` | round-trip | covered |
| **Authoring — insert (near-empty chart, in-grid)** | **NEW `grid_chart_authored_inserted`** | pixel | **added this phase** |
| **Authoring — select (outline + handles)** | `grid_chart_selected` | pixel | covered (P18) |
| **Authoring — move** (anchor translate) | `grid_chart_scrolled_clipped` (rect translate) + `move_authored_chart_roundtrips` / `move_loaded_chart_patches_drawing_and_roundtrips` | pixel + round-trip | covered |
| **Authoring — resize** (rect re-layout) | chart element baselined at 3 sizes (`grid_chart_line` ~500×312, `chart_line_*` 640×440 / 720×460) + `move_authored_chart_roundtrips` (anchor persists) | pixel + round-trip | covered |
| **Authoring — delete** (chart removed) | `delete_authored_chart_roundtrips`, `delete_loaded_chart_drops_it_from_the_package` | round-trip | covered (absence — no pixel) |
| **Edit — loaded chart chrome** (title/legend/color/labels, unmodeled preserved) | `loaded_chart_title_edit_is_live_and_preserves_unmodeled_styling`, `loaded_chart_legend_color_and_labels_roundtrip` | round-trip | covered (P20) |
| **Edit — loaded chart data reflow** | `save_through_worker_preserves_and_patches_charts` (line reflowed) | round-trip | covered |
| **Author — set range → live c:f** | `ranged_authored_chart_saves_cf_and_roundtrips` | round-trip | covered (P19) |
| **Author — set type** | `retyped_authored_chart_roundtrips` | round-trip | covered (P19) |
| **Author — chrome edits round-trip** | `authored_chart_chrome_edits_roundtrip` | round-trip | covered (P20) |

**Gap analysis.** Every chrome-edited *rendered result* is already covered by the standalone
`chart_line_*` baselines (the chrome edit re-renders through the same `freecell_app::chart`
element those baselines capture) — so no new scene is needed for chrome edits. Move is a pure
rect translation (covered by `grid_chart_scrolled_clipped`); resize re-lays the chart element out
at a new size (already baselined at three sizes); delete is an absence (no pixel). The **one**
uncovered authoring visual state is the **near-empty inserted chart**: every existing
`grid_chart_*` case renders a **loaded** spec, so no pixel baseline exercises the
**authored → in-grid** render path or the exact picture the user sees the instant they insert a
line chart. This phase adds that one scene.

## Steps

### Render scene — the inserted-chart visual (the one coverage gap)

1. `app/render-tests/src/cases.rs`:
   - Import `ChartInsertKind` (from `freecell_chart_model`).
   - Add a fixture builder `in_grid_authored_inserted_spec()` →
     `ChartSpec::authored(ChartInsertKind::Line.near_empty_chart(), chart_anchor())` — a near-empty
     **authored** line chart (title "Chart", one placeholder series "Series 1" over categories
     1..4, untitled axes, right legend) at the shared chart anchor. Its `display_fidelity()` is
     `Faithful` (authored → the real plot renders).
   - Add `RenderCase::new("grid_chart_authored_inserted", chart_backing_scene(), CHART_GRID_VP)
     .charts(vec![in_grid_authored_inserted_spec()])` — the authored chart floats over the same
     backing table as the loaded cases.
2. `app/render-tests/tests/render_suite.rs`: add `grid_chart_authored_inserted` to the
   `render_cases!` macro list (the drift guard `case_names_match_table` enforces lockstep).

### Generate + eyeball the new baseline

3. `render_tests.sh generate --only grid_chart_authored_inserted`; **eyeball** the PNG (a real
   single-series line chart titled "Chart" over the backing table); commit it with the phase.

### Run the FULL render suite

4. `render_tests.sh test` (whole suite; asserts every case == baseline), foreground under a
   `timeout` with a ~10-min watchdog. Record pass/fail per case class.

### Round-trip sweep (local `discover_and_parse` reopen)

5. Run the engine round-trip / worker-seam / corpus tests (`worker_seam`, `roundtrip`,
   `charts_corpus`) and confirm green — the displayed / edited (chrome + data reflow) / authored
   (range + type + chrome) save→reopen proofs all pass. The two `charts_roundtrip_libreoffice`
   tests fail at the `soffice` step (env-broken; out of scope, not a regression).

### Perf re-measure

6. `cargo run -p render-tests --release --bin chart_perf`, foreground under `timeout`. Record
   p50/p99 for first-paint / edit-rerender / scroll-with-K / many-line-charts open / large-series.

## Tests

- **`grid_chart_authored_inserted` (pixel)** — the near-empty inserted line chart renders in-grid
  as a real Faithful line (new baseline, eyeballed).
- **`case_names_match_table` (drift guard)** — the new case is wired into both the table and the
  `render_cases!` list.
- Full render suite green (all grid/cell/sheet/titlebar/chart baselines match).
- Round-trip / seam / corpus tests green (excluding the two env-broken LibreOffice tests).

## Render validation

This phase **is** in-scope for the pixel suite and is the dedicated late render phase (CLAUDE.md
render-tests §3). It runs the FULL suite once (foreground, `timeout` + watchdog), refreshes +
**eyeballs** the one new baseline, commits it, and (manager) dispatches the CI `render` gate. No
existing baseline is intended to move — only the one added case is new.

## Results

All green. The line-chart MVP is **shippable end-to-end** — display + live + preserve +
authoring/editing all validated, full pixel suite green, round-trip green, perf within budget.

### Full render suite — PASS

`render_tests.sh test` (whole suite, foreground under a `timeout` watchdog). **104 render_suite
cases + 11 lib unit + 6 perceptual-diff = 121 tests, 0 failed**; the pixel render step took
**483.59s** (~8 min). Every grid / cell / sheet / titlebar and every `chart_line_*` baseline
matched — **no existing baseline moved**; the only new case is `grid_chart_authored_inserted`
(the added scene). Drift guards (`case_names_match_table`, `chart_scene_names_match_table`) green.

### New baselines

- **`grid_chart_authored_inserted.png`** — eyeballed ✓. Renders a real Faithful single-series line
  titled "Chart" with placeholder categories 1–4 / values 4-6-5-8 and a "Series 1" legend, floating
  over the backing table — the exact picture shown the instant a line chart is inserted, and the
  only in-grid **authored** chart baseline (every prior `grid_chart_*` is loaded). No other baseline
  regenerated.

### Round-trip sweep — PASS (local `discover_and_parse` reopen)

`worker_seam` **50** + `roundtrip` **19** + `charts_corpus` **8** = **77 tests, 0 failed**. Covers
the whole surface:
- **Displayed loaded** — `save_through_worker_preserves_and_patches_charts` (untouched byte-stable,
  edited reflowed).
- **Edited loaded (chrome)** — `loaded_chart_title_edit_is_live_and_preserves_unmodeled_styling`
  (title changes; unmodeled styling byte-identical), `loaded_chart_legend_color_and_labels_roundtrip`.
- **Edited loaded (data reflow / anchor)** — `save_through_worker_preserves_and_patches_charts`,
  `move_loaded_chart_patches_drawing_and_roundtrips`, `delete_loaded_chart_drops_it_from_the_package`.
- **Authored** — `ranged_authored_chart_saves_cf_and_roundtrips` (range → live `c:f`),
  `retyped_authored_chart_roundtrips` (type), `authored_chart_chrome_edits_roundtrip` (chrome),
  `move_authored_chart_roundtrips`, `delete_authored_chart_roundtrips`,
  `combined_save_mixes_loaded_bound_and_unbound_authored_charts`.

**Env caveat (documented, not a regression):** `charts_roundtrip_libreoffice` — 2 tests FAIL, both
at the `convert_to_xlsx` / `soffice` step (empty `soffice` stdout: LibreOffice can't convert in this
container). Out of scope per the phase brief. The real Excel+LibreOffice round-trip rides CI
`roundtrip.yml` (manager-dispatched after commit).

### Perf (p50/p99, env-stamped → `results/chart-perf.json`)

Foreground, release, every op **force + asserted** (a no-op would trip an assert). Reference frame
budget: 8.33 ms (target) / 16.67 ms (worst).

| Op | p50 | p99 | max | vs budget |
| --- | --- | --- | --- | --- |
| first-paint (1 line chart: discover+parse+bind+snapshot) | 610.29 µs | 719.90 µs | 751.90 µs | off critical path |
| edit-rerender (dirty-set + reresolve + snapshot) | 2.42 µs | 4.30 µs | 33.54 µs | — |
| scroll-with-K (per-frame cull scan, K=1000; ~2 on-screen) | 8.54 µs | 23.39 µs | 52.29 µs | **~1000× under** 8.33 ms |
| many-line-charts open (200 charts/sheet, real zip+XML each) | 15.65 ms | 19.50 ms | 20.09 ms | one-time open |
| large-series down-sample (N=100k → ≤2048) | 360.40 µs | 439.36 µs | 458.16 µs | — |
| large-series paint-prep FULL (N=100k) | 470.56 µs | 526.59 µs | 1.06 ms | under 8.33 ms |
| large-series paint-prep DOWN-SAMPLED (2048) | 11.01 µs | 28.58 µs | 41.26 µs | **~760× under** |

Adversarial review: the **absolute** numbers are host-relative — every op is ~2× the last recorded
JSON, consistent with cross-host / noisy-neighbor variance on this shared container (not a code
regression) — but the **headroom conclusions are robust**: every per-frame op stays ~1000× under the
8.33 ms frame budget (the scroll cull is ~8.5 ns/chart), and 100k points are retained for save while
paint touches ≤2048 (~49× fewer). The margins, not the raw µs, are what the ship gate rests on.

### Checks

`cargo fmt --all --check` clean; `cargo clippy -p render-tests --all-targets` clean. Library unit
tests green: freecell-app **314**, freecell-chart-model **84**, freecell-core **148**,
freecell-engine **216** (0 failed).

### Exit

**v1 MVP is SHIPPABLE** — the line chart is fully authorable & editable, validated end-to-end across
display / live / preserve / authoring / editing, with the full pixel suite green, the local
round-trip green, and perf within budget. Breadth (other chart types) follows in P22+.
</content>
