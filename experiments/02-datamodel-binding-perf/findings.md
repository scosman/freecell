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

> **Fairness correction (this revision).** An earlier draft measured Formualizer's
> **literal bulk-load** (memory scenario + scrolling/cascade seeds) through its slow
> *interactive* `write_range` overlay path — which builds a graph vertex per cell and
> re-compacts whole 32K-row Arrow chunks on every write (super-linear) — while IronCalc
> was loaded near-optimally. That made Formualizer look like it "couldn't reach 10⁷
> cells" and was "~5× less memory-dense," which were **artifacts of the wrong API, not
> the engine.** This revision loads literals through Formualizer's **columnar Arrow
> bulk-ingest** path (`engine_mut().begin_bulk_ingest_arrow()` →
> `ArrowBulkIngestBuilder::{add_sheet, append_row, finish}`, verified in
> `formualizer-eval-0.7.0/src/engine/arrow_ingest.rs`; installs `ArrowSheet`s directly,
> no vertices/overlay/rebuilds) and loads IronCalc through its direct `set_user_input`
> path — **each engine's fastest native loader** — via a new
> `SpreadsheetEngine::bulk_load_block` trait method. Peak memory is now taken from a
> **fresh process per engine** (`scenarios -- mem`) so whole-process `VmHWM` isn't
> polluted by earlier scenarios. The corrected numbers **reverse** the build/memory/scale
> conclusions and the perf-lens lean. What did **not** change (CR-confirmed
> trustworthy): viewport reads, the 1M serial-cascade recompute (both ~2 s, FAIL), the
> fan-out recompute, writes, and IronCalc's batching penalty.

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
   columnar lanes vs IronCalc's nested `HashMap`) affect access patterns, load cost, and
   memory density?
4. **General engine perf.** Are cascades fast? The 1,000,000-cell `=PREV+1` chain
   (< 100 ms target), plus a wide fan-out shape.
5. **Memory.** Load an order-10⁷-cell workbook + edit — is RAM reasonable?
6. **Which engine leans ahead on a pure perf lens** for the Phase-7 (Sub-project G)
   engine decision?

---

## What was done

### Harness (three isolated Cargo projects, no shared workspace — architecture §1)

- **`common/`** (`binding_common`) — engine-neutral. Holds the `SpreadsheetEngine`
  trait (`engine.rs`, the "binding surface" — including `bulk_load_block`, the fair
  native-loader hook), the three binding **designs** (`binding.rs`: D1/D2/D3 +
  `BindingCache`), the five **scenarios** (`scenario.rs`, `Profile` `dev`/`full` sizes),
  the shared **driver** (`runner.rs`: `run_suite` + `run_memory_only`), and the
  **results writer** (`report.rs` → per-run JSON + `summary.md`). A tiny in-crate
  `FakeEngine` lets `common`'s own 25 tests exercise everything without a real engine.
- **`formualizer/`** (`formualizer_bench`) — `FormualizerEngine` over
  `formualizer::Workbook`; `read_viewport` uses native columnar `read_range`;
  **`bulk_load_block` uses the Arrow bulk-ingest path**; `set_batch` uses `write_range`
  (the interactive path, used only for *formula* batches and small edits now);
  `recompute` uses `evaluate_all`; `drain_dirty` reads the `ChangeLog`.
- **`ironcalc/`** (`ironcalc_bench`) — `IronCalcEngine` over `ironcalc_base::Model`;
  `bulk_load_block` and writes use `set_user_input` (literals need no `evaluate`);
  `recompute` is the full-workbook `Model::evaluate()`; `read_viewport` loops per-cell
  (no range read). Plus `tests/smoke.rs` (regression-locks *no incremental recalc*, *no
  range read*, *styles present on read*).

Both engine bins call the **same** `run_suite`, so driving logic is identical.

### Methodology (credibility + fairness)

1. **Build/load is timed separately from the measured op.** Cascade scenarios build the
   shape once and record **build time** as its own metric (`build_ms`); only the
   edit→recompute (→read) cycle is timed.
