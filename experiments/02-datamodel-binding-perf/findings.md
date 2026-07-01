# Sub-project C — Datamodel Binding & Engine Performance (Formualizer vs IronCalc)

> Status: **complete**. Two-engine bake-off (functional_spec §6.C, architecture §5,
> §1.1). Both engines run the **same** scenarios (built on the frozen
> `experiments/shared/datagen`) and the **same** metrics
> (`experiments/shared/bench_util`) through one shared driver
> (`common::run_suite`), so the numbers are directly comparable. Structure follows
> functional_spec §5.2.

**Environment (stamped on every recorded JSON).** Linux x86_64, **4 logical cores**,
~15 GB RAM, **no GPU/display**, Rust 1.94.1. CPU: `Intel(R) Xeon(R) Processor @
2.80GHz`. Engines: **Formualizer 0.7.0**, **IronCalc / ironcalc_base 0.7.1**. Commit
stamp `783a515`, date `2026-07-01`. All perf numbers below are from the recorded
`full`-profile run; reproduce with the commands in *What was done*.

---

## Questions

From functional_spec §6.C, the core technical risk — how the engine↔UI binding drives
perf/scale, evaluated on **both** candidate engines:

1. **Writes.** Is `set_value` as cheap as it looks? What does batching / recalc
   triggering actually cost on each engine?
2. **Reads / binding.** How do we pull a whole viewport's values as we scroll fast, and
   refresh it as data changes? Per-cell vs range APIs? Caching + invalidation? Which of
   three binding designs (D1 naive / D2 bulk / D3 cached+changelog) wins, and why?
3. **Arrow / storage model.** How does each engine's backing store (Formualizer's Arrow
   columnar lanes vs IronCalc's nested `HashMap`) affect access patterns and the ideal
   binding design?
4. **General engine perf.** Are cascades fast? The 1,000,000-cell `=PREV+1` chain
   (< 100 ms target), plus a wide fan-out shape.
5. **Memory.** Load an order-10⁷-cell workbook + edit — is RAM reasonable?
6. **Which engine leans ahead on a pure perf lens** for the Phase-7 (Sub-project G)
   engine decision?

---

## What was done

### Harness (three isolated Cargo projects, no shared workspace — architecture §1)

- **`common/`** (`binding_common`) — engine-neutral. Holds the `SpreadsheetEngine`
  trait (`engine.rs`, the "binding surface"), the three binding **designs**
  (`binding.rs`: D1/D2/D3 + `BindingCache`), the five **scenarios** (`scenario.rs`,
  parameterised by a `Profile` with `dev`/`full` sizes), the shared **driver**
  (`runner.rs::run_suite`), and the **results writer** (`report.rs` → per-run JSON +
  `summary.md`). A tiny in-crate `FakeEngine` lets `common`'s own 25 tests exercise
  everything without a real engine. This crate never depends on an engine.
- **`formualizer/`** (`formualizer_bench`) — `FormualizerEngine: impl
  SpreadsheetEngine` over `formualizer::Workbook`; `read_viewport` uses the native
  columnar `read_range`; `set_batch` uses the deferred-dirty `write_range`; `recompute`
  uses `evaluate_all`; `drain_dirty` reads the append-only `ChangeLog`. Plus a
  `scenarios` bin and Criterion `benches/perf.rs`.
- **`ironcalc/`** (`ironcalc_bench`) — `IronCalcEngine: impl SpreadsheetEngine` over
  `ironcalc_base::Model`; writes via `set_user_input` (deferred); `recompute` via the
  **full-workbook** `Model::evaluate()`; `read_viewport` loops per-cell (no range read);
  `drain_dirty` mirrors edited addresses. Plus `tests/smoke.rs` (an API-surface capture
  that regression-locks *no incremental recalc*, *no range read*, *styles present on
  read*), a `scenarios` bin, and a mirrored Criterion bench.

Both engine bins call the **same** `run_suite` with a factory closure, so the driving
logic is byte-for-byte identical across engines.

### Methodology fixes applied this phase (the credibility work)

The prior harness produced non-credible numbers and hung at scale. Root causes found
and fixed:

