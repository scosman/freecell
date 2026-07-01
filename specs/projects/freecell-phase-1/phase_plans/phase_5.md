---
status: draft
---

# Phase 5: UI PoC — GPUI Proof-of-Concept (Sub-project E)

## Overview

Sub-project E is the **only UI sub-project** and is **engine-neutral** (no
spreadsheet engine — a static datamodel provider). It answers: *can GPUI render an
Excel-max spreadsheet grid at the §5.4 perf bar, and how does raw `gpui` compare to
`gpui-component`?*

It ships **one macOS/Metal app** with two purposes (functional_spec §6.E):
1. **Interactive scrolling app** — the human scrolls/jumps around to judge feel.
2. **"Run Test" harness** (menu item + CLI flag) — runs scripted scroll / fast-scroll
   / horizontal / jump-to-cell / random-jump sequences by advancing the viewport
   frame-by-frame, measures per-frame render time + newly-visible-cell load latency,
   computes p50/p99/max, logs to `results/`, and prints measured PASS/FAIL vs §5.4
   (frame p99 ≤ 8.3 ms / 120 fps; ≤ 16.6 ms worst-case; cell-load p99 < 2 ms).

**Two variants over the same provider, compared:**
- `raw-gpui/` — a custom virtualized grid on raw `gpui` primitives. Visible range via
  **cumulative-size prefix sums + binary search** over variable row heights / column
  widths (architecture §7).
- `gpui-component/` — its virtualized `Table`/list. Part of the finding is *whether it
  handles 2D + variable sizes at Excel-max scale*.

**CRITICAL ENVIRONMENT REALITY.** GPUI targets macOS/Metal and **cannot build or run
in this headless Linux container** (no GPU/display; `gpui-component` git-pins a Zed
commit with heavy system deps). So this phase:
- Does **not** `cargo build`/run the GPUI crates here — that would fail on system libs.
- Maximizes compile-correctness by **mirroring real examples** from
  `github.com/longbridge/gpui-component` and `github.com/zed-industries/zed` (Cargo git
  pins, imports, `Render`/`Context`/`Window` APIs).
- Puts **all engine-neutral logic** (provider adapter, layout math, harness
  script/stats/gating, results recording) into a **`poc-core/` crate that IS
  `cargo check`-able on Linux** (no `gpui` dep), so the load-bearing logic is verified
  in-container. Only the thin gpui rendering shells stay unverifiable here.
- Attestation is `Checks: NA` / `Tests: NA` for the gpui shells, but `poc-core` checks
  + tests DO run in-container (reported honestly).
- The **human runs the app on their Mac** and reports build success, feel, and the Run
  Test PASS/FAIL numbers; `findings.md` has a **"HUMAN RUN REQUIRED"** section listing
  exactly what to run.

The static datamodel provider already exists: `shared/datagen`'s
`trait CellSource { fn cell(row,col) -> CellData }` + `SyntheticSheet` (deterministic
generator: varied text, numbers, ~15% highlights, scattered bold/italic, variable
row/col widths incl. very-wide columns, Excel-max constants). This phase **consumes it
read-only by relative path** and adds only the render-facing glue.

## Layout

```
04-ui-poc/
  findings.md                    # §5.2 headings + HUMAN RUN REQUIRED section
  poc-core/                      # engine-neutral, cargo-check-able on Linux (NO gpui)
    Cargo.toml                   # deps: datagen, bench_util (relative paths)
    src/
      lib.rs
      layout.rs                  # ColumnLayout/RowLayout: prefix sums + binary search
      style.rs                   # CellData -> render-ready ARGB / weights (gpui-free)
      harness.rs                 # scripted viewport script, per-frame sample recording
      report.rs                  # p50/p99/max + GateResult + BenchResult -> results/
      config.rs                  # grid dims, viewport, overscan, PASS/FAIL thresholds
  raw-gpui/                      # macOS-only cargo project (gpui git pin)
    Cargo.toml                   # deps: gpui (git pin), poc-core, datagen, bench_util
    src/main.rs                  # Application bootstrap, menu, CLI flag
    src/grid.rs                  # custom virtualized grid (absolute-positioned cells)
    README.md                    # macOS build/run + Run Test instructions
  gpui-component/                # macOS-only cargo project (gpui + gpui-component pins)
    Cargo.toml
    src/main.rs
    src/table.rs                 # TableDelegate over the provider
    README.md
  scripts/
    build_and_run.sh             # one-command build+run (interactive)
    run_test.sh                  # one-command headless-ish "Run Test" + dump results
    README.md                    # what the human pulls/runs/reports
  results/
    .gitkeep                     # runtime PASS/FAIL JSON lands here on the Mac run
```

## Steps

