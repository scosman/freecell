---
status: draft
---

# Phase 2: Sub-project C — Datamodel Binding & Engine Perf (two-engine bake-off)

## Overview

This phase answers the **core technical risk** (functional_spec §6.C, architecture
§5): how do we bind a spreadsheet engine to the FreeCell UI so that viewport reads
and change cascades stay inside the frame budget on Excel-max sheets, and does the
engine hit the §5.4 perf targets? Because the Phase-1 gate left the **engine
undecided**, this is a **two-engine bake-off**: the *same* scenarios (built on the
frozen `shared/datagen`) and the *same* metrics (`shared/bench_util`) run against
**both Formualizer 0.7** and **IronCalc 0.7**, so numbers are directly comparable.

The deliverable is a head-to-head `findings.md` (functional_spec §5.2 headings)
covering API suitability for the binding layer, missing/needed features, perf (both
engines, env-stamped, PASS/FAIL vs §5.4), memory, a **recommended binding design**,
and a **perf-lens engine lean** feeding the Phase-7 (Sub-project G) engine decision.

### Verified grounding facts (probed in-container this phase)

- **IronCalc builds headlessly.** Crates are `ironcalc` + `ironcalc_base` (note the
  **underscore** — `ironcalc-base` does not resolve), both **0.7.1** on crates.io;
  a fresh crate depending on them compiled in ~32 s. *(This was the primary roadblock
  risk from the gate; it is cleared.)*
- **IronCalc core API** (`ironcalc_base::Model`): `Model::new_empty(name, locale, tz,
  lang)`, `set_user_input(sheet:u32, row:i32, col:i32, String)`,
  `get_cell_value_by_index(sheet,row,col) -> Result<CellValue>`,
  `get_formatted_cell_value(..)`, `evaluate(&mut self)`, `get_all_cells()`,
  `get_style_for_cell(..) -> Style`, `get_cell_style_index`, `set_column_width` /
  `set_row_height`. Higher crate `ironcalc`: `import::load_from_xlsx_bytes`,
  `export::save_to_xlsx` / `save_xlsx_to_writer`.
- **IronCalc `UserModel`** (`ironcalc_base::UserModel`, from `common`): `new_empty`,
  `set_user_input(sheet,row,col,&str)` (auto-evaluates unless paused), `undo`/`redo`,
  `pause_evaluation` / `resume_evaluation`, `flush_send_queue() -> Vec<u8>` +
  `apply_external_diffs(&[u8])` (a `Diff::SetCellValue { old, new }` diff-list — the
  changelog analog, built for **collaborative sync**, not for driving incremental
  recompute).
- **IronCalc has no incremental recalc.** `Model::evaluate()` **clears every computed
  cell and re-evaluates the whole workbook** (`self.cells.clear(); for cell in
  get_all_cells() { evaluate_cell(..) }`). `UserModel::set_user_input` calls
  `evaluate_if_not_paused()` → a **full-sheet recompute per edit**. This is the
  decisive architectural contrast with Formualizer's dirty-tracking engine and shapes
  the cascade benchmark design (see §Steps / IronCalc caveats).
- **IronCalc has no bulk/range read.** Reads are per-cell (`get_cell_value_by_index`);
  `get_range` is internal to formula eval only. Storage is confirmed
  `HashMap<i32, HashMap<i32, Cell>>` (non-columnar). Styles *are* exposed on read
  (`get_style_for_cell`), which is *better* than Formualizer's read path.
- **Formualizer 0.7.0 API** is captured in `00-stack-decision/findings.md` §A and the
  smoke crate: `Workbook::new`, `set_value` / `set_formula` (1-based), bulk
  `set_values` / `set_formulas` / `write_range`, `evaluate_cell` (incremental,
  recomputes on precedent edit), `read_range(&RangeAddress) -> Vec<Vec<LiteralValue>>`
  (columnar range view), `evaluate_cells(&[..])` batch, `evaluate_all`, parallel eval
  via `WorkbookConfig.eval.enable_parallel`, and an append-only `ChangeLog`
  (`set_changelog_enabled`, `changelog().take_from(idx)`) for dirty tracking.

### Indexing note (comparability)

