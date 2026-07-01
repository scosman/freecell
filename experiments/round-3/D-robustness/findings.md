# Investigation D — Engine robustness (cycles / malformed input / worker recovery)

> Phase-3 investigation D (functional_spec §6-D, architecture §5). FreeCell recomputes
> the **whole workbook** on every edit, on the SP1 worker thread that **owns** the
> `Model` (`experiments/round-2/01-async-interop`). So a cycle that hangs, or bad input
> that crashes, would lock or take down the app. This is the cheap-but-load-bearing
> robustness gate before the build commit. Every claim below is backed by a runnable
> probe (`cargo test`, 9 assertions; `cargo run --release` for the env-stamped
> `results/robustness.json`) or a cited IronCalc 0.7.1 source location.

## Question(s)

1. **Circular references** — does IronCalc detect a cycle (`A1=A1`; mutual `A1=B1,B1=A1`;
   a long ring) and return a typed error, or hang / stack-overflow? *(GATE — a cycle must
   not lock the app.)*
2. **Malformed / pathological input** — do giant, deeply-nested, and syntactically
   invalid formulas produce a graceful typed error, **not a panic**? *(GATE.)*
3. **Worker-panic recovery** — if an `evaluate()`/apply can die on bad input, does an
   SP1-style worker thread that owns the model survive? What recovery does the build need
   — `catch_unwind`, restart-the-worker, or "evaluate can't panic on user input"?
   *(DELIVERABLE.)*

## What was done

An independent Cargo project `robustness` (`experiments/round-3/D-robustness/`), depending
**read-only** on the frozen `../../round-2/harness` (for `cpu_model` env stamping) and
`../../shared/bench_util` (`Environment`), and directly on `ironcalc_base` 0.7.1 (same pin
as the harness `Cargo.lock`). The robustness of the engine is a `Model`-level property, so
the probes drive `ironcalc_base::Model` directly — the harness adapter hides
`get_cell_type`, which is precisely the typed-error signal we assert on.

- **`src/lib.rs`** — the probes + isolation primitives:
  - `error_probe(input)` / `cycle_probe(kind)` — write input at A1 on a fresh `Model`,
    `evaluate()`, read back `(CellType, CellValue)`. `is_error = (CellType == ErrorValue)`.
  - `wide_add(n)` (`=1+1+…`, `n` terms), `nested_parens(d)` (`=((((1))))`, depth `d`) —
    pathological generators.
  - `run_in_bounded_thread(stack, deadline, f)` — runs `f` on a spawned thread with an
    explicit stack + join deadline: a **hang** → `TimedOut`, an **unwind panic** →
    `Panicked` (caught). (A stack-overflow **abort** is NOT caught here — see below.)
  - `run_recursion_in_subprocess_with(bin, shape, size)` + `recursion_child` — the ONLY
    safe way to probe a stack-overflow input: re-exec the `robustness` binary in a **child
    process**, observe its exit status. An abort kills the child, not us. `find_overflow_
    ceiling` bisects the overflow depth.
  - `worker_recovery_probe(adversarial)` — a minimal **SP1-shaped worker** that owns a
    `Model` on a spawned thread and applies edits inside `catch_unwind`; feed it the
    adversarial input, then a known-good `=2+3`, and check the good result (`5`) comes
    back (worker not poisoned).
- **`src/main.rs`** — runs every probe **foreground**, prints a table, and writes the
  env-stamped `results/robustness.json`. Also dispatches the `--nested-parens|--wide-flat
  <size>` child subcommands.
- **`tests/robustness.rs`** — the 9 GATE assertions.

**Reproduce:** from `experiments/round-3/D-robustness/`:
`cargo test --release` (assertions) and `cargo run --release` (table + `results/`).
Both foreground; the child-isolated overflow probes bisect via O(log n) child launches.

### IronCalc 0.7.1 source citations (validated empirically, not assumed)
- **Cycle guard** — `evaluate_cell` marks each formula cell `CellState::Evaluating` before
  descending; re-entering an `Evaluating` cell returns `Error::CIRC`
  (`ironcalc_base-0.7.1/src/model.rs:801-848`, esp. `824-829`). A **marker guard**, so a
  cycle terminates with `#CIRC!` rather than recursing the whole ring.