2. **The measured op is forced and asserted.** The 1M cascade edits the head, recomputes,
   reads the tail, and **asserts `tail == head + (len-1)`** (`recompute_verified: true`).
   The memory scenario spot-checks the last-loaded far-corner cell; the scrolling
   scenario asserts the region's far corner is non-empty. Bulk-ingested values were
   independently verified to read back correctly (head/mid/far and via `read_range`).
3. **Fair native loaders.** Large literal loads go through `bulk_load_block` — the
   engine's optimal ingest — so build/memory compare like-for-like (this is the fix).
4. **Clean peak memory.** The authoritative peak-RSS number comes from a **fresh process
   per engine** (`scenarios -- mem`); the in-suite run additionally reports the
   load-attributable `delta_bytes`.
5. **Bounded work.** 1M chain timed over 5 deterministic reps; fan-out is 1,000×1,000 (a
   5,000×5,000 shape is ~100 s/recompute on Formualizer — a recorded ceiling). Every run
   finishes in the foreground in ≤ ~90 s per engine.

### Reproduce

```sh
# from experiments/02-datamodel-binding-perf/
( cd common      && cargo test )                                       # 25 harness tests
( cd formualizer && cargo test && \
  cargo run --release --bin scenarios -- full 783a515 && \
  cargo run --release --bin scenarios -- mem  783a515 )                # full ~75 s + mem ~2 s
( cd ironcalc    && cargo test && \
  cargo run --release --bin scenarios -- full 783a515 && \
  cargo run --release --bin scenarios -- mem  783a515 )                # full ~32 s + mem ~7 s
# → results/{formualizer,ironcalc}/*.json + results/summary.md
```

Run `mem` **separately** (its own process) so `VmHWM` reflects only that load. Inputs
come only from `shared/datagen`; metrics only from `shared/bench_util`.

---

## Results / evidence

All from the recorded `full` run (JSON under `results/`, summarised in
`results/summary.md`); memory is the fresh-process figure. "PASS/FAIL" is vs §5.4.

### Headline comparison table

| Scenario / metric | **Formualizer 0.7.0** | **IronCalc 0.7.1** | §5.4 target | Verdict |
|---|---|---|---|---|
| **Scrolling viewport read** — D2 (bulk) p99 | **222 µs** | **392 µs** | < 2 ms | ✅ both PASS |
| &nbsp;&nbsp;D1 (per-cell) p99 | 310 µs | 419 µs | < 2 ms | ✅ both PASS |
| &nbsp;&nbsp;D3 (cached, warm) p99 | 434 µs | 585 µs | < 2 ms | ✅ both PASS |
| **Cascade → visible** (100k chain) D2 p50 / p99 | 138 ms / 199 ms | 107 ms / 109 ms | ≤ 16.6 ms | ❌ both FAIL |
| **1M `=PREV+1` cascade recompute** p50 / p99 | 1.87 s / 3.24 s | 2.11 s / 2.15 s | < 100 ms | ❌ both FAIL |
| **Wide fan-out** 1,000×1,000 recompute p50 | 3.51 s | **77.5 ms** | (frame) | IronCalc **45× faster** |
| **Memory: 10M-cell literal load** (native bulk-ingest) | **1.73 s** | 6.13 s | ~10⁷ | **both reach 10⁷** |
| &nbsp;&nbsp;— peak RSS (fresh process) | **0.18 GB** | 1.63 GB | sane multiple | **Formualizer ~9× denser** |
| &nbsp;&nbsp;— bytes/cell (f64 literals) | **~17 B** | ~162 B | — | Arrow lanes win |
| **1M chain build** (formula-graph construction) | 27.3 s | **6.15 s** | (discovery) | IronCalc 4.4× faster |
| **Fan-out 1,000×1,000 build** | 3.41 s | **0.085 s** | (discovery) | IronCalc 40× faster |
| **Single write** (mean, incl. per-edit recompute) | 149 µs | 32 µs | (discovery) | — |
| **Batched write** of 1,000 cells (total) | 191 ms | 534 µs | (discovery) | — |
| &nbsp;&nbsp;— single-total ÷ batched ratio | 0.84× | **60.5×** | (discovery) | see *Writes* |

