---
status: complete
---

# Architecture: FreeCell — Phase 1 (Technical De-risking)

This is the technical design for a **research/de-risking** effort, so the
"architecture" is mostly: the experiment workspace, the **agent-swarm
orchestration** that runs the sub-projects, and the **benchmark/measurement
methodology**. Per-sub-project implementation detail is intentionally deferred to
**post-gate phase plans** (see §9), because the specifics depend on what the stack
gate (Sub-project A) discovers about Formualizer's real API.

## 0. Grounding facts (verified in this environment)
- Rust **1.94.1**; container has **4 cores / 15 GB RAM**, **no GPU, no display**.
- **crates.io fetch + build works in-container** (verified).
- **`formualizer v0.7.0` is on crates.io and compiles headlessly** here in ~58 s
  (pulls `formualizer-workbook`, `formualizer-sheetport`). Feature flags observed:
  `eval`, `parse`, `workbook`, `csv`, `sheetport`, `system-clock`, `portable-wasm`,
  and optional `calamine`, `umya`, `js-runtime`, `wasm-js`, `tracing`,
  `tracing_chrome`. The `calamine`/`umya` features are the likely file-I/O and
  style path (umya-spreadsheet preserves styles) — to be confirmed at the gate.
- **GPUI cannot build/run here** (no GPU/display, heavy system deps) → the UI
  sub-project (E) targets **macOS/Metal** and is run by the human.

## 1. Repository & Workspace Layout

Each sub-project is a **self-contained, independent Cargo project** under its own
`experiments/NN-*/` folder — **not** members of a shared Cargo workspace.

**Why independent (not a workspace):** parallel sub-projects must never contend on
shared files. A shared workspace means a shared root `Cargo.toml` (members list)
and a shared `Cargo.lock` + `target/` lock, which **serializes** parallel `cargo
build`s and creates a root-manifest write hazard. Independent projects give each
parallel editor a fully disjoint folder (its own `Cargo.toml`, `Cargo.lock`,
`target/`), so file edits, builds, and `git` path-scoping are race-free. Cost:
some crates (Arrow/Formualizer) compile more than once across projects — acceptable
for an experiments repo.

```
experiments/
  README.md                      # index + how to run everything
  shared/                        # committed once at scaffolding; READ-ONLY to parallel phases
    datagen/                     # synthetic-sheet + sample-file generators (lib crate)
    bench_util/                  # timing/percentile/results-recording helpers (lib crate)
  00-stack-decision/             # Sub-project A (GATE)
    findings.md
    smoke/                       # Cargo project: Formualizer smoke test
  01-file-support/               # Sub-project B
    findings.md  <cargo project>
  02-datamodel-binding-perf/     # Sub-project C
    findings.md  <cargo project(s)>  results/
  03-formatting/                 # Sub-project D
    findings.md  <cargo project>  results/
  04-ui-poc/                     # Sub-project E (macOS only)
    findings.md
    raw-gpui/        <cargo project>
    gpui-component/  <cargo project>
    scripts/         # macOS build/run (one command)
    results/         # logged pass/fail from in-app "Run Test"
  05-round-2-proposal/           # Sub-project F
    round_2_explorations.md
  06-engine-bakeoff/             # Sub-project G (NEW) — engine decision (decision.md)
  SYNTHESIS.md                   # Sub-project H: overall go/no-go + Round-2 pointer
```

`shared/datagen` and `shared/bench_util` are tiny library crates that the
engine sub-projects depend on by **relative path**. They are created and frozen at
scaffolding time so parallel phases only *consume* them (read-only), never edit
them. If a phase needs a change there, it escalates (a shared edit breaks the
parallel-editor invariant).

## 1.1 Engine bake-off (post-gate revision, 2026-07-01)

The Phase 1 gate settled **UI = GPUI** but left the **engine undecided**, so the
engine-dependent experiments are now a **two-engine bake-off** (Formualizer vs
IronCalc):

- `01-file-support/`, `02-datamodel-binding-perf/`, `03-formatting/` each gain
  isolated per-engine subfolders **`formualizer/`** and **`ironcalc/`** (perf also
  gets a `common/` holding the shared engine-abstraction trait + scenario
  definitions). Both engines run against the **same `datagen` inputs, `bench_util`
  metrics, and identical scenarios**, so numbers are directly comparable; each
  folder's `findings.md` is a **head-to-head comparison** (API suitability,
  missing/needed features, perf, fidelity).
