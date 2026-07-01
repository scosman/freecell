---
status: draft
---

# Phase SP2: End-to-end large styled `.xlsx` open (time + peak memory)

## Overview

SP2 closes functional_spec §5.4's one un-run perf target: open a real **≥100 MB
styled `.xlsx`** from a fresh process and record **wall-clock open time**, **peak RSS
from a separately-spawned child process**, a **stage breakdown** (as far as IronCalc's
opaque API allows), and **time-to-first-paint** (when cached values become queryable,
separately from full-recompute-ready). The judgment GATE is: open in **seconds, not
minutes**, with peak RSS a **sane multiple of file size**; the off-ramp trigger is
minutes-scale open or RSS ballooning ≫10× uncompressed (functional_spec SP2).

Everything lives in **`experiments/round-2/02-xlsx-open/`**, an independent Cargo
project depending read-only by relative path on the frozen `../harness` (IronCalc 0.7.1
adapter + canonical `peak_rss()`), `../../shared/datagen` (cell/format content, frozen),
and `../../shared/bench_util` (env-stamped results). `shared/datagen` is **frozen** — it
has no `.xlsx` writer, so the writer lives in **my** folder and produces the file from
committed code via **IronCalc's own native styled writer** (`export::save_to_xlsx`).

### Key IronCalc API facts (verified against the pinned 0.7.1 source)

- **Write:** `ironcalc::export::save_to_xlsx(&Model, path)` — refuses to overwrite an
  existing file (so the generator removes a stale target first). Writes zip with
  sharedStrings.xml, styles.xml, per-sheet worksheets, and for formula cells emits
  `<f>…</f><v>{cached}</v>` — the cached value is persisted **iff** the model was
  `evaluate()`d before save. My generator evaluates before saving so the file carries
  real cached results (the premise behind time-to-first-paint).
- **Read:** `ironcalc::import::load_from_xlsx(path, locale, tz, lang) -> Model`.
  `from_workbook` runs `parse_formulas()` (builds the formula AST) but **does NOT call
  `evaluate()`** — so a freshly loaded model is queryable with cached `<v>` values
  *before* any recompute. This is the mechanism that makes time-to-first-paint ≈ load
  time, distinct from full-recompute-ready (load + `evaluate()`).
- **Values from cache:** `Model::get_cell_value_by_index` reads the stored cell value
  (cached `v` for formula cells) — no eval on read. Confirms TTFP is queryable at load.
- **Styling:** `Model::set_cell_style(sheet,row,col,&Style)` (dedups via
  `get_style_index_or_create`); `Style { font{b,i,sz,color,name}, fill{fg_color},
  alignment{horizontal}, num_fmt, border }`. `set_column_width`/`set_row_height` for
  band widths. `add_sheet(name)` for multiple sheets. Colors are `#RRGGBB` strings.
- **Stage opacity (architecture §8 risk):** IronCalc exposes exactly two coarse public
  seams — `load_from_xlsx_bytes(bytes,…)` (parse+build a `Workbook` from in-memory
  bytes) and `Model::from_workbook` (build the evaluatable model). It does **not** expose
  sub-stage hooks (unzip / XML parse / shared-strings / style ingest / graph build are
  fused inside `load_from_excel`). SP2 records the coarsest honest split — **file read →
  parse+build (Workbook) → model build (from_workbook) → first eval** — and says plainly
  where finer breakdown is impossible.

## Steps

1. **`Cargo.toml`** — independent project `xlsx_open` (not a workspace member).
   Dependencies: `round2_harness = { path = "../harness" }` (canonical `peak_rss`, the
   IronCalc pin), `ironcalc = "0.7"` + `ironcalc_base = "0.7"` (writer/reader/styles),
   `datagen = { path = "../../shared/datagen" }` (cell/format content — read-only),
   `bench_util = { path = "../../shared/bench_util" }` (env stamp + JSON results),
   `serde`/`serde_json`, `anyhow`. Three bins: `gen`, `open`, `measure`. `.gitignore`
   = `/target` and the generated `*.xlsx` (large binary, reproduced from code).

2. **`src/lib.rs` — the styled-`.xlsx` generator (`generate_workbook` + `write_xlsx`).**
   Builds a multi-sheet IronCalc `Model` from committed, deterministic content:
   - Pull cell content + format from `datagen::SyntheticSheet` (values: number/text/empty
     mix; formats: bold/italic/highlight/align) so content is realistic and reproducible.
   - **Realistic mix per spec:** across **N sheets** (default 4), write a `rows × cols`
     block of literal values (from `SyntheticSheet`), a **formula column** per sheet
     (`=SUM(...)` / `=A{r}+B{r}` cascade referencing literals so eval does real work and
     the graph is non-trivial), **shared strings** (text values reuse a bounded word pool
     → dense sharedStrings.xml), and **per-cell styles** (bold/italic/fills/align/number
     formats) deduped by IronCalc's style table. Include a few **band styles**
     (`set_column_width`, one styled row/col) for realism.
   - Map `datagen` format → IronCalc `Style`: `format.bold→font.b`, `italic→font.i`,
     `highlight→fill.fg_color = #RRGGBB`, `h_align→alignment.horizontal`, plus rotate a
     small set of `num_fmt` codes so styles.xml is non-trivial.
   - **`evaluate()` once** after all writes (so cached `<v>` are correct and persisted),
     then `save_to_xlsx`. Size is driven by a `target_bytes` knob: grow rows until the
     written file crosses ≥100 MB (measured on disk), capping if memory pressure risks
     the ~15 GB box — **record the ceiling as a finding** rather than OOM.