All cascade/memory numbers carry `recompute_verified: true` / `populated_verified:
true` in their JSON — the timed work is proven real.

### 1. Writes — is `set_value` cheap?

- **Formualizer.** Single `set_value` + incremental `recompute` averages **149 µs**;
  batching 1,000 literals through `write_range` totals **191 ms**, so single-vs-batched
  ≈ **0.84×** (no batching win on this interactive path — it is the same super-linear
  overlay path, which is exactly why bulk *loads* must use Arrow ingest, not `write_range`).
- **IronCalc.** Single `set_value` + full `evaluate()` averages **32 µs**, but batching
  is **60.5× cheaper** — every un-batched `set_user_input` that auto-evaluates pays a
  whole-workbook recompute. *The binding MUST batch edits on IronCalc.*

**Finding:** both engines push the binding toward **batched edits with an explicit
recompute**, never per-cell auto-eval. On IronCalc the penalty is the recompute; on
Formualizer the interactive `write_range` path itself is the cost (dodged for bulk loads
by the Arrow ingest path).

### 2. Reads / binding — D1 vs D2 vs D3, and the viewport target

**Both engines clear < 2 ms comfortably** (all p99 ≤ 585 µs) for an ~1,800-cell viewport,
across all designs.

- **D2 (bulk) is the best read design on both.** On Formualizer it uses the native
  columnar `read_range`; on IronCalc D2 is *emulated* by a per-cell loop in the adapter —
  yet still wins and still clears 2 ms. The native-range-read advantage is real but **not
  decisive at viewport scale** (~10³–10⁴ cells).
- **D1 (naive per-cell)** is within a couple hundred µs of D2 at this size.
- **D3 (cached) is slowest on a cold pan** (prime = bulk read + HashMap populate); it only
  helps on repeat reads against an unchanged window. **D3 cannot beat D2 on a cascade:**
  neither engine exposes a *downstream* dirty set (the change feed reports only edit
  sites), so after an offscreen edit that cascades in, D3 must re-prime the whole window
  (one bulk read) — collapsing to D2 + overhead.

### 3. Storage-model effect (Arrow vs HashMap) — Arrow's advantage confirmed

With each engine loaded through its **native** path, the stack-decision hypothesis
(`00-stack-decision/findings.md`) that Formualizer's **Arrow columnar** store is the
stronger huge-sheet load/memory bet is **confirmed**:

- **Bulk literal load: Formualizer wins ~3.5×.** 10M `f64` literals via Arrow bulk-ingest:
  **1.73 s** vs IronCalc's 6.13 s. Formualizer's ingest is **O(cells)** (validated
  linear: 1M→0.23 s, 10M→1.85 s, 20M→3.42 s single-shot).
- **Memory density: Formualizer wins ~9×.** 10M cells fit in **0.18 GB** (~**17
  bytes/cell** — close to the ~9 bytes/cell f64 floor plus a per-row type tag and small
  overhead) vs IronCalc's **1.63 GB** (~162 bytes/cell — the boxed nested-`HashMap`
  `Cell` cost). At Excel-max densities this is the decisive difference the "stupid-fast on
  huge sheets" thesis was betting on.
- **The earlier "IronCalc out-scales / ~5× denser / Formualizer capped at 4.2M" result
  was an artifact** of routing Formualizer's load through the interactive `write_range`
  overlay (per-cell vertex + whole-chunk recompaction → super-linear: 400k ≈ 40 s, 2M
  did not finish in 120 s, ~0.9 GB/M). Using the correct API removes it entirely.
- **Native range read** (Formualizer `read_range`) is Arrow's other visible edge, though
  not decisive at viewport scale (§2).

### 4. Cascades — neither engine hits the 1M target (recompute unchanged, real)

- **1M `=PREV+1` chain recompute: both FAIL < 100 ms by ~19–21×.** Formualizer p50
  **1.87 s** (p99 3.24 s); IronCalc p50 **2.11 s** (p99 2.15 s, tighter). Inherent to a
  1M-deep **serial** chain — every engine must touch all 10⁶ cells in order; it is
  unparallelisable, so Formualizer's parallel eval can't help.
