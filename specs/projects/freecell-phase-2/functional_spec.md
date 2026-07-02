---
status: complete
---

# Functional Spec: FreeCell — Phase 2 (Round-2 Technical De-risking)

> Read `project_overview.md` first — it carries the locked decisions (engine =
> IronCalc, UI = GPUI raw-gpui grid, formatting = IronCalc-native styles), the
> inherited Phase-1 evidence, and the working conventions. This spec turns the
> remaining **real technical unknowns** into experiments with explicit success
> criteria. It is **not** a plan to start building the app in `experiments/` —
> anything that is "build the feature" belongs to the real app, not Phase 2 (§2).

## 1. Purpose

Phase 2 is a **second de-risking round**, not a product build. Phase 1 returned
**"GO, with conditions"**: the everyday case is proven fast, but a few genuine
technical unknowns remain that could still force an engine pivot or a design change.
Phase 2 measures exactly those unknowns with reproducible evidence, so Stage 3 can
decide *build / adjust / pivot* on numbers.

The governing filter for what belongs in Phase 2: **is this a real technical unknown
that could kill or pivot the project?** If yes, it's an experiment. If it's just
"write the feature" (grid selection, inline editing, CSV polish, an API wrapper),
it's out — that's the real build.

All work continues under `experiments/round-2/`, each experiment producing a
`findings.md` + runnable code + committed results, reusing the frozen Phase-1 harness.
**Phase 2 is fully in-container and autonomous — no macOS/GPU dependency** (see §3).

## 2. Scope

