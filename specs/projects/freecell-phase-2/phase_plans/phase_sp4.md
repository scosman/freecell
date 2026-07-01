---
status: complete
---

# Phase SP4: Styled viewport read at scale + style-API coverage

## Overview

SP4 closes two unknowns for the "IronCalc-native styles as source of truth" decision
(overview §2), both against the pinned **IronCalc 0.7.1** the frozen `round-2/harness`
uses:

1. **Styled viewport-read GATE.** Phase-1 measured the newly-visible viewport read at
   **392 µs p99 (value-only, 1,800 cells, D2)**. SP4 must confirm that reading
   **value AND style (`get_style_for_cell`) per visible cell** over a viewport+overscan
   window (~10³–10⁴ cells) at **Excel-max positions** still holds **p99 < 2 ms**. Styles
   are now read straight from IronCalc, so this must be *measured*, not assumed. We report
   p50/p99 (env-stamped, foreground, force+assert the reads are real) and compare against
   the value-only baseline to expose the added style cost.

2. **Style-API coverage probe.** With **assertions against IronCalc's real public API**
   (verify, don't assume), determine whether it exposes (a) **per-cell** styles (known
   yes), (b) **row/column band** styles, and (c) **empty-cell** styling. If band or
   empty-cell styling is missing from the public API, that **reopens the overview §2
   formatting decision** (may force a scoped side-store) — and we say so plainly.

**API facts already verified from the pinned 0.7.1 source** (informs the plan; the code
re-asserts each at runtime so nothing is assumed):
- `Model::get_style_for_cell(sheet, row, col) -> Result<Style, String>` resolves the
  **effective** style via `get_cell_style_index`, whose documented precedence is
  **cell → row band → column band → default** (`model.rs:1958-1994`).
- Band setters/getters exist on `Model`: `set_row_style`, `set_column_style`,
  `get_row_style`, `get_column_style` (`model.rs:2307-2361`). NOTE the row-band HACK
  (`worksheet.rs:125-144`): `set_row_style` only marks a row `custom_format` (hence
  resolvable through `get_style_for_cell`) when the style index is non-default — so the
  empty-cell/band probe must use a **non-default** style, else the fallthrough won't fire.
- The `Style` struct (`types.rs:323`) carries `font` (b/i/u/strike/sz/color/name),
  `fill` (pattern_type/fg_color/bg_color), `border` (per-side `BorderItem`), `alignment`
  (horizontal/vertical/wrap_text), and `num_fmt` — the attributes FreeCell needs per cell.

The frozen harness `IronCalcEngine::read_viewport` is **value-only** (`get_value` per
cell) and its trait has no style read, so SP4 adds its own **value+style** read path in
this experiment crate, calling `engine.model().get_style_for_cell(...)` (the adapter
exposes `.model() -> &Model`). This keeps `../harness` untouched (read-only) while reusing
its `IronCalcEngine`, `Viewport`, `Profile`, `seed_region`, `pan_path`, and `SEED`.

## Steps

1. **Scaffold the independent Cargo project** `experiments/round-2/04-styled-read/`
   (NOT a workspace member; mirror SP2/SP3):
   - `Cargo.toml` — `name = "styled_read"`, `publish = false`, deps: `round2_harness`
     (path `../harness`, read-only), `bench_util` (path `../../shared/bench_util`),
     `ironcalc_base = "0.7"` (for `Style`/`Model` types in the probe), `serde`,
     `serde_json`, `anyhow`. Two bins: `bench`, `probe`. `[lib]` for the shared logic.
   - `.gitignore` → `/target`.

2. **`src/lib.rs` — the value+style read core** (unit-testable, no I/O):
   - `pub struct StyledCell { pub value: EngineValue, pub bold: bool, pub fill_argb:
     Option<String>, pub num_fmt: String }` — a compact projection of what the UI reads
     per cell (value + a few load-bearing style fields, proving the style was really
     fetched, not the whole `Style` clone which would flatter the number).
   - `pub fn read_styled_viewport(engine: &IronCalcEngine, vp: Viewport) -> Vec<StyledCell>`
     — per visible cell: `engine.get_value(r,c)` **and**
     `engine.model().get_style_for_cell(sheet=0, r+1, c+1)`; map into `StyledCell`.
     This is the measured op: value + style, per cell, no native bulk style read exists
     (documented finding — same shape as the value-only per-cell loop).
   - `pub fn assert_styled_read_real(cells: &[StyledCell]) -> (usize, usize)` — a
     credibility guard returning `(non_empty_values, styled_cells)`; the caller asserts
     both are `> 0` so we can never be "fast" by reading an empty/unstyled grid.
   - Reuse harness `Profile::full()` viewport (60×30 ≈ 1,800 cells) for the baseline-
     comparable window, plus an **overscan** variant (~10⁴ cells) so the GATE covers the
     10³–10⁴ band the spec names.