- IronCalc gets its own **smoke / API-surface capture** (mirroring Phase 1's
  Formualizer smoke), as the first step of the perf phase.
- New **`06-engine-bakeoff/decision.md`** (Sub-project G) aggregates B/C/D (+ A) into
  a case for each engine + a recommendation → **human engine sign-off**. The final
  synthesis becomes **Sub-project H** (`SYNTHESIS.md`).
- **UI is settled (GPUI, no competitor);** `04-ui-poc/` is unchanged, engine-neutral.

## 2. Agent-Swarm Orchestration

Follows the spec skill's per-phase structure exactly: a **manager** spawns a
**coding** sub-agent and **CR** sub-agent(s), runs the attestation → CR → commit
loop, and never writes code itself. Phase 1's twist is that several of these
per-phase managers run **in parallel**.

### 2.1 Topology
```
Phase-1 Coordinator (top manager)
├─ Phase 0: Scaffolding            (serial) ── DONE
├─ Phase 1: GATE = Sub-project A   (serial; manager→coding→CR) ── DONE → proceed; UI=GPUI; engine=bake-off
│            └─►  HUMAN SIGN-OFF (cleared)
└─ After the gate, launch in PARALLEL (each engine-dependent phase runs BOTH engines):
   ├─ Phase C (02 binding/perf)    manager→coding→CR  ── own review (risky); Formualizer + IronCalc, shared harness
   ├─ Phase E (04 UI PoC, macOS)   manager→coding→CR  ── own review + Mac UI sign-off; GPUI only (engine-neutral)
   └─ Phase BDF (batched review):
        ├─ editor: 01-file-support   (both engines)
        ├─ editor: 03-formatting     (both engines)
        └─ editor: 05-round-2-proposal
   ── then (after B/C/D land) ──
   Phase G: 06-engine-bakeoff      manager→coding→CR  ── own review + HUMAN engine sign-off ── decision.md
   ── then ──
   Phase H: Synthesis              (serial; last) ── SYNTHESIS.md
```
This realizes the **hybrid review-gate** decision: **C**, **E**, and the **engine
decision (G)** are gated individually (G with a human engine sign-off); **B/D/F**
batch into one review. Each lead manager may spawn its own **helper sub-agents** (web
research, per-engine adapters, benchmark iteration) — bounded depth (coordinator →
lead → helpers). Research loops **iterate until targets are met or ~2–3 rounds pass
with no improvement**, then report.

### 2.2 Parallel-editor isolation (REQUIRED — inject into every parallel manager/coding/CR prompt)
Because parallel phases share one branch and working tree, every parallel manager
and its sub-agents get this invariant **verbatim** in their prompt:

> **You are one of several agents editing this repository at the same time.**
> Operate **only** inside your assigned folder `experiments/NN-<name>/` (plus
> read-only use of `experiments/shared/` and the `specs/` docs). **Never** edit the
> repo root, another sub-project's folder, or `experiments/shared/`.
> For every git operation, **scope to your folder only**:
> `git status -- experiments/NN-<name>/`, `git diff -- experiments/NN-<name>/`,
> `git add experiments/NN-<name>/`. **Never** run `git add -A` / `git add .` /
> `git commit -a`. Your CR and attestation cover **only** your folder's diff.
> If you believe you must touch a shared or out-of-folder file, **stop and
> escalate** to your manager instead of editing it.

### 2.3 Commit safety
File **edits** happen in parallel (disjoint folders → no file conflicts). To avoid
`.git/index`/`HEAD` races, **git commits are serialized**: the Phase-1 Coordinator
admits one phase's commit step at a time (each commit is path-scoped to that
phase's folder via §2.2). Because folders are disjoint, serialized path-scoped
commits never conflict.

> **Hardening option (recommended if available):** run each parallel phase in its
> own **git worktree** (`isolation: "worktree"`), branched off the post-gate
> commit, then merge the disjoint branches back. Worktrees give each editor a
> private index/working copy — true isolation — and the disjoint-folder merges are
> conflict-free. The §2.2 invariant still applies inside each worktree. The
> serialized-commit model (above) is the fallback when worktrees aren't used.

