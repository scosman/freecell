---
status: complete
---

# Phase 10: Render validation + perf (late phase)

## Overview

P1–P9 landed the full first-pass CF feature (engine seam, worker `Command::AddCondFmt` + published
rule list, value-dependent render cache, sidebar authoring/management, persistence). CF changes
**grid/cell pixels** (fills / font colour, value-dependent), so it is **in-scope** for the pixel
render suite. This phase adds the missing render coverage + a targeted perf check — the dedicated
late phase per CLAUDE.md "Render tests" §3. No product code changes; only the render-test harness
(a new `Scene` CF builder), new `cf_*` render cases + baselines, and a headless CF perf bin.

The render suite drives the **real** stack: `scene.rs` spawns a `DocumentClient`, applies inputs +
styles as **real worker commands**, waits for the worker publish, and reads back the real
`Publication` + `SheetCaches`. Adding CF coverage = teaching the `Scene` builder to send a real
`Command::AddCondFmt` so the captured cache carries the P3 CF-folded fills, then declaring cases.

The manager runs the **full** pixel suite + dispatches the CI `render` gate afterward (not here);
this phase iterates with the `cf_` subset only.

## Steps

1. **`app/render-tests/src/scene.rs` — a CF `Scene` builder.**
   - Import `CfRuleSpec` from `freecell_core`.
   - Add a `cond_fmt: Vec<(String, CfRuleSpec)>` field to `Scene`, initialized empty in `new()`.
   - Add the builder method:
     ```rust
     /// Adds a conditional-formatting rule over the A1 `range` — a real `Command::AddCondFmt`
     /// worker edit. The worker folds the winning rule's differential into the published style
     /// cache via the extended-style path (P3), and `build_sources` drains to idle after sending
     /// it, so the captured `SheetCache` carries the value-dependent CF fills / font colour the
     /// grid then paints.
     pub fn cond_fmt(mut self, range: &str, spec: CfRuleSpec) -> Self { … }
     ```
   - In `build_sources`, after the `styles` loop and before the hidden-row/col + viewport sends,
     emit one `Command::AddCondFmt { sheet, range, spec }` per rule. Placing it after the value
     inputs means the first CF fold already sees the values; the value publish would re-fold anyway,
     and the final `drain_to_idle` + the viewport-time `build_and_store_cache(cf = has_cond_fmt)`
     guarantee the settled resident cache reflects CF. No change to `drain_to_idle` is needed — it
     already waits for the worker to go idle, which covers the CF-folded `StyleCacheUpdated`.

2. **`app/render-tests/src/cases.rs` — new `cf_*` cases + local spec helpers.**
   - Import `CfColorStop, CfFormat, CfRuleSpec, CfTextOp, CfThresholdKind, CfValueOp` from
     `freecell_core`.
   - Add a small local helper `cf_fill_text(fill, text) -> CfFormat` (fill + text colour, the two the
     first-pass highlight cases show), mirroring the existing `edge()` / `all_edges()` shorthands.
   - Add three cases (snake_case, `cf_` prefix; `GRID_VP` so the whole column + headers show):
     - **`cf_highlight_greater_than`** — a labelled column of numbers straddling a threshold + a
       `CellIs > 100` rule with a light-red fill + dark-red text. Baseline: the > 100 cells filled,
       the rest plain (proves value-dependent highlight renders).
     - **`cf_color_scale_3`** — a labelled column of ascending numbers + a 3-stop
       green→yellow→red color scale (Min / Percentile-50 / Max). Baseline: an interpolated
       top-to-bottom gradient across the cells (proves the scale interpolates into the cache).
     - **`cf_highlight_text_contains`** — a labelled column of status text + a `Text Contains
       "Fail"` rule with the same fill. Baseline: the "Fail" cells filled, the rest plain (a second
       highlight family, cheap).

3. **`app/render-tests/tests/render_suite.rs` — register the three case names** in the
   `render_cases!` macro list (a new "Conditional formatting (P10)" line), so the auto-generated
   `#[test]` per case exists and `case_names_match_table` stays green.

