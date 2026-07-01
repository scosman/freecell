---
status: complete
---

# Phase 0: Scaffolding

## Overview

Phase 0 stands up the `experiments/` workspace skeleton and the two shared library
crates that every later sub-project consumes **read-only**. Per `architecture.md`
§1, each sub-project is an **independent** Cargo project (NOT a shared workspace):
there is no root `Cargo.toml` and no root `Cargo.lock`. The two shared crates —
`experiments/shared/datagen` and `experiments/shared/bench_util` — are standalone
library crates that later sub-projects depend on by **relative path**.

This phase is scaffolding ONLY. It does NOT start any engine benchmark, file I/O,
formatting, or GPUI work. It builds:

1. The `experiments/` directory tree skeleton (folders + placeholder `findings.md`
   stubs so the layout is real and self-documenting), matching functional_spec §5.1
   and architecture §1.
2. `experiments/README.md` — the index of sub-projects + how to run everything.
3. `experiments/shared/datagen` — deterministic synthetic-sheet + sample-file
   generators, as a real, tested Rust library crate with a clean, documented API.
4. `experiments/shared/bench_util` — timing / percentile / results-recording
   helpers, as a real, tested Rust library crate with a clean, documented API.

The shared crates are designed to be **frozen** after this phase. They are kept
dependency-light and engine-agnostic on purpose: the actual spreadsheet engine and
file/xlsx-writer choices are deferred to the Sub-project A gate (architecture §9),
so datagen must not couple to them. datagen therefore emits an engine-neutral cell
model + formula-pattern descriptions + CSV; bench_util only does timing, stats, and
results recording.

## Steps

### 1. Directory skeleton + findings stubs

Create the tree under `experiments/` (architecture §1, functional_spec §5.1):

```
experiments/
  README.md
  shared/
    datagen/      (cargo lib crate)
    bench_util/   (cargo lib crate)
  00-stack-decision/   { findings.md, smoke/.gitkeep }
  01-file-support/     { findings.md }
  02-datamodel-binding-perf/ { findings.md, results/.gitkeep }
  03-formatting/       { findings.md, results/.gitkeep }
  04-ui-poc/           { findings.md, raw-gpui/.gitkeep, gpui-component/.gitkeep,
                         scripts/.gitkeep, results/.gitkeep }
  05-round-2-proposal/ { round_2_explorations.md }
```

Each `findings.md` / `round_2_explorations.md` is a short stub that names the
sub-project, marks it "not started (Phase 0 scaffolding)", and lists the headings
required by functional_spec §5.2 (Questions / What was done / Results / Conclusion /
Recommended design + next-best / Risks). `.gitkeep` files keep empty dirs in git.
These stubs are placeholders only — later phases own and fill them.

### 2. `experiments/README.md`

Index of all sub-projects (A–G mapped to folders), the environment grounding facts
(architecture §0), the parallel-editor isolation rule pointer (architecture §2.2),
and **how to run everything**: for each shared crate and each sub-project, the
standard per-crate commands (`cargo fmt`, `cargo clippy`, `cargo build`,
`cargo test`, and `cargo bench` where relevant). Explicitly state that this is NOT
a Cargo workspace and that crates are built independently from their own dirs.

### 3. `experiments/shared/datagen` (lib crate)

`cargo new --lib`, edition 2024. Dependency-light (no engine, no xlsx writer).

Public API (engine-neutral, deterministic):

- `pub struct CellAddress { pub row: u32, pub col: u32 }` with `a1()` -> Excel A1
  string (e.g. `(0,0)` -> `"A1"`), and `EXCEL_MAX_ROWS`/`EXCEL_MAX_COLS` consts
  (1_048_576 / 16_384, functional_spec §5.4).
- Formatting model (proxy for "a big difficult sheet", architecture §7):
  ```rust
  pub struct CellFormat {
      pub bold: bool,
      pub italic: bool,
      pub highlight: Option<Rgb>,   // fill; ~10-20% of cells
      pub h_align: HAlign,
  }
  pub struct Rgb { pub r: u8, pub g: u8, pub b: u8 }
  pub enum HAlign { Left, Center, Right }
  pub enum CellValue { Empty, Number(f64), Text(String) }
  pub struct CellData { pub value: CellValue, pub format: CellFormat }
  ```
- `pub trait CellSource { fn cell(&self, row: u32, col: u32) -> CellData; }`
  (the static datamodel provider trait from architecture §7 — defined here so the
  UI PoC consumes it read-only).
- `pub struct SyntheticSheet { seed: u64, rows: u32, cols: u32 }` implementing
  `CellSource`: deterministic per-(row,col) hash → varied text lengths, numbers,
  ~10-20% highlighted, scattered bold/italic. Deterministic = same (seed,row,col)
  always yields the same `CellData` (no RNG state, no globals) so it's reproducible
  and thread-safe. A tiny splitmix64-style hash keeps it dependency-free.
- Variable sizes: `fn col_width(&self, col) -> f32`, `fn row_height(&self, row) -> f32`
  — deterministic, with a few "very wide" columns (architecture §7).
- Formula-pattern generators (engine-neutral; return cell address + formula string
  so any engine phase can feed them in):
  ```rust
  pub struct FormulaCell { pub addr: CellAddress, pub formula: String }
  pub fn linear_chain(len: u32, col: u32) -> impl Iterator<Item = FormulaCell>;   // =PREV+1 chain
  pub fn wide_fanout(sources: u32, dependents: u32) -> Vec<FormulaCell>;          // fan-out shape
  ```
  (cross-sheet / volatile shapes are described in docs but kept minimal; later
  phases extend their own crates, not this one.)