### In scope — five experiments (§6), each answering a real unknown
1. **SP1** — Non-blocking recompute & the engine↔render interop seam *(the crux)*.
2. **SP2** — End-to-end large styled `.xlsx` open (time + peak memory).
3. **SP3** — Function-parity audit (coverage + correctness vs Excel).
4. **SP4** — Styled viewport read at scale + style-API coverage.
5. **SP5** — Long-tail style-roundtrip fidelity (a check on IronCalc's file I/O).

Plus a **Phase-2 synthesis** updating the go/adjust/pivot recommendation.

### Out of scope (explicitly — these are *building* or already-settled)
- **Grid features / GPUI work** — selection, inline editing, frozen panes, and
  re-measuring frame perf. GPUI is **validated** (Phase-1 PoC; human-blessed feel).
  These belong to the real build, not a de-risking round.
- **The merges / conditional-formatting side-store** — no IronCalc API; a major
  feature left OPEN (overview §2). Phase 2 records the gap + scope-trap only.
- **CSV hardening, an IronCalc load-API wrapper** — building/tooling.
- **Memory-ceiling extrapolation** — Phase-1's 10M-cell memory measurement was
  sufficient; not re-litigated here.
- **GPL #55470** — a pre-distribution packaging/legal task, tracked (overview §2),
  **not** a technical unknown → not a Phase-2 experiment.
- **Formualizer work** — it is the documented off-ramp only; Phase 2 measures IronCalc.
- Production import/export, error handling, persistence, packaging.

### Deliberately deferred
- Anything the Phase-2 synthesis tags for "Round 3".
- The merges/CF technical design (gated behind the overview §2 scope decision).

## 3. Environment & Division of Labor

**Phase 2 runs entirely in the headless Linux container (4c/~15 GB, no GPU),
autonomously.** Dropping the GPUI experiment removes the macOS dependency Phase 1
had — every experiment builds, runs, and measures in-container, and those numbers
are authoritative (a 4-core floor; real hardware is faster).

Binding conventions from Phase 1 still hold: **all benchmarks run FOREGROUND with
`timeout`** — never `nohup`/`&`/background monitors (a Phase-1 agent burned ~600k
tokens flailing on a background poller). If a run is too slow, **cap the scale and
record the ceiling as a finding.**

## 4. Sequencing, Gating & the Off-Ramp

No up-front human gate (the engine is chosen). One **off-ramp checkpoint** partway:

1. **Engine-risk cohort first (SP1, SP2, SP3)** — the three that could prove IronCalc
   unfit. Run in parallel (disjoint folders).
2. **Off-ramp checkpoint (human review of SP1–SP3).** If any shows IronCalc cannot
   credibly meet the bar — recompute can't be made non-blocking with a clean seam;
   a 100 MB `.xlsx` opens in minutes or blows memory; function coverage/correctness
   is fundamentally short — **the swarm pauses and surfaces it for a human go/pivot
   decision** (overview §2 off-ramp) before investing further. Clean → proceed.
3. **Build-out cohort (SP4, SP5)** — in-container, parallel, assume the engine holds.
4. **Phase-2 synthesis (last)** — consumes all findings → Stage 3 recommendation.

## 5. Cross-Cutting Conventions

Phase-1 conventions carry over verbatim; this only notes what's new. Originals:
Phase-1 `functional_spec.md` §5.2–5.5 and `architecture.md` §2–§3 — **read them.**

### 5.1 Layout
Round-2 work lives under **`experiments/round-2/NN-*/`**, one self-contained,
**independent Cargo project** per experiment (NOT a workspace — Phase-1 isolation
rationale). Each depends by **relative path, read-only** on:
- `experiments/shared/datagen` + `experiments/shared/bench_util` (frozen), and
- the Phase-1 `02-datamodel-binding-perf` harness (`SpreadsheetEngine` trait +
  scenarios) and IronCalc adapter — copied into a frozen `round-2/harness/` lib crate
  at scaffolding (architecture §1), so Round-2 numbers stay comparable to Phase-1.

Changes to a frozen shared crate → **escalate**, don't edit in place.

### 5.2 Findings & benchmark standards
Same as Phase-1 §5.2 (findings headings) and §5.3 (reproducible from one command;
p50/p99; env-stamped; committed code-generated inputs). Plus the hard-won discipline
(overview §7): separate build/load time from the measured op; use IronCalc's *best*
API; **force + assert** the measured op; **adversarially review any surprising
number** before it drives a conclusion (a Phase-1 sub-project shipped a *backwards*
result from the wrong load API — caught in review).

### 5.3 Perf targets
Unchanged (Phase-1 §5.4): newly-visible-cell load < ~2 ms; 1M-cell cascade recompute
< 100 ms *(known-FAIL — the Phase-2 point is the non-blocking UX, not the raw
number)*; open 100 MB+ `.xlsx` in seconds with sane peak memory. Each experiment
states which target(s) it gates on.

## 6. Experiments

Format: **Questions / Approach / Deliverables / Pass criteria**, where criteria are
either a **GATE** (hard measured pass/fail) or **DISCOVERY** (record + judge).

---

### SP1 — Non-blocking recompute & the engine↔render interop seam  *(THE key experiment)*

**Why it's the crux.** IronCalc has no incremental recalc: every edit runs a
full-workbook `evaluate()` — O(all cells), ~2 s at 1M — that mutates the model. Run
on the render path, every edit on a big sheet freezes the app. The real deliverable
is a **clean interop seam between the IronCalc engine and the GPU grid** where
recompute is **non-blocking** and each half can be its best — the renderer stays at
frame rate, the engine stays the authoritative model. *(The exact Rust concurrency
mechanism is an **output** of this experiment, chosen to fit IronCalc's real API —
not pre-decided. "Non-blocking," not any specific threading design, is the goal.)*

**Questions.**
- **Non-blocking:** how do we run `evaluate()` so the render loop never stalls a
  frame, even during a multi-second recompute? What does IronCalc's API allow — can
  the model be shared/read while an eval runs, moved to run elsewhere, or only
  snapshot/cloned (and at what cost)?
- **Eval lifecycle / serialization:** inspect `evaluate()`'s lifecycle. Is it
  reentrant, or must we **serialize to one eval at a time** (a lock/queue)? Any
  start / progress / completion signals? Can it be stepped or chunked?
- **Change awareness (the interesting unknown):** does IronCalc expose a **stream /
  pub-sub of cells changed by an eval** — ideally *as it runs*, acceptably *after*?
  Motivating case: a 1M-cell cascade where only ~30 cells are on screen — can we
  repaint just those as they settle?
  - **Live stream** → progressive visible updates during eval.
  - **Post-eval diff only** (e.g. the `UserModel` diff-list) → repaint visible cells
    once eval completes.
  - **Neither** → pick and **lock a fallback**: re-pull the visible cells (the SP4
    < 2 ms read) on a ~100 ms timer while eval runs, **or** simply wait for eval to
    finish then re-pull. **None of these is a dealbreaker** — even the simplest is
    acceptable. The job is to learn which IronCalc forces and lock the seam around it.

**Approach.** Investigate and write down IronCalc's API (eval signature/lifecycle;
`Send`/`Sync`; any changed-cells / subscription / diff API; reentrancy; read-during-
eval safety). Measure `evaluate()` latency across **sizes {10⁴…10⁷} × DAG shapes
{sparse, wide fan-out, deep-serial 1M chain, cross-sheet, volatile}** (foreground,
force + assert the tail changed). Build a **minimal non-blocking harness**: a driver
"render loop" that must stay responsive while an eval runs concurrently, updating the
visible cells via whichever mechanism IronCalc supports (stream → progressive; diff/
wait → on completion; else poll). Measure render-loop responsiveness, the visible-
update path, and the staleness window (edit → visible cells fresh).