1. **Build/load is timed separately from the measured op.** Every cascade scenario
   builds its shape once via the engine's **batched** path (`write_range` /
   `set_user_input`-then-single-`evaluate`), records **build time as its own metric**
   (`build_ms` in each JSON), and times **only** the edit→recompute (→read) cycle.
2. **The measured op is forced and asserted.** For the 1M cascade we edit the head, run
   a full recompute, read the tail, and **assert `tail == head + (len-1)`**
   (`recompute_verified: true` in the JSON). A no-op or cached read cannot satisfy this,
   so a recorded number is provably a genuine recompute. The scrolling scenario asserts
   the far corner of the region is non-empty (can't be "fast" on an empty grid); the
   memory scenario spot-checks the last-loaded cell.
3. **Bounded work / honest step-down.** The 1M chain is kept at spec scale but timed
   over 5 reps (a full recompute is deterministic). The memory load targets 10⁷ cells
   but is **wall-clock-budgeted (90 s)**: an engine that can't reach the target
   **steps down and records the ceiling it did reach** rather than hanging. Fan-out is
   a 1,000×1,000 shape (a 5,000×5,000 shape costs ~100 s/recompute on Formualizer —
   recorded as a ceiling finding, below). Every run completes in the foreground in
   minutes.

### Reproduce

```sh
# from experiments/02-datamodel-binding-perf/
( cd common      && cargo test )                                  # 25 harness tests
( cd formualizer && cargo test && \
  cargo run --release --bin scenarios -- full 783a515 )           # ~3 min
( cd ironcalc    && cargo test && \
  cargo run --release --bin scenarios -- full 783a515 )           # ~30 s
# → writes results/{formualizer,ironcalc}/*.json + results/summary.md
# Micro-benches (optional): ( cd <engine> && cargo bench )
```

Inputs come only from `shared/datagen` (`linear_chain`, `wide_fanout`,
`SyntheticSheet`); metrics only from `shared/bench_util` (`LatencyStats`, `GateResult`,
`Environment`). A `dev` profile (`-- dev`) runs the identical code at tiny sizes for a
fast smoke.

---

## Results / evidence

All from the recorded `full` run (JSON under `results/`, summarised in
`results/summary.md`). "PASS/FAIL" is against the functional_spec §5.4 target.

### Headline comparison table

| Scenario / metric | **Formualizer 0.7.0** | **IronCalc 0.7.1** | §5.4 target | Verdict |
|---|---|---|---|---|
| **Scrolling viewport read** — D2 (bulk) p99 | **452 µs** | **326 µs** | < 2 ms | ✅ both PASS |
| &nbsp;&nbsp;D1 (per-cell) p99 | 481 µs | 391 µs | < 2 ms | ✅ both PASS |
| &nbsp;&nbsp;D3 (cached, warm) p99 | 705 µs | 562 µs | < 2 ms | ✅ both PASS |
| **Cascade → visible** (100k chain) D2 p50 / p99 | 135 ms / 188 ms | 107 ms / 115 ms | ≤ 16.6 ms | ❌ both FAIL |
| **1M `=PREV+1` cascade recompute** p50 / p99 | 1.77 s / 3.20 s | 2.02 s / 2.12 s | < 100 ms | ❌ both FAIL |
| &nbsp;&nbsp;— 1M chain **build** (batched) | 25.2 s | **5.9 s** | (discovery) | IronCalc 4.3× faster |
| **Wide fan-out** 1,000×1,000 recompute p50 | 3.47 s | **83 ms** | (frame) | IronCalc **42× faster** |
| &nbsp;&nbsp;— fan-out **build** | 2.9 s | 89 ms | (discovery) | IronCalc 33× faster |
| **Single write** (mean, incl. per-edit recompute) | 197 µs | 33 µs | (discovery) | — |
| **Batched write** of 1,000 cells (total) | 200 ms | 539 µs | (discovery) | — |
| &nbsp;&nbsp;— single-total ÷ batched ratio | **0.98×** | **60.7×** | (discovery) | see *Writes* |
| **Memory load** — cells actually reached (90 s budget) | **4.2 M (capped)** | **10.0 M (6.6 s, not capped)** | ~10⁷ | IronCalc reaches target |
| &nbsp;&nbsp;— peak RSS at that load / per-M-cells | 3.88 GB / ~0.92 GB | 1.68 GB / **~0.17 GB** | sane multiple | IronCalc ~5× denser |

All cascade/memory numbers carry `recompute_verified: true` / `populated_verified:
true` in their JSON — the timed work is proven real.

### 1. Writes — is `set_value` cheap?

- **Formualizer.** A single `set_value` + incremental `recompute` averages **197 µs**.
  But the batched `write_range` of 1,000 cells took **200 ms total** — so the
  single-total-vs-batched ratio is **~0.98× (no batching win here)**. The reason is a
  **super-linear `write_range` cost** (see the ceiling finding below): the one big batch
  pays the same growing-sheet penalty the 1,000 small writes do. Batching still matters
  for *formula* batches (one propagation instead of N), but for bulk **literals**
  Formualizer's write path is the bottleneck, not the recompute.
- **IronCalc.** A single `set_value` + full `evaluate()` averages **33 µs** on a small
  sheet, but the batched path is **60.7× cheaper** than doing the writes one-by-one —
  because every un-batched `set_user_input` that auto-evaluates pays a **whole-workbook
  recompute**. *The binding layer MUST batch edits on IronCalc*; a naive "evaluate per
  keystroke" loop is quadratic. IronCalc's raw write (store a string, no graph) is very
  cheap; its cost is entirely in `evaluate()`.

**Finding:** "is `set_value` cheap?" has opposite answers. On Formualizer the *write*
(graph/store mutation) is the expense; on IronCalc the *recompute* is. Both push the
binding toward **batched edits with an explicit recompute**, never per-cell auto-eval.

### 2. Reads / binding — D1 vs D2 vs D3, and the viewport target

**Both engines hit the < 2 ms viewport-read target comfortably** (all p99 < 705 µs) for
an ~1,800-cell viewport, across all three designs. Nuances:

- **D2 (bulk) is the best read design on both.** On Formualizer it uses the native
  columnar `read_range` (its headline advantage); on IronCalc D2 is *emulated* by a
  per-cell loop inside the adapter (no native range read exists) — yet IronCalc's
  per-cell `HashMap` reads are fast enough that **D2 still wins and still clears 2 ms**.
  So the range-read advantage is real but **not decisive at viewport scale** — a
  viewport is only ~10³–10⁴ cells, small enough that per-cell reads are fine.
- **D1 (naive per-cell)** is within tens of µs of D2 on both engines at this size.
- **D3 (cached + changelog) is the *slowest* on a cold pan** (Formualizer 705 µs,
  IronCalc 562 µs p99) because priming the cache = one bulk read **plus** the HashMap
  populate; it only pays off when reads repeat against an unchanged window. **Critically,
  D3 cannot beat D2 on a cascade** (Scenario 2): neither engine exposes a *downstream*
  dirty set — the change feed reports only **edit sites**, not the cells whose computed
  values changed — so after an offscreen edit that cascades into the window, D3 must
  conservatively **re-prime the whole window** (one bulk read), collapsing to D2 plus
  overhead. This is a key binding-design result (see *Recommended binding design*).

### 3. Storage-model effect (Arrow vs HashMap) — the surprise

The stack-decision doc (`00-stack-decision/findings.md`) hypothesised Formualizer's
**Arrow columnar** store would be the stronger huge-sheet perf/memory bet vs IronCalc's
nested `HashMap`. **The measured results invert that expectation** on this hardware:

- **Bulk write is super-linear on Formualizer.** `write_range` of a literal block scales
  ~quadratically with the populated sheet extent: 80k cells ≈ 2.2 s, 200k ≈ 11 s, 400k
  ≈ 40 s, and a single 2M-cell seed **did not complete in 120 s**. Chunking the writes
  did **not** help (400k chunked ≈ 40 s), so the cost is tied to the **growing sheet**,
  not the batch size — consistent with delta-overlay/compaction work on every write into
  a larger store. IronCalc seeds the same 2M block in **1.8 s** and 10M in **6.6 s**.
- **Memory density favours IronCalc here.** IronCalc held 10M literal cells in
  **1.68 GB** (~0.17 GB/M); Formualizer held 4.2M in 3.88 GB (~0.92 GB/M — ~5× heavier).
  For a bench of dense `f64` literals the Arrow lanes did **not** materialise the
  expected density advantage in 0.7.0 (overlay overhead dominates before compaction).
- **Range read** is the one place Arrow shows its intended shape (native `read_range`),
  but as noted it isn't decisive at viewport scale.

This is the single most important perf-lens finding of the phase and directly
contradicts an assumption Sub-project G would otherwise inherit.

### 4. Cascades — neither engine hits the 1M target

- **1M `=PREV+1` chain recompute: both FAIL < 100 ms by ~18–32×.** Formualizer p50
  **1.77 s** (p99 3.20 s); IronCalc p50 **2.02 s** (p99 2.12 s, notably tighter). This
  is inherent to a **1M-deep *serial* dependency chain**: every correct engine must
  touch all 10⁶ cells in order, and the chain is unparallelisable, so Formualizer's
  parallel-eval config can't help. **Build** diverges sharply (Formualizer 25.2 s vs
  IronCalc 5.9 s) for the same super-linear-write reason as §3.