- **Typed errors** — `Error::CIRC` → `#CIRC!` (`expressions/token.rs:113`); an error cell
  reads back as `CellValue::String("#CIRC!")` (`cell.rs:159-178`) and `get_cell_type` →
  `CellType::ErrorValue` (`cell.rs:122`, `types.rs:153-160`).
- **No up-front rejection** — a bad formula is stored, not rejected by `set_user_input`
  (`model.rs:1518-1608`); the error surfaces on `evaluate()`.
- **Recursive parser, no depth cap** — `Parser::parse_expr → … → parse_primary →
  parse_expr` (`expressions/parser/mod.rs:331-580`); the evaluator walks the tree
  recursively. This is the stack-overflow vector.

## Results / evidence

Environment: Intel Xeon @ 2.80 GHz, x86_64 Linux, 4 cores, IronCalc 0.7.1
(`results/robustness.json`). All wall times are single-shot on a fresh `Model`.

### 1. Circular references — PASS (typed error, no hang)
| Case | Result | `CellType` | Wall |
|---|---|---|---|
| `A1=A1` (self) | `#CIRC!` | `ErrorValue` | 0.7 ms |
| `A1=B1, B1=A1` (mutual) | `#CIRC!` (both) | `ErrorValue` | 0.2 ms |
| ring `A1→A2→…→A1000→A1` | `#CIRC!` | `ErrorValue` | 4.1 ms |

Every cycle returns a typed `#CIRC!` in **single-digit ms** — no hang, no stack overflow,
even for a 1000-cell ring (the marker guard, not deep recursion, is what fires). Each ran
under a 20-30 s bounded-thread deadline; a hang would have surfaced as `TimedOut` and
failed the assertion.

### 2. Malformed / invalid input — PASS (typed error, no panic)
`=1+`, `=SUM(`, `=@#$%`, `=)(`, `=(`, `=A1:`, `=IF(`, `=1/0`, `=0/0`, `=1E308*1E308`,
`=SQRT(-1)` → all `CellType::ErrorValue` (`#ERROR!` for syntax, the specific error for
math), **no panic**, sub-ms each.
- Adversarial dig: `="unterminated` is **not** an error — IronCalc's lexer recovers it as
  the plain string `unterminated`. That is still **graceful** (no panic; the GATE cares
  about "not a panic"), just a value rather than an error cell. Asserted explicitly.

### 3. Giant flat formula — PASS (computes, no panic)
`=1+1+…` at 1000 / 5000 / 8000 terms → `1000` / `5000` / `8000`, no panic (2-18 ms).

### 4. Deep-recursion overflow — the one real crash mode (DISCOVERY)
Two shapes drive the recursive parser and **stack-overflow** past a ceiling — an **abort
(SIGABRT via the stack guard page), NOT a catchable panic**. Child-isolated bisection on
the **default** stack:

| Shape | OK up to | Aborts by |
|---|---|---|
| nested parens `=((…1…))` | ~2726 depth | ~2755 |
| wide flat `=1+1+…` | ~11870 terms | ~11897 |

The ceiling scales ~linearly with stack size (measured across 2/8/64/256 MiB during
development): **nested ~340 depth/MiB, flat ~1465 terms/MiB**. Consequences for FreeCell:
- The SP1 worker is a **spawned** thread → default ~2 MiB stack → the *lower* ceilings
  (~490 nesting depth / ~2832 flat terms). It is **more** vulnerable than the 8 MiB main
  thread, not less.
- A stack overflow **aborts the whole process**; `catch_unwind` cannot save it (verified:
  a control panic *is* caught, the overflow abort is *not*).

### 5. Worker-panic recovery — DELIVERABLE
The SP1-shaped worker (owns the `Model`, applies edits inside `catch_unwind`) was fed
`=A1` (cycle), `=1+`, `=SUM(`, and a 5000-term flat formula, each followed by a good
`=2+3`. Every time: `adversarial_panicked = false`, `recovered = true` (`=2+3 ⇒ 5`).
**No user input made `evaluate()` unwind-panic** — across the whole corpus, bad input is
always a typed error, never a panic. The only way to kill the worker is the stack-overflow
**abort** (§4), which no wrapper catches.

