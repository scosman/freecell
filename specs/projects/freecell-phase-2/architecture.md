---
status: complete
---

# Architecture: FreeCell — Phase 2 (Round-2 Technical De-risking)

Like Phase 1, this is a **research/de-risking** effort, so the "architecture" is:
the experiment workspace + reuse strategy, the **agent-swarm orchestration**, the
**measurement methodology**, and — because it's the load-bearing hard problem — the
**async-recompute design (SP1) worked out in full**. Remaining per-sub-project detail
(exact bench parameters, case tables) is deferred to **phase plans** (§10), matching
Phase-1's approach. Read Phase-1 `architecture.md` first; this doc only adds/changes.

## 0. Grounding facts (inherited + Phase-2-relevant)
- Rust 1.94+; container **4 cores / ~15 GB RAM, no GPU / no display**; crates.io
  works. GitHub scoped to `scosman/freecell`.
- **Engine = IronCalc 0.7.x** (`ironcalc` / `ironcalc_base`). Pin the **same version
  Phase 1 used** so numbers stay comparable. Known shape: nested-`HashMap` storage
  (~162 B/cell); full-workbook `evaluate()` (no incremental recalc); `UserModel`
  exposes an edit **diff-list** (edit-sites only); native styled `.xlsx` I/O;
  persists cached results (values paint before recompute); no CSV / merges / CF API.