- **Wide fan-out (1,000×1,000): both FAIL the frame budget, but 42× apart.**
  Formualizer p50 **3.47 s** vs IronCalc **83 ms**. Formualizer's `=SUM(range)`
  recompute over 1,000 dependents × 1,000 sources is far more expensive than IronCalc's
  here (likely range-view materialisation per SUM). This shape is a *discovery* metric,
  but the gap is large and consistent with §3.

**Takeaway:** a 1M-deep instant cascade is **not achievable on either engine today** in
this environment; the credible product path is **not** "recompute 1M synchronously in a
frame" but **async/off-thread recompute with a progress state**, or avoiding
million-deep serial chains. This is the biggest honest gap vs §5.4 and belongs in the
Round-2 list.

### 5. Memory

Covered in §3. IronCalc reached the **10⁷-cell target** (6.6 s, **1.68 GB** peak RSS —
a sane multiple). Formualizer **capped at 4.2 M cells** within the 90 s budget at
3.88 GB; extrapolating its super-linear write curve, 10⁷ would need many minutes and
~9 GB — **impractical in this environment with its current bulk-load path**. Recorded
honestly as a ceiling, not a hang.

### Ceilings that had to be capped (honest step-downs)

| Thing | Ceiling reached in-budget | Why capped |
|---|---|---|
| Formualizer bulk literal load | ~4.2 M cells @ 90 s (3.88 GB) | super-linear `write_range` (O(sheet extent)) |
| Formualizer single 2M seed | did not finish in 120 s | same |
| Fan-out shape | 1,000×1,000 (not 5,000×5,000) | 5k² ≈ 96 s build + 107 s recompute on Formualizer |
| 1M chain reps | 5 reps (deterministic recompute) | each rep is a real 1.8–3.2 s recompute |

