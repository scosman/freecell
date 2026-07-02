---
status: complete
---

# Architecture: FreeCell — Phase 2 (Round-2 Technical De-risking)

Like Phase 1, this is a **research/de-risking** effort, so the "architecture" is: the
experiment workspace + reuse strategy, the **agent-swarm orchestration**, the
**measurement methodology**, and — because it's the load-bearing hard problem — the
**engine↔render interop design space (SP1)**. Remaining per-experiment detail (exact
bench parameters, the SP3 case table) is deferred to **phase plans** (§8), matching
Phase 1. Read Phase-1 `architecture.md` first; this doc only adds/changes.

## 0. Grounding facts (inherited + Phase-2-relevant)
- Rust 1.94+; container **4 cores / ~15 GB RAM, no GPU / no display**; crates.io
  works. GitHub scoped to `scosman/freecell`. **Phase 2 is fully in-container** (the
  GPUI experiment was dropped — GPUI is validated), so every phase is autonomous.
- **Engine = IronCalc 0.7.x** (`ironcalc` / `ironcalc_base`). Pin the **same version
  Phase 1 used** for comparability. Known shape: nested-`HashMap` storage
  (~162 B/cell); full-workbook `evaluate()` (no incremental recalc, mutates the
  model); `UserModel` exposes an edit **diff-list** (edit-sites only); native styled
  `.xlsx` I/O; persists cached results; no CSV / merges / CF API.
- **The engine↔render concurrency story is the top open question (SP1).** Whether the
  model is readable during an eval, movable, or clone-only; whether eval is reentrant
  or must be serialized; and whether IronCalc emits a changed-cells stream — are all
  things **SP1 discovers from the API**, not assumptions baked in here. §4 gives the
  *goal* (non-blocking + a clean seam) and the *design space*, not a prescribed
  mechanism.

## 1. Repository Layout & Reuse Strategy

Round-2 work lives under **`experiments/round-2/`**. Phase-1 folders are **frozen**
(read-only); Round-2 never edits them.

```
experiments/
  shared/                         # Phase-1, FROZEN (datagen, bench_util)
  02-datamodel-binding-perf/      # Phase-1, FROZEN — source of the harness we copy
  03-formatting/                  # Phase-1, FROZEN — source SP5 extends (by copy)
  round-2/
    harness/                      # NEW shared crate — created + FROZEN at scaffolding
                                  #   = verbatim copy of 02/common (SpreadsheetEngine
                                  #     trait + scenarios) + 02/ironcalc (IronCalc
                                  #     adapter) as a LIB crate + a peak_rss() helper.
                                  #     Read-only downstream.
    01-async-interop/             # SP1  (independent Cargo project)
    02-xlsx-open/                 # SP2
    03-function-parity/           # SP3
    04-styled-read/               # SP4
    05-style-fidelity/            # SP5
    SYNTHESIS.md                  # Phase-2 synthesis → Stage 3
```