3. **`src/bin/gen.rs`** — CLI: `gen [target_mb] [out_path]`. Generates the file
   foreground, prints written size + generation wall-clock (kept **separate** from open
   time per benchmark discipline), and asserts the file is ≥ the requested size (or logs
   the capped ceiling). Reproducible from one command.

4. **`src/lib.rs` — the open/measurement path (`open_stages`).** A function that opens a
   given `.xlsx` and returns a struct of stage durations + a post-open probe:
   - `t_read`: read file bytes to memory (`std::fs::read`).
   - `t_parse_build`: `load_from_xlsx_bytes(&bytes, name, locale, tz)` → `Workbook`
     (unzip + XML parse + shared-strings + style ingest, **fused** — the honest coarse
     seam).
   - `t_model_build`: `Model::from_workbook(workbook, lang)` (parse formulas / graph
     build, no eval).
   - `t_first_paint`: from process start to first **cached** value queryable — i.e. after
     `t_read + t_parse_build + t_model_build`, assert a known formula cell reads its
     cached value **without** `evaluate()`. This is time-to-first-paint.
   - `t_first_eval`: `model.evaluate()` (full-recompute-ready). Assert the recomputed
     value matches the cached one (force + assert the measured op).
   - **Force + assert:** read a black-box-held sentinel cell (a formula whose expected
     value is known from generation) at both the cached stage and post-eval stage;
     `std::hint::black_box` the reads so nothing is optimized away.

5. **`src/bin/open.rs`** — **the fresh child process** that does the real measurement.
   Opens the file via `open_stages`, stamps **peak RSS from `round2_harness::peak_rss()`**
   (the canonical VmHWM helper — NOT `sysinfo::peak_rss_bytes`), and prints a single JSON
   line with `{file_bytes, uncompressed_bytes?, t_read, t_parse_build, t_model_build,
   t_first_paint, t_first_eval, peak_rss}`. This binary does nothing else, so its VmHWM is
   the honest open-only high-water mark (architecture §3: peak RSS must come from a fresh
   child, not the polluted harness process).

6. **`src/bin/measure.rs`** — the orchestrator (parent). Given the generated file, it
   **spawns `open` as a separate child process** K times (default 3, foreground, via
   `std::process::Command`), parses each child's JSON line, aggregates
   (min/median/max per stage; peak RSS = max across runs), computes the **RSS multiple**
   (peak_rss / file_bytes and / uncompressed_bytes), applies the judgment GATE
   ("seconds not minutes" + "sane RSS multiple"), and writes:
   - `results/open_stage_timings.json` (per-run + aggregated `BenchResult`, env-stamped
     via `bench_util::Environment::detect`),
   - `results/open_summary.json` (gate verdict, TTFP, dominant stage, RSS multiple),
   - `results/env.txt` (CPU/OS/cores/commit + file size + IronCalc version).
   Fresh child per run guarantees cold peak RSS.

7. **`findings.md`** — functional_spec §5.2 headings (Questions / What was done /
   Results / Interpretation / Threats to validity / Verdict). Reports open wall-clock,
   peak RSS + the multiple + dominant stage, the stage breakdown with the honest note on
   opacity, time-to-first-paint vs full-recompute-ready, and the GATE / off-ramp verdict.
   Written via Bash heredoc if the Write hook blocks report-named files.

8. **Reproduce-from-one-command** README note + a `scripts/` is not needed (bins are the
   commands). Commit `results/` but **not** the multi-hundred-MB `.xlsx` (gitignored;
   reproduced by `cargo run --release --bin gen`).

## Tests

- **`generate_roundtrips_small`** — generate a tiny in-memory model, save to a temp
  `.xlsx`, reload, assert a known literal and a known **formula's cached value** read
  back correctly (proves the writer emits cached `<v>` and the reader queries it without
  eval).
- **`styles_survive_generation`** — a generated cell with bold+highlight+align, after
  save+reload, reports those style attributes via `get_style_for_cell` (proves the
  style-mapping path actually writes styles, so the file is genuinely *styled*).
- **`open_stages_orders_first_paint_before_first_eval`** — on a small generated file,
  `open_stages` returns `t_first_paint` reachable **before** `t_first_eval`, and the
  cached sentinel value equals the post-eval value (proves TTFP is a real, earlier
  milestone and the cached result is correct).
- **`first_paint_needs_no_eval`** — load a file, read the sentinel formula cell's cached
  value **without** calling `evaluate()`, assert it is the expected number (directly
  proves "cached values queryable before recompute").
- **`peak_rss_helper_is_canonical_nonzero`** — assert `round2_harness::peak_rss()` (not
  `sysinfo::peak_rss_bytes`) returns a plausible non-zero figure in this process.
- **`measure_parses_child_json`** — a unit test that the child-JSON parse/aggregate in
  `measure` round-trips a synthetic child output line (so the parent↔child contract is
  covered without spawning).
- **Bin-level sanity (not a unit test, run in CI-of-record):** `gen` at a small size then
  `measure` produces a `results/open_summary.json` with a gate verdict — exercised
  manually foreground with `timeout`, results committed.
