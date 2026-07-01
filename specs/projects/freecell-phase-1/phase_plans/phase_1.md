---
status: draft
---

# Phase 1: Stack Decision (GATE — Sub-project A)

## Overview

Phase 1 is the **gating** sub-project (functional_spec §6.A, architecture §4). It
answers, with evidence, the question that unblocks the whole project: *Is
**Formualizer + GPUI** (or a better-ranked alternative) a stack we can confidently
build FreeCell on?* It has two deliverables:

1. A **Formualizer smoke test** — a small, correctness-focused Cargo crate at
   `experiments/00-stack-decision/smoke/` that exercises and, crucially,
   **documents the real Formualizer 0.7.0 API surface**. This captured surface is
   the input that later phase plans (B–E) are written against, so it must probe the
   areas those phases depend on: single-cell read/eval, range/bulk reads, parallel
   evaluation, update-subscription / dirty tracking / change notification,
   styles/formatting/metadata exposure, and how Apache Arrow is surfaced.

2. **Stack research + a ranked recommendation** — synthesized from two parallel
   web-research helper agents (engine landscape; GPU/native-UI landscape) into
   `experiments/00-stack-decision/findings.md` following the functional_spec §5.2
   headings, ending in a defensible ranked stack recommendation the human signs off.

This phase does **not** benchmark (that is Sub-project C) and does not build any
part of the real app. It is pure de-risking. Work stays inside
`experiments/00-stack-decision/` (+ read-only `experiments/shared/` and `specs/`).

## Captured API facts (from crate source in `~/.cargo/registry`, `formualizer 0.7.0`)

These are the load-bearing facts the smoke test verifies and documents:

- **Meta crate `formualizer`** re-exports layers behind features. For the smoke test
  we enable `eval, parse, workbook, calamine, csv, umya, json` (umya is needed for
  `Workbook::to_xlsx_bytes`; calamine for reading `.xlsx`; csv for CSV).
- **Core type: `formualizer::Workbook`** (from `formualizer-workbook`). Rows/cols are
  **1-based** everywhere in this API (`RangeAddress::new` rejects 0).
  - Build/mutate: `Workbook::new()`, `has_sheet(&str)`, `add_sheet(&str)`,
    `set_value(sheet, row, col, LiteralValue)`, `set_formula(sheet, row, col, &str)`,
    `set_values(...)`, `set_formulas(...)`, `write_range(sheet, start, BTreeMap<(u32,u32), CellData>)`.
  - Single-cell read/eval: `get_value(sheet,row,col) -> Option<LiteralValue>`,
    `get_formula(...) -> Option<String>`, `evaluate_cell(sheet,row,col) -> Result<LiteralValue>`.
  - Bulk/range: `read_range(&RangeAddress) -> Vec<Vec<LiteralValue>>` (columnar
    range view under the hood), `evaluate_cells(&[(&str,u32,u32)]) -> Result<Vec<LiteralValue>>`,
    `evaluate_cells_cancellable(..)`, `evaluate_all()`, `build_recalc_plan()` +
    `evaluate_with_plan(..)`, `get_eval_plan(..)`.
- **`LiteralValue`** (from `formualizer-common`): `Int, Number, Text, Boolean, Array,
  Date, DateTime, Time, Duration, Empty, Pending, Error`.
- **Parallel eval** is a real, first-class knob: `formualizer::EvalConfig {
  enable_parallel: bool, max_threads: Option<usize>, .. }`, set via
  `WorkbookConfig` / `engine_mut().config`.
- **Change notification / dirty tracking**: `Workbook::changelog() -> &ChangeLog`
  (from `formualizer-eval`), an append-only audit trail of `ChangeEvent`
  (`SetValue{old,new}`, `SetFormula`, spill, edge, named-range events…), with
  compound grouping, `events()`, `take_from(index)`, `len()`. Enabled via
  `set_changelog_enabled(true)`. Plus `undo()`/`redo()` and `action(..)`
  transactions. This is the substrate for "subscribe to visible-cell updates".