- **`Model` threading is an OPEN empirical question** (SP1's top risk): whether
  `Model` is `Send` and what a clone costs are *measured in SP1*, not assumed here.
  The §4 design gives a primary path (assumes `Send` + affordable clone) **and** a
  fallback (value-projection snapshot) so the sub-project can proceed either way.
- **GPUI still cannot build in-container** → SP6 is macOS-only, human-run. Everything
  else is authoritative in-container.

## 1. Repository Layout & Reuse Strategy

Round-2 work lives under **`experiments/round-2/`**. Phase-1 folders are **frozen**
(read-only); Round-2 never edits them.

```
experiments/
  shared/                         # Phase-1, FROZEN, read-only (datagen, bench_util)
  02-datamodel-binding-perf/      # Phase-1, FROZEN — source of the harness we copy
  03-formatting/                  # Phase-1, FROZEN — source SP5 extends (by copy)
  04-ui-poc/                      # Phase-1, FROZEN — SP6 evolves raw-gpui (by copy)
  round-2/
    harness/                      # NEW shared crate — created + FROZEN at scaffolding
                                  #   = verbatim copy of 02/common (SpreadsheetEngine
                                  #     trait + scenarios) + 02/ironcalc (IronCalc
                                  #     adapter), as a LIB crate. Read-only downstream.
    01-recompute-async/           # SP1  (independent Cargo project)
    02-xlsx-open/                 # SP2
    03-function-parity/           # SP3
    04-binding-style/             # SP4
    05-style-fidelity/            # SP5
    06-gpui-grid/                 # SP6  (macOS; evolves 04-ui-poc/raw-gpui)
    07-residual/                  # SP7
    SYNTHESIS.md                  # Phase-2 synthesis → Stage 3
```

**Why a `round-2/harness/` copy instead of depending into `02/`:** Phase-1 crates
are frozen; the IronCalc adapter there is a *bench* crate, not necessarily a clean
lib. Copying the trait + adapter + scenarios **verbatim** into one `harness/` lib
crate (pinned to the same IronCalc version) gives Round-2 a single, stable,
**read-only** engine seam that keeps numbers comparable to Phase-1 without mutating
frozen code. `harness/` is created and frozen at scaffolding (Phase-1's `shared/`
pattern); if a sub-project needs a change to it, it **escalates**.

Each Round-2 sub-project is an **independent Cargo project** (NOT a workspace — same
parallel-editor isolation rationale as Phase-1 `architecture.md` §1) depending by
**relative path, read-only** on `../harness` and `../../shared/*`. `target/` is
gitignored repo-wide. Repeated Arrow/IronCalc compiles are accepted (as in Phase 1).

## 2. Agent-Swarm Orchestration

Same structure as Phase-1 `architecture.md` §2: a **coordinator** spawns, per phase,
a **manager → coding sub-agent → CR sub-agent(s)**, running the attestation → CR →
commit loop; the manager never writes code itself. Parallel phases run concurrently.

### 2.1 Topology
```
Phase-2 Coordinator
├─ Phase 2.0: Scaffolding (serial) ── create round-2/ + round-2/harness/ (frozen)
├─ Engine-risk cohort (PARALLEL):     SP1, SP2, SP3
│     └─►  OFF-RAMP CHECKPOINT ── human review of SP1–SP3 vs the overview §2 off-ramp
├─ Build-out cohort (PARALLEL):       SP4, SP5, SP7
├─ SP6 (macOS, human-run) ── may run in parallel throughout; measured numbers gated on human
└─ Phase-2 Synthesis (serial; last) ── SYNTHESIS.md
```

### 2.2 Parallel-editor isolation (REQUIRED — inject verbatim into every parallel agent)
Reuse Phase-1 `architecture.md` §2.2 **exactly**, with the folder token set to each
agent's `experiments/round-2/NN-<name>/`. Recap: operate only inside your folder
(plus read-only `round-2/harness/`, `shared/`, `specs/`); **never** edit the repo
root, another sub-project, or a frozen crate; git-scope every command to your folder
(`git add experiments/round-2/NN-<name>/`; **never** `git add -A`/`.`/`commit -a`);
CR + attestation cover only your folder's diff; if you must touch a shared/frozen
file, **stop and escalate**.

### 2.3 Commit safety
Serialized, path-scoped commits (Phase-1 §2.3): the coordinator admits one phase's
commit at a time; disjoint folders → conflict-free. **Worktree isolation
(`isolation: "worktree"`) is the recommended hardening** for the parallel cohorts.

### 2.4 Off-ramp checkpoint (new)
After SP1–SP3 land, the coordinator **pauses and presents** their findings against
the overview §2 off-ramp (async recompute can't stay responsive / 100 MB open is
minutes-or-memory-hungry / parity fundamentally short). Clean → proceed to build-out.
Triggered → surface to the human before further investment. This replaces Phase-1's
up-front stack gate.

## 3. Measurement Methodology (additions to Phase-1 §3)

Phase-1 §3 stands (Criterion + `bench_util` timers; p50/p99/max; committed
code-generated inputs; env-stamped results; PASS/FAIL asserts; **foreground-only**
runs with `timeout`; adversarial review of surprising numbers). Round-2 adds:

- **Fresh-process peak-RSS (SP2, SP1 clone, SP7 density).** Peak memory must be read
  from a **separately-spawned child process** measuring its own high-water mark
  (Linux `VmHWM` from `/proc/self/status`, or `getrusage(RUSAGE_SELF).ru_maxrss`),
  reported by the child — *not* inferred from the harness process (whose allocator
  state is polluted by prior work). `bench_util` gains a small `peak_rss()` helper
  (added at scaffolding, in `round-2/harness/`, then frozen).
- **UI-thread-blocking measurement (SP1).** The async-recompute prototype must
  measure the *synchronous* cost incurred on the simulated UI thread per edit
  (enqueue + optimistic stale-mark), separately from the worker's recompute
  wall-clock and the staleness window. These are three distinct budgets.
- **Golden-file correctness (SP3).** Expected outputs are committed alongside cases;
  the harness diffs IronCalc output vs expected and reports a pass rate + itemized
  failures. Error values (`#DIV/0!` etc.) are compared as typed errors, not strings.

## 4. SP1 — Async-Recompute Architecture (designed in full)

This is the Phase-2 crux and is designed here so the coding agent executes, not
designs. IronCalc has **no incremental recalc** and `evaluate()` **mutates the Model
in place and is not interruptible**; a full recompute is ~2 s at 10⁷ cells. The UI
thread must never block on it.

### 4.1 The engine-actor model (primary design)
Run the `Model` on a **dedicated engine thread** (actor). The UI thread never touches
the `Model` directly.

- **Commands (UI → engine):** an `mpsc`/`crossbeam` channel of `EngineCmd`
  (`SetCell{sheet,row,col,input}`, `SetStyle{...}`, `Open`, `Save`, …). The UI-thread
  cost of an edit is **only** "push a command + optimistically mark the edited cell's
  cached value stale-but-painted" — O(1), well under a frame.
- **Debounce + supersede (in the actor):** the engine thread drains *all* queued
  commands, **applies them in order to the Model**, then runs **one** `evaluate()`.
  Commands that arrive during an in-flight `evaluate()` queue up and are applied in
  the *next* batch. "Cancellation" = **coalescing** these queued edits and running a
  single evaluate for the batch (an in-flight evaluate is never interrupted). A short
  debounce window (e.g. ~16–50 ms, tuned) batches typing bursts.
- **Publish (engine → UI):** after `evaluate()`, publish the freshly-evaluated,
  read-only state to the UI via an **`ArcSwap<ReadState>`** (lock-free). UI reads
  always see the latest published `ReadState`, never blocking, never tearing.

### 4.2 The read/publish problem — two candidate `ReadState` designs (SP1 picks with data)
`evaluate()` mutates the *engine's* Model, so the UI cannot read the same instance.
Two ways to give the UI a stable readable snapshot; SP1 **measures both and
recommends**:

- **(P) Double-buffered Model.** Keep the engine's live `Model` plus publish an
  `Arc<Model>` clone of the just-evaluated state; UI reads via
  `get_cell`/`get_style_for_cell` on the `Arc<Model>` (this exercises the SP4
  viewport path directly). **Cost = one `Model` clone per publish** — the number SP1
  must measure (clone of a 10⁶–10⁷ model). Requires `Model: Send + Clone`. Best if
  clone is affordable (hundreds of ms is fine — it's off the UI thread and only
  widens the staleness window, not the frame budget).
- **(F) Value-projection snapshot (fallback).** After `evaluate()`, extract a compact
  immutable read model — `Arc<{ values: …, styleIds: …, format table }>` covering
  *populated* cells (or built lazily per requested viewport) — and publish that. UI
  reads from the projection, not a Model. Avoids full-Model clone; cost = projection
  build. **Use if clone is prohibitive or `Model` isn't `Send`/`Clone`.**

Either way the UI thread is decoupled and the viewport read stays on a stable
immutable structure (feeding SP4's <2 ms value+style gate).

### 4.3 Staleness UX
Between an edit and the next publish, the UI shows **last-published values** (IronCalc
persists cached results, so even a freshly-opened file paints immediately) with a
lightweight **"recalculating…"** indicator; edited cells optimistically show their new
literal input. The **staleness window** (edit → fresh publish) = debounce + apply +
evaluate (+ clone/projection) and is a *discovery* number, not a frame gate.

### 4.4 What SP1 builds & asserts
A headless prototype of the actor loop (no GPUI needed — the "UI thread" is a driver
loop) that: drives scripted edit bursts; asserts **UI-thread per-edit work < frame
budget** (GATE); asserts **N rapid edits ⇒ ≤ small bounded evaluate() count** (GATE,
coalescing); records the latency matrix, clone/projection cost, and staleness window
(DISCOVERY). This proves the architecture the real app will use.

## 5. SP2–SP5, SP7 — design-level (detail → phase plans)

- **SP2 (xlsx-open):** `datagen` gains a styled-`.xlsx` generator (values + formulas
  + styles + multiple sheets + shared strings) sized ≥100 MB. A **child-process**
  harness opens it, stamps VmHWM, and times stages via checkpoints around
  unzip/parse/shared-strings/style-ingest/graph-build/first-eval (as far as IronCalc's
  API lets us instrument; where it's opaque, record the coarser number honestly).
  Time-to-first-paint = time until cached values are queryable, measured separately.
- **SP3 (function-parity):** two artifacts — a **coverage** diff (IronCalc's
  registered function list vs a committed canonical Excel list, categorized), and a
  **golden-file** harness: a committed `cases` table (formula, inputs, expected value
  *or* expected error), run through the IronCalc adapter, diffed, pass-rate + failures
  reported. ≥~100 cases spanning error propagation, coercion, dates/locale,
  arrays/spill. Case table is data, not code, so it grows cheaply.
- **SP4 (binding-style):** copy the `02` viewport-read benchmark; extend the read
  closure to fetch **value + `get_style_for_cell`** per visible cell; run the same
  scroll/jump scenarios at Excel-max; report p99 (GATE <2 ms). Separately, an
  API-exposure probe crate asserts (or fails to) row/col **band** styles and
  **empty-cell** styles via IronCalc's public API — the result is a finding that may
  reopen the overview §2 decision. Plus a cache-invalidation module keyed on
  `UserModel`'s diff-list with a unit test (edit → only affected visible cells dirty).
- **SP5 (style-fidelity):** copy/extend `03-formatting`'s harness into a comprehensive
  attribute matrix; generate a long-tail styled `.xlsx`; round-trip; probe-assert each
  attribute → matrix {survives/lossy/dropped}. Merges/CF explicitly excluded (record
  as OPEN).
- **SP7 (residual):** small test crates — CSV RFC-4180 edge cases; an IronCalc
  load-API ergonomics writeup + a thin wrapper sketch; a storage-density extrapolation
  (measured B/cell × populated-fraction scenarios → RAM at Excel-max, with the tipping
  point flagged).

## 6. SP6 — GPUI Grid Maturation (design-level, macOS)

Evolve `04-ui-poc/raw-gpui` into `round-2/06-gpui-grid/` (copy, don't mutate the
frozen PoC). Additions:
- **Inline cell editing** (enter edit mode on a cell, commit/cancel), **selection
  ranges** (click-drag + shift/ctrl), **frozen panes** (frozen top rows / left cols
  that don't scroll). Reuse the PoC's prefix-sum + binary-search virtualization for
  variable sizes.
- **In-app "Run Test"** extended to record the §5.4 numbers *with the new
  interactions present* (scripted scroll/jump + an edit-while-scrolling sequence);
  logs PASS/FAIL to `results/`.
- **PNG-baseline render tests:** capture the grid to PNG on macOS and diff against
  committed known-good images (a foretaste of the product's rendering-test strategy).
  Pixel-comparison runs on the Mac (real GPU output).
- **GPL #55470 fix:** patch `sum_tree` (via a `[patch]` on the git dep or a vendored
  fork) to swap `ztracing::instrument` → `tracing::instrument`; verify `cargo tree`
  shows **no** `ztracing`/`zlog`/`ztracing_macro` (GPL-3.0) in the build. Document for
  the pre-distribution legal sign-off.

The exact interaction wiring and GPUI APIs are a phase-plan detail (written by the
SP6 lead against the real GPUI surface, on the Mac side).

## 7. Error Handling, Testing, Dependencies

- **Error handling:** experiments use `anyhow`; an unmet target is a **recorded
  finding**, never a silent skip or a panic that hides the number.
- **Testing:** each in-container sub-project ships **correctness tests + benchmarks**
  (Round-2 continues standing up reusable test/bench infra). For SP6, "Run Test" +
  PNG baselines are the tests. Force+assert every measured op.
- **Dependencies:** `ironcalc`/`ironcalc_base` pinned to the Phase-1 version;
  `criterion` dev-dep; `zip`/OOXML deps only as IronCalc's own I/O needs. SP6 pins
  `gpui` + `gpui-component` via the same git revs Phase-1 used (+ the #55470 patch).
  **No new engine** (IronCalc only).

## 8. Doc Organization (1-phase) & Phase Plans
**Single `architecture.md`, no `components/` dir** (same as Phase 1). Detailed
per-sub-project design — exact bench parameters, the SP3 case table, SP6 interaction
wiring, precise size/shape grids — is written into each phase's
**`phase_plans/phase_N.md` by its lead agent** at implementation time, against the
real IronCalc/GPUI API surface. The one hard problem that could not wait (SP1's async
architecture) is designed above (§4).

## 9. Risks (technical)
- **`Model` not `Send` / clone too costly (SP1).** Mitigated by the §4.2 fallback
  (value-projection snapshot); if *both* paths are infeasible the async architecture
  is in real trouble — the top thing SP1 must settle early.
- **Instrumentation opacity (SP2).** IronCalc may not expose stage boundaries; record
  the coarsest honest breakdown rather than inventing precision.
- **Parity list canonicalization (SP3).** "Excel's ~500" needs a committed canonical
  source; document which list is used so coverage % is reproducible.
- **PNG determinism (SP6).** GPU/font rendering can vary across machines; baselines
  are captured on the human's Mac and diffed with a tolerance, documented.
- **Frozen-crate drift.** If IronCalc's pinned version is bumped, Phase-1 comparability
  weakens; keep the pin, note any forced bump as a finding.
