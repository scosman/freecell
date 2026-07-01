---
status: complete
---

# Functional Spec: FreeCell — Phase 2 (Round-2 Technical De-risking)

> Read `project_overview.md` first — it carries the locked decisions (engine =
> IronCalc, UI = GPUI raw-gpui grid, formatting = IronCalc-native styles), the
> inherited Phase-1 evidence, and the working conventions. This spec turns the
> overview's §5 agenda into sub-projects **with explicit success criteria**.

## 1. Purpose

Phase 2 is a **second research / de-risking round**, not a product build. Phase 1
returned **"GO, with conditions"**: the everyday case is proven fast, but the
extremes are *credible-by-design but unmeasured*, and the chosen engine (IronCalc)
has known weaknesses. Phase 2 exists to **measure the conditions** — turn each
open risk into reproducible evidence — so Stage 3 can decide *build the real app /
adjust / pivot* on numbers, not hope.

Concretely, Phase 2 answers: **do IronCalc's known weaknesses (no incremental
recalc, non-columnar storage, ~345 builtins) and the un-measured extremes
(large-`.xlsx` open, styled viewport reads, GPUI on real hardware) hold up under
the FreeCell bar — or do they trip the engine off-ramp?**

All work continues under `experiments/` (Round-2 work under `experiments/round-2/`,
see §5.1), each sub-project producing a `findings.md` + runnable code + committed
results, reusing the frozen Phase-1 harness.

## 2. Scope

### In scope
- Seven de-risking sub-projects (§6), mapped 1:1 to the overview's §5 agenda.
- Extending — never rewriting — the frozen Phase-1 harness (`shared/datagen`,
  `shared/bench_util`, the `02` `SpreadsheetEngine` trait + IronCalc adapter).
- A **Phase-2 synthesis** updating the go/adjust/pivot recommendation for Stage 3.

### Out of scope (explicitly)
- Building the real FreeCell application or its production UI/chrome.
- **Designing or building the merges / conditional-formatting side-store.** No
  IronCalc API exists for either; they are major features left OPEN (overview §2).
  Phase 2 only *records* the gap and its scope-trap; it does not design a solution.
- Formualizer work. It is the documented off-ramp only; Phase 2 measures IronCalc.
- Production file import/export, error handling, persistence, packaging.
- Excel feature coverage beyond what a *parity audit* (SP3) needs to measure.

### Deliberately deferred (to a later round / the real build)
- Anything the Phase-2 synthesis tags for "Round 3".
- The merges/CF technical design (gated behind the §2 scope decision).

## 3. Environment & Division of Labor

Unchanged from Phase 1 — a hard constraint that shapes every sub-project.

| Work type | Where it runs | Who drives | Authoritative? |
|-----------|---------------|------------|----------------|
| Research / writing | Anywhere | Agent | — |
| UI-less Rust (engine, file, perf, memory, parity benches) | Headless Linux container (4c/~15 GB, no GPU) | Agent, autonomously | **Yes** (4-core numbers are a floor; Mac is faster) |
| GPUI grid (macOS/Metal app + in-app "Run Test") | **macOS (Metal)** | Agent writes code + build scripts; **human runs it** and reports numbers/feel | **Yes** (in-app measured PASS/FAIL) |

Rules (from Phase 1, still binding): **GPUI cannot build in-container** — don't
fight it; write code + macOS scripts and hand it to the human. **All benchmarks run
FOREGROUND with `timeout`** — never `nohup`/`&`/background monitors (a Phase-1 agent
burned ~600k tokens flailing on a background poller). If a run is too slow, **cap the
scale and record the ceiling as a finding.**

## 4. Sequencing, Gating & the Off-Ramp