**Why a `round-2/harness/` copy instead of depending into `02/`:** Phase-1 crates are
frozen; the IronCalc adapter there is a *bench* crate, not necessarily a clean lib.
Copying the trait + adapter + scenarios **verbatim** into one `harness/` lib crate
(same IronCalc version pin) gives Round-2 a single, stable, **read-only** engine seam
that keeps numbers comparable without mutating frozen code. Created + frozen at
scaffolding (Phase-1's `shared/` pattern); needed changes → **escalate**.

Each experiment is an **independent Cargo project** (NOT a workspace — Phase-1
isolation rationale) depending by **relative path, read-only** on `../harness` and
`../../shared/*`. `target/` gitignored repo-wide. Repeated IronCalc compiles accepted.

## 2. Agent-Swarm Orchestration

Same structure as Phase-1 `architecture.md` §2: a **coordinator** spawns, per phase, a
**manager → coding sub-agent → CR sub-agent(s)**, running the attestation → CR →
commit loop; the manager never writes code itself. Parallel phases run concurrently.

### 2.1 Topology
```
Phase-2 Coordinator
├─ Phase 2.0: Scaffolding (serial) ── create round-2/ + round-2/harness/ (frozen)
├─ Engine-risk cohort (PARALLEL):     SP1, SP2, SP3
│     └─►  OFF-RAMP CHECKPOINT ── human review of SP1–SP3 vs the overview §2 off-ramp
├─ Build-out cohort (PARALLEL):       SP4, SP5
└─ Phase-2 Synthesis (serial; last) ── SYNTHESIS.md
```
All phases build/run **in-container**; numbers are authoritative there.

### 2.2 Parallel-editor isolation (REQUIRED — inject verbatim into every parallel agent)
Reuse Phase-1 `architecture.md` §2.2 **exactly**, folder token =
`experiments/round-2/NN-<name>/`. Recap: operate only inside your folder (plus
read-only `round-2/harness/`, `shared/`, `specs/`); **never** edit the repo root,
another experiment, or a frozen crate; git-scope every command to your folder
(`git add experiments/round-2/NN-<name>/`; **never** `git add -A`/`.`/`commit -a`);
CR + attestation cover only your folder's diff; must touch a shared/frozen file →
**stop and escalate**.

### 2.3 Commit safety
Serialized, path-scoped commits (Phase-1 §2.3): coordinator admits one commit at a
time; disjoint folders → conflict-free. **Worktree isolation (`isolation: "worktree"`)
recommended** for the parallel cohorts.

### 2.4 Off-ramp checkpoint
After SP1–SP3 land, the coordinator **pauses and presents** their findings against the
overview §2 off-ramp (no clean non-blocking seam / 100 MB open is minutes-or-memory /
parity fundamentally short). Clean → proceed to build-out. Triggered → surface to the
human before further investment. Replaces Phase-1's up-front stack gate.

## 3. Measurement Methodology (additions to Phase-1 §3)

Phase-1 §3 stands (Criterion + `bench_util` timers; p50/p99/max; committed
code-generated inputs; env-stamped results; PASS/FAIL asserts; **foreground-only** runs
with `timeout`; adversarial review of surprising numbers). Round-2 adds:

- **Fresh-process peak-RSS (SP2).** Peak memory read from a **separately-spawned child
  process** measuring its own high-water mark (Linux `VmHWM` from `/proc/self/status`,
  or `getrusage(RUSAGE_SELF).ru_maxrss`), reported by the child — *not* inferred from
  the harness process (allocator polluted by prior work). `bench_util`'s `peak_rss()`
  helper (added to `round-2/harness/` at scaffolding, then frozen).
- **Render-loop responsiveness (SP1).** The non-blocking harness measures the
  *synchronous* per-tick cost on the simulated render/UI thread while an eval runs
  concurrently, **separately** from the recompute wall-clock and the staleness window.
  Three distinct budgets — don't conflate them.
- **Golden-file correctness (SP3).** Expected outputs committed alongside cases; the
  harness diffs IronCalc output vs expected, reports a pass rate + itemized failures;
  errors compared as **typed** errors, not strings.

## 4. SP1 — Engine↔Render Interop Seam (design space, not a locked mechanism)

The Phase-2 crux. The **goal is fixed; the mechanism is an output of the experiment.**
Goal: a clean seam where **recompute never blocks the render loop** and both halves stay
their best (GPU renderer at frame rate; IronCalc the authoritative model). IronCalc has
**no incremental recalc** and `evaluate()` **mutates the model in place and is not
interruptible** (~2 s at 10⁷). SP1's job is to learn what IronCalc's API permits and
**lock the seam design around that** — *not* to adopt a pre-chosen concurrency design.
(The human explicitly cautioned against prescribing the Rust concurrency model here.)

### 4.1 Three API questions that select the design
SP1 must answer these from IronCalc's real API, then pick the design they imply:
1. **Where can eval run without blocking the render loop?** Can the model be *read
   while an eval runs* (→ eval concurrently, read live)? If not, can it be *moved* to
   run elsewhere (is it `Send`), or is a *snapshot/clone* the only safe route (→
   measure clone cost)? "Non-blocking" is the requirement; the vehicle follows.
2. **Must evals be serialized?** Is `evaluate()` reentrant, or do we need a
   one-at-a-time lock/queue? Any lifecycle signals (start/progress/done)?
3. **How does the renderer learn what changed?** Live changed-cells **stream** (best)
   → post-eval **diff** (e.g. `UserModel` diff-list) → **nothing** (fallback: poll the
   visible-cell read on a timer, or wait-then-repull).

### 4.2 The design space (SP1 picks + justifies one)
- **Read-live (if the model is safely readable during/around eval):** render reads the
  current values every frame; changed-cells stream or diff tells it what's dirty. No
  clone. Best case.
- **Snapshot/publish (if reads and evals can't overlap):** after each eval, publish an
  immutable readable view the render loop consumes (a model clone, or a compact
  value+styleId projection of populated cells). Cost = clone/projection per publish —
  **measured**; it's off the render loop, so it widens the staleness window, not the
  frame budget.
- **Change-propagation, in priority order:** live stream → progressive visible repaint;
  post-eval diff → repaint visible on completion; **fallback** (locked if neither
  exists): re-pull the SP4 <2 ms visible read on a ~100 ms timer during eval, or wait
  for completion then re-pull. All acceptable — the deliverable is a *locked* choice
  matched to IronCalc's API, not a preference.

### 4.3 Staleness UX
Between an edit and the next fresh read/publish, the render loop shows **last-known
values** (IronCalc persists cached results, so even a freshly-opened file paints
immediately) with a light **"recalculating…"** indicator; edited cells optimistically
show their new literal input. Staleness window = the time to the next fresh visible
data — a **discovery** number, not a frame gate.

### 4.4 What SP1 builds & asserts
A headless prototype (no GPUI — the "render loop" is a driver ticking at frame cadence)
that drives scripted edit bursts while an eval runs concurrently and updates the
visible cells via whichever mechanism IronCalc supports. Asserts **render-loop per-tick
work < frame budget** (GATE) even during a 10⁶–10⁷ eval; asserts **N rapid edits ⇒ ≤ a
small bounded eval count** (GATE, coalescing/debounce). Records the latency matrix, any
change-stream capability, serialization requirement, clone/projection cost, and the
staleness window (DISCOVERY). Output: the **locked interop-seam design** the real app
will build on.

## 5. SP2–SP5 — design-level (detail → phase plans)

- **SP2 (xlsx-open):** `datagen` gains a styled-`.xlsx` generator (values + formulas +
  styles + multiple sheets + shared strings) sized ≥100 MB. A **child-process** harness
  opens it, stamps VmHWM, and times stages via checkpoints around unzip / parse /
  shared-strings / style-ingest / graph-build / first-eval (as far as the API allows;
  record the coarser honest number where opaque). Time-to-first-paint = time until
  cached values are queryable, measured separately.
- **SP3 (function-parity):** a **coverage** diff (IronCalc's registered functions vs a
  committed canonical Excel list, categorized) + a **golden-file** harness: a committed
  `cases` table (formula, inputs, expected value *or* typed error) run through the
  adapter, diffed, pass-rate + failures reported. ≥~100 cases across error propagation,
  coercion, dates/locale, arrays/spill. Cases are data, so the suite grows cheaply.
- **SP4 (styled-read):** copy the `02` viewport-read benchmark (via `round-2/harness`);
  extend the read closure to fetch **value + `get_style_for_cell`** per visible cell;
  run the same scroll/jump scenarios at Excel-max; report p99 (GATE <2 ms). Separately,
  an API-exposure probe asserts (or fails to) row/col **band** and **empty-cell**
  styles via the public API — a finding that may reopen the overview §2 decision.
- **SP5 (style-fidelity):** copy/extend `03-formatting`'s harness into a comprehensive
  attribute matrix; generate a long-tail styled `.xlsx`; round-trip; probe-assert each
  attribute → {survives / lossy / dropped}. Merges/CF excluded (record OPEN).

## 6. Error Handling, Testing, Dependencies

- **Error handling:** `anyhow`; an unmet target is a **recorded finding**, never a
  silent skip or a panic that hides the number.
- **Testing:** each experiment ships **correctness tests + benchmarks**; force+assert
  every measured op.
- **Dependencies:** `ironcalc`/`ironcalc_base` pinned to the Phase-1 version;
  `criterion` dev-dep; `zip`/OOXML deps only as IronCalc's own I/O needs. **No new
  engine, no GPUI deps** (IronCalc only; Phase 2 is headless).

## 7. Doc Organization (1-phase) & Phase Plans
**Single `architecture.md`, no `components/` dir** (same as Phase 1). Detailed
per-experiment design — exact bench parameters, the SP3 case table, precise size/shape
grids — is written into each phase's **`phase_plans/phase_N.md` by its lead agent** at
implementation time, against the real IronCalc API. The one problem that couldn't wait
(SP1's interop design space) is framed above (§4); SP1 *closes* it with API findings.

## 8. Risks (technical)
- **No overlap of reads and eval, and clone too costly (SP1).** Then we're on
  snapshot-on-completion with a multi-second staleness window on huge edits —
  acceptable but caps the "live" feel. SP1 must settle this early; it's the top risk.
- **Instrumentation opacity (SP2).** IronCalc may not expose stage boundaries; record
  the coarsest honest breakdown rather than inventing precision.
- **Parity list canonicalization (SP3).** "Excel's ~500" needs a committed canonical
  source; document which, so coverage % is reproducible.
- **Style API thinner than assumed (SP4).** No band/empty-cell styling → the native-
  styles decision needs a scoped side-store; that's a finding.
- **Frozen-crate drift.** If IronCalc's pinned version is bumped, comparability weakens;
  keep the pin, note any forced bump as a finding.