1. **`poc-core/` crate (the load-bearing, in-container-checkable core).**
   `Cargo.toml` deps: `datagen = { path = "../../shared/datagen" }`,
   `bench_util = { path = "../../shared/bench_util" }`. **No gpui.** edition 2024.
   - `config.rs`: `PocConfig` — grid `rows`/`cols` (default Excel-max
     1_048_576 × 16_384 via `datagen::{EXCEL_MAX_ROWS, EXCEL_MAX_COLS}`), viewport
     `width`/`height` (px), `overscan` rows/cols, header sizes, seed. Threshold
     consts: `FRAME_TARGET_NS = 8_300_000` (120 fps), `FRAME_WORST_NS = 16_600_000`
     (60 fps), `CELL_LOAD_TARGET_NS = 2_000_000`.
   - `layout.rs`: `Axis` with a **cumulative-size prefix-sum + binary search** mapping
     of scroll offset → first visible index, and index → pixel offset, over variable
     sizes. Because Excel-max has 1M+ rows, do **not** materialize a full prefix-sum
     array; use a **chunked/segment-summed** structure (block sums of size B, plus a
     within-block scan) so memory stays O(n/B) and lookups stay O(log + B). Public:
     ```rust
     pub struct Axis { /* count, block sums, size_fn */ }
     impl Axis {
         pub fn new(count: u32, sizer: impl Fn(u32) -> f32 + 'static) -> Self;
         pub fn total(&self) -> f64;
         pub fn offset_of(&self, index: u32) -> f64;          // px to start of index
         pub fn index_at(&self, offset: f64) -> u32;          // first index at/after px
         pub fn visible_range(&self, scroll: f64, extent: f64, overscan: u32)
             -> std::ops::Range<u32>;                          // [start, end)
     }
     ```
     Sizers come from `SyntheticSheet::col_width` / `row_height`.
   - `style.rs`: gpui-free render-ready conversion of a `CellData` into primitives the
     shells consume: `RenderCell { text: String, argb: u32 (fill or white), text_argb:
     u32, bold: bool, italic: bool, align: Align }`. `text` formats numbers/text/empty.
     `Rgb -> 0xRRGGBB` helper. This keeps *both* shells' cell-drawing identical and
     testable without gpui.
   - `harness.rs`: the **viewport script**. `ScriptStep { scroll_to: (f64,f64) }`
     produced by generators: `scroll_down`, `fast_scroll`, `horizontal`,
     `jump_to_cell`, `random_jump` (seeded, deterministic). A `Harness` advances the
     script one step/frame at a time, exposes the target `(scroll_x, scroll_y)` for the
     shell to apply, and records two samples per frame:
     `frame_render_ns` (measured by the shell around its render) and `cell_load_ns`
     (measured by the shell around pulling newly-visible `CellData`). Provide the
     newly-visible-cell computation here (`newly_visible(prev_range, cur_range)`), so
     the shell only times the provider pulls.
     ```rust
     pub struct FrameSample { pub frame_render_ns: u64, pub cell_load_ns: u64,
         pub newly_visible: u32 }
     pub struct Harness { /* steps, cursor, samples */ }
     impl Harness {
         pub fn scripted(cfg: &PocConfig) -> Self;            // full canonical script
         pub fn next_viewport(&mut self) -> Option<(f64, f64)>; // None => done
         pub fn record(&mut self, s: FrameSample);
         pub fn samples(&self) -> &[FrameSample];
     }
     ```
   - `report.rs`: turn recorded samples into `bench_util::LatencyStats` for frame time
     and cell-load, build `GateResult`s (frame p99 ≤ 8.3 ms; frame max ≤ 16.6 ms;
     cell-load p99 ≤ 2 ms), assemble a `bench_util::BenchResult` (name = variant, date
     passed in, `Environment::detect(commit)`), print human PASS/FAIL summaries, and
     write JSON to `results/<variant>-runtest.json`. Provide
     `fn finalize(variant, date, commit, samples, out_dir) -> RunReport` returning the
     gates + a printed report string, so `main.rs` (both shells) is a thin call.
   - `lib.rs`: re-exports; module docs.