`datagen` addresses are **0-based** `(row, col)`. Formualizer is **1-based**;
IronCalc rows/cols are **1-based** (`i32`), sheet is `u32`, and `A1` = `(0,1,1)`.
Each adapter owns the `+1` conversion so both engines evaluate the **identical
logical sheet** produced by `datagen`.

## Structure

```
experiments/02-datamodel-binding-perf/
  findings.md                     # head-to-head comparison (this phase fills it)
  common/                         # lib crate: engine-abstraction trait + scenarios
    Cargo.toml
    src/lib.rs                    # re-exports
    src/engine.rs                 # SpreadsheetEngine trait (the binding surface)
    src/scenario.rs               # scenario definitions over shared/datagen
    src/binding.rs                # D1/D2/D3 binding designs (engine-generic)
    src/report.rs                 # results-dir + summary.md writer over bench_util
  formualizer/                    # adapter + bench crate for Formualizer 0.7
    Cargo.toml
    src/lib.rs                    # FormualizerEngine: impl common::SpreadsheetEngine
    benches/perf.rs               # Criterion micro/throughput benches
    src/bin/scenarios.rs          # runs all 5 scenarios, writes results/ + prints PASS/FAIL
  ironcalc/                       # smoke + adapter + bench crate for IronCalc 0.7
    Cargo.toml
    src/lib.rs                    # IronCalcEngine: impl common::SpreadsheetEngine
    tests/smoke.rs                # API-surface capture (mirrors Phase-1 Formualizer smoke)
    benches/perf.rs               # Criterion micro/throughput benches
    src/bin/scenarios.rs          # runs all 5 scenarios, writes results/ + prints PASS/FAIL
  results/                        # committed machine-readable output (JSON) + summary.md
    formualizer/*.json  ironcalc/*.json  summary.md
```

Three independent Cargo projects (`common`, `formualizer`, `ironcalc`) per
architecture §1 (no shared workspace). `formualizer` and `ironcalc` both depend on
`common` and on `shared/datagen` + `shared/bench_util` by relative path
(`../shared/...`, `../common`). `common` depends only on `shared/datagen` +
`shared/bench_util` (engine-neutral — it must never pull in an engine).

## The engine-abstraction trait (`common/src/engine.rs`)

The trait is the "binding surface": every operation FreeCell's binding needs, so both
adapters expose an identical API the scenarios drive. Values use a tiny neutral
`EngineValue` enum (`Number(f64)`, `Text(String)`, `Bool(bool)`, `Empty`, `Error`)
so scenarios never touch engine-specific value types.

```rust
pub enum EngineValue { Empty, Number(f64), Text(String), Bool(bool), Error(String) }

/// One logical cell edit or seed: a literal value or a formula string.
pub enum CellInput { Value(EngineValue), Formula(String) }

pub struct Viewport { pub row0: u32, pub col0: u32, pub rows: u32, pub cols: u32 }

/// The binding surface both engines implement. All coords are 0-based (datagen
/// space); adapters convert to their engine's 1-based indexing internally.
pub trait SpreadsheetEngine {
    fn name(&self) -> &'static str;

    // Build / load
    fn new_blank() -> Self where Self: Sized;

    // Writes
    fn set_value(&mut self, row: u32, col: u32, v: EngineValue);
    fn set_formula(&mut self, row: u32, col: u32, formula: &str);
    /// Bulk write with a single recompute at the end (batched path).
    fn set_batch(&mut self, cells: &[(u32, u32, CellInput)]);

    // Reads / eval
    fn get_value(&self, row: u32, col: u32) -> EngineValue;
    fn evaluate_cell(&mut self, row: u32, col: u32) -> EngineValue;
    /// Bulk viewport read. Adapters use a native range API where available
    /// (Formualizer read_range); otherwise loop get_value (IronCalc).
    fn read_viewport(&self, vp: Viewport) -> Vec<EngineValue>;

    // Recompute
    /// Recompute after edits. Adapters: Formualizer -> incremental (evaluate_cells /
    /// evaluate_all as appropriate); IronCalc -> Model::evaluate() (full recompute).
    fn recompute(&mut self);

    // Edit -> dirty / changelog
    /// Enable change tracking (Formualizer changelog / IronCalc UserModel diff-list).
    fn enable_change_tracking(&mut self);
    /// Addresses touched since the last drain (best-effort per engine); used by D3.
    fn drain_dirty(&mut self) -> Vec<(u32, u32)>;

    // Capability flags (so scenarios/findings can note what's native vs emulated)
    fn caps(&self) -> EngineCaps;
}

pub struct EngineCaps {
    pub native_range_read: bool,      // Formualizer true, IronCalc false
    pub incremental_recalc: bool,     // Formualizer true, IronCalc false
    pub parallel_eval: bool,          // Formualizer true, IronCalc false
    pub change_log: bool,             // both true (different shapes)
    pub styles_on_read: bool,         // Formualizer false, IronCalc true
}
```