IronCalc hit every target scale in-budget (10M memory, 1M chain, 1k² fan-out).

---

## Conclusion (direct answers)

- **Viewport reads: solved on both.** The < 2 ms newly-visible-cell target is met with
  wide margin (p99 ≤ 705 µs) on Formualizer *and* IronCalc, with **D2 bulk-range** the
  best design. IronCalc meets it **without** a native range API. The reading half of the
  binding is not a risk on either engine.
- **1M-cell instant cascade: not met on either engine** (1.77 s / 2.02 s vs 100 ms). A
  serial million-deep chain is inherently linear-and-serial; neither engine's design
  changes that. FreeCell needs an **async-recompute** UX story regardless of engine — a
  real, evidenced gap vs §5.4.
- **Storage-model expectation inverted.** On this hardware IronCalc **out-performs**
  Formualizer on bulk build (4–5×), fan-out recompute (42×), and memory density (~5×),
  and reaches the 10⁷-cell scale Formualizer could not. Formualizer's Arrow-columnar
  promise did **not** translate into a huge-sheet perf/memory win in 0.7.0; its
  `write_range` bulk-load path is super-linear.
- **Writes: batch, don't per-cell-eval.** On IronCalc batching is 60.7× cheaper (avoids
  per-edit full recompute); on Formualizer the write path itself is the cost. Both point
  to the same binding rule.
