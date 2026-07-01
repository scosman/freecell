---
status: complete
---

# FreeCell — Phase 2 (Round-2 Technical De-risking)

> **This document is a self-contained handoff.** It is written for an agent starting
> with **no prior conversation context**. It carries the decisions, evidence, agenda,
> file map, and working conventions you need to run Phase 2. Read it fully before
> doing anything. Phase-1 detail lives under `experiments/` and
> `specs/projects/freecell-phase-1/` — pointers are in §6.

---

## 0. How to start (for the picking-up agent)

1. Read this whole file.
2. Skim the Phase-1 capstone: `experiments/SYNTHESIS.md` (the go/no-go), and
   `experiments/06-engine-bakeoff/decision.md` (why IronCalc; and its "what would
   change the recommendation" off-ramp).
3. Skim the ranked agenda source: `experiments/05-round-2-proposal/round_2_explorations.md`.
4. Then run the spec flow for **this** project (`freecell-phase-2` is already the
   active project): write the **functional spec** (what each Round-2 experiment
   validates + pass criteria), then **architecture** (how; reuse the Phase-1 harness),
   then the **implementation plan**, then implement. Do **not** skip the functional
   spec — Round-2 needs explicit success criteria per experiment.
5. Honor the **locked decisions** (§2) and the **working conventions** (§7). Several of
   those conventions were learned the hard way in Phase 1 — ignoring them wastes large
   amounts of time/tokens.

---

## 1. What FreeCell is (the product)

A **better spreadsheet app**: GPU-rendered (à la Zed/Ghostty), Rust, resource-efficient
on huge spreadsheets, with **Excel compatibility** as a documented/tested feature set
that grows without regressions, **deep testing** (a test suite that grows with every
bug/feature; rendering tests vs known-good PNGs), and a great, approachable UI (menus,
dark mode). North star: **stupid-fast on huge sheets**, Excel-max = **1,048,576 rows ×
16,384 cols**.

The build is **agentic + spec-driven** in stages: **Stage 1** = validate core technical
questions (done — see §3). **Stage 2** = validate more technical uncertainty (**this
project**). **Stage 3** = decide whether to keep going / start building the real app.
No app exists yet; all work so far is experiments.

## 2. Locked decisions from Phase 1 (do NOT relitigate; build on these)

- **Engine = IronCalc** (`ironcalc` / `ironcalc_base` 0.7.x, MIT/Apache-2.0). A human
  chose it over Formualizer on **delivery-risk** grounds (funded team ~4k stars vs
  Formualizer's single-author 0.x; native styled `.xlsx` I/O; persists cached formula
  results). **Its known costs, which drive the Round-2 agenda:** no incremental recalc
  (every edit = full-workbook `evaluate()`), no native range read (per-cell viewport
  loop), no CSV, no merges/conditional-formatting API, nested-`HashMap` storage
  (~162 B/cell, ~9× less dense than Formualizer's Arrow). **Off-ramp:** if Round-2's
  large-`.xlsx`-open or function-parity results disappoint, the engine choice can be
  revisited (see `06-engine-bakeoff/decision.md` §4). Formualizer remains the
  documented alternative.
- **UI = GPUI** (Zed's framework, Apache-2.0), macOS/Metal primary. The **grid** is a
  **custom raw-gpui** widget (gpui-component's `DataTable` can't do variable row heights
  and materializes per-column at 16k cols); **gpui-component** (longbridge, Apache-2.0)
  is kept for **app chrome** (menus/dialogs/panels). Human-validated on a Mac ("worked
  great"). ⚠️ **Legal:** a GPL-3.0 transitive dep (`gpui → sum_tree → ztracing`/`zlog`/
  `ztracing_macro`, all GPL-3.0-or-later; Zed issue #55470) is statically linked; it's a
  runtime no-op with a trivial fix (swap `ztracing::instrument` → `tracing::instrument`
  in `sum_tree`), **still open upstream** — must be removed/patched + legal-signed-off
  before distributing a proprietary binary. Not a blocker for experiments.
- **Formatting = IronCalc's native style storage is the source of truth.** IronCalc
  reads / writes / round-trips per-cell styles (bold/italic/underline, font
  size/color/name, fill, border, alignment, number format) + row/col sizing natively
  through `.xlsx` (probe-backed, Phase-1 Sub-project D). Save/load is the engine's own
  API — **no FreeCell `FormatStore` side-table, and no style-adapter layer.** (Don't
  build indirection for 0.x `Style` churn that may never happen; add a thin adapter
  only if/when IronCalc actually breaks us.) *This reverses the Phase-1 Sub-project-D
  recommendation, which proposed an engine-neutral side-table **"regardless of which
  engine wins"** — that hedge was load-bearing only while the engine was undecided and
  because Formualizer surfaced no styles at all; both premises died when IronCalc was
  chosen, and a duplicate authoritative store just buys sync-on-edit + flush-on-save
  complexity and extra memory for no remaining benefit.*
  **The one gap: merged cells + conditional formatting have no public IronCalc API.**
  These are **major features, left OPEN — not designed here.** If pursued, each needs
  its own technical design. ⚠️ **Scope trap:** persisting either almost certainly means
  FreeCell taking over `.xlsx` **writing** entirely (IronCalc's saver won't emit what
  its API can't model), which ~10×'s scope — gate hard before committing to either.

## 3. Phase-1 outcome & the evidence you inherit

**Verdict: GO, with conditions.** The everyday case is proven fast; the extremes are
credible-by-design but unmeasured. Key numbers (4-core Xeon Linux box, in-container;
Mac is faster; UI perf not measured in-container):

- **Viewport read (scroll path): PASS both.** Best design (D2 bulk) p99 = **392 µs
  IronCalc** / 222 µs Formualizer vs **<2 ms** target. IronCalc clears it *without* a
  native range API (per-cell loop).
- **1M-cell `=PREV+1` cascade recompute: FAIL both.** ~**2.11 s IronCalc** / 1.87 s
  Formualizer vs **<100 ms** target. Inherently serial; **async off-thread recompute is
  mandatory regardless of engine.**
- **Fan-out (1000×1000) recompute:** IronCalc **77.5 ms** vs Formualizer 3.51 s (IC
  wins wide shapes).
- **10M-cell load:** IronCalc 6.13 s / 1.63 GB (~162 B/cell) via `set_user_input`;
  Formualizer 1.73 s / 0.18 GB via Arrow bulk-ingest. **Both reach 10⁷ cells.** NOTE:
  this is *in-memory ingest*, **not** an end-to-end `.xlsx` open (that is un-measured —
  Round-2 #2).
- **File I/O:** both round-trip values/formulas/sheets/dates. IronCalc = native styled
  writer + **persists cached results** (values paint on reload before recompute), but
  **no CSV** (a ~40-line RFC-4180 bridge was written). Formualizer = first-class CSV but
  drops cached results + no styles on read.
- **Formatting:** IronCalc reads/writes styles natively (bold/italic/size/fill/border/
  align/num-fmt, survive `.xlsx` round-trip; probe-backed). **No merges/CF API.**
- **Function coverage:** IronCalc **345** registered builtins (source-counted) vs
  Excel's ~500. Raw count, **not** a parity audit.
- **GPUI PoC:** custom raw-gpui virtualized grid + gpui-component variant built; the
  engine-neutral `poc-core` (virtualization math + perf harness) is **CI-tested
  in-container (20 tests)**; human confirmed it renders/scrolls well on macOS. The
  numeric §5.4 "Run Test" gates were **not yet recorded** on a Mac.

**A Phase-1 process lesson baked into the evidence:** an early Sub-project-C draft
reported a *backwards* conclusion (Formualizer "can't scale") that was an artifact of
using the wrong load API; **adversarial review caught and reversed it before it drove
the engine decision.** Keep that discipline (§7).

## 4. Phase 2 goal

De-risk the **conditions on the GO** — i.e. validate (or falsify) the still-open
questions before committing to build the real app — with reproducible evidence.
Success = each agenda item (§5) answered with committed benchmarks/tests + a findings
doc, and a firmed-up recommendation for Stage 3 (proceed to real-app build / adjust /
pivot per the off-ramp). A well-evidenced "this doesn't hold" is a successful Phase 2.

## 5. The Round-2 agenda (ranked; source: `05-round-2-proposal/round_2_explorations.md`)

Detailed approaches are in that doc — read it. In priority order:

1. **IronCalc full-`evaluate()` cost at Excel-max + the async-recompute UX** *(highest —
   the chosen engine's core weakness).* Every edit fires a full-workbook `evaluate()`
   (O(all cells); 1M cascade ~2.11 s, FAIL). Sweep edit→recompute latency by **sheet
   size (10⁴→10⁷) × formula density / DAG shape** (literal, sparse, wide fan-out,
   deep-serial, cross-sheet, volatile). Prototype **off-thread + debounced + cancellable
   recompute** against a `Model` snapshot, with a "recalculating" UX that keeps
   IronCalc's persisted cached values painted. Confirm the UI thread never blocks.
2. **End-to-end large styled `.xlsx` open** *(closes §5.4's one un-run target).* Generate
   a real 100 MB+ styled `.xlsx` from committed code; measure open time + **peak RSS from
   a fresh process**, broken out (unzip / XML parse / shared-strings / style ingest /
   graph build / first eval). Measure time-to-first-paint (cached values) separately.
3. **Function-parity audit** *(Excel-compat is the headline promise, least-proven).*
   IronCalc's 345 registered builtins vs Excel ~500: coverage diff + a golden-file
   Excel-correctness harness (edge cases, `#DIV/0!`/`#N/A` error semantics, locale/date,
   array/spill). Could reopen the engine choice.
4. **IronCalc binding layer + native style read at scale.** Confirm the per-cell
   viewport loop holds <2 ms with **real formatting**: **add `get_style_for_cell` to the
   viewport benchmark** and confirm reading value + style per visible cell in the loop
   stays under budget at scale (styles are now read straight from IronCalc — §2). **Also
   validate IronCalc's style API actually exposes what FreeCell needs** — per-cell
   attributes, row/col **band** styles, and **empty-cell** styling (verify, don't assume;
   if band/empty-cell styling is thin, that's a finding that reopens the formatting
   design). Design cache invalidation against IronCalc's `UserModel` diff-list
   (edit-sites-only, no downstream-dirty).
5. **Long-tail style-roundtrip fidelity** — exact colors / border styles / number-format
   codes / rich text across `.xlsx` round-trips (Phase-1 probed representative attributes
   only). **Merges + conditional formatting stay OPEN** (see §2): no IronCalc API — each
   is a major feature needing its own technical design **and** the "take over `.xlsx`
   writing" scope gate; **not designed in Phase 2.**
6. **GPUI grid maturation:** inline cell editing, selection ranges, frozen panes; **record
   the still-pending §5.4 "Run Test" numbers on a Mac** (frame p50/p99 + PASS/FAIL);
   rendering-correctness **PNG baseline** tests; **resolve GPL #55470**.
7. **Residual gaps:** CSV hardening, IronCalc load-API friction, untested recompute
   shapes, storage-density extrapolation to true Excel-max.

## 6. Where everything lives (file map)

- **Phase-1 specs:** `specs/projects/freecell-phase-1/` — `project_overview.md`,
  `functional_spec.md` (esp. **§5.4 perf targets**, §5.2 findings-doc format, §5.5
  success), `architecture.md` (esp. **§2 swarm orchestration + parallel-editor
  isolation**, §3 benchmark methodology), `implementation_plan.md`, `phase_plans/`.
- **Phase-1 experiments:** `experiments/`
  - `shared/datagen` + `shared/bench_util` — **frozen, reuse read-only** (committed
    generators; env-stamped p50/p99 stats + PASS/FAIL gating).
  - `02-datamodel-binding-perf/` — the **`SpreadsheetEngine` trait + IronCalc &
    Formualizer adapters + the scenario harness**. Round-2 perf work should extend this,
    so numbers stay comparable to Phase-1 baselines. `results/` has committed JSON.
  - `01-file-support/`, `03-formatting/` — engine file/format adapters + findings.
  - `04-ui-poc/` — `poc-core` (GPUI-free, tested), `raw-gpui/`, `gpui-component/`,
    `scripts/` (macOS build/run), `findings.md` (has the "HUMAN RUN REQUIRED" ask).
  - `00-stack-decision/` — the engine/UI research + Formualizer API smoke.
  - `06-engine-bakeoff/decision.md`, `05-round-2-proposal/round_2_explorations.md`,
    `SYNTHESIS.md`.

## 7. Working conventions (READ — several are hard-won)

- **Environment:** headless Linux container, **4 cores / ~15 GB RAM, no GPU / no
  display**, Rust 1.94+. crates.io works. GitHub access scoped to `scosman/freecell`.
  Develop on your session's designated feature branch; **commit + push regularly**
  (a stop-hook nags on uncommitted work; the container is ephemeral). The Phase-1
  artifacts are on branch `claude/freecell-spreadsheet-spec-f2qhzs` — make sure they're
  present on your branch before building on them.
- **Engine/benchmarks run in-container and are authoritative** (4-core numbers are a
  floor). **UI (GPUI) cannot build in-container** (no GPU/system libs) — write code +
  macOS build scripts and have the **human run it on a Mac and report numbers/feel**.
  Don't fight the GPUI build in-container.
- **Perf targets (§5.4 "Excel-max & buttery"):** grid 1,048,576 × 16,384; scroll 120 fps
  (~8.3 ms frame, ≤16.6 ms worst case); load newly-visible cells < ~2 ms; 1M-cell cascade
  recompute < 100 ms *(note: FAILs today — the point is the async UX)*; open 100 MB+
  `.xlsx` in seconds with sane peak memory. Treat as goals to measure toward + honestly
  grade.
- **Experiments structure:** each experiment in its **own folder**; **independent Cargo
  projects (NOT a shared workspace)** so parallel editors never contend on a root
  manifest/lockfile/target lock; depend on `shared/*` by relative path; each experiment
  emits a `findings.md` (functional_spec §5.2 headings) + committed `results/`. `target/`
  is gitignored repo-wide.
- **Spec-driven + agent swarm (see phase-1 `architecture.md` §2):** manager → coding
  sub-agent → CR sub-agent per phase. **Parallel-editor isolation:** each phase edits
  only its folder; path-scoped git (`git add <folder>`, never `git add -A` from a
  parallel worker); serialize commits.
- **Benchmark discipline (Phase-1 failure modes to avoid):**
  - **Run benchmarks FOREGROUND with `timeout`.** Do **NOT** use `nohup`/`&`/background
    monitors/waiters — a Phase-1 agent flailed for ~57 min polling a background monitor
    and burned ~600k tokens. If a run is slow, cap the scale and record the ceiling as a
    finding; never detach-and-poll.
  - **Separate build/load time from the measured operation;** build via each engine's
    **best** API (using the wrong ingest API produced a backwards conclusion once).
  - **Force + assert the measured op** (e.g. mutate head → recompute → assert the tail
    value) so a benchmark can't silently measure a no-op or cached read.
  - Use the **shared harness + identical scenarios** across variants so numbers are
    comparable; report **p50/p99**, environment-stamped.
  - **Adversarially review surprising results** before they drive a decision — a
    too-good or too-bad number usually means the harness is wrong, not the engine.
- **Findings must be honest:** grade against targets; call out what's un-measured or
  capped; a registered-function *count* is not a parity *audit*; a "feel" check is not a
  numeric gate.

## 8. Tech references
- IronCalc: https://github.com/ironcalc/IronCalc (crates: `ironcalc`, `ironcalc_base`)
- GPUI: in https://github.com/zed-industries/zed (git-pinned; pin a known-good rev)
- gpui-component: https://github.com/longbridge/gpui-component
- (Alternative engine, if the off-ramp triggers) Formualizer:
  https://github.com/psu3d0/formualizer
