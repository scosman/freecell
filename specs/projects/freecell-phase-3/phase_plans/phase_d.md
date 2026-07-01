---
status: complete
---

# Phase D: Engine Robustness (circular refs / malformed input / worker-panic recovery)

## Overview

Investigation D (functional_spec §6-D, architecture §5) is the cheap robustness check
that must clear before the build commit. Every FreeCell edit triggers a **full-workbook
`evaluate()`** on the SP1 worker thread (`round-2/01-async-interop`), so a circular
reference that hangs or a malformed formula that panics would lock or poison the app.

This phase builds a probe crate in `experiments/round-3/D-robustness/` that feeds
IronCalc 0.7.1 (via the frozen `round2_harness` `Model` adapter) three input classes and
**asserts** the engine's behavior:

1. **Circular references** — `A1=A1`, mutual `A1=B1 / B1=A1`, and a longer N-cell cycle:
   IronCalc must return a typed error (`#CIRC!`, `CellType::ErrorValue`) and **not hang or
   stack-overflow**. This is the GATE.
2. **Malformed / pathological input** — giant formulas, deeply-nested parentheses,
   syntactically invalid formulas: graceful typed error, **not a panic**.
3. **Worker-panic recovery** — a panic inside the worker's `evaluate()`/apply step
   poisons the SP1 worker (it owns the `Model`). Test `catch_unwind` and a restart path,
   and recommend one of {`catch_unwind`, restart-worker, "evaluate can't panic on user
   input"} on the evidence.

### What the IronCalc source already tells us (cited, to validate against — not to assume)

- **Circular refs are detected, not infinitely recursed.** `Model::evaluate_cell`
  (`ironcalc_base-0.7.1/src/model.rs:801-848`) marks each formula cell
  `CellState::Evaluating` before descending; re-entering an `Evaluating` cell returns
  `Error::CIRC` (`model.rs:824-829`) — a marker-based cycle guard, so a cycle terminates
  with `#CIRC!` rather than looping. The probe must **confirm this empirically under a
  timeout** (adversarial: don't trust the source read alone).
- **Error cells are typed.** `Error::CIRC` renders `#CIRC!`
  (`expressions/token.rs:113`); an error cell reads back as
  `CellValue::String("#CIRC!")` (`cell.rs:159-178`) and `get_cell_type` →
  `CellType::ErrorValue` (`cell.rs:122`, `types.rs:153-160`). So the typed assertion is
  `get_cell_type == ErrorValue` **and** the string is the expected `#…!`.
- **`set_user_input` doesn't reject bad formulas up front.** A syntactically invalid
  formula is stored as a formula whose parse produces an error node; it surfaces as an
  error value on `evaluate()`, not as an `Err` from `set_user_input`
  (`model.rs:1518-1608`). So "malformed input → error, not panic" is an **evaluate-time**
  property.
- **The parser/evaluator are recursive with no explicit depth cap.** `Parser::parse_expr`
  → … → `parse_primary` → `parse_expr` (`expressions/parser/mod.rs:331-580`) recurse per
  paren nesting; the evaluator walks the parsed tree recursively. Deeply-nested parens are
  therefore the prime stack-overflow candidate — the probe must push nesting and record
  the ceiling (a stack overflow aborts the process, so run it in a **child thread with a
  bounded stack / under `timeout`** and treat an abort as a recorded finding, not a wedge).

## Steps

1. **`Cargo.toml`** — already scaffolded (`round2_harness`, `datagen`, `bench_util`,
   `ironcalc_base` 0.7.1, `anyhow`). Add nothing new unless needed. Keep the `[[bin]]`
   `robustness` and add a `[lib]` so tests can exercise the probe functions. Add
   `serde_json` only if we emit a `results/` JSON (we will, for the env-stamped summary).

2. **`src/lib.rs`** — the probe library. Expose:
   - `fn error_probe(input: &str) -> CellOutcome` — writes `input` at A1 (plus any needed
     helper cells) on a fresh `Model`, calls `evaluate()`, reads back
     `(CellType, CellValue)`. `CellOutcome` = `{ cell_type, value_string, is_error }`.
     Uses `round2_harness::IronCalcEngine::from_model` / a raw `Model` — whichever gives
     direct `get_cell_type` + `get_cell_value_by_index` access (the adapter hides
     `get_cell_type`, so probe the raw `ironcalc_base::Model` directly here; that is
     local probe code, harness untouched).
   - `fn cycle_probe(kind: CycleKind) -> CellOutcome` — builds `A1=A1` (Self), mutual
     `A1=B1,B1=A1` (Mutual), and a longer cycle of length N (`Chain(n)`:
     `A1=A2, A2=A3, …, A(n)=A1`), evaluates, returns the outcome at the head cell.
   - `fn nested_parens(depth: usize) -> String` and `fn wide_sum(n: usize) -> String`
     (giant formula `=1+1+…` or `=SUM(A1,A1,…)`) — pathological-input generators.
   - `fn run_in_bounded_thread<F, T>(stack_bytes, timeout, f) -> ThreadResult<T>` — spawns
     `f` on a thread with an explicit stack size and a join deadline, so a recursion
     abort/hang is contained; returns `Completed(T)`, `TimedOut`, or `Panicked(msg)`
     (via `catch_unwind` inside the thread).
   - `fn worker_eval_with_catch_unwind(...)` — a minimal SP1-shaped worker: owns a
     `Model` on a spawned thread, applies an edit + `evaluate()` inside
     `std::panic::catch_unwind(AssertUnwindSafe(...))`, reports whether it panicked and
     whether the thread/model survived to serve a subsequent read. This is the
     worker-recovery experiment.

3. **`src/main.rs`** — replace the stub. Run every probe **foreground**, print a compact
   pass/fail table (cycle behavior, malformed-input behavior, worker-recovery outcome),
   and write an env-stamped `results/robustness.json` (via `bench_util::Environment` +
   `serde_json`) summarizing: for each input class, the observed `CellType` / error
   string / timed-out? / panicked?, and the recommended worker strategy. The binary is
   the reproducible entry point (`cargo run --release`).

4. **`tests/robustness.rs`** — the real assertions (this is where the GATE lives):
   - `circular_self_ref_returns_error_not_hang`
   - `circular_mutual_ref_returns_error`
   - `circular_long_cycle_returns_error`
   - `invalid_formula_is_error_not_panic`
   - `deeply_nested_parens_no_panic_or_bounded_finding`
   - `giant_formula_evaluates_without_panic`
   - `worker_survives_or_recovers_from_bad_eval`
   Each asserts the concrete outcome (error type / no-panic / recovery), and cycle/paren
   tests run through `run_in_bounded_thread` so a hypothetical hang/overflow is a recorded
   `TimedOut`/`Panicked` finding rather than a wedged test process.

5. **`findings.md`** — Phase-1 §5.2 headings: Question(s); What was done (approach + code
   pointers + reproduce commands); Results/evidence (per-input-class outcomes with the
   IronCalc source citations); Conclusion (graded against the GATE); Recommended worker
   design + next-best alternative; Risks/open questions. Commit `results/robustness.json`.

## Tests

- **`circular_self_ref_returns_error_not_hang`** — `A1=A1` → after `evaluate()`, A1 is
  `CellType::ErrorValue` with string `#CIRC!`; completes well within the bounded-thread
  deadline (proves no hang).
- **`circular_mutual_ref_returns_error`** — `A1=B1, B1=A1` → both cells error (`#CIRC!`),
  no hang.
- **`circular_long_cycle_returns_error`** — an N-cell ring (e.g. N=1000) → head cell
  errors (`#CIRC!`), no hang/stack-overflow; records that the marker guard scales.
- **`invalid_formula_is_error_not_panic`** — `=1+`, `=SUM(`, `=@#$%`, `=)(` →
  `evaluate()` does not panic; cell is `ErrorValue` (or a `#ERROR!`/`#NAME?` typed error),
  `set_user_input` did not `Err` (or if it did, that's recorded — still not a panic).
- **`deeply_nested_parens_no_panic_or_bounded_finding`** — increasing paren depth on a
  bounded-stack thread; assert either a clean typed result **or** a recorded
  `Panicked/TimedOut` ceiling (the honest finding: at depth D the parser aborts). Either
  way, the **main-thread process is not killed** — that's the point of the bounded thread.
- **`giant_formula_evaluates_without_panic`** — `=1+1+…` with thousands of terms and a
  wide `SUM` → completes with a numeric result, no panic.
- **`worker_survives_or_recovers_from_bad_eval`** — drive the SP1-shaped worker with an
  input designed to (attempt to) panic `evaluate()`; assert that with `catch_unwind` the
  worker thread stays alive and a subsequent good edit still evaluates (recovery proven),
  OR — if no user input can make `evaluate()` panic — assert the corpus of adversarial
  inputs all return normally (evidence for the "evaluate can't panic on user input"
  recommendation). The test encodes whichever the evidence supports and the findings doc
  states the recommendation.
