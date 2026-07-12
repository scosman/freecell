---
status: complete
---

# Phase 4: Render-test harness (chart scenes)

## Overview

Bring the **chart** capture / scene / diff path (deferred from P1) into the app's
`render-tests` crate so a chart **widget** can be rendered headless (Xvfb + Mesa lavapipe +
`xrefresh` + ImageMagick `import`) and **perceptually diffed** against a committed baseline —
the same proven mechanism `render-tests` already uses for the grid (charts/architecture §7,
implementation_plan P4).

The capture stack, the perceptual diff (`diff.rs`, ported from round-3 C), and the
Xvfb/lavapipe/`xrefresh`/`import` mechanism **already live in `render-tests`** (lifted for the
grid in the MVP project). So this phase does **not** re-lift that infrastructure — it **adds the
chart-specific layer on top of it**:

- a **chart scene registry** (`chart_scene.rs`) that builds `freecell_chart_model::Chart`
  fixtures, mirroring the PoC's `scenes.rs` but trimmed to the render-test need;
- a **chart render entry point** (`render.rs::run_chart_scene`) that opens one window hosting
  `freecell_app::chart::chart_element(&chart)` (the P1-lifted render widgets) in a
  gpui-component `Root`, exactly as the PoC's `run_render_chart` did;
- **chart dispatch** in the `render_scene` bin (`--chart <name>`) and a **chart capture path**
  (`capture.rs::render_charts`) that reuses the existing per-case Xvfb capture core;
- a **chart pixel-diff test** wired into the existing gate + failure-artifact machinery;
- one **committed baseline** for one PoC chart scene (`chart_line_multi`), generated + eyeballed.

**Exit criterion (P4):** one PoC chart scene renders headless + diffs green in the
`render-tests` harness. **No app/grid integration** — the chart is rendered standalone in its
own window (in-grid `ChartLayer` is P8).

### Why `chart_line_multi` as the one proof scene

The plan's next production phase is the **line renderer** (P5), and the multi-series line is the
make-or-break Gate-1 scene (functional_spec §3/§7): it exercises the full chrome — multiple
series on **one shared value scale**, a nice-tick numeric value axis, category axis, title, both
axis titles, and a legend whose swatches match the line colors. It is the richest single proof
that the chart render → capture → diff path works end-to-end, so it is the scene to baseline
now. (P5 adds the production line renderer + its own baselines through this same harness; this
P4 baseline is expected to be regenerated then.)

## Design: reuse the existing capture core, add the chart layer

The grid and chart paths differ only in **which widget** renders and **which flag** the
`render_scene` bin dispatches on. Everything downstream (own-Xvfb-per-case sizing, `xrefresh`,
find-window-by-size, `import`, blank-check, perceptual diff, gate, failure artifacts) is
identical, so the chart path threads through the **same** capture core and the **same** diff /
gate / baseline machinery.

`capture.rs` is refactored to expose that core as a generic `capture_window(launch_cmd,
viewport, icd, out, label)` (+ `capture_script(launch_cmd, …)` + `default_exit_after_ms()`),
the shape the PoC's own capture harness already used. The grid `render_one` delegates to it with
`launch = "<bin> --case <name> --exit-after-ms <n>"`; the new `render_charts` delegates with
`launch = "<bin> --chart <name> --exit-after-ms <n>"`. This is a **pure refactor of the grid
path** — the generated bash script is byte-identical, so grid pixels do not move (verified by
rendering a grid case after the change).

## Steps

1. **`app/render-tests/Cargo.toml`** — add `freecell-chart-model.workspace = true` (needed to
   build `Chart` fixtures directly; today only pulled transitively via `freecell-app`).

2. **New `app/render-tests/src/chart_scene.rs`** — the chart scene registry (gpui-free data):
   ```rust
   pub struct ChartScene { pub name: &'static str, pub viewport: (u32, u32), pub chart: Chart }
   pub fn all() -> Vec<ChartScene>          // seeded with one scene
   pub fn get(name: &str) -> Option<ChartScene>
   ```
   Seed with `chart_line_multi` (the Gate-1 multi-series line, from PoC `scenes::line_multi`),
   viewport `(720, 460)`. `all()` returns a `Vec` so later phases add rows trivially (mirrors
   the grid `cases::all()` pattern). Unit tests: every scene is name-lookupable + non-empty;
   `chart_line_multi` is a multi-series `Line`.

3. **`app/render-tests/src/render.rs`** — add the standalone chart render path:
   - `struct ChartSceneView { chart: Chart }` whose `Render` calls
     `freecell_app::chart::chart_element(&self.chart)` (fallback: a white `div` if `None`),
     wrapped in a gpui-component `Root` — the PoC's `ChartView` shape.
   - `pub fn run_chart_scene(scene_name: &str, exit_after_ms: u64) -> Result<()>` — look the
     scene up, open a viewport-sized window at the origin (same `WindowOptions` as the grid
     path), register the bundled **Inter** fonts (`freecell_app::shell::register_fonts`, so
     chart text is font-stable like the grid), self-quit on the executor timer.

