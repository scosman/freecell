---
status: complete
---

# Phase 7: Render-test harness + initial suite

## Overview

Build the automated pixel-truth suite for the grid (`components/render_test_harness.md`):
render the **real** `GridView` over scenes produced by the **real** engine
(`freecell-engine` worker / `DocumentClient`), capture PNGs on Linux under
**Xvfb + Mesa lavapipe** (the capture variant the Phase-1 spike proved — option 2:
render to an X window, force presentation with `xrefresh`, capture the root with
ImageMagick `import`), and perceptually diff against committed baselines (the diff ported
from round-3 C `ci_rendering`). Ship a `generate_baselines` tool, a README documenting the
human baseline-review process, an initial ~45-case suite with committed baselines, and
wire it into the required Linux CI job.

This is the **first place the real engine meets the real grid**: scenes flow
engine → publication + style/geometry cache → `GridView` → pixels.

## Key design decisions (recorded in DECISIONS_TO_REVIEW.md)

- **Capture = Phase-1 spike option 2, per case as an isolated subprocess.** Each case is
  rendered by a thin gpui bin (`render_scene`) that opens ONE window sized to the case
  viewport at origin `(0,0)`; the harness (Rust, mirroring `linux_render_spike.sh`) runs
  `xrefresh` then `import -window root` and crops to the viewport. Subprocess-per-case =
  full isolation, no window-resize API, no stale-pixel races. The harness discovers the
  lavapipe ICD and sets the software-Vulkan env on the child, so only a `DISPLAY` (Xvfb)
  is required externally.
- **Scene builder drives the real worker.** Values/formulas/errors/booleans and
  number formats go through `SetCellInput` (IronCalc **infers** currency/percent/thousands/
  date from the input string — probed). Bold/italic/underline/fill go through
  `SetStyleAttr` (the real style-cache mirror). The worker produces the real `Publication`
  + `SheetCaches`.
- **Command-less render features are injected into the real read model.** The MVP worker
  protocol has **no command** for alignment, explicit font colour, or column/row geometry
  (these come from opened files, not edits). Render features exercising them
  (`cell_align_*`, `cell_tall_row`, `cell_wide_column`, `grid_variable_geometry`) apply
  the change to the real `SheetCache` the grid consumes (`set_col_width`,
  `set_cell_style`, …) — the same mutators the worker itself uses — after the worker builds
  it. Documented; this is how Phase 6 itself tested alignment/geometry.
- **`cell_number_negative_red`: the `[Red]` number-format COLOUR is deferred.** The worker
  publishes `PublishedCell.text_color = None` (Phase-4 decision; the palette-index→RGB
  mapping is future work), so the baseline shows the negative number correctly formatted in
  the default colour. The case stays in the table so the feature is tracked; its baseline
  updates when text_color is wired.
- **Suite gates on `DISPLAY`.** With no display (the plain `cargo test --workspace` CI
  step) the render integration test skips; the GPUI-free diff unit tests always run. A new
  required CI step runs `scripts/render_tests.sh` (xvfb-run + lavapipe) → real pixel gate,
  replacing the Phase-1 informational spike step.

## Steps

1. **`render-tests/Cargo.toml`** — add deps: `freecell-app`, `freecell-engine`,
   `freecell-core`, `gpui`, `gpui_platform`, `gpui-component`, `gpui-component-assets`,
   `image`, `anyhow`, `tracing-subscriber` (workspace pins). Declare `[lib]`, `[[bin]]
   render_scene`, `[[bin]] generate_baselines`.
2. **`src/diff.rs`** — port round-3 C `ci_rendering`: `DiffOptions {per_channel_tolerance:
   12, fail_fraction: 0.005}`, `DiffReport`, `diff_images`, `diff_png_files`, plus a new
   `diff_image(a,b,opts) -> RgbaImage` that paints differing pixels red for the failure
   artifact. GPUI-free.