Phase 2 has **no up-front human gate** (unlike Phase 1's stack gate) — the engine is
already chosen. Instead it has an **engine off-ramp checkpoint** partway through:

1. **Engine-risk cohort first (SP1, SP2, SP3).** These three measure IronCalc's
   load-bearing weaknesses (recompute cost, large-file open, function parity). They
   can run in parallel (disjoint folders).
2. **Off-ramp checkpoint (human review of SP1–SP3 findings).** If any of the three
   shows IronCalc cannot credibly meet the bar (async recompute can't keep the UI
   responsive; a 100 MB `.xlsx` opens in minutes or blows memory; function
   coverage/correctness is fundamentally short), **surface it for a human
   decision** per the overview §2 off-ramp before investing further. A clean
   checkpoint → proceed. *(The picking-up agent runs SP1–SP3, then presents results
   and pauses for this review rather than silently barrelling on.)*
3. **Build-out cohort (SP4, SP5, SP7).** In-container, parallel, assume the engine
   holds.
4. **GPUI cohort (SP6).** macOS, human-run; can proceed in parallel throughout
   (engine-independent), but its *measured* numbers require the human.
5. **Phase-2 synthesis (last).** Consumes all findings → Stage 3 recommendation.

## 5. Cross-Cutting Conventions

Phase-1 conventions carry over verbatim; this section only notes what's new or
Round-2-specific. The originals live in Phase-1 `functional_spec.md` §5.2–5.5 and
`architecture.md` §2–§3 — **read them.**

### 5.1 Layout
Round-2 work lives under **`experiments/round-2/NN-*/`**, one self-contained,
**independent Cargo project** per sub-project (NOT a shared workspace — same
isolation rationale as Phase 1). Each depends by **relative path, read-only** on:
- `experiments/shared/datagen` + `experiments/shared/bench_util` (frozen), and
- the Phase-1 `02-datamodel-binding-perf/common` (`SpreadsheetEngine` trait +
  scenarios) and `.../ironcalc` (the IronCalc adapter) — **now also frozen and
  read-only**, so Round-2 perf numbers stay directly comparable to Phase-1
  baselines. (Exact reuse mechanics are an architecture-step detail.)

If a sub-project genuinely needs a change to a frozen shared crate, it **escalates**
(a shared edit breaks the parallel-editor invariant) — it does not edit in place.

### 5.2 Findings & benchmark standards
Same as Phase-1 §5.2 (findings-doc headings: Questions / What was done / Results /
Conclusion / Recommended design / Risks) and §5.3 (reproducible from one command;
p50/p99; env-stamped; committed inputs generated by code). Plus the **hard-won
discipline** (overview §7): separate build/load time from the measured op; build via
IronCalc's *best* API; **force + assert** the measured op so it can't measure a
no-op/cached read; **adversarially review any surprising number** before it drives a
conclusion (a Phase-1 sub-project shipped a *backwards* result from the wrong load
API — caught in review).

### 5.3 Perf targets
The §5.4 "Excel-max & buttery" targets are unchanged (Phase-1 `functional_spec.md`
§5.4): grid 1,048,576 × 16,384; scroll 120 fps (~8.3 ms; ≤16.6 ms worst-case);
newly-visible-cell load < ~2 ms; 1M-cell cascade recompute < 100 ms *(known-FAIL —
the Phase-2 point is the async UX, not the raw number)*; open 100 MB+ `.xlsx` in
seconds with sane peak memory. Each sub-project below states which target(s) it
gates on.

## 6. Sub-Projects

Each maps to one overview §5 agenda item. Format: **Questions / Approach /
Deliverables / Pass criteria**, where pass criteria are either a **GATE** (hard
measured pass/fail) or a **DISCOVERY** metric (record + judge reasonableness).

---

### SP1 — IronCalc recompute cost & async-recompute UX  *(highest priority; the chosen engine's core weakness)*

**Questions.**
- How does full-workbook `evaluate()` latency scale across **sheet size {10⁴, 10⁵,
  10⁶, 10⁷}** × **DAG shape {all-literals baseline, sparse ~1 % formulas, wide
  fan-out (1000×1000), deep-serial chain (1M `=PREV+1`), cross-sheet, volatile
  (NOW/RAND)}**? (The all-literals row isolates the fixed cost of `evaluate()`
  touching every cell even with nothing to recompute — IronCalc's structural tax.)
- Can recompute run **off the UI thread** so editing/scrolling never blocks? The
  crux: `evaluate()` mutates the `Model`. Is `Model` **`Send`** (movable to a
  worker)? What does a **snapshot/clone** of a 10⁶–10⁷ `Model` cost (the enabler
  for evaluate-on-a-copy-then-swap)? If cloning is prohibitive and `Model` isn't
  `Send`, that is a **critical architectural finding**.
- Does **debounce + supersede** tame rapid edits? (Note: a running `evaluate()`
  is **not interruptible** — "cancellation" means coalescing edits and dropping
  superseded *queued* recomputes, not killing an in-flight eval.)

**Approach.**
- Extend the `02` harness/`datagen` to emit the size × shape matrix; benchmark
  `evaluate()` p50/p99 per cell, foreground, force+assert the tail value changed.
- Measure `Model` clone/snapshot cost and confirm `Send`-ability empirically.
- Prototype the **async recompute loop**: UI-thread edit → mark cached values
  "stale-but-painted" → hand a snapshot to a worker → `evaluate()` → post back →
  swap; debounce bursts; drop superseded queued recomputes. Instrument
  **UI-thread blocking time per edit** and the **staleness window** (edit →
  fresh values swapped in).

**Deliverables.** `round-2/01-recompute-async/findings.md`; the recompute-latency
matrix (committed `results/`); the async-recompute prototype + its measured
UI-thread-blocking and staleness numbers; a recommended recompute architecture.

**Pass criteria.**
- **DISCOVERY:** the full latency matrix recorded (p50/p99, env-stamped). The 1M
  serial chain is expected to remain **~2 s (FAIL vs <100 ms)** — recorded, not the
  gate.
- **GATE:** with the async prototype, **UI-thread work per edit stays < one frame
  (< 8.3 ms; hard-fail > 16.6 ms)** even while a 10⁶–10⁷-cell recompute is in
  flight — i.e. the app stays interactive (scroll/edit) during a multi-second
  recompute, painting IronCalc's persisted cached values meanwhile.
- **GATE:** debounce/supersede provably bounds work — assert *N* rapid edits
  trigger **≤ a small bounded number** of full `evaluate()` runs (coalescing works).
- **DISCOVERY:** `Model` clone cost at 10⁶/10⁷ recorded; if off-thread eval is
  infeasible without an unaffordable clone, that is flagged to the off-ramp.

---

### SP2 — End-to-end large styled `.xlsx` open  *(closes §5.4's one un-run target)*

**Questions.** How long does a fresh process take to open a real **100 MB+ styled
`.xlsx`**, and what is **peak RSS**? Where does the time go (unzip / XML parse /
shared-strings / style ingest / dependency-graph build / first eval)? How fast is
**time-to-first-paint** — cached values shown before recompute (IronCalc persists
cached results)?

**Approach.** Extend `datagen` to synthesize a ≥100 MB styled `.xlsx` from
committed code (realistic mix: values, formulas, styles, multiple sheets, shared
strings). Measure open time + **peak RSS from a fresh process** (not warm; a
separate spawned process so allocation isn't polluted by the harness), broken out
by stage. Measure time-to-first-paint separately from full-recompute-ready.

**Deliverables.** `round-2/02-xlsx-open/findings.md`; the file generator; open-time
+ peak-RSS + stage-breakdown results (committed); time-to-first-paint number.

**Pass criteria.**
- **DISCOVERY / judgment GATE:** open completes in **seconds, not minutes**, with
  **peak RSS a sane multiple of file size** (record the multiple). Stage breakdown
  recorded so the dominant cost is known.
- **DISCOVERY:** time-to-first-paint recorded.
- **Off-ramp trigger:** if open is minutes-scale or RSS balloons unreasonably
  (e.g. ≫10× uncompressed size), record it and surface at the checkpoint (§4).

---

### SP3 — Function-parity audit  *(Excel-compat is the headline promise; least-proven)*

**Questions.** How close is IronCalc's ~345 registered builtins to Excel's ~500 in
**coverage** *and* **correctness** — edge cases, error semantics
(`#DIV/0!`/`#N/A`/`#VALUE!`/`#REF!` propagation), empty-cell & type coercion, date
serials/locale, and array/spill behavior?

**Approach.**
- **Coverage diff:** IronCalc's registered function set vs a canonical Excel
  function list; categorize gaps by real-world importance (common vs obscure).
- **Golden-file correctness harness:** a committed table of formula test cases with
  **known-correct Excel outputs** (≥ ~100 cases spanning the edge categories
  above); run each through IronCalc, diff against expected, PASS/FAIL per case.

**Deliverables.** `round-2/03-function-parity/findings.md`; the coverage matrix; the
golden-file harness + per-case results; a categorized gap list.

**Pass criteria.**
- **DELIVERABLE:** coverage matrix (present/absent, categorized) committed.
- **GATE (measured):** golden-file harness runs ≥ ~100 cases with a recorded
  pass rate; individual failures itemized.
- **Judgment / off-ramp:** Excel-compat is **credibly achievable** — missing
  functions are implementable/contributable (not fundamental) and error/coercion
  semantics are mostly right. If a large fraction of *common* functions are missing
  or semantics are systematically wrong, **flag for the engine off-ramp** (§4).

---

### SP4 — IronCalc binding layer + native style read at scale

**Questions.**
- Does the per-cell **viewport read loop still hold p99 < 2 ms when it also reads
  the style** (`get_style_for_cell`) per visible cell, at Excel-max positions?
  (Phase-1 measured *value-only* at 392 µs; styles are now read straight from
  IronCalc per overview §2, so this must be confirmed, not assumed.)
- Does IronCalc's style API actually **expose what FreeCell needs** — per-cell
  attributes, **row/column band styles**, and **empty-cell styling**? (Excel styles
  whole empty rows/cols; verify IronCalc models this via its public API.)
- How do we **invalidate a viewport cache** against IronCalc's edit diff-list
  (`UserModel` diff = edit-sites only, no downstream-dirty)?

**Approach.** Extend the `02` viewport-read benchmark to read **value + style** per
visible cell; measure p50/p99 across viewport+overscan (~10³–10⁴ cells) at Excel-max
positions, foreground. Probe the style API for band/empty-cell coverage (assertions,
not assumptions). Design + unit-test a cache-invalidation scheme keyed on the edit
diff-list.

**Deliverables.** `round-2/04-binding-style/findings.md`; the extended viewport
benchmark + results; the style-API-exposure findings; the cache-invalidation design
+ a correctness test.

**Pass criteria.**
- **GATE:** value + style viewport read stays **p99 < 2 ms** at Excel-max for a
  viewport+overscan window.
- **DELIVERABLE / decision-reopener:** style-API-exposure documented; **if row/col
  band or empty-cell styling is missing from the public API, that is a finding that
  reopens the overview §2 formatting decision** (may force a scoped side-store).
- **GATE (correctness):** the cache-invalidation test shows an edit refreshes only
  the affected visible cells (no stale reads, no over-invalidation).

---

### SP5 — Long-tail style-roundtrip fidelity

**Questions.** Beyond the representative attributes Phase-1 probed, do **exact
colors, all border styles, every number-format code, alignment, and rich text**
survive a load → edit → save → reload `.xlsx` round-trip via IronCalc?

**Approach.** Extend the `03-formatting` harness with a comprehensive attribute
matrix; generate a styled `.xlsx` covering the long tail from committed code;
round-trip; diff each attribute, probe-backed. **Merges + conditional formatting are
explicitly OUT** (no IronCalc API; overview §2) — recorded as a known gap, not
designed.

**Deliverables.** `round-2/05-style-fidelity/findings.md`; the fidelity matrix
(attribute × {survives / lossy / dropped}) with probe evidence.

**Pass criteria.**
- **DELIVERABLE:** fidelity matrix committed, each entry probe-backed.
- **Judgment GATE:** the common long tail (colors, standard borders, number
  formats, alignment) round-trips faithfully; any lossy/dropped attributes are
  documented with severity. Merges/CF recorded OPEN (not a Phase-2 deliverable).

---

### SP6 — GPUI grid maturation  *(macOS, human-run)*

**Questions.** On real hardware, does the raw-gpui grid **hit the §5.4 gates**
(record the numbers Phase-1 left pending)? Can we add the interactions a real grid
needs — **inline cell editing, selection ranges, frozen panes** — without breaking
perf? Does rendering match **known-good PNG baselines**? Can we **remove the GPL
#55470 dependency**?

**Approach.** Mature the `04-ui-poc/raw-gpui` PoC: inline edit, range selection,
frozen panes; wire the in-app **"Run Test"** to record §5.4 frame/cell-load numbers
on the Mac; add **PNG-baseline render-correctness** tests (Mac-run — real GPU
output); apply the **#55470 fix** (patch `sum_tree` to swap
`ztracing::instrument` → `tracing::instrument`) and verify the GPL-3.0 dep is gone
from the tree.

**Deliverables.** `round-2/06-gpui-grid/` (evolving from `04-ui-poc/raw-gpui`):
matured grid + macOS `scripts/`; committed "Run Test" logs (`results/`); PNG
baselines + test harness; `findings.md` with the measured verdict and a
cargo-tree/license check confirming #55470 is resolved.

**Pass criteria.**
- **GATE (measured on Mac, human-run):** "Run Test" reports **frame p99 ≤ 8.3 ms**
  (120 fps) normal / **≤ 16.6 ms** worst-case, and **cell-load p99 < 2 ms**, at
  Excel-max, *with the new interactions present* (not a bare grid).
- **GATE (feel):** human confirms inline edit / selection / frozen panes work and
  scrolling stays smooth.
- **GATE:** PNG-baseline render tests pass (rendering is pixel-correct vs committed
  known-good images).
- **GATE:** patched build has **no GPL-3.0 dependency** in `cargo tree`; documented
  for the pre-distribution legal sign-off.

---

### SP7 — Residual gaps  *(batched; lower priority; cleanup/discovery)*

**Questions & Approach.** Address the smaller carried-forward items in one batched
sub-project, each with a short finding:
- **CSV hardening:** RFC-4180 edge cases for the ~40-line bridge (quotes, embedded
  newlines, delimiters, BOM, huge files) — tests, not just happy path.
- **IronCalc load-API friction:** document the two-step load + four locale/tz/lang
  args; propose a FreeCell wrapper ergonomics.
- **Storage-density extrapolation:** from measured ~162 B/cell, extrapolate RAM for
  realistic populated fractions toward true Excel-max; flag where it becomes
  untenable and what that implies (paging? cap?).
- **Untested recompute shapes** not covered by SP1 (if any surface there).

**Deliverables.** `round-2/07-residual/findings.md` + any small test crates.

**Pass criteria.** **DISCOVERY:** each item addressed with an honest finding; no
hard gates (these are cleanup + extrapolation, not thesis-defining).

---

### Phase-2 Synthesis  *(final; serial)*

**Deliverable.** `experiments/round-2/SYNTHESIS.md`: an updated **build / adjust /
pivot** recommendation for Stage 3, citing SP1–SP7 evidence, explicitly stating
whether the engine off-ramp was triggered, and listing any Round-3 carry-forward.

**Pass criteria.** A defensible, evidence-backed Stage-3 recommendation the human
can act on. A well-evidenced "this condition doesn't hold → adjust/pivot" is a
successful Phase 2.

## 7. Risks & What Could Invalidate the Approach

- **`Model` not `Send` / clone too expensive (SP1).** If IronCalc can't be moved or
  cheaply snapshotted off-thread, the async-recompute architecture — mandatory
  given no incremental recalc — gets much harder. This is the single biggest
  Phase-2 risk; it's SP1's top question.
- **Function parity worse than the raw count suggests (SP3).** 345 registered ≠ 345
  correct; a count is not an audit. Systematic semantic gaps could reopen the engine
  choice.
- **Style API thinner than assumed (SP4).** If band/empty-cell styling isn't in
  IronCalc's public API, the "native styles as source of truth" decision needs a
  scoped side-store after all — a finding, not a silent patch.
- **`.xlsx` open cost (SP2).** OOXML parse + style ingest at 100 MB is un-measured;
  if it's minutes or memory-hungry, it dents the "open huge files" promise.
- **UI numbers only exist on the Mac (SP6).** In-container cannot measure frames;
  the authoritative GPUI pass/fail depends on the human running "Run Test."
- **Scope creep.** The pull to start building the real app is strong. Phase 2 stops
  at measurement + a Stage-3 recommendation.

## 8. Resolved Decisions
- **No UI design step.** The GPUI grid is a maturing PoC/perf rig, not product UI;
  its behavior is specified inline in SP6. (Same as Phase 1.)
- **No merges/CF design.** Explicitly out of scope (overview §2); Phase 2 records
  the gap + scope-trap only.
- **Engine is fixed (IronCalc); no bake-off.** Phase 2 measures IronCalc against the
  bar; Formualizer is the off-ramp reference only.
- **Reuse, don't rewrite, the Phase-1 harness** (frozen `shared/` + `02` trait/
  adapter), so Round-2 numbers stay comparable to Phase-1 baselines.
- **Off-ramp checkpoint after SP1–SP3** (human review), not an up-front gate.