### Adapter mapping

| Trait op | Formualizer 0.7 | IronCalc 0.7 |
|---|---|---|
| `new_blank` | `Workbook::new()` | `Model::new_empty("bench","en","UTC","en")` |
| `set_value` | `set_value(sheet,r+1,c+1,LiteralValue)` | `set_user_input(0,r+1,c+1,fmt!(v))` |
| `set_formula` | `set_formula(sheet,r+1,c+1,f)` | `set_user_input(0,r+1,c+1,f)` (`=..`) |
| `set_batch` | `write_range` / `set_formulas` (deferred-dirty) | loop `Model::set_user_input` then one `evaluate()` |
| `get_value` | `get_value(sheet,r+1,c+1)` | `get_cell_value_by_index(0,r+1,c+1)` |
| `evaluate_cell` | `evaluate_cell(sheet,r+1,c+1)` | `evaluate()` then `get_cell_value_by_index` |
| `read_viewport` | `read_range(&RangeAddress::new(..))` | loop `get_cell_value_by_index` |
| `recompute` | `evaluate_all()` / batch after edits | `Model::evaluate()` (full) |
| `enable_change_tracking` | `set_changelog_enabled(true)` | wrap in `UserModel` (diff-list) |
| `drain_dirty` | `changelog().take_from(mark)` → addrs | decode `flush_send_queue()` diffs → addrs |

> **IronCalc caveat baked into the design:** because IronCalc lacks incremental
> recalc, `recompute` == full `evaluate()`. The cascade→visible benchmark therefore
> measures IronCalc's *full-sheet* recompute per edit — that is the honest cost and
> the whole point of the comparison. We record it as-is and gate it against the same
> §5.4 target; a FAIL here is a real finding, not a harness artifact.

## The binding designs (`common/src/binding.rs`)

Engine-generic over `SpreadsheetEngine`, so both engines run the same D1/D2/D3 logic:

- **D1 — Naive per-cell.** On each viewport change, pull every visible cell via
  `get_value` (one call per cell).
- **D2 — Bulk/range.** Pull the visible rectangle in one `read_viewport` call
  (native range read on Formualizer; per-cell loop inside the adapter on IronCalc —
  the comparison surfaces exactly what that costs).
- **D3 — Cached + changelog.** A `BindingCache` holds the visible window; reads hit
  the cache; after an edit, `drain_dirty` marks cells and only the intersection of
  dirty ∩ visible is re-pulled (via `read_viewport`) and refreshed in cache.

`BindingCache`: `HashMap<(u32,u32), EngineValue>` window store with
`prime(vp)`, `read(row,col)`, and `apply_dirty(dirty, vp)`.

## The scenarios (`common/src/scenario.rs`)

Each scenario is a pure builder over `datagen` + a runner that takes `&mut impl
SpreadsheetEngine`, returns `Vec<Duration>` (per-iteration latencies) so
`bench_util::LatencyStats` + `GateResult` gate them. Sizes are **parameterised**
(a small `dev` profile for `cargo test`, a large `full` profile for the recorded
runs) so tests stay fast and the recorded numbers use spec-scale inputs.

1. **`scrolling_viewport_read`** — seed a synthetic sheet region via
   `datagen::SyntheticSheet` (values only; formatting not needed for value reads),
   sweep the viewport across it (a deterministic pan path), time **one viewport pull
   per step** for D1/D2/D3. Gate p99 ≤ **2 ms** (§5.4 "load newly-visible cells").