## Conclusion (graded against the GATE)

- **GATE — circular refs return an error and do not hang: PASS.** `#CIRC!`
  (`CellType::ErrorValue`) in single-digit ms for self / mutual / 1000-ring, via a marker
  guard (`model.rs:824-829`), not deep recursion. No iteration cap needed.
- **GATE — malformed input → error, not panic: PASS.** All syntactically-invalid /
  math-error / pathological inputs return typed errors; none panic. (`="unterminated`
  recovers to a string — graceful.)
- **DISCOVERY — one real crash mode: deep-recursion stack overflow (an abort).** Not just
  nested parens — a long flat operator chain overflows too. Uncatchable by `catch_unwind`.
  This is a **FreeCell-side input-validation requirement**, surfaced before the build, not
  an IronCalc defect that blocks it.

Nothing here fires an off-ramp: the engine's cycle + error handling is sound, and the one
crash mode has a cheap, well-understood FreeCell-side mitigation.

## Recommended design + next-best alternative

**Recommended (worker robustness):** *"evaluate can't panic on user input — so cap the
input and wrap the eval defensively."* Concretely, three layers, cheapest first:

1. **Pre-eval input cap (the real fix).** Before handing a formula to IronCalc, reject
   (surface as `#ERROR!` in the UI) any formula exceeding a **length** and **nesting-depth**
   bound — e.g. depth ≤ 128 and length ≤ a few KB (Excel itself caps nesting at 64 and
   formula length at 8192 chars, so a conservative cap is also spec-compatible and well
   under the ~490-depth / ~2832-term worker ceilings). This eliminates the only crash mode
   at its source, cheaply, and is the load-bearing recommendation.
2. **Larger worker stack (defense-in-depth).** Spawn the SP1 worker with an explicit large
   stack (e.g. `thread::Builder::stack_size(64 MiB)`), raising the ceiling ~30× (~21k
   nesting depth). Cheap; buys margin, but never *eliminates* the risk on its own — so it
   supplements, not replaces, the cap.
3. **`catch_unwind` around apply+evaluate (belt-and-braces).** Wrap the worker's
   `set_user_input`+`evaluate` in `catch_unwind(AssertUnwindSafe(...))`. Empirically it
   catches nothing today (no user input unwind-panics), but it costs ~nothing and protects
   against a future IronCalc that *does* `panic!` on some input — turning a would-be
   process kill into a recoverable per-edit error. **Note the limit:** it does **not**
   catch the stack-overflow abort — layer 1 is what handles that.

**Next-best alternative (restart-the-worker):** if a future IronCalc introduces an
input that aborts *without* deep recursion (so a cap can't predict it), run each
`evaluate()` in a **short-lived child process / isolated eval sandbox** and restart it on a
non-zero exit, re-loading the model from the last good `to_bytes()` snapshot (SP1 already
keeps this snapshot route). Heavier (a process boundary + re-load per crash) and only
worth it if layer 1's static cap proves insufficient — not needed on today's evidence.

## Risks / open questions carried forward
- **Cap tuning is a product/build decision.** The exact depth/length limits (and whether
  to match Excel's 64 / 8192) are for the build; D establishes only that a cap is
  *required* and roughly where the ceilings sit.
- **Ceilings are stack-size- and platform-dependent.** Numbers are the 4-core Linux
  container floor with default stacks; macOS/main-thread stacks differ. The *shape*
  (linear in stack, abort not panic) is the durable finding, not the exact depth.
- **`evaluate()` panic-freedom is empirical, not guaranteed.** The corpus is adversarial
  but finite; layers 2-3 exist precisely so a future IronCalc regression degrades to a
  recoverable error rather than a crash. A version bump should re-run this probe.
- **Timeout-based hang defense not needed today** (cycles are fast), but if a future
  IronCalc could genuinely spin, the SP1 seam has no eval cancellation (`evaluate()` is
  non-interruptible) — the fallback would again be the isolated-eval-subprocess route.