4. **`app/render-tests/src/bin/cf_perf.rs` — headless CF edit-path perf bin** (CLAUDE.md Benchmarks
   conventions). Measures the **edit → cache-rebuild → repaint-ready** worker round-trip latency on a
   populated 256×8 range, CF-on vs CF-off, through the **real** `DocumentClient`:
   - Build two fixtures (same populated values): one with a `CellIs > 100` rule over the whole range
     ("A1:H256"), one with no rule.
   - Per iteration: flip one in-range cell's value across the threshold (a real change every time →
     never a measured no-op), send `SetCellInput`, and time from send until the worker goes quiet
     (drain events, measuring send → **last** event, excluding the trailing idle gap). Warm up, then
     record p50/p99/max.
   - **FORCE + ASSERT**: the CF fixture's resident cache carries a CF fill on a matching cell (the CF
     fold provably happened); the non-CF fixture's does not; the edit emitted a publish. Report
     p50/p99 for CF-on vs CF-off, environment-stamped, and write `results/cf-perf.json`.
   - Run FOREGROUND under `timeout`; never backgrounded. Headless (worker thread, no GPU) like
     `chart_perf`.

## Tests

- **Render cases** (each is a `#[test]` via `render_cases!`): `cf_highlight_greater_than`,
  `cf_color_scale_3`, `cf_highlight_text_contains` — rendered through the real grid + engine and
  diffed against committed baselines. Iterated with `render_tests.sh test cf_`.
- **`case_names_match_table`** — guards the macro list vs `cases::all()` (kept in sync).
- **Baselines**: generate, then verify `git status baselines/` shows ONLY the three new `cf_*.png`
  added and NO existing baseline changed (CF is `has_cond_fmt`-gated → non-CF baselines must be
  byte-identical). Eyeball each new PNG (filled-vs-plain for the highlights; a gradient for the
  scale).
- **Perf**: `cf_perf` prints p50/p99 for CF-on vs CF-off with the force+assert guards; a non-CF
  sheet's edit path stays on the cheap touched-cell mirror (the P3 gate), CF-on adds the bounded
  full-range CF rebuild. (Scroll is unaffected by construction — CF lives in the resident cache the
  scroll path already reads wait-free; the existing `perf_harness` styled-frame + zero-engine-calls
  gate covers it.)

## Notes / non-goals

- No product code changes — the grid already paints `RenderStyle` fill / font colour; CF only
  populates them in the cache (P3). So this phase cannot move a non-CF baseline.
- The full pixel suite run + CI `render` dispatch is the **manager's** step after this phase (per the
  prompt), not done here.

## What was done

- **Render cases + scene builder** (steps 1-3) were already scaffolded; verified they compile and
  render over the real GridView + engine stack. The `Scene::cond_fmt` builder sends a real
  `Command::AddCondFmt`; the P3 value-dependent cache folds the winning rule's differential into the
  published `RenderStyle`, so the captured `SheetCache` carries the CF fills the grid paints.
- **3 CF baselines generated + eyeballed + committed** (`414840a`), via `render_tests.sh generate
  --only cf_` so no existing baseline was touched:
  - `cf_highlight_greater_than` — `CellIs > 100` fills 150/200/175/120 light-red / dark-red; the rest
    plain.
  - `cf_color_scale_3` — a smooth green→yellow→red interpolation across 10→100 (midpoint 50 yellow).
  - `cf_highlight_text_contains` — `Text Contains "Fail"` fills the two "Fail" cells; Pass/Pending plain.
  These pixel captures confirm CF renders correctly end-to-end — and validate the BUG-1 fix (CF now
  applies on rule add, no value nudge needed) at the pixel level.
- **`cf_perf` bin** (step 4) written, committed (`c0ab04a`): headless CF edit-path benchmark through
  the real `DocumentClient`, CF-on vs CF-off, force+asserted, `results/cf-perf.json` committed.

## Verification

- **Subset** `render_tests.sh test cf_` → 3/3 pass (against the fresh baselines).
- **Full pixel suite via the CI `render` gate** (the required truth): dispatched on the branch (commit
  `414840a`) and **green** — `render (Xvfb + lavapipe)` completed **success**, the full suite step
  passing in ~25 min. (A full *local* run was not done: software lavapipe on this container renders at
  ~185 s/case, so all ~150 cases locally would take hours; the CI gate is the authoritative full-suite
  validation per CLAUDE.md, and the 3 new cases were validated + eyeballed locally.)
- **Perf** (release, 4-core Xeon @2.8 GHz, `414840a`, n=250): edit CF-on p50 **7.86 ms** / p99
  **11.07 ms**; CF-off p50 **3.38 ms** / p99 **5.88 ms**; delta +4.5 ms — the bounded 2048-cell CF
  re-fold the touched-cell mirror skips (quantifies GAPS-CF8). Force+asserts all fired (matching cell
  filled in CF-on / not in CF-off; the flipped cell's fill tracked its value each iteration).
- **`cargo fmt --all --check`** clean; **`cargo build --workspace`** green.

**First pass complete → presented for review.**