- **What we could not determine here.** (a) UI-side render/frame budget — not measurable
  in this headless container (Sub-project E, macOS/Metal). (b) Whether Formualizer's
  super-linear write is fixable via a different ingest API (e.g. a columnar bulk builder
  not surfaced in 0.7.0's public `write_range`) — we used the documented batched API;
  an upstream bulk-ingest path *might* change the picture and is flagged for G. (c) Async
  parallel recompute of *non-serial* graphs — our shapes were serial-chain and
  SUM-fanout; a DAG with wide independent layers could exploit Formualizer's parallel
  eval (untested).

---

## API suitability for the binding layer

| Capability the binding needs | Formualizer 0.7.0 | IronCalc 0.7.1 |
|---|---|---|
| Native bulk/range **read** | ✅ `read_range` (columnar) | ❌ per-cell loop only |
| Incremental / dirty **recalc** | ✅ (evaluate_all recomputes dirty set) | ❌ full-workbook `evaluate()` only |
| Parallel eval | ✅ `EvalConfig.enable_parallel` | ❌ single-threaded |
| Change feed for a cache | ✅ append-only `ChangeLog` (poll) | ⚠️ `UserModel` diff-list (collab-sync, poll) |
| **Downstream** dirty set (cascaded cells) | ❌ edit-sites only | ❌ edit-sites only |
| Styles on the **read** path | ❌ 0.7 hard-codes `style: None` | ✅ `get_style_for_cell` |
| Bulk **write** at scale | ⚠️ super-linear `write_range` | ✅ cheap `set_user_input` |
| Recompute at scale | ⚠️ 1M serial chain 1.8 s | ⚠️ full re-eval; 1M 2.0 s, fan-out fast |

**Net API read:** IronCalc's surface is *thinner* (no range read, no incremental recalc,
no parallelism) but every gap is **cheaply emulable in the binding** and its primitives
are fast; plus it gives us **styles on read** for free (relevant to Sub-project D).
Formualizer's surface is *richer* (range read, incremental, parallel, changelog) — the
better-shaped API on paper — but its bulk-write scaling undercuts the huge-sheet thesis
it was chosen for, and it hides styles on read.

## Missing / needed features (both engines)

1. **A downstream-dirty subscription** — "which *computed* values changed after this
   edit," not just edit sites. Its absence is why D3 collapses to D2 on a cascade. Both
   engines would need this to make a cached binding beat a bulk re-read on change.
2. **Async / cancellable recompute** to keep the UI responsive during a multi-second
   large-graph recalc (Formualizer has `evaluate_cells_cancellable`; IronCalc has
   nothing — it would need an off-thread `evaluate()` on a snapshot).
3. **Formualizer:** a **linear (or better) bulk-ingest** path for large literal loads,
   and **styles surfaced on read** (currently a Sub-project D workaround).
4. **IronCalc:** a **native range read** and **any incremental recalc** would remove the
   two biggest binding work-arounds and its per-edit-recompute cliff.

---

## Recommended binding design

**Recommend D2 (bulk/range read) as the default, with a thin D3 cache layered on top for
the steady-state (no-edit) scroll — engine-agnostic.**

- **D2 is the measured winner for reads** on both engines and needs no engine-specific
  cleverness: pull the visible rectangle (+ overscan) in one call per viewport change.
  On Formualizer that's the native `read_range`; on IronCalc it's a per-cell loop the
  adapter hides — **both clear 2 ms**.
- **Layer D3 only as a steady-state optimisation:** keep the last viewport's values in a
  `BindingCache`; serve repeat reads from it while nothing changes. **On any edit,
  re-prime the visible window with a D2 bulk read** (do *not* trust an edit-site dirty
  set to cover cascades — neither engine reports downstream cells). This is exactly what
  `BindingCache::refresh_after_edits` does.
- **Always batch writes** and drive recompute explicitly (never per-keystroke
  auto-eval) — mandatory on IronCalc (60.7× penalty), good hygiene on Formualizer.