- CSV sample-file generation (no deps): `write_csv<W: Write>(w, &dyn CellSource,
  rows, cols) -> io::Result<()>` and a `csv_string(...)` convenience. xlsx
  generation is deliberately deferred (writer choice is gated, architecture §6/§9);
  a doc note states this and points to Sub-project B.

Module layout: `lib.rs` (re-exports + crate docs), `cell.rs` (addr/format/value/
CellData/CellSource), `synthetic.rs` (SyntheticSheet + sizing), `formula.rs`
(formula patterns), `csv.rs` (CSV writer).

### 4. `experiments/shared/bench_util` (lib crate)

`cargo new --lib`, edition 2024. Deps: `serde` (derive), `serde_json`. No wall-clock
calls inside deterministic recording paths (architecture §3: relative date passed
in).

Public API:

- Timing: `pub struct Stopwatch` (wraps `std::time::Instant`) with `start()`,
  `elapsed() -> Duration`; `pub fn time_once<T>(f) -> (T, Duration)`. These use the
  real monotonic clock for *measurement* only (allowed); the *recording* path takes
  the date as a parameter.
- Stats over latencies:
  ```rust
  pub struct LatencyStats {
      pub count, pub min_ns, pub max_ns, pub mean_ns,
      pub p50_ns, pub p99_ns,   // percentiles (architecture §3 latency reporting)
  }
  impl LatencyStats { pub fn from_durations(&[Duration]) -> Option<Self>; }
  pub fn percentile_ns(sorted: &[u64], pct: f64) -> u64;   // nearest-rank
  ```
- Pass/fail gating (architecture §3 / functional_spec §5.4):
  ```rust
  pub enum Verdict { Pass, Fail }
  pub struct GateResult { pub name, pub measured_ns, pub target_ns, pub verdict }
  pub fn gate_max(name, measured: LatencyStats-derived, target) -> GateResult;
  impl GateResult { pub fn print(&self); }  // prints "PASS/FAIL name: measured vs target"
  ```
- Results recording (serde-serializable, env-stamped, relative date passed in):
  ```rust
  pub struct Environment { pub cpu, pub os, pub cores, pub commit }   // strings
  impl Environment { pub fn detect(commit: impl Into<String>) -> Self; } // os/cores from std; commit passed in
  pub struct BenchResult {
      pub name: String,
      pub input_size: u64,
      pub date: String,          // relative date PASSED IN (no wall-clock here)
      pub environment: Environment,
      pub stats: Option<LatencyStats>,
      pub gates: Vec<GateResult>,
      pub extra: serde_json::Value,
  }
  impl BenchResult {
      pub fn new(name, input_size, date, environment) -> Self;
      pub fn to_json_pretty(&self) -> String;
      pub fn write_json(&self, path) -> io::Result<()>;
  }
  ```
  `Environment::detect` reads `std::env::consts::OS` and
  `std::thread::available_parallelism()` (no date). `Verdict`/`GateResult`/
  `LatencyStats`/`Environment`/`BenchResult` all derive `Serialize`.

Module layout: `lib.rs` (re-exports + docs), `timing.rs`, `stats.rs`, `gate.rs`,
`record.rs`.

### 5. Per-crate checks

Inside each of `experiments/shared/datagen` and `experiments/shared/bench_util`,
run and iterate to clean: `cargo fmt --all -- --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo build`, `cargo test`.

### 6. Mark plan complete

(Manager handles commit; this agent does not commit.)

## Tests

### datagen
- `cell_address_a1_mapping`: `(0,0)->"A1"`, `(0,25)->"Z1"`, `(0,26)->"AA1"`,
  `(0,16383)->"XFD1"`, row offset correct (`(1,0)->"A2"`).
- `excel_max_constants`: consts equal 1_048_576 and 16_384.
- `synthetic_is_deterministic`: same (seed,row,col) yields identical `CellData`
  across two independent `SyntheticSheet` instances and repeated calls.
- `synthetic_seed_varies`: different seeds produce differing output over a sample.
- `synthetic_highlight_ratio_in_band`: over a large sample, highlighted fraction is
  within ~5-30% (loose band around the 10-20% target so it never flakes).
- `synthetic_has_bold_italic_and_varied_text`: sample contains some bold, some
  italic, and a range of text lengths (incl. at least one long/"wide" value).
- `col_width_has_wide_columns`: at least one column exceeds a "very wide" threshold;
  widths deterministic.
- `linear_chain_formulas`: first cell has no `+1`-of-prev (or is a seed literal),
  subsequent cells reference the previous cell's A1 (`=A1+1` form), length matches.
- `wide_fanout_shape`: produces the requested source/dependent counts and each
  dependent references sources.
- `csv_roundtrip_shape`: `csv_string` over a small `SyntheticSheet` yields the right
  number of lines/columns and is stable across runs (deterministic).

### bench_util
- `percentile_nearest_rank`: known sorted vector → expected p50/p99/max.
- `latency_stats_from_durations`: handles empty (`None`), single element, and a
  known multi-element set (min/max/mean/p50/p99 correct).
- `gate_pass_and_fail`: measured < target → Pass; measured > target → Fail;
  `GateResult` fields populated.
- `environment_detect_no_date`: `Environment::detect("abc123")` sets commit, fills
  os/cores, and contains no date field (date lives only on `BenchResult`).
- `bench_result_json_roundtrip`: build a `BenchResult` with a passed-in date, gates,
  and stats; `to_json_pretty` contains the name, date string, input size, and
  "PASS"/"FAIL"; deserializes back to an equal value.
- `write_json_creates_file`: `write_json` to a temp path writes valid JSON that
  re-parses (uses scratch/temp dir).