- **Arrow model**: `formualizer-eval` depends on real Apache `arrow` crates
  (`arrow, arrow-array, arrow-buffer, arrow-cast, arrow-schema, arrow-select`).
  Cell "truth" is an Arrow-backed columnar store (the engine journal has
  `ArrowOp`/`ArrowUndoBatch`; `range_view` reads columnar). `AccessGranularity`
  (`Cell/Range/Sheet/Workbook`) and `LoadStrategy` (incl. `LazyRange{row_chunk,
  col_chunk}`) describe the columnar access/lazy-load model.
- **File I/O**: `CalamineAdapter` (read `.xlsx`), `CsvAdapter` (read/write CSV),
  `UmyaAdapter` (read/write `.xlsx`, styles). Loaded via
  `Workbook::from_reader(backend, LoadStrategy, WorkbookConfig)`. Save to bytes via
  `Workbook::to_xlsx_bytes()` (uses umya).
- **Styles / formatting (KEY FINDING to verify)**: `traits::CellData` carries only a
  `style: Option<StyleId>` (opaque `u32`). `BackendCaps.styles` is `true` for umya,
  `false` for calamine — **but** both backends' read paths hard-code `style: None`
  in 0.7.0, so styles/formatting are **not surfaced through the CellData read path**.
  Formatting must be read from the underlying `umya_spreadsheet` workbook directly.
  The smoke test documents this so Sub-project D designs around it.

## Steps

### 1. Create the smoke crate

`experiments/00-stack-decision/smoke/` via `cargo new --lib` semantics (edition 2024),
but it is primarily a **test crate**: the real work lives in `tests/` integration
tests plus a small documented `lib.rs` that captures the API surface in module docs.

`Cargo.toml`:
- `[dependencies] formualizer = { version = "0.7", default-features = false,
  features = ["eval","parse","workbook","calamine","csv","umya","json","system-clock"] }`
  and `anyhow` (architecture §8 error handling).
- `[dev-dependencies] datagen = { path = "../../shared/datagen" }` — reuse the
  engine-neutral CSV generator to produce a tiny CSV input from committed code
  (functional_spec §5.3: inputs generated by committed code, not hand-made binaries).
- No dependency on `bench_util` (this is correctness, not a benchmark).

### 2. `src/lib.rs` — documented API-surface capture + small helpers

Crate-level docs summarizing the captured surface (the bullets above). A few thin,
tested helper fns that make the probes readable and reusable:

- `pub fn new_workbook_with_changelog() -> Workbook` — `Workbook::new()` +
  `set_changelog_enabled(true)`; used by the dirty-tracking probe.
- `pub fn build_sum_workbook() -> anyhow::Result<Workbook>` — a tiny in-memory
  workbook: `A1=1`, `A2=2`, `A3==A1+A2`, returns it built + graph-prepared.
- `pub fn xlsx_bytes_from(wb: &Workbook) -> anyhow::Result<Vec<u8>>` — wraps
  `to_xlsx_bytes()`.
- `pub fn load_xlsx_bytes(bytes: &[u8]) -> anyhow::Result<Workbook>` — open via
  `CalamineAdapter::open_bytes` + `Workbook::from_reader`.
- `pub fn load_csv_str(csv: &str) -> anyhow::Result<Workbook>` — open via
  `CsvAdapter::open_bytes` + `from_reader`.

Each helper's doc comment names the exact upstream methods/types it uses, so the
file doubles as living API documentation.

### 3. `tests/smoke.rs` — the probes (correctness assertions)

Integration tests, each asserting real behavior AND serving as a documented probe:

1. `builds_and_evaluates_dependent_cell` — build `A1=1,A2=2,A3==A1+A2`; assert
   `evaluate_cell("Sheet1",3,1) == Number(3.0)`. Then `set_value(A1, 10)`;
   re-evaluate `A3`; assert it recalculated to `12.0`. (Confirms recalc on mutate.)
2. `range_bulk_read_returns_grid` — build a small 3×3 block; `read_range(&RangeAddress)`
   returns the expected `Vec<Vec<LiteralValue>>`; `evaluate_cells(&[..])` returns the
   batch in order. (Confirms range + bulk read APIs.)