- **Move large recomputes off the UI thread.** Since a big cascade is 1–3 s on *either*
  engine, the binding must run recompute async and paint a "recalculating" state, then
  re-prime the viewport when it completes. This is the real design consequence of the
  cascade FAIL, and it is **engine-independent**.

**Next-best alternative:** pure **D2 with no cache** (simpler; the cache's win is
marginal because reads are already < 1 ms and every edit invalidates it anyway). If a
future engine version exposes a **downstream-dirty subscription**, revisit a *true* D3
that refreshes only changed visible cells — that's the only thing that would make caching
clearly beat plain bulk re-reads on a cascade.

---

## Perf-lens engine lean (input to Sub-project G)

**On a pure performance-and-scale lens, this phase leans IronCalc — the opposite of the
pre-bake-off expectation.** Evidence:

- **Reaches the scales that matter.** 10⁷-cell load in 6.6 s at 1.68 GB; Formualizer
  capped at 4.2 M and ~3.9 GB in the same budget.
- **Faster where it's not already instant.** 4–5× faster bulk build, **42× faster**
  fan-out recompute, ~5× denser memory. Tied on the (both-failing) 1M serial chain
  (2.0 s vs 1.8 s).
- **Reads meet target without a range API** — the one architectural edge Formualizer
  keeps (native `read_range`) is not decisive at viewport scale.
- **Bonus: styles on read** help Sub-project D.

This lean is **perf-only and must not be read as the engine decision.** It deliberately
excludes the factors Sub-project G must weigh and A/B/D own: **function coverage** (both
~300+, unmeasured here), **file fidelity** (Sub-project B), **formatting model**
(Sub-project D), and **maturity/bus-factor/license** — where the stack doc rated
IronCalc *higher* (funded team, 29 contributors, ~4k stars) than the effectively
single-author Formualizer. So on perf **and** maturity the arrows now point the *same*
way (IronCalc), which is a meaningful shift for G. Two caveats keep it from being
decisive: (1) Formualizer's super-linear write **might** be an artifact of the public
`write_range` API rather than the core store — an upstream bulk-ingest path could change
the memory/build story; G should probe this before finalising. (2) Formualizer's
richer, better-shaped **API** (incremental recalc, parallel eval, changelog) is the
better long-term substrate *if* the write scaling is fixed.

**Recommendation to G:** treat the "Arrow ⇒ stupid-fast huge sheets" premise as
**not yet demonstrated** for Formualizer 0.7.0, and IronCalc as the current perf **and**
maturity front-runner — pending G's checks on function coverage, fidelity (B), and
whether Formualizer has an unshipped/undocumented bulk-ingest path.

---

## Risks / open questions (carried forward)

- **1M instant cascade unachievable on either engine (1.8–2.0 s).** FreeCell needs an
  **async-recompute UX**; "synchronous 1M recalc in a frame" is off the table.
  → Round-2 + product design.
- **Formualizer bulk-write is super-linear** (O(sheet extent)); can't reach 10⁷ cells
  in-budget via `write_range`. Is there a linear bulk-ingest path not surfaced in 0.7.0?
  → probe upstream in Sub-project G before finalising the engine choice.
- **No downstream-dirty subscription on either engine** → a cached binding (D3) can't
  beat bulk re-read (D2) on a cascade; a *true* incremental-visible refresh needs an
  engine feature neither ships.
- **Formualizer hides styles on read** (0.7 `style: None`); IronCalc exposes them. Owned
  by Sub-project D, but it also nudges the engine lean.
- **UI-side budget unmeasured here** (headless, no GPU). The 120 fps / < 8.3 ms render
  target and real cell-load-under-scroll are Sub-project E (macOS/Metal).
- **Parallel recompute of non-serial DAGs untested** — our shapes were serial-chain and
  SUM-fanout. Formualizer's parallel eval could help a wide independent-layer graph;
  IronCalc has no parallel path. Worth a Round-2 shape.
- **Coverage/fidelity not measured here** — the perf lean must be combined with B
  (file), D (formatting), and function-coverage counts in G, not read in isolation.