- **1M chain *build* diverges: 27.3 s (Formualizer) vs 6.15 s (IronCalc).** This is a
  **real, fair** gap and is *not* the literal-load artifact: it is the one-time cost of
  constructing a **1,000,000-vertex CSR dependency graph**. Decomposed on Formualizer
  (measured): staging the formula text ≈ 2.4 s; the first `evaluate_all` that builds +
  evaluates the graph ≈ 24 s. Routing formulas through the batched formula loader
  (`set_formulas`, which internally uses `begin_bulk_ingest`) only *shifts* the cost
  (staging 1.2 s, first-eval 20.6 s ≈ 21.8 s total) — there is **no bulk path that avoids
  building the 1M-node graph.** So ~20–27 s of graph construction is inherent to a 1M
  *formula* graph on this hardware (unlike the *literal* load, which the Arrow path makes
  instant). IronCalc builds its graph lazily/cheaper here but pays it back on every
  recompute (full re-eval).
- **Wide fan-out (1,000×1,000): both FAIL the frame budget, 45× apart.** Formualizer p50
  **3.51 s** vs IronCalc **77.5 ms**; build 3.41 s vs 0.085 s. Formualizer's
  `=SUM(range)` recompute/graph-build over 1,000 dependents × 1,000 sources is far more
  expensive here (range-view + SUM-graph cost). A discovery metric, but a large, real gap.

**Takeaway:** a 1M-deep instant cascade is **not achievable on either engine** in this
environment; the product path is **async/off-thread recompute with a progress state**, or
avoiding million-deep serial chains. Biggest honest gap vs §5.4; belongs in Round-2.

### 5. Memory

Both engines reach the **10⁷-cell target** via their native loader. **Formualizer:
1.73 s, 0.18 GB peak RSS** (fresh process; ~17 B/cell). **IronCalc: 6.13 s, 1.63 GB**
(~162 B/cell). Both are sane multiples of the raw data; Formualizer's Arrow lanes are
~9× denser. (Single-shot probes confirm Formualizer scales linearly to 20M cells /
0.32 GB, with headroom to spare in the 15 GB box.)

### Ceilings that had to be capped (honest step-downs)

| Thing | Ceiling in-budget | Why capped |
|---|---|---|
| Fan-out shape | 1,000×1,000 (not 5,000×5,000) | 5k² ≈ 96 s build + 107 s recompute on Formualizer |
| 1M chain reps | 5 reps (deterministic recompute) | each rep is a real 1.9–3.2 s recompute |

Both engines reached the 10⁷-cell memory target and the 1M chain in-budget on the fair
paths; no engine-level scale cap remains (unlike the earlier draft's Formualizer memory
cap, which was the `write_range` artifact).

---

## Conclusion (direct answers)

- **Viewport reads: solved on both.** < 2 ms met with wide margin (p99 ≤ 585 µs); **D2
  bulk-range** best. IronCalc meets it **without** a native range API. Not a risk.
- **1M-cell instant cascade: not met on either engine** (1.87 s / 2.11 s vs 100 ms). A
  serial million-deep chain is inherently linear-and-serial; FreeCell needs an
  **async-recompute** UX regardless of engine — a real, evidenced gap.
- **Huge-sheet load & memory: Arrow (Formualizer) wins on the fair path.** 10M literals
  in 1.73 s at 0.18 GB (~17 B/cell) vs IronCalc's 6.13 s / 1.63 GB (~162 B/cell). This
  **restores** the stack-decision expectation; the earlier inversion was a wrong-API
  artifact. The one caveat: you **must** use Formualizer's Arrow bulk-ingest, not its
  interactive `write_range`, for large literal loads (a hard binding-layer rule).
- **Formula-graph build is Formualizer's real cost** (~20–27 s for a 1M-node graph, vs
  ~6 s for IronCalc), and **no bulk path avoids it** — it's inherent graph construction,
  not an artifact. Relevant for open-time on formula-heavy workbooks.
- **Writes: batch, don't per-cell-eval** (60.5× on IronCalc; on Formualizer use Arrow
  ingest for bulk).