### 2.4 Where work runs
| Phase | Builds/runs | Authoritative numbers |
|-------|-------------|-----------------------|
| 0, A, B, C, D, F, G | In-container (headless Rust) | **In-container** (4c/15GB) |
| E (UI PoC) | **macOS/Metal**, run by human | **macOS** (in-app "Run Test") |

(Per the decision: container is authoritative for everything except the UI; the UI
runs on the Mac.)

## 3. Benchmark & Measurement Methodology (engine sub-projects)
- **Harness:** [Criterion] for micro/throughput numbers; small custom timers
  (`shared/bench_util`) for end-to-end latencies where Criterion's model doesn't
  fit (e.g. "edit → cascade → read visible").
- **Latency reporting:** report **p50/p99/max**, not just means, wherever a
  distribution matters (viewport reads, cascade updates).
- **Inputs:** generated by committed code in `shared/datagen` (large synthetic
  sheets, styled `.xlsx`, CSV) — never hand-built binaries; anyone can regenerate.
- **Results:** each engine phase writes machine-readable results to its `results/`
  (JSON + a human-readable `summary.md`), **stamped** with environment
  (CPU/OS/commit), input size, and a relative date passed in (no wall-clock calls
  inside deterministic code).
- **Pass/fail gating:** each benchmark asserts against the §5.4 targets of the
  functional spec and prints `PASS`/`FAIL` with the measured number. "Measure &
  report" metrics (file-load time, memory envelope) record + judge reasonableness
  rather than hard-fail.
- **≥2 designs compared** wherever the brief calls for it (binding patterns;
  raw-gpui vs gpui-component); report the winner with evidence.

## 4. Sub-Project A — Stack Decision (GATE) — designed in full
**Outputs:** `00-stack-decision/findings.md` (ranked recommendation + risks) and a
`smoke/` Cargo project.

**Smoke test (in-container):** add `formualizer` (features `eval,parse,workbook` +
`calamine`/`umya` as needed); programmatically (a) build a small workbook, (b) load
a tiny `.xlsx` and a CSV, (c) read a cell value, (d) set a value and re-evaluate a
dependent cell, (e) inspect what API exists for **range reads**, **bulk/iterator
access**, **update subscription/dirty tracking**, and **styles/metadata**. Capture
the **actual API surface** (method names, types, Arrow exposure) — this is the
input that unblocks the post-gate phase plans for B–E.

**Research (web helpers):** engine landscape (Formualizer maturity, function
coverage vs Excel ~500, file fidelity, license, maintenance/bus-factor, perf
ceiling) and GPU/native-UI landscape (GPUI as a standalone dep, `gpui-component`
as the practical component layer, alternatives). Produce **2–4 alternative stacks,
ranked, with reasoning**.

**Gate:** human signs off on *go (Formualizer + GPUI)* or *pivot*. If pivot, B–F
phase plans are re-scoped to the chosen stack before any parallel work starts.

## 5. Sub-Project C — Datamodel Binding & Engine Perf (design-level)
The core risk. Design the **access pattern**, implement **≥2 candidate binding
designs**, benchmark them:

- **D1 Naive per-cell:** UI pulls each visible cell via the engine's single-cell
  read/eval on every viewport change.
- **D2 Bulk/range:** UI pulls the visible rectangle via a range/iterator API in one
  call; relies on Arrow columnar access.
- **D3 Cached + subscription:** a binding cache holds the visible window; reads hit
  cache; engine edits mark dirty cells and notify the cache for the visible set;
  invalidation keyed on the dependency graph.

**Benchmarks (vs §5.4):**
1. **Scrolling read** — sweep the viewport rapidly across the Excel-max grid;
   per-viewport read p50/p99 (target < ~2 ms).
2. **Cascade → visible update** — edit a cell that cascades (incl. cross-sheet /
   offscreen) → fetch now-visible values; end-to-end p50/p99 within a frame budget.
3. **Cascade throughput** — 1,000,000-cell `=PREV+1` chain; edit head → full
   recompute < 100 ms; plus extra shapes (wide fan-out, cross-sheet, volatile fns).
4. **Writes** — challenge `set_value` cost (single vs batched; recalc triggering).
5. **Memory** — load ~10⁷-cell workbook + edit; peak RSS recorded.

**Deliverable:** recommended binding design + next-best, plus "other perf-critical
areas to validate" — all reproducible.