3. **`src/scene.rs`** — `Scene` fluent builder (`input`, `bold`/`italic`/`underline`/
   `fill` over a `CellRange`, `align`, `font_color`, `col_width`, `row_height`, `publish`
   window) and `build_sources(&Scene) -> anyhow::Result<GridDataSources>` that spawns a
   `DocumentClient(NewWorkbook)`, applies edits, sets the viewport, drains to idle, reads
   the real `Publication` + caches, then applies the command-less cache injections.
4. **`src/cases.rs`** — `RenderCase { name, scene, viewport, selection, loading,
   force_scrollbars, reveal }` + `all() -> Vec<RenderCase>` with the ~45 cases from the
   component doc's inventory. Single source of truth (the bin looks up by name).
5. **`src/render.rs`** — the gpui side: `render_case_window(case, cx)` opens a window sized
   to the viewport with a `GridView` over `build_sources`, applying the case's
   selection/loading/force_scrollbars/reveal, inside a `Root`. `run_render_scene(name, ms)`
   wraps `application().run(...)` + the exit timer (mirrors `main.rs`).
6. **`src/capture.rs`** — `render_all(out_dir, filter) -> Result<()>`: for each case,
   spawn `render_scene --case <name> --exit-after-ms N` (lavapipe env + inherited DISPLAY),
   wait, `xrefresh`, wait, `import -window root`, crop to the viewport, assert non-blank.
   `have_display()` helper.
7. **`src/lib.rs`** — wire the modules; re-export `DiffOptions`, `diff_*`, `cases`,
   `render_all`.
8. **`src/bin/render_scene.rs`** — parse `--case` / `--exit-after-ms`, call
   `run_render_scene`.
9. **`src/bin/generate_baselines.rs`** — `render_all(baselines_dir, filter)` with a
   changed/unchanged summary and `--only <prefix>`.
10. **`tests/perceptual_diff.rs`** — port round-3 C's 6 discriminating-power tests
    (identical pass, within-tolerance pass, genuine-change fail, dimension mismatch,
    threshold discriminating, png round-trip). GPUI-free, always run.
11. **`tests/render_suite.rs`** — `render_all_once()` (OnceLock; renders every case into a
    temp dir when `DISPLAY` is set, else marks skipped) + one `#[test]` per case (macro)
    that diffs `<tmp>/<name>.png` vs `baselines/<name>.png`, writing
    `target/render-failures/<name>.{actual,baseline,diff}.png` + stats on failure;
    `baseline_missing_fails_actionably`.
12. **`scripts/render_tests.sh`** — xvfb-run + lavapipe wrapper for `cargo test -p
    render-tests` (and baseline regen), mirroring `linux_render_spike.sh`.
13. **CI (`.github/workflows/checks.yml`)** — replace the informational render-spike step
    with a required `scripts/render_tests.sh` step; upload `target/render-failures/` on
    failure.
14. **Generate + spot-check baselines**; update `render-tests/README.md` with the pinned
    runner image + Mesa/lavapipe version and the tolerance constants.

## Tests

- **Diff unit tests** (`tests/perceptual_diff.rs`, GPUI-free): `identical_images_pass`,
  `within_tolerance_perturbation_passes`, `genuine_change_fails`,
  `dimension_mismatch_errors`, `threshold_is_discriminating`, `png_roundtrip_diff` —
  the diff has real discriminating power (C's proven cases).
- **Render suite** (`tests/render_suite.rs`): one `#[test]` per RenderCase asserting the
  fresh render perceptually matches the committed baseline within tolerance; the suite is
  green (deterministic lavapipe render). `baseline_missing_fails_actionably` — a case
  without a baseline fails telling the human to run `generate_baselines`, not a panic.
- **Scene builder** covered indirectly (every case exercises `build_sources`); a small
  `scene_number_formats_infer` check that the engine yields the expected formatted display
  strings guards the inference assumption.