- **What we could not determine here.** (a) UI-side render/frame budget — headless (E,
  macOS/Metal). (b) Whether Formualizer's 1M-formula-graph build can be materially sped
  up by a config we didn't find (we tried `write_range` and `set_formulas`/deferred-graph;
  both ~20–27 s). (c) Parallel recompute of *non-serial* DAGs (our shapes were serial
  chain + SUM-fanout).

---

## API suitability for the binding layer

| Capability the binding needs | Formualizer 0.7.0 | IronCalc 0.7.1 |
|---|---|---|
| Native bulk/range **read** | ✅ `read_range` (columnar) | ❌ per-cell loop only |
| Native bulk **literal ingest** | ✅ `begin_bulk_ingest_arrow` (O(cells), ~17 B/cell) | ✅ `set_user_input` (fast, but ~162 B/cell) |
| Incremental / dirty **recalc** | ✅ (evaluate_all recomputes dirty set) | ❌ full-workbook `evaluate()` only |
| Parallel eval | ✅ `EvalConfig.enable_parallel` | ❌ single-threaded |
| Change feed for a cache | ✅ append-only `ChangeLog` (poll) | ⚠️ `UserModel` diff-list (collab-sync, poll) |
| **Downstream** dirty set (cascaded cells) | ❌ edit-sites only | ❌ edit-sites only |
| Styles on the **read** path | ❌ 0.7 hard-codes `style: None` | ✅ `get_style_for_cell` |
| Interactive bulk **write** (`write_range`) | ⚠️ super-linear — use Arrow ingest for loads | ✅ cheap per-cell writes |
| Recompute at scale | ⚠️ 1M serial chain ~1.9 s; fan-out slow | ⚠️ full re-eval; 1M ~2.1 s; fan-out fast |
| 1M-formula-graph build | ⚠️ ~20–27 s (inherent graph construction) | ✅ ~6 s |

**Net API read:** Formualizer's surface is richer *and* its Arrow ingest/columnar store
now demonstrably delivers the huge-sheet load/memory advantage — provided the binding
uses the **bulk-ingest** path (there are two ingest APIs, and the interactive one is a
trap). IronCalc's surface is thinner (no range read, no incremental recalc, no
parallelism) but its primitives are simple and it gives **styles on read** for free
(Sub-project D). Its weaker memory density and per-edit full-recompute are the costs.

## Missing / needed features (both engines)

1. **A downstream-dirty subscription** ("which *computed* values changed"), not just edit
   sites — the reason D3 can't beat D2 on a cascade.
2. **Async / cancellable recompute** for multi-second large-graph recalc (Formualizer has
   `evaluate_cells_cancellable`; IronCalc would need off-thread `evaluate()` on a snapshot).
3. **Formualizer:** faster 1M-formula-graph construction, and **styles surfaced on read**
   (Sub-project D workaround today). Its literal-ingest and memory density are already good.
4. **IronCalc:** a **native range read** and **any incremental recalc**; denser storage.

---

## Recommended binding design

**Recommend D2 (bulk/range read) as the default, with a thin D3 cache for the
steady-state (no-edit) scroll, and native bulk-ingest for large loads — engine-agnostic
where possible.**

- **D2 is the measured read winner** on both engines; pull the visible rectangle
  (+ overscan) in one call per viewport change (native `read_range` on Formualizer;
  per-cell loop hidden in the IronCalc adapter — both clear 2 ms).
- **Layer D3 only as a steady-state optimisation:** cache the last viewport; serve repeat
  reads from it; **on any edit, re-prime the window with a D2 bulk read** (don't trust an
  edit-site dirty set to cover cascades). This is `BindingCache::refresh_after_edits`.
