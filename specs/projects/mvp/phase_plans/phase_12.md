---
status: complete
---

# Phase 12: Perf harness + CI gates

## Overview

Wire the POC "Run Test" scenario (`experiments/04-ui-poc/poc-core`) against the **real**
`GridView` reading from **real** engine-produced `Publication` + `SheetCaches`, over a
**1M×100 styled fixture**, and measure the `architecture.md §4` budgets:

- frame p99 ≤ 8.33 ms, worst frame ≤ 16.67 ms,
- newly-visible cell-load p99 < 2 ms,
- **ZERO engine calls on the scroll/render path** (asserted via an instrumented engine
  counter + a negative control).

Then commit **buffered Linux CI gates** (= 2× the p99 calibrated on this runner image,
per the product call in `architecture.md §9`) and wire them into `perf-gates.yml`
(Phase 1 left it a placeholder).

### Measurement reality (lavapipe) — what is / isn't representative

This container renders through **Mesa lavapipe (software Vulkan)** — GPU *present* times
are NOT representative of real hardware, so we do **not** gate on end-to-end painted
frame time. We measure the **CPU render-build path** the POC measured (data resolution +
element construction) plus the **engine-call counter** (fully representative). This
matches the POC's methodology exactly (its `frame_render_ns` timed element building, not
GPU present). Text shaping + rasterization happen inside gpui after `render()` returns and
under lavapipe are unrepresentative; they are a `macos-verify` (real-hardware) concern —
recorded in DECISIONS_TO_REVIEW.md, not gated here.

## Steps

1. **Engine-call counter** (`freecell-engine/src/instrument.rs`, new): a process-global
   `AtomicU64`; `pub fn engine_call_count() -> u64`, `pub fn reset_engine_call_count()`,
   `pub(crate) fn record_engine_call()`. Increment `record_engine_call()` at the entry of
   every model-touching `WorkbookDocument` method (`formatted_value`, `cell_content`,
   `evaluate`, `set_cell_input`, `clear_contents`, `font_flag`, `set_font_flag`,
   `set_fill`, style/geometry reads, sheet ops, undo/redo). Re-export from `lib.rs`. This
   is THE IronCalc boundary; the grid never holds a `WorkbookDocument`, so the render path
   can never bump it — that is exactly what the gate proves. Unit test = the negative
   control (a real read/edit increments the counter).

2. **Perf harness core** (`freecell-core/src/perf.rs`, new — engine-free, gpui-free,
   unit-tested; ports `poc-core`): `PerfConfig`, `Viewport`, `Move`, `Harness::scripted`
   (the 5-phase scroll/jump script), `FrameSample { frame_render_ns, cell_load_ns,
   newly_visible, elements }`, `LatencyStats` (p50/p99/max), `Gate`, `RunReport`, and the
   true-budget thresholds (`FRAME_TARGET_NS`, `FRAME_WORST_NS`, `CELL_LOAD_TARGET_NS`).
   No serde (core stays dependency-free); JSON is written by the binary. Ports poc-core's
   determinism / stats / gate tests.

3. **Real-grid measurement hook** (`freecell-app/src/grid/view.rs`): factor the
   frame-dependent element build out of `render()` into `build_grid_layers(&mut self,
   frame, timing: Option<&mut FrameTiming>) -> Vec<AnyElement>` (shared by `render()` and
   the perf path — no logic drift), and add `pub fn measure_frame(&mut self, scroll_x,
   scroll_y, viewport_w, viewport_h, prev) -> (FrameSample, (Range,Range))`: sets the
   active sheet's scroll (clamped via the real `layout::clamp_scroll`), runs the real
   `resolve_frame` + `build_grid_layers`, times the data-load segment (style snapshot +
   publication cell-index scan) and the whole build, `black_box`es + asserts the built
   layer set is non-empty (FORCE + ASSERT: can't measure a no-op).

4. **Fixture + driver** (`render-tests/src/perf.rs` + `src/bin/perf_harness.rs`, new): a
   1M×100 STYLED fixture built through the **real** `DocumentClient` (like `scene.rs`):
   variable column widths (POC-style, incl. wide cols), dense **col-band styles** across
   all 100 cols (so every visible cell at any scroll depth is styled — near-worst-case
   element build the whole sweep), a spread of row-height overrides (variable geometry at
   1M scale), and a densely valued+styled top band (rows 0..256 × cols 0..100) published
   so the heaviest frames carry real text + a full ~25k-cell publication scan (the exact
   O(published) per-frame scan `view.rs` flags for Phase 12). The driver opens the real
   grid under gpui, drives `Harness` through `measure_frame`, then:
   - snapshots `engine_call_count()` before/after the whole sweep → asserts **zero** delta
     (the scroll-path gate),
   - **negative control**: sends one real edit to the still-alive worker, drains, asserts
     the counter DID climb (the gate isn't vacuous),
   - builds a `RunReport`, environment-stamps it, writes `results/perf-runtest.json`,
     prints p50/p99, and in `--gate` mode exits non-zero if any committed CI threshold or
     the zero-engine-calls gate is breached.

5. **Calibrate + commit thresholds**: run the harness (release, foreground, `timeout`),
   read p50/p99, adversarially review, set `CI_FRAME_P99_NS/CI_FRAME_MAX_NS/
   CI_CELL_LOAD_P99_NS = 2× calibrated p99/max` as documented constants in `perf.rs`,
   commit `results/perf-runtest.json`, and record the numbers + environment + lavapipe
   caveats in `DECISIONS_TO_REVIEW.md`.

6. **CI** (`.github/workflows/perf-gates.yml`): replace the placeholder with a build +
   `render-tests/scripts/perf.sh --gate` step (foreground, `xvfb-run`, lavapipe),
   required-green at the committed buffered thresholds.

## Tests

- `freecell-core::perf`: `script_is_deterministic_and_covers_all_moves`,
  `harness_advances_and_terminates`, `viewports_stay_non_negative`,
  `newly_visible_2d_set_difference`, `seeded_random_jumps_are_reproducible`, stats math
  (`p50/p99/max`), gate pass/fail (`all_gates_pass_under_target`,
  `frame_gate_fails_over_120fps_budget_but_within_60fps`, `cell_load_gate_fails_over_2ms`).
- `freecell-engine`: `engine_call_counter_registers_real_model_work` — a real read/edit
  increments `engine_call_count()` (proves the zero-calls gate can register failure).
- `freecell-app`: `measure_frame_builds_nonempty_layers_and_times_them` /
  `measure_frame_scroll_moves_viewport` (gpui test context) — the hook measures real work.
- `render-tests`: the perf binary itself is the gate; a `perf.rs` unit test asserts the
  fixture builder yields a densely-styled 1M-dim cache + a non-empty publication.
- The harness asserts (in-run): the scroll swept ≥ N distinct visible ranges, layers
  non-empty, zero engine-call delta across the sweep, and a nonzero delta under the
  negative control.