4. **`app/render-tests/src/bin/render_scene.rs`** — dispatch: `--chart <name>` →
   `run_chart_scene`; else `--case <name>` → `run_render_scene` (unchanged grid path); neither →
   usage error. Keep exit codes.

5. **`app/render-tests/src/capture.rs`** — extract the generic capture core and add the chart
   entry:
   - `pub fn default_exit_after_ms() -> u64` = `((settle+present)*1000)+8000` (the exact value
     the grid script already baked in).
   - `fn capture_window(launch_cmd, viewport, icd, out, label)` — own-Xvfb run + blank-check
     (the body lifted verbatim from the grid `render_one`).
   - `fn capture_script(launch_cmd, icd, viewport, out)` — the bash template, now parameterized
     on `launch_cmd` instead of a `RenderCase` (byte-identical output for the grid caller).
   - Grid `render_one` delegates to `capture_window` with a `--case` launch string.
   - `pub fn render_charts(render_scene_bin, out_dir, only) -> Result<Vec<String>>` — iterate
     `chart_scene::all()` (honoring the `only` prefix), capture each via `capture_window` with a
     `--chart` launch string.

6. **`app/render-tests/src/lib.rs`** — `pub mod chart_scene;`, re-export `render_charts`, note
   the chart scenes in the crate doc.

7. **`app/render-tests/src/bin/generate_baselines.rs`** — after grid `render_all`, also
   `render_charts` into the same staging dir; classify (new/changed/unchanged vs the committed
   baseline) + copy all. The `--only <prefix>` filter composes cleanly: `--only chart_` renders
   only chart scenes (no grid case name starts with `chart_`); `--only cell_` renders only grid
   cells.

8. **`app/render-tests/tests/render_suite.rs`** — add the chart pixel gate **additively**
   (reusing every existing helper — `gate`, `Rendered`, `require_baseline`, `target_subdir`,
   `write_failure_artifacts`, `DiffOptions`):
   - a second `CHART_RENDERED: OnceLock<Rendered>` + `chart_rendered()` that renders all chart
     scenes once (via `render_charts`) into `target/chart-render-actual` under the **same gate**;
   - `check_chart_case(name)` (mirrors `check_case`);
   - a `chart_render_cases! { chart_line_multi }` macro → one `#[test]` per scene + a
     `CHART_SCENE_NAMES` slice + `chart_scene_names_match_table` drift guard.
   Filtering `render_tests.sh test chart_` runs only the chart `#[test]`s → only the chart
   `OnceLock` initializes → only chart scenes render (the grid `OnceLock` stays cold).

9. **Generate + eyeball + commit the baseline.** Run `setup_render_env.sh` (capture stack), then
   `render_tests.sh generate --only chart_`; **Read the generated `baselines/chart_line_multi.png`
   with vision** to confirm it is a real multi-series line chart (three colored lines on one
   scale, numeric value axis, category axis, title, axis titles, legend), then commit it with the
   harness.

10. **`app/render-tests/README.md`** — a short "Chart scenes" note (the parallel `chart_scene`
    table + the `chart_` filter), so future agents know where chart scenes live.

## Tests

- **`chart_scene.rs` (unit, no GPU):** `every_scene_is_lookupable_and_nonempty`;
  `line_multi_is_a_multi_series_line`.
- **`render_suite.rs` (gate + drift, no GPU):** `chart_scene_names_match_table` (the
  `chart_render_cases!` list and `chart_scene::all()` cannot drift) — runs on every `cargo test
  --workspace`.
- **`render_suite.rs` (pixel gate, `FREECELL_RENDER=1` + capture stack):** `chart_line_multi` —
  renders the chart widget headless and perceptually diffs it against the committed baseline
  (green = the exit criterion). Skips cleanly without `FREECELL_RENDER`; fails loudly if
  requested but the capture stack is missing (reuses the existing `gate`).

## Render validation (this phase)

Per CLAUDE.md, this phase renders a **new scene** but does **not** move any existing grid /
cell / sheet / titlebar baseline (charts render in their own standalone window). So:

- Run `setup_render_env.sh` first (capture stack not yet installed).
- Verify with the **relevant chart scene only**: `render_tests.sh test chart_` — do **not** run
  the full pixel suite (deferred to the manager's late validation phase).
- Also render one **grid** case (`render_tests.sh test cell_plain`) once, to confirm the
  `capture.rs` refactor left the grid capture path byte-identical.
- Commit the eyeballed `chart_line_multi` baseline with the harness.

## Checks (foreground, under `timeout`)

Run the full workspace gate green (from `app/`):
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build --workspace`
- `cargo test --workspace`
- `RUSTDOCFLAGS="-D warnings" cargo doc -p render-tests`
- plus `render_tests.sh test chart_` (+ a `cell_` sanity render).
</content>
</invoke>