2. **`raw-gpui/` shell (custom virtualized grid on raw gpui).**
   `Cargo.toml`: `gpui = { git = "https://github.com/zed-industries/zed", rev = "<pinned>" }`
   (pin mirrored from gpui-component's own manifest for cross-compat), plus `poc-core`,
   `datagen`, `bench_util` by path. Mirror a real gpui example for bootstrap.
   - `src/main.rs`: `Application::new().run(|cx| { … })` — build the `SyntheticSheet`
     provider + `Axis`es, register a `RunTest` action + a "Run Test" menu item
     (`cx.set_menus`), parse a `--run-test` CLI flag (auto-run the harness on launch,
     dump results, exit), open a window with the `Grid` root view.
   - `src/grid.rs`: a `Grid` entity implementing `Render`. State: scroll offset,
     provider, axes, harness (when running). `render`:
     - Compute `visible_range` for rows and cols from scroll + `viewport_size`.
     - **Absolutely position** each visible cell (`div().absolute().left(px).top(px)
       .w(px).h(px)`) using `Axis::offset_of` — white bg, grey `border_1`, fill on
       highlight, `font_weight`/`italic` from `RenderCell`, header row/col.
     - Handle `on_scroll_wheel` to update the offset (interactive mode).
     - When a harness is active, on each frame: apply `next_viewport`, time the render
       + the newly-visible provider pulls, `record`, request the next frame
       (`cx.on_next_frame` / `request_animation_frame` — exact call per research), and
       on script end call `poc-core::report::finalize`, print, write JSON, and quit.
   - `README.md`: exact macOS commands.

3. **`gpui-component/` shell (its virtualized Table).**
   `Cargo.toml`: `gpui` + `gpui-component` git pins **copied verbatim** from
   gpui-component's own example manifest (same rev), plus `poc-core`/`datagen`/
   `bench_util`. Call `gpui_component::init(cx)` per its examples.
   - `src/table.rs`: a `TableDelegate` (exact trait/methods per research) whose
     `cols_count`/`rows_count` come from `PocConfig`, `col_width` from the provider's
     `col_width`, and `render_td`/`render_th` build cells from `RenderCell` (same
     `style.rs` path as raw-gpui, so both look identical). If `gpui-component`'s table
     can't do variable **column** widths or true 2D virtualization at this scale,
     record that as a **finding** (implement the best it supports and note the gap).
   - `src/main.rs`: bootstrap + the same `RunTest` action / `--run-test` flag driving
     the harness by programmatically setting the table's scroll/selected row per frame
     (using whatever scroll API the Table exposes; fall back to a raw scroll handle if
     needed), then `finalize`/print/write/quit — identical reporting path.
   - `README.md`: macOS commands.

4. **`scripts/` (macOS one-command).**
   - `build_and_run.sh <variant>`: `cd` into the chosen variant and
     `cargo run --release` (interactive). Defaults to `raw-gpui`.
   - `run_test.sh <variant>`: `cargo run --release -- --run-test`, which runs the
     harness, writes `results/<variant>-runtest.json`, prints PASS/FAIL, exits.
   - `README.md`: pull → run → report loop the human follows (build both variants,
     feel each, run the test on each, paste the printed PASS/FAIL + JSON path).

5. **`findings.md`** (§5.2 headings): fill Questions / What was done / Results-evidence
   (marked *pending the Mac run*) / Conclusion (*pending*) / Recommended design +
   next-best / Risks. Add a prominent **"HUMAN RUN REQUIRED"** section: the exact
   `scripts/` commands, what to look for (does it build? does it scroll smoothly? the
   Run Test PASS/FAIL lines and the `results/*.json`), and where to paste numbers so
   the raw-vs-gpui-component comparison + verdict can be completed.

6. **Verify what CAN be verified in-container:** `cargo check` + `cargo test` **only**
   in `poc-core/` (no gpui). Do NOT build the gpui shells. Update the experiments
   `README.md`? No — that's root-level/shared; leave it (path-scoped to `04-ui-poc/`).

## Tests (in `poc-core/`, run in-container)

- `axis_offset_and_index_roundtrip`: `index_at(offset_of(i)) == i` across variable
  sizes; boundaries (0, last).
- `axis_visible_range_covers_viewport`: the returned range fully covers `[scroll,
  scroll+extent]` plus overscan on both sides, and is clamped to `[0, count)`.
- `axis_total_matches_sum`: `total()` equals the naive sum of sizes for a small axis.
- `axis_handles_excel_max_without_oom`: constructing an `Axis` for 1_048_576 rows and
  querying near the end returns quickly and doesn't allocate O(n) (segment structure).
- `style_number_and_text_and_empty_formatting`: `RenderCell` text renders numbers,
  text, and empty distinctly; bold/italic/align/argb carry through from `CellData`.
- `harness_script_is_deterministic_and_terminates`: `scripted` yields a fixed,
  reproducible sequence of viewports for a seed and ends (`next_viewport -> None`);
  covers each move type (down/fast/horizontal/jump/random).
- `harness_newly_visible_counts`: `newly_visible` of two overlapping ranges equals the
  set difference size; disjoint ranges count the full new range.
- `report_gates_pass_and_fail`: given synthetic sub-target samples → all gates PASS;
  given over-target samples → the right gate FAILs; JSON writes and round-trips via
  `bench_util`.
- `report_writes_results_json`: `finalize` writes a well-formed `BenchResult` JSON to a
  temp dir with the frame + cell-load stats and gates embedded.

## Non-goals / notes

- No `cargo build` of gpui shells in-container (macOS/Metal only) — expected and honest.
- `shared/` is read-only; consumed by relative path only. No edits outside
  `04-ui-poc/`.
- Manager commits; this phase does not commit and never `git add -A`.
