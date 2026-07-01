---
status: complete
---

# Phase 2.0: Scaffolding — frozen shared Round-2 harness

## Overview

Create the **frozen, read-only shared harness** every Round-2 experiment (SP1–SP5)
depends on (architecture §1, §3). It is an **independent Cargo LIB crate** at
`experiments/round-2/harness/` (its own `Cargo.toml`/`Cargo.lock`/`target/`; NOT a
workspace member) that carries, **verbatim** from the frozen Phase-1 crates:

- the `SpreadsheetEngine` trait + neutral value/coordinate types + benchmark scenarios
  (from `experiments/02-datamodel-binding-perf/common/`), and
- the IronCalc adapter (`impl SpreadsheetEngine` for IronCalc) pinned to the **same
  IronCalc version** Phase-1 used (0.7 / locked 0.7.1) (from
  `experiments/02-datamodel-binding-perf/ironcalc/src/lib.rs`).

Plus a new `peak_rss()` helper for fresh-process peak-memory measurement (lives here
because `shared/bench_util` is frozen). Dependencies on `shared/datagen` and
`shared/bench_util` are by relative path. The crate documents that it is **FROZEN /
read-only** for downstream Round-2 experiments.

Copying (not depending into `02/`) is the architecture's chosen reuse strategy: it
gives Round-2 one stable, read-only engine seam that keeps numbers comparable without
mutating frozen Phase-1 code (architecture §1 "Why a `round-2/harness/` copy").

## Steps

1. **Create `experiments/round-2/harness/Cargo.toml`** — a lib crate `round2_harness`:
   - `[dependencies]`: `datagen = { path = "../../shared/datagen" }`,
     `bench_util = { path = "../../shared/bench_util" }`, `serde` (derive), `serde_json`,
     `ironcalc = "0.7"`, `ironcalc_base = "0.7"` (exact same pins as
     `02/ironcalc/Cargo.toml`), plus `libc` (for the `getrusage` peak-RSS fallback).
   - `publish = false`.

2. **Copy engine-neutral modules verbatim** from `02/common/src/` into
   `harness/src/`, unchanged (same names/behavior so numbers stay comparable):
   - `engine.rs` — `SpreadsheetEngine` trait + `EngineValue`/`CellInput`/`Viewport`/
     `EngineCaps`.
   - `binding.rs` — `Design`/`BindingCache`/`read_under` (scenarios depend on these).
   - `scenario.rs` — the five benchmark scenarios + `Profile`/`targets`.
   - `fake.rs` — in-crate `FakeEngine` used by the copied unit tests (keeps the
     harness self-validating without a real engine).
   - `report.rs` — env-stamped results recording (`ScenarioResult`, `write_all`, …).
   - `runner.rs` — the shared benchmark driver (`run_suite`, `run_memory_only`).
   - `sysinfo.rs` — `peak_rss_bytes()` (VmHWM) + `cpu_model()` platform helpers.
   These are all engine-neutral (no Formualizer dependency), so they come across as-is.

3. **Copy the IronCalc adapter** from `02/ironcalc/src/lib.rs` into
   `harness/src/ironcalc.rs`, verbatim except the **mechanically required** import
   rewrite: `binding_common::{…}` → `crate::engine::{…}` (the trait now lives in the
   same crate). Names, mapping logic, and behavior are otherwise identical. Re-export
   its `IronCalcEngine` from `lib.rs`.

4. **Add `harness/src/peak_rss.rs`** — a `peak_rss() -> u64` helper for fresh-process
   peak-memory measurement, returning **bytes** (documented):
   - Linux: read `VmHWM` from `/proc/self/status` (kB → bytes).
   - Fallback (non-Linux, or if `/proc` read fails): `getrusage(RUSAGE_SELF).ru_maxrss`
     (Linux reports kB, macOS reports bytes — documented; convert on Linux).
   Signature: `pub fn peak_rss() -> u64;` returns the process peak RSS high-water mark
   in bytes.

5. **Write `harness/src/lib.rs`** — crate root doc comment stating the crate is
   **FROZEN / read-only** for Round-2 (consume, never edit; needed changes → escalate),
   `pub mod` declarations for every module, and convenient re-exports (the
   `SpreadsheetEngine` trait + neutral types, `IronCalcEngine`, `peak_rss`, scenario
   `Profile`/`targets`, runner entry points).

6. **Add `experiments/round-2/README.md`** — a short index of the Round-2 subtree
   (harness + the five experiment folders + SYNTHESIS.md) per architecture §1, noting
   the harness is frozen.

7. **Add `harness/tests/smoke.rs`** — the required smoke test (see Tests) proving the
   copied IronCalc adapter works through the copied trait and that `peak_rss()` returns
   a plausible non-zero value.

8. **Build + test** in `experiments/round-2/harness/` with a foreground `timeout`
   (IronCalc compile is slow; raise the timeout rather than backgrounding if needed).

## Tests

- **Copied unit tests (verbatim):** all `#[cfg(test)]` modules carried over from
  `02/common` (engine, binding, scenario, fake, report, runner) and from the IronCalc
  adapter (`value_input_rendering`, `cell_value_conversion`) — they validate the
  scenarios/bindings/report against `FakeEngine` and the adapter's value conversions.
- **`smoke_ironcalc_adapter_roundtrip` (new, `tests/smoke.rs`):** construct
  `IronCalcEngine::new_blank()` via the copied trait, set two literal cells and a
  `=A1+B1` formula, `recompute()`, then `get_value()` back and assert the evaluated
  result — proving the copied adapter compiles and works standalone.
- **`smoke_read_viewport` (new):** seed a couple of cells and assert `read_viewport`
  returns them, exercising the neutral read path through IronCalc.
- **`peak_rss_is_plausible_nonzero` (new):** assert `peak_rss()` returns a non-zero
  value in a sane range (well above a few MB, below the container's ~15 GB).