- **Always batch writes**; **for large literal loads, use the engine's bulk-ingest**
  (Formualizer's Arrow ingest — never `write_range` — is the single most important
  Formualizer-specific rule; IronCalc's `set_user_input` loop is fine).
- **Move large recomputes off the UI thread.** A big cascade is 1–3 s on *either* engine;
  run recompute async, paint a "recalculating" state, re-prime the viewport on completion.

**Next-best alternative:** pure **D2 with no cache** (simpler; the cache's win is
marginal because reads are already < 1 ms and every edit invalidates it anyway). If a
future engine version exposes a **downstream-dirty subscription**, revisit a *true* D3
that refreshes only changed visible cells — that's the only thing that would make caching
clearly beat plain bulk re-reads on a cascade.

---

## Perf-lens engine lean (input to Sub-project G)

**On a corrected, fair perf-and-scale lens, the lean is now roughly even with a slight
edge to Formualizer on the huge-sheet axis that motivated the whole project — reversing
this doc's earlier (wrong-API) lean toward IronCalc.** Evidence:

- **Huge-sheet load & memory (the core thesis): Formualizer wins** — 10M literals in
  1.73 s at 0.18 GB (~17 B/cell) vs IronCalc's 6.13 s / 1.63 GB (~162 B/cell). At
  Excel-max scale the ~9× density and ~3.5× load speed are the decisive numbers, and they
  vindicate the Arrow-columnar bet.
- **Viewport reads:** both PASS; Formualizer's native `read_range` is a mild edge.
- **Formula-graph build: IronCalc wins** (~6 s vs ~20–27 s for a 1M-node graph) — matters
  for open-time on formula-dense sheets.
- **Recompute:** tied on the (both-failing) 1M serial chain (~1.9 s vs ~2.1 s); **IronCalc
  wins the fan-out by ~45×** (77 ms vs 3.5 s).
- **Bonus for IronCalc: styles on read** (Sub-project D); **bonus for Formualizer:**
  incremental recalc, parallel eval, changelog (richer binding substrate).

**Net:** on the dimension FreeCell was scoped around — *stupid-fast on huge sheets* —
**Formualizer's Arrow store is the better-evidenced bet** once loaded correctly, so the
premise is **now demonstrated** (the opposite of the earlier draft). IronCalc remains
competitive-to-better on formula-graph build and fan-out recompute, and stronger on
maturity/bus-factor/license per the stack doc (funded team, 29 contributors, ~4k stars
vs an effectively single-author crate). So perf now favours Formualizer on the headline
axis while maturity favours IronCalc — a genuine trade-off for G to weigh, **not** a
one-sided call.

**Recommendation to G:** treat the "Arrow ⇒ stupid-fast huge sheets" premise as
**demonstrated for load & memory** (10M cells, 1.73 s, 0.18 GB) — *conditional on using
Formualizer's bulk-ingest API, not `write_range`* — and weigh it against IronCalc's
maturity, faster formula-graph build, and fan-out recompute. The cascade-recompute gap
(both ~2 s vs 100 ms) is engine-independent and does not discriminate.

---

## Risks / open questions (carried forward)

- **1M instant cascade unachievable on either engine (~1.9–2.1 s).** FreeCell needs an
  **async-recompute UX**. → Round-2 + product design.
- **Formualizer has two ingest APIs; the interactive `write_range` is a super-linear
  trap** for large literal loads (per-cell vertex + chunk recompaction). The binding must
  use `begin_bulk_ingest_arrow` for bulk loads. Document this prominently for the team.
- **Formualizer 1M-formula-graph build is ~20–27 s** and no bulk path avoids it (inherent
  graph construction). Open-time cost on formula-dense workbooks; probe whether a future
  version parallelises graph build. → G / Round-2.
- **No downstream-dirty subscription on either engine** → D3 can't beat D2 on a cascade.
- **Formualizer hides styles on read** (0.7 `style: None`); IronCalc exposes them (D).
- **IronCalc storage is ~9× less dense** and has **no incremental recalc** (full re-eval
  per edit) — the huge-sheet costs to weigh against its maturity edge.
- **UI-side budget unmeasured here** (headless). 120 fps / < 8.3 ms render and real
  cell-load-under-scroll are Sub-project E (macOS/Metal).
- **Parallel recompute of non-serial DAGs untested** — Formualizer's parallel eval could
  help a wide independent-layer graph; IronCalc has none. Round-2 shape.
- **Coverage/fidelity not measured here** — combine this perf lean with B (file), D
  (formatting), and function-coverage counts in G, not in isolation.