3. **`src/bin/bench.rs` — the styled-read benchmark** (foreground; separate build from
   measured op; force+assert):
   - Seed a region with harness `seed_region` (native bulk-ingest, cheap), positioned so
     the scroll/jump path visits **Excel-max positions**. Because IronCalc addressing is
     sparse `HashMap`, seed a band *at* Excel-max (near row 1,048,576 / col 16,384) and
     apply a **mix of per-cell, row-band, and column-band styles** across it, so the
     measured read exercises the real style-resolution fallthrough at the maximal
     coordinate — not a cheap origin read.
   - Two window sizes: **`viewport` (~1,800 cells, baseline-comparable)** and
     **`overscan` (~10⁴ cells)**. For each, run the harness scroll/jump `pan_path` at
     Excel-max, timing `read_styled_viewport` per pan step with `bench_util::time_once`.
   - Compute `LatencyStats` (p50/p99/max) per window; gate p99 against
     `targets::VIEWPORT_READ_NS` (2 ms) via `GateResult::max`.
   - **Value-only control** in the same process/positions (call harness
     `read_viewport`) so the results carry the added style cost as a delta, and the
     Phase-1 392 µs baseline is cited in findings.
   - Force+assert: black-box the result vectors; assert `non_empty_values > 0` **and**
     `styled_cells > 0` before recording (refuse to record a bogus number).
   - Write env-stamped `results/styled_read.json` (+ a human `results/env.txt` and
     `results/summary.md`) via `BenchResult` + `Environment::detect`. p50/p99, window
     size, Excel-max position, value-only vs value+style, GATE verdict all recorded.

4. **`src/bin/probe.rs` — style-API coverage probe** (assertions, not assumptions):
   Each capability is a runtime assertion against IronCalc's public API; the bin prints
   PASS/absent per capability and writes `results/style_api_coverage.json` +
   `results/style_api_coverage.md`. Probes:
   - **(a) Per-cell styles** — `set_cell_style` a non-default `Style` at a cell; assert
     `get_style_for_cell` reads the same bold/fill/num_fmt back. (Known yes; re-proven.)
   - **(b) Row band** — `set_row_style(sheet, R, non_default)`; assert **an untouched
     cell in row R** resolves that style via `get_style_for_cell` (proves band applies to
     the whole row, incl. cells never individually set). Assert `get_row_style` returns
     it. Repeat for **column band** via `set_column_style` / `get_column_style`.
   - **(c) Empty-cell styling** — assert a **valueless** cell that lies under a row/col
     band (or given a direct `set_cell_style` with no value) resolves the band/cell style
     through `get_style_for_cell` while `get_cell_value_by_index` is `None`/empty. This is
     the "Excel styles whole empty rows/cols" case.
   - **Precedence** — set a column band, then a row band over it, then a per-cell style
     on one cell; assert resolution order **cell > row > column > default** matches the
     documented `get_cell_style_index` order (so FreeCell can rely on it).
   - Each probe records a verdict; the summary states plainly whether band + empty-cell
     styling are **supported** (→ overview §2 decision stands) or **missing** (→ decision
     reopens, scoped side-store needed).

5. **`findings.md`** (functional_spec §5.2 headings: Question(s) / What was done /
   Results-evidence / Conclusion / Recommended design + alternative / Risks) written via
   a Bash heredoc (report-name hook may block Write). Cover: the GATE result (value+style
   p99 vs 2 ms, at both window sizes, at Excel-max), the value-only vs value+style delta
   and the 392 µs baseline, and the style-API coverage verdict with the decision-reopener
   statement. Commit `results/`.

6. **README.md** (brief; heredoc) — one-command reproduce + subtree note, mirroring
   SP2/SP3.

## Tests

- `read_styled_viewport_reads_value_and_style` — seed a couple of styled cells, assert
  the returned `StyledCell`s carry both the value and the set style (bold/fill/num_fmt).
- `assert_styled_read_real_counts` — a vec with some non-empty + styled cells returns
  positive counts; an all-empty/unstyled vec returns zeros (guards the credibility check).
- `per_cell_style_roundtrips` — `set_cell_style` → `get_style_for_cell` reads it back.
- `row_band_applies_to_untouched_cell` — `set_row_style(non_default)` → an untouched cell
  in that row resolves the band style (and `get_row_style` returns it).
- `column_band_applies_to_untouched_cell` — same for `set_column_style` / `get_column_style`.
- `empty_cell_styling_resolves` — a valueless cell under a band resolves the style while
  its value reads empty.
- `style_precedence_cell_over_row_over_column` — the documented cell>row>col>default order
  holds.
- `excel_max_read_is_addressable` — a styled read at (1_048_575, 16_383) 0-based
  (IronCalc's LAST_ROW/LAST_COLUMN after +1) succeeds and returns real data (guards the
  Excel-max positioning against an off-by-one that would read outside the sheet).