**Deliverables.** `round-2/01-async-interop/findings.md`; the evaluate() latency
matrix (committed `results/`); the minimal non-blocking harness; and — the real
output — a **locked engine↔render interop-seam design**: how recompute stays
non-blocking, how the renderer learns what to repaint (stream / diff / poll / wait,
justified by IronCalc's actual API), and how the two halves stay decoupled.

**Pass criteria.**
- **GATE:** the driver render loop **never blocks on recompute** — per-tick work
  stays < one frame (< 8.3 ms; hard-fail > 16.6 ms) even while a 10⁶–10⁷-cell eval
  is in flight.
- **DELIVERABLE:** a written, defensible interop-seam design, with the change-
  propagation mechanism **chosen and justified by IronCalc's real API** (and a locked
  fallback if no live stream exists).
- **DISCOVERY:** evaluate() latency matrix (p50/p99); whether a live change-stream
  exists; whether one-eval-at-a-time serialization is required; snapshot/clone cost
  if that's the only safe read route. The 1M serial chain staying ~2 s (FAIL vs
  <100 ms) is expected and recorded — not the gate.

---

### SP2 — End-to-end large styled `.xlsx` open  *(closes §5.4's one un-run target)*

**Questions.** How long does a fresh process take to open a real **100 MB+ styled
`.xlsx`**, and what is **peak RSS**? Where does the time go (unzip / XML parse /
shared-strings / style ingest / graph build / first eval)? How fast is **time-to-
first-paint** — cached values shown before recompute (IronCalc persists cached
results)?

**Approach.** Extend `datagen` to synthesize a ≥100 MB styled `.xlsx` from committed
code (realistic mix: values, formulas, styles, multiple sheets, shared strings).
Measure open time + **peak RSS from a fresh, separately-spawned process** (not warm),
broken out by stage as far as IronCalc's API allows (record the coarsest honest
number where it's opaque). Measure time-to-first-paint separately.

**Deliverables.** `round-2/02-xlsx-open/findings.md`; the file generator; open-time +
peak-RSS + stage-breakdown results (committed); time-to-first-paint number.

**Pass criteria.**
- **Judgment GATE:** open completes in **seconds, not minutes**, with **peak RSS a
  sane multiple of file size** (record the multiple + dominant stage cost).
- **DISCOVERY:** time-to-first-paint recorded.
- **Off-ramp trigger:** minutes-scale open or ballooning RSS (≫10× uncompressed) →
  record and surface at the checkpoint (§4).

---

### SP3 — Function-parity audit  *(Excel-compat is the headline promise; least-proven)*

**Questions.** How close is IronCalc's ~345 registered builtins to Excel's ~500 in
**coverage** *and* **correctness** — edge cases, error semantics
(`#DIV/0!`/`#N/A`/`#VALUE!`/`#REF!` propagation), empty-cell & type coercion, date
serials/locale, and array/spill behavior?

**Approach.**
- **Coverage diff:** IronCalc's registered function set vs a **committed canonical
  Excel function list** (documented source, so coverage % is reproducible),
  categorized by real-world importance (common vs obscure).
- **Golden-file correctness harness:** a committed `cases` table (formula, inputs,
  expected value *or* typed error) with **known-correct Excel outputs** (≥ ~100 cases
  spanning the edge categories); run each through the IronCalc adapter, diff, report
  pass rate + itemized failures. Errors compared as typed errors, not strings.

**Deliverables.** `round-2/03-function-parity/findings.md`; the coverage matrix; the
golden-file harness + per-case results; a categorized gap list.

**Pass criteria.**
- **DELIVERABLE:** coverage matrix committed.
- **GATE (measured):** golden-file harness runs ≥ ~100 cases with a recorded pass
  rate; failures itemized.
- **Judgment / off-ramp:** Excel-compat is **credibly achievable** — missing
  functions are implementable/contributable and semantics are mostly right. A large
  fraction of *common* functions missing, or systematically wrong semantics →
  **flag for the engine off-ramp** (§4).

---

### SP4 — Styled viewport read at scale + style-API coverage