2. **`cascade_to_visible_update`** — build a `datagen::linear_chain` whose tail spills
   into an offscreen region, place a small visible viewport away from the head, then
   **edit the head cell and re-read the visible viewport**; time the whole
   edit→recompute→read. Gate p99 within one **frame budget (16.6 ms)**. Run for
   D1/D2/D3. (This is where Formualizer incremental vs IronCalc full-recompute
   diverges hardest.)
3. **`cascade_recompute_1m`** — build a **1,000,000-cell** `datagen::linear_chain`
   (`=PREV+1`) via `set_batch`, edit the head, time the full recompute. Gate
   ≤ **100 ms** (§5.4). Plus **`wide_fanout`** shape (`datagen::wide_fanout`, e.g.
   1 source → many dependents) timed and reported (discovery, gated at frame budget).
4. **`writes_single_vs_batched`** — N `set_value`s one-by-one (each triggering the
   engine's per-edit recompute policy) vs one `set_batch` of the same N. Report both
   distributions; the ratio is the finding (challenges "is set_value cheap?").
5. **`memory_load_and_edit`** — populate ~**10^7** cells (a chunked synthetic block,
   e.g. 10^7 literals; size tuned to fit the 15 GB box), edit, record **peak RSS**
   (read `/proc/self/statm` × page size on Linux — a tiny platform helper local to
   each bench bin, not in `shared/`). Discovery metric (record + judge), not a hard
   gate.

## Results & reporting (`common/src/report.rs`)

- Each scenario run produces a `bench_util::BenchResult` (env via
  `Environment::detect(commit).with_cpu(<parsed /proc/cpuinfo model name>)`, date
  passed in as a `&str` const for this phase, input size, `LatencyStats`, `GateResult`s,
  and an `extra` JSON bag carrying `{ engine, design, scenario, secondary metrics
  (e.g. peak_rss_bytes, batched_vs_single_ratio) }`).
- `write_all(results_dir, &[BenchResult])` writes one JSON per (engine, scenario,
  design) under `results/<engine>/` and appends a human-readable `results/summary.md`
  table (engine × scenario × design × p50/p99 × PASS/FAIL). The `scenarios` bins call
  this; committed `results/` is the machine-readable deliverable.
- The `scenarios` bins print each `GateResult::summary()` (PASS/FAIL + number) to
  stdout so a single documented command reproduces the verdict.

## Steps

1. **`common` crate.** `cargo init --lib`; `Cargo.toml` deps `datagen`,
   `bench_util` (relative), `serde`/`serde_json` (for `extra`). Implement
   `engine.rs` (trait + `EngineValue`/`CellInput`/`Viewport`/`EngineCaps`),
   `scenario.rs` (5 scenario builders + runners, parameterised sizes),
   `binding.rs` (D1/D2/D3 + `BindingCache`), `report.rs` (results writer +
   `summary.md` appender). A tiny in-crate `FakeEngine` (HashMap-backed, trivial
   incremental) lets `common`'s own unit tests exercise scenarios/bindings/report
   **without any real engine** (keeps `common` engine-neutral and fast).
2. **`formualizer` crate.** Deps: `common`, `datagen`, `bench_util` (relative),
   `formualizer = "0.7"` (features `eval,parse,workbook` — file features not needed
   here), `criterion` (dev), `serde_json`. Implement `FormualizerEngine`
   (`impl common::SpreadsheetEngine`) mapping per the table above; `read_viewport`
   uses `read_range`; `drain_dirty` reads the changelog; `recompute` uses
   `evaluate_all`/batch. `src/bin/scenarios.rs` runs all 5 scenarios × applicable
   designs at the `full` profile, writes `results/formualizer/`, prints PASS/FAIL.
   `benches/perf.rs` adds Criterion micro-benches (viewport read D1 vs D2 vs D3;
   single vs batched write) for stable throughput numbers.
3. **`ironcalc` crate.** Deps: `common`, `datagen`, `bench_util` (relative),
   `ironcalc = "0.7"`, `ironcalc_base = "0.7"`, `criterion` (dev), `serde_json`.
   First `tests/smoke.rs` — an **API-surface capture** mirroring the Formualizer
   smoke: probes that document `new_empty`, `set_user_input`, `evaluate` (full
   recompute), `get_cell_value_by_index`, `get_style_for_cell` (styles present on
   read), `UserModel` diff-list, and regression-lock the **no-incremental-recalc** and
   **no-range-read** facts. Then `IronCalcEngine` (`impl SpreadsheetEngine`) per the
   table; `read_viewport` loops per-cell; `drain_dirty` decodes `UserModel` diffs.
   `src/bin/scenarios.rs` + `benches/perf.rs` mirror the Formualizer crate.
4. **Run recorded scenarios** for both engines at the `full` profile; commit JSON +
   `summary.md` under `results/`. Capture the environment stamp (CPU from
   `/proc/cpuinfo`, cores, commit `783a515`, date `2026-07-01`).
5. **Write `findings.md`** (functional_spec §5.2 headings): Questions; What was done
   (crates, exact reproduce commands, inputs via `datagen`, metrics via `bench_util`);
   Results/evidence (both engines, per scenario, D1/D2/D3, p50/p99/max, PASS/FAIL,
   memory RSS, env stamp); Conclusion (incl. "couldn't determine X because Y");
   **Recommended binding design** + next-best; **perf-lens engine lean** (feeds
   Sub-project G); Risks/open questions.

## Tests

**`common`** (against the in-crate `FakeEngine`, fast/deterministic):
- `engine_value_roundtrip` — `EngineValue` conversions/equality behave.
- `binding_cache_reads_after_prime` — D3 cache returns primed values.
- `binding_cache_apply_dirty_refreshes_only_visible` — dirty ∩ visible refresh; a
  dirty cell outside the viewport is not fetched.
- `d1_d2_d3_agree` — for a fixed sheet + viewport, D1, D2, D3 return identical values
  (correctness parity across designs).
- `scenario_scrolling_read_runs` — returns one latency sample per pan step.
- `scenario_cascade_to_visible_reflects_edit` — after editing the head, the visible
  read reflects the new cascaded value (uses `FakeEngine` with `+1` semantics).
- `scenario_1m_chain_builder_shape` — builder emits the expected chain length/head.
- `scenario_writes_single_vs_batched_both_run` — both paths produce samples.
- `report_writes_json_and_summary` — `report::write_all` creates per-scenario JSON
  and appends a `summary.md` row (temp dir).

**`formualizer`** (`tests/adapter.rs`):
- `sets_and_reads_value`, `evaluate_cell_reflects_precedent_edit` (A3=A1+A2 → edit
  A1 → A3 updates), `read_viewport_matches_get_value` (range read == per-cell),
  `changelog_drain_reports_edited_cell`, `caps_flags_are_correct`
  (native_range_read, incremental_recalc, parallel_eval, change_log true;
  styles_on_read false).
- `d1_d2_d3_agree_on_real_engine` — the three designs agree over a Formualizer-backed
  synthetic region.

**`ironcalc`** (`tests/smoke.rs` API capture + `tests/adapter.rs`):
- Smoke: `builds_empty_model`, `set_user_input_and_read`,
  `formula_evaluates_after_full_evaluate`, `styles_present_on_read`
  (`get_style_for_cell` returns bold/fill), `usermodel_diff_list_records_edit`,
  `evaluate_is_full_recompute` (regression-lock: no incremental API),
  `no_native_range_read` (documents per-cell read requirement).
- Adapter: `sets_and_reads_value`, `formula_cascades_after_recompute`,
  `read_viewport_matches_get_value`, `dirty_drain_reports_edited_cell`,
  `caps_flags_are_correct` (native_range_read false, incremental_recalc false,
  parallel_eval false, change_log true, styles_on_read true).
- `d1_d2_d3_agree_on_real_engine`.

Scenario correctness is asserted in `common` against `FakeEngine`; the per-engine
adapter tests assert the mapping is faithful. Benchmarks themselves aren't unit-
tested for timing (that's the recorded run), but the `full`-vs-`dev` size profiles
keep `cargo test` fast while the recorded run uses spec-scale inputs.