3. `xlsx_roundtrip_via_umya_and_calamine` — build workbook → `to_xlsx_bytes()` →
   `load_xlsx_bytes()` → assert values survive the round trip (a formula cell's
   cached/evaluated value and a literal). Uses no committed binary — the `.xlsx` is
   produced by committed code (umya). (Confirms `.xlsx` load path.)
4. `csv_load_reads_values` — generate a tiny CSV with `datagen::csv_string` over a
   small `SyntheticSheet`, load via `CsvAdapter`, assert a couple of cells read back.
   (Confirms CSV load path + reuses the shared generator.)
5. `changelog_tracks_edits` — enable changelog; `set_value`/`set_formula` a couple
   cells; assert `changelog().events()` grew and contains a `SetValue`/`SetFormula`
   with the new value. (Confirms dirty-tracking / change-notification substrate.)
6. `parallel_eval_config_is_exposed` — construct a `WorkbookConfig` with
   `eval.enable_parallel = true` / `max_threads = Some(n)`; build a workbook with it;
   `evaluate_all()` succeeds. (Confirms parallel-eval knob is real and usable.)
7. `styles_not_surfaced_through_celldata` — DOCUMENTS the negative finding:
   `CalamineAdapter.capabilities().styles == false` and a umya-read `CellData.style`
   is `None`. Asserted so the finding is regression-locked and unambiguous for
   Sub-project D. (Records the styles gap as evidence, not a silent omission.)

If any assumed API is missing at compile/test time, that is itself a finding to
record in `findings.md` rather than a blocker (architecture §8, §10).

### 4. Web research (helper sub-agents, run in parallel)

Two `general-purpose` helper agents (already launched at plan time) research:
(a) the spreadsheet-engine landscape (Formualizer maturity, ~coverage vs Excel's
~500 fns, file fidelity, license, maintenance/bus-factor, perf ceiling, Arrow
model, and alternatives: ironcalc, calamine+writers, build-our-own); (b) the
GPU/native-UI landscape (GPUI standalone vs Zed coupling, `gpui-component`,
alternatives: egui/egui_table, Xilem+Vello, Iced, Slint, Freya, raw wgpu). The lead
synthesizes their reports.

### 5. `findings.md` — synthesis

Fill `00-stack-decision/findings.md` to the functional_spec §5.2 standard:
**Questions / What was done / Results & evidence / Conclusion / Recommended design +
next-best alternative / Risks & open questions**. It must contain the **captured
Formualizer API surface** (from steps 2–3, with concrete method/type names) and a
**ranked** stack recommendation (2–4 full stacks), with reasoning and explicit
risks, so a human can sign off go/pivot.

### 6. Checks

Inside `smoke/`: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D
warnings`, `cargo build`, `cargo test`. Iterate until clean/passing. Do not commit
(manager handles commits).

## Tests

- `builds_and_evaluates_dependent_cell`: dependent cell evaluates to the sum, and
  recomputes after mutating a precedent (recalc confirmed).
- `range_bulk_read_returns_grid`: `read_range` returns the expected 2D grid and
  `evaluate_cells` returns a batch of the right length/order.
- `xlsx_roundtrip_via_umya_and_calamine`: workbook → xlsx bytes → reloaded workbook
  preserves the probed values (file load confirmed, no committed binary).
- `csv_load_reads_values`: a `datagen`-generated CSV loads and reads back expected
  cells (CSV load confirmed; shared generator reused).
- `changelog_tracks_edits`: edits append `ChangeEvent`s to the changelog with the
  new values (dirty-tracking / notification substrate confirmed).
- `parallel_eval_config_is_exposed`: a workbook built with `enable_parallel`/
  `max_threads` evaluates successfully (parallel-eval knob confirmed).
- `styles_not_surfaced_through_celldata`: calamine reports `styles=false` and a umya
  `CellData.style` is `None` (styles-gap finding regression-locked for Sub-project D).