**Questions.**
- Does the per-cell viewport read hold **p99 < 2 ms when it also reads the style**
  (`get_style_for_cell`) per visible cell, at Excel-max? (Phase-1 measured *value-
  only* at 392 µs; styles are now read straight from IronCalc per overview §2, so
  this must be confirmed, not assumed.)
- Does IronCalc's style API **expose what FreeCell needs** — per-cell attributes,
  row/column **band** styles, and **empty-cell** styling? (Excel styles whole empty
  rows/cols; verify via the public API, don't assume.)

**Approach.** Extend the `02` viewport-read benchmark (via `round-2/harness`) to fetch
**value + style** per visible cell; run the same scroll/jump scenarios at Excel-max
positions, foreground; report p50/p99. Probe the style API for band/empty-cell
coverage with explicit assertions.

**Deliverables.** `round-2/04-styled-read/findings.md`; the extended benchmark +
results; the style-API-coverage findings.

**Pass criteria.**
- **GATE:** value + style viewport read stays **p99 < 2 ms** at Excel-max for a
  viewport+overscan window (~10³–10⁴ cells).
- **DELIVERABLE / decision-reopener:** style-API coverage documented; **if row/col
  band or empty-cell styling is missing from the public API, that reopens the
  overview §2 formatting decision** (may force a scoped side-store).

---

### SP5 — Long-tail style-roundtrip fidelity  *(a check on IronCalc's file I/O)*

**Questions.** Beyond the representative attributes Phase-1 probed, do **exact colors,
all border styles, every number-format code, alignment, and rich text** survive a
load → edit → save → reload `.xlsx` round-trip via IronCalc?

**Approach.** Extend the `03-formatting` harness (by copy) into a comprehensive
attribute matrix; generate a long-tail styled `.xlsx` from committed code; round-trip;
probe-assert each attribute. **Merges + conditional formatting are OUT** (no IronCalc
API; overview §2) — recorded as a known gap, not designed.

**Deliverables.** `round-2/05-style-fidelity/findings.md`; the fidelity matrix
(attribute × {survives / lossy / dropped}) with probe evidence.

**Pass criteria.**
- **DELIVERABLE:** fidelity matrix committed, each entry probe-backed.
- **Judgment GATE:** the common long tail (colors, standard borders, number formats,
  alignment) round-trips faithfully; lossy/dropped attributes documented with
  severity. Merges/CF recorded OPEN.

---

### Phase-2 Synthesis  *(final; serial)*

**Deliverable.** `experiments/round-2/SYNTHESIS.md`: an updated **build / adjust /
pivot** recommendation for Stage 3, citing SP1–SP5, explicitly stating whether the
engine off-ramp was triggered, and listing any Round-3 carry-forward.

**Pass criteria.** A defensible, evidence-backed Stage-3 recommendation. A
well-evidenced "this condition doesn't hold → adjust/pivot" is a successful Phase 2.

## 7. Risks & What Could Invalidate the Approach

- **No clean interop seam (SP1) — the biggest risk.** If IronCalc's model can't be
  read during an eval *and* clone is unaffordable *and* it exposes no changed-cells
  signal, we're forced onto wait-then-repull with a multi-second staleness window on
  huge edits. Acceptable, but it caps the "live" feel — SP1 must surface exactly
  which constraints IronCalc imposes.
- **Function parity worse than the raw count (SP3).** 345 registered ≠ 345 correct; a
  count is not an audit. Systematic gaps could reopen the engine choice.
- **Style API thinner than assumed (SP4).** No band/empty-cell styling → the "native
  styles as source of truth" decision needs a scoped side-store — a finding, not a
  silent patch.
- **`.xlsx` open cost (SP2).** OOXML parse + style ingest at 100 MB is un-measured; if
  minutes-scale or memory-hungry, it dents the "open huge files" promise.
- **Scope creep.** The pull to start building the real app is strong. Phase 2 stops at
  measurement + a Stage-3 recommendation.

## 8. Resolved Decisions
- **No UI / GPUI work in Phase 2.** GPUI is validated; grid features + perf
  re-measurement are the real build. Phase 2 is fully in-container.
- **No merges/CF design.** Out of scope (overview §2); record the gap + scope-trap.
- **No memory-ceiling experiment.** Phase-1's 10M-cell memory test was sufficient.
- **Engine is fixed (IronCalc); no bake-off.** Formualizer is the off-ramp reference.
- **Reuse, don't rewrite, the Phase-1 harness** (frozen `shared/` + copied
  `round-2/harness/`), so Round-2 numbers stay comparable.
- **Off-ramp checkpoint after SP1–SP3** (human review), not an up-front gate.
