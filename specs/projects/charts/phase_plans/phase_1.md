---
status: complete
---

# Phase 1: Crate scaffolding & placement

## Overview

Seed the production chart crates by lifting the proven PoC layers
(`experiments/chart-poc/`) into their FreeCell homes **by charter**
(`architecture.md §2`), so the `app/` workspace compiles, the PoC unit tests pass in
their new homes, and nothing about non-chart behavior changes. No chart is wired into
the grid or the file I/O paths yet — this phase is *placement only*. Later phases widen
the model (P2), add the fidelity accessor (P3), lift the capture harness into
`render-tests` (P4), build the production line renderer (P5), and wire load/render/save
into the app (P7–P10).

### Placement map (architecture.md §2)

| PoC source | Production home | Notes |
|---|---|---|
| `chart-poc/chart-model/src/lib.rs` | **new crate `freecell-chart-model`** | gpui-free, ironcalc-free — the stable seam. Dedicated sibling crate, as the architecture recommends ("keep it explicit + core lean"). |
| `chart-poc/load-save/src/{load,save,xlsx,authoring}.rs` | **`freecell-engine::chart`** | the engine owns IronCalc + file I/O + the zip/roxmltree second pass. |
| `chart-poc/chart-render/src/{palette,ticks,stacking,style,chrome,line,bar,area,pie,scatter}.rs` + `chart_element` dispatch | **`freecell-app::chart`** | needs gpui + gpui-component. |

### Two deliberate scope decisions (flagged for the reviewer)

1. **Copy into homes; leave `experiments/chart-poc/` frozen as-is.** Experiments are the
   repo's historical de-risking record (their `findings.md` + committed `results/` PNGs
   reference the exact code that produced them), and the PoC is a *separate* Cargo
   workspace that does not touch the `app/` build. The production crates are *seeded from*
   the PoC and then diverge (P2 widens the model, P5 rewrites the line renderer); the PoC
   stays as the immutable PoC snapshot. So this is a copy-into-homes, not a delete-the-PoC.
   If the manager prefers the PoC source physically removed, that is a trivial reversible
   follow-up.