## 6. Sub-Projects B & D — File Support / Formatting (design-level)
- **B (file):** test Formualizer-native `.xlsx`/CSV read+write via its
  `calamine`/`umya` features first; **fallback** = direct `calamine` (read) +
  `rust_xlsxwriter` or `umya-spreadsheet` (write). Round-trip test: generate →
  load → mutate → save → reload → diff; document what survives. Recommend a design
  + next-best.
- **D (formatting):** probe what styles/metadata Formualizer (and umya underneath)
  exposes (row/col sizes, bold/italic, fills/borders, font size, number formats);
  test whether format edits survive a save. Candidate stores: **native (umya
  styles)** / **side-table keyed by cell** / **custom Arrow-backed store**.
  Recommend + next-best, with the load→edit→save verdict.

## 7. Sub-Project E — GPUI PoC (design-level, macOS)
- **Static datamodel provider:** a `trait CellSource { fn cell(row,col) -> CellData }`
  returning value + formatting, backed by a **deterministic procedural generator**
  (varied text lengths, numbers, ~10–20 % highlighted cells, scattered bold/italic,
  variable row/col widths, some very wide cells) — a proxy for a big, difficult
  sheet. No engine connected.
- **Virtualization:** render only visible cells + overscan. Variable row/col sizes
  handled via **cumulative-size prefix sums + binary search** to map scroll offset
  → visible range. Targets the Excel-max grid.
- **Two variants, compared:** (1) **raw gpui** — custom virtualized grid; (2)
  **gpui-component** — its virtualized table/list (assess whether it does 2D +
  variable sizes; that's part of the finding).
- **"Run Test" harness (in-app):** drives scripted **scroll / fast-scroll /
  horizontal / jump-to-cell / random-jump** sequences by advancing the viewport
  programmatically frame-by-frame; instruments **per-frame render time** and
  **newly-visible-cell load latency**; computes p50/p99/max; **logs to
  `results/`**; prints **PASS/FAIL**: frame p99 ≤ 8.3 ms (120 fps) normal / ≤ 16.6 ms
  (60 fps) worst-case; cell-load p99 < 2 ms.
- **Look:** white bg, grey gridlines, cells with text+borders, fills/bold/italic,
  headers, variable widths. Minimal; agent drives the rest.
- **macOS scripts:** one-command build+run; the human runs interactively (feel) and
  via "Run Test" (numbers), then reports the log. Optional screenshot/known-good-PNG
  render check.

## 8. Cross-Cutting
- **Error handling:** experiments use `anyhow`; a failed/unmet target is a
  **recorded finding**, not a silent skip or a panic that hides the number.
- **Logging:** benchmarks print structured PASS/FAIL + numbers; the UI app logs to
  `results/`.
- **Testing strategy:** engine phases ship correctness tests **and** benchmarks
  (Phase 1 is partly about standing up reusable test/bench infra — a foretaste of
  the product's regression-test goal). For E, the "Run Test" harness *is* the test.
  Render-correctness via known-good PNGs is optional in Phase 1.
- **Dependencies:** engine pinned to `formualizer` 0.7.x (features per phase);
  `criterion` dev-dep; `calamine`/`rust_xlsxwriter`/`umya-spreadsheet` as needed for
  B/D. E pins `gpui` + `gpui-component` via git on macOS.

## 9. Doc Organization (1-phase) & Phase Plans
**Single `architecture.md`, no `components/` dir.** Detailed per-sub-project design
(exact APIs, function signatures, bench parameters) is written into each phase's
**`phase_plans/phase_N.md` by its lead agent *after* the gate**, so it reflects the
real Formualizer API surface the smoke test uncovers. This avoids designing B–E on
pre-gate assumptions.

## 10. Risks (technical)
- **Formualizer API mismatch:** assumed range/subscription/style APIs may not
  exist; the gate smoke test pins reality before B–E commit to a design.
- **gpui-component 2D virtualization:** may only virtualize rows; if columns all
  render, the Excel-max width target needs custom work — a finding, not a blocker.
- **Repeated heavy compilation:** independent projects recompile Arrow/Formualizer
  several times; acceptable, but watch disk/time (mitigate later with sccache if
  needed — not in shared target dir, which would serialize builds).
- **Serialized commits** are a tiny throughput cost vs. the correctness win;
  worktrees (§2.3) remove even that.