2. **The capture harness is NOT moved here.** `chart-render`'s `scenes.rs` / `render.rs`
   (window driver) / `capture.rs` and every experiment binary (`render_scene`, `capture`,
   `fixtures`, `render_loaded`, `capture_loaded`) are the subject of **P4** ("Lift the
   capture harness into `render-tests`"). Moving them into `freecell-app` now would add
   throwaway binary surface that P4 relocates again. Only the render *library* (widgets +
   shared infra + `chart_element` dispatch) lands in `freecell-app::chart`.

## Steps

1. **New crate `app/crates/freecell-chart-model/`.**
   - `Cargo.toml`: package `freecell-chart-model`, `[lib] name = "freecell_chart_model"`,
     workspace-inherited `version/edition/license/rust-version`, `publish = false`,
     `[lints] workspace = true`. No dependencies (pure std).
   - `src/lib.rs`: the model, copied from `chart-poc/chart-model/src/lib.rs` verbatim
     (update only the module-header doc comment to name the new home). Types + tests
     unchanged (`Color`, `BarDir`, `Grouping`, `ChartKind`, `Category`, `SeriesData`,
     `Series`, `Axis`, `LegendPosition`, `Legend`, `Chart`, `format_number`).

2. **Workspace wiring (`app/Cargo.toml`).**
   - Add `"crates/freecell-chart-model"` to `[workspace] members`.
   - Add to `[workspace.dependencies]`:
     - `freecell-chart-model = { path = "crates/freecell-chart-model" }`
     - `zip = "0.6"` and `roxmltree = "0.19"` (both already resolved in `Cargo.lock`; the
       chart file layer needs them at runtime).

3. **`freecell-engine::chart` module.**
   - Create `crates/freecell-engine/src/chart/` with `mod.rs` (module decls + re-exports,
     mirroring `load-save/src/lib.rs`) and `load.rs`, `save.rs`, `xlsx.rs`, `authoring.rs`
     copied from the PoC with two mechanical rewrites:
     - `chart_model::` → `freecell_chart_model::`,
     - intra-layer `crate::{load,save,xlsx,authoring}` → `super::{…}` (they are siblings
       under `chart`).
   - `crates/freecell-engine/src/lib.rs`: add `pub mod chart;`.
   - `crates/freecell-engine/Cargo.toml`: add `freecell-chart-model.workspace = true`,
     `anyhow.workspace = true`, `roxmltree.workspace = true`, and **promote** `zip` from a
     dev-dependency to `zip.workspace = true` under `[dependencies]` (the existing
     `worker::run` test that used the dev `zip` still resolves it as a runtime dep).

4. **`freecell-app::chart` module.**
   - Create `crates/freecell-app/src/chart/` with `mod.rs` (module decls for
     `palette, stacking, ticks, style, chrome, line, bar, area, pie, scatter` + the
     `pub fn chart_element(&Chart) -> Option<gpui::AnyElement>` dispatch, mirroring
     `chart-render/src/lib.rs` minus the harness modules) and the ten widget/infra files
     copied from the PoC with the same two mechanical rewrites (`chart_model::` →
     `freecell_chart_model::`; `crate::{palette,ticks,…}` → `super::{…}`).
   - `crates/freecell-app/src/lib.rs`: add `pub mod chart;`.
   - `crates/freecell-app/Cargo.toml`: add `freecell-chart-model.workspace = true`
     (gpui / gpui_platform / gpui-component / gpui-component-assets already present).

5. **Green the workspace.** `cargo fmt --all`, then iterate `cargo clippy --workspace
   --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace`
   until clean.

## Tests

All tests are the PoC's own unit tests, carried along verbatim with the moved code — they
must pass in their new homes (the phase's "PoC unit tests pass" exit criterion). No new
tests are written: this is a pure relocation and adding tests would not guard anything the
PoC did not already guard.

- **`freecell-chart-model`** (4): `color_hex_round_trips`,
  `category_labels_render_text_and_numbers`, `series_len_reflects_underlying_data`,
  `chart_round_trips_through_accessors`.
- **`freecell-engine::chart`** (14):
  - `load`: `parses_column_chart_kind_values_and_chrome`,
    `scatter_maps_two_value_axes_and_xy_series`, `doughnut_reads_hole_size_and_pie_has_none`,
    `missing_chart_group_is_an_error`.
  - `xlsx`: `resolves_relative_and_absolute_targets`, `rels_part_naming`,
    `parses_rels_skipping_external`.
  - `save`: `relative_part_between_worksheet_and_drawing`,
    `patch_worksheet_injects_drawing_and_binds_r_namespace`,
    `merge_content_types_adds_missing_chart_overrides`, `roundtrip_preserves_charts`
    (drives the real IronCalc load + writer via the pinned fork).
  - `authoring`: `each_authored_chart_part_parses_to_expected_kind`,
    `written_fixture_loads_three_charts_with_cached_values`, `fixture_loads_in_ironcalc`.
- **`freecell-app::chart`** (headless — the widget builders, not `paint`):
  `line`, `bar`, `area`, `pie`, `scatter` module tests (shared-scale coverage, stacked/
  percent segment math, distinct palette colors, kind rejection).
- **Guard preserved:** `freecell-core::tests::dependency_rule` still passes —
  `freecell-chart-model` is gpui/ironcalc-free, engine gains only `zip`/`roxmltree`/
  `anyhow`/`freecell-chart-model` (none gpui), so core stays gpui/ironcalc-free and engine
  stays gpui-free.

## Render validation

**Out of scope for this phase (no pixel suite run).** No chart is wired into `GridView`;
the `freecell-app::chart` widgets are dormant library code and cannot move any grid /
cell / sheet / titlebar baseline (CLAUDE.md render-test scope). In-grid chart rendering —
and its render-test coverage — begins at **P8**. This phase validates only via the
carried-over unit tests + the standard `fmt`/`clippy`/`build`/`test` gate.
