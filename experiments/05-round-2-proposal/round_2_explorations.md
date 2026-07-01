# Sub-project F — Round-2 Technical Exploration Proposal

> Status: **complete** — Phase 6 (functional_spec §6.F; project_overview "Round 2
> technical exploration"). A **ranked** list of follow-up technical explorations to
> de-risk next, synthesized from the committed Phase-1 evidence (Sub-projects A–E) and
> the decisions made at the gate. This is a **synthesis / writing** deliverable — no
> benchmarks were run here; every quantitative claim is cited from a Phase-1 findings
> doc or the engine decision.

## Decisions this list is built on (locked in Phase 1)

Two gate decisions reshape what Round 2 must de-risk:

- **UI = GPUI**, human-confirmed on macOS: a **raw-gpui custom grid** for the sheet,
  with **`gpui-component` for the surrounding chrome** (menus/dialogs/panels).
  Rationale in Sub-project E (`04-ui-poc/findings.md`): `gpui-component`'s `DataTable`
  cannot do variable row heights (`uniform_list`-based) and materializes a per-column
  object at 16,384 cols, so the *grid itself* is raw-gpui; the component library is
  still the fast path for everything around it.
- **Engine = IronCalc** (Sub-project G, `06-engine-bakeoff/decision.md`) — a **human
  decision** that *overrode* that document's soft go-with-conditions lean toward
  Formualizer, prioritizing **delivery risk** (funded team, ~4k stars, 29
  contributors, native styled I/O) over peak huge-sheet performance. This flips the
  whole Round-2 agenda: the risks to de-risk are now **IronCalc's**, not
  Formualizer's. Three validations were **explicitly deferred to Round 2** by the
  decision (`06-engine-bakeoff/decision.md`, HUMAN DECISION section): (1) end-to-end
  large-`.xlsx` open with styles, (2) IronCalc full-`evaluate()` cost at Excel-max
  scale, (3) a function-parity audit of IronCalc's 345 registered builtins. Those
  three lead the ranking below.

**Ranking principle.** Ordered by impact on the eventual go/no-go. The chosen engine's
**core structural weakness — no incremental recalc — ranks first**, followed by the
two other explicitly-deferred validations, then the binding/formatting architecture
FreeCell must build around IronCalc, then UI maturation, then residual A–E gaps.

---

## Ranked explorations

### 1. IronCalc full-`evaluate()` cost at Excel-max scale + the async-recompute UX

**What / why (highest priority — the chosen engine's core weakness).** IronCalc has
**no incremental recalc**: every edit triggers a **full-workbook `evaluate()`**
(Sub-project C: *"No — full-workbook `evaluate()` only"*; Sub-project B confirms the
same on open). On small sheets this is cheap, but it is **O(all cells)** on Excel-max
sheets. The committed evidence already shows the failure envelope: the 1M-cell
`=PREV+1` cascade recomputes in **~2.11 s p50** on IronCalc (vs the §5.4 <100 ms
target — FAIL by ~21×), and even the "cascade → visible" (100k chain) is **107 ms**
(FAIL vs the 16.6 ms frame). Because this fires on *every keystroke-commit*, it is the
single biggest open risk for the engine we chose — a daily-papercut risk, not just a
worst-case one. This is why Sub-project C's recommended binding design already says
*"Move large recomputes off the UI thread… run recompute async, paint a
'recalculating' state, re-prime the viewport on completion."* Round 2 must **measure
the cost surface and design the UX that hides it.**

**Rough approach.**
- Sweep edit→recompute latency across two axes: **sheet size** (10⁴ → 10⁷ populated
  cells) and **formula density / DAG shape** (mostly-literal; sparse independent
  formulas; wide `=SUM(range)` fan-out — IronCalc did this in **77.5 ms** for
  1,000×1,000, a bright spot; deep serial chains — its weak shape; cross-sheet;
  volatile `NOW()`/`RAND()`). Reuse the frozen `shared/datagen` + `shared/bench_util`
  harness and the `SpreadsheetEngine` trait from `02-datamodel-binding-perf/` so
  numbers stay comparable to Phase 1.
- Prototype the **off-thread + debounced recompute** binding: batch edits, run
  `evaluate()` on a background thread against a snapshot/clone of the `Model`,
  **debounce** rapid typing so N keystrokes coalesce into one eval, and cancel/supersede
  an in-flight eval when a newer edit lands. Measure the debounce window vs
  perceived-latency trade-off, and how stale the visible viewport is allowed to get.
- Design and prototype the **"recalculating" UX state**: keep the last-good values
  painted (IronCalc *persists cached results*, per Sub-project B, so stale-but-valid
  values are always available), show a non-blocking progress indicator, and re-prime
  the viewport (D2 bulk read) on completion. Confirm the UI thread never blocks on
  `evaluate()`.
- Probe whether IronCalc can be made cheaper: does re-evaluating only after a batch
  (not per-cell) already help? Is there any snapshot/partial-eval seam upstream, or is
  an upstream incremental-recalc contribution the only real fix? Record the answer as
  an engine-roadmap input.

**What it de-risks.** The one §5.4 target the chosen engine structurally fails
(1M-cell cascade <100 ms) and the per-edit interactivity of the whole product. It
turns "IronCalc re-evaluates everything on every edit" from an unbounded fear into a
measured cost with a concrete mitigation (async + debounce + recalculating state) —
or, if the numbers are worse than the ~2 s Phase-1 point sample at higher density,
surfaces it as a genuine go/no-go concern while there is still time to weigh an
upstream fix.

### 2. End-to-end large-`.xlsx` open (time + peak RSS, *with styles*) on IronCalc

**What / why.** This closes **functional_spec §5.4's only un-run target** — *"Open a
100 MB+ `.xlsx`; record load time and peak memory"* — which Sub-projects B and C both
explicitly left open (B: *"we also did not stress a 100 MB+ file"*; C measured Arrow
*ingest*, not file open). The engine decision calls this gap out directly: IronCalc's
huge-sheet numbers we have are the **10M-cell `set_user_input` bulk load (6.13 s /
1.63 GB, ~162 B/cell)** — that is *in-memory ingest*, **not** an end-to-end `.xlsx`
open, which additionally pays **OOXML unzip + XML parse + shared-strings** cost and a
parse-time peak-RSS spike that was *"never benchmarked at scale for either engine"*
(`06-engine-bakeoff/decision.md`). And critically it must be measured **with styles
loaded**, because IronCalc reads styles natively on the same path (Sub-project B/D) —
so its file-open cost is inseparable from the styled-read cost that Formualizer would
have deferred to a side umya load.

**Rough approach.**
- Generate a **real 100 MB+ styled `.xlsx`** from committed code (extend `datagen` with
  an OOXML writer, or drive umya/`rust_xlsxwriter`): millions of populated cells, a
  realistic ~10–15% styled (fills/bold/number-formats per datagen's own model),
  multiple sheets, shared strings, some formulas. Keep it regenerable (§5.3 — no
  hand-made binary fixtures).
- Time `import::load_from_xlsx_bytes` + `Model::from_workbook` end-to-end; capture
  **peak RSS** from a **fresh process** (the clean-measurement method Sub-project C
  established) and the load-attributable delta. Break out the cost: unzip vs XML parse
  vs shared-strings vs style ingest vs graph build vs (mandatory-or-not) first
  `evaluate()`. Recall IronCalc *persists cached results*, so values can paint before
  any recompute — measure time-to-first-paint separately from time-to-fully-recomputed.
- Report against §5.4's "seconds not minutes / memory a sane multiple of file size"
  discovery bar; scale to the ceiling the 15 GB box (or the Mac) allows.

**What it de-risks.** The last unmeasured §5.4 metric, and the specific
IronCalc-flavored version of it (styled native open, full-workbook eval on open). It
answers "does opening a real big workbook take seconds or minutes, and does peak memory
blow the RAM envelope?" — a question the "stupid-fast on huge sheets" thesis cannot
ship without, and one where IronCalc's ~162 B/cell density (~9× less dense than the
Arrow alternative we passed on) makes the memory answer genuinely uncertain at
Excel-max.

### 3. Function-parity audit — IronCalc 345 builtins vs Excel ~500

**What / why.** *"Excel compatibility required"* is a headline product promise
(project_overview), yet the only number we have is a **raw registry count**: IronCalc
exposes **345 distinct registered builtins** (`06-engine-bakeoff/decision.md`,
source-counted from the `Function` enum) against Excel's ~500 — and **neither IronCalc
nor Formualizer publishes a per-function parity matrix**. The decision doc is explicit
that a *raw count is not parity*: *"A registered function can still differ from Excel
on edge cases, error semantics, locale, or array behavior."* The decision even names
this audit as something that *"could move this recommendation."* With IronCalc now
chosen, we need to know **which functions are missing** and **how Excel-correct the
345 actually are**, because this axis was Formualizer's main non-perf edge (410 raw)
and we accepted a lower count on maturity grounds — that trade must be validated, not
assumed.

**Rough approach.**
- **Coverage diff:** enumerate the 345 registered IronCalc functions vs a canonical
  Excel function list; produce the missing set and triage it by importance (is
  XLOOKUP/XMATCH there? dynamic arrays FILTER/UNIQUE/SORT with spill? LET/LAMBDA?
  common financial/stat/text functions?).
- **Correctness harness:** build a golden-file test suite (a foretaste of the
  product's Excel-compatibility test strategy from project_overview) — inputs +
  Excel-computed expected outputs — covering **edge cases, error semantics**
  (`#DIV/0!`, `#N/A`, `#VALUE!`, error propagation), **locale/date** behavior, and
  **array/spill** semantics for a representative slice of the 345. Compare IronCalc's
  results cell-by-cell.
- Record failures as the seed of the growing regression suite (project_overview:
  *"add tests for every bug fix"*). Note anything that would need an upstream IronCalc
  contribution vs a FreeCell-side shim.

**What it de-risks.** The core "Excel compatibility" product promise on the chosen
engine, and validates the maturity-over-count trade the human made at the gate. A bad
result here is the kind of finding that reopens the engine choice (per the decision
doc's own "what would change the recommendation") — better to know in Round 2 than
post-commit.

### 4. Binding layer for IronCalc specifically

**What / why.** Sub-project C benchmarked the binding *designs* (D1/D2/D3) on both
engines, but the chosen engine has **two thin spots** that need IronCalc-specific
validation at product scale:

- **No native range read** — the viewport is served by a **per-cell loop hidden in the
  adapter**. It *did* clear the target (D2 **392 µs p99 < 2 ms**), but that was an
  ~1,800-cell window over *values only*. Round 2 must confirm it still holds **<2 ms
  under real formatting** — i.e. when each visible cell also resolves a style
  (`get_style_for_cell`) and a rendered/number-formatted string — across the full
  overscan window the raw-gpui grid actually paints, at Excel-max scroll positions.
- **The change feed is IronCalc's `UserModel` diff-list** (Sub-project A/C: a *poll*
  model, collab-sync-oriented), and like Formualizer's changelog it reports **only edit
  sites, not downstream-dirty cells** — which is exactly why D3 (cached) *cannot beat
  D2 on a cascade* in Phase 1. The cache-invalidation strategy for visible cells must
  be designed and validated against IronCalc's actual diff-list shape.

**Rough approach.**
- Re-run the viewport-read benchmark on IronCalc with the **full render pull**
  (value + `Style` + formatted text) per visible cell, at the real overscan size and at
  deep scroll offsets, and confirm p99 < 2 ms (or find the size where it breaks).
- Prototype the cache-invalidation loop against `UserModel`'s diff-list: after a batched
  edit, drain the diff, and (since it's edit-sites-only) re-prime the visible rectangle
  with a D2 bulk read (`BindingCache::refresh_after_edits` from Phase 1). Measure
  whether a thin steady-state D3 cache is worth keeping on IronCalc or whether pure D2
  is simpler for equal result (Phase 1's next-best alternative).
- Feed forward the "downstream-dirty subscription" as a wish-list item: it's the only
  thing that would let a cache beat bulk re-read on a cascade — record whether IronCalc
  could expose it upstream.

**What it de-risks.** That the viewport read budget (<2 ms/frame) survives contact with
**real formatting and real scale** on an engine that has *no native range read*, and
that cache invalidation is correct against IronCalc's specific change-notification
model — the two binding facts that Phase 1 measured only in simplified form.

### 5. FreeCell `FormatStore` design + prototype (engine-neutral side-table)

**What / why.** Sub-project D's decisive recommendation is a **FreeCell-owned,
engine-neutral `FormatStore`** (interned `StyleId → Style` + sparse `(row,col) →
StyleId` + row/col band maps + a merges list) as the app's render source of truth,
*regardless of engine*. On IronCalc it is **not strictly mandatory** (native styles
exist) but it is **still recommended** — D's exact reasoning: it *"decouples FreeCell's
style vocabulary from a 0.x engine's `Style` churn"* (IronCalc is 0.7.1, pre-1.0, its
`Style` API can move), *keeps the model swappable*, and lets the GPUI datamodel
provider read styles *"directly from FreeCell memory without a per-cell engine
round-trip on the render hot path"* — which ties directly into exploration #4's render
pull. D flagged one interaction it did **not** work out: **row/col insert/delete must
shift both** the engine's addresses and the store's keys in lockstep.

**Rough approach.**
- Prototype the interned side-table shape D specified; populate it on load from
  IronCalc's native `Style` (and, on the file layer, keep the umya/native writer as the
  save adapter so styles round-trip), and serve the render hot path from it (no per-cell
  engine call while scrolling — validate this against exploration #4's <2 ms budget).
- Work out the **row/col insert-delete semantics**: define how a row insert re-keys the
  sparse `(row,col)` map and shifts the band maps + merges, and keep it consistent with
  the engine's own address shift. This is the concrete gap D named; get it right before
  it's load-bearing.
- Confirm the store handles the sparse/repetitive reality datagen models (~10–20%
  styled) with aggressive interning, and measure its memory so it doesn't quietly
  reintroduce the density cost IronCalc already has.

**What it de-risks.** The formatting architecture the whole app renders through, its
isolation from IronCalc's pre-1.0 `Style` churn, and the specific unsolved
insert/delete-shift interaction — turning D's design from paper into a validated
component.

### 6. Style read→write roundtrip fidelity + merges / conditional-formatting side-store

**What / why (carried forward from the gate's Captured Notes — kept).** This was
explicitly tagged a **Round-2 experiment** by the human at the gate (see Captured Notes
below) and remains one. Sub-project D deliberately did a *capability probe*, not an
*exhaustive fidelity sweep*: it proved IronCalc round-trips a representative attribute
set (bold/italic/font-size/fill/number-format/borders/alignment/row-col-size) through
real `.xlsx`, but *"exact preservation of the long tail (all border styles,
theme/indexed colours, every number-format code, rich text) across round-trips is
unproven."* Two hard gaps sit alongside it: **IronCalc has NO public API for merged
cells and NO conditional-formatting API at all** (D probe-locked both as `None`; the
methods don't even compile) — so both need a **FreeCell side-store**, and that
side-store's design + persistence is unbuilt.

**Rough approach.**
- **Fidelity sweep:** load a richly-styled `.xlsx` (long-tail attributes: every border
  style, theme/indexed/RGB colours, the full number-format-code space, rich text,
  alignment variants), edit formatting, save, reload, and diff attribute-by-attribute —
  recording exactly what survives IronCalc's native path and what silently degrades.
  This is the hands-on roundtrip validation D's design section only asserted.
- **Merges + conditional formatting side-store:** design the FreeCell-owned store for
  the two things IronCalc cannot represent (merges list + CF rules), including how they
  serialize on save (IronCalc's writer won't carry them — need a umya/OOXML
  post-process or upstream work) and how CF rules evaluate against the render pipeline.
  Verify write fidelity of a mutate-and-persist cycle (D marked umya CF write fidelity
  `Unverified`).

**What it de-risks.** Whether "we open and re-save your styled workbook without
losing formatting" holds beyond the happy path, and the two Excel formatting features
(merges, conditional formatting) the chosen engine simply does not support — both of
which a credible spreadsheet needs.

### 7. GPUI grid maturation + rendering-regression baseline

**What / why.** Sub-project E is **code-complete but its measured verdict is still
pending the human's Mac run** — the §5.4 frame (p99 ≤ 8.3 ms) and cell-load (p99 <
2 ms) gates have **no recorded numbers yet** (the GPUI crates can't build in the
headless container). Getting those quantitative "Run Test" numbers is the first
unfinished piece. Beyond that, the PoC is a *perf rig*, not a grid: it has no in-cell
editing, no selection ranges, and no frozen panes — all of which change the render and
hit-testing hot paths and could move the frame budget. And project_overview makes
**PNG rendering-baseline tests** a first-class product goal (*"rendering tests compare
to known-good PNG… highlight/bold/italic, and 100 combinations"*); E flagged this as an
optional foretaste — Round 2 should establish the approach.

**Rough approach.**
- **Close out E's measured verdict first:** run the existing `scripts/run_test.sh both`
  on a Mac, record the raw-gpui vs gpui-component frame p50/p99/max + cell-load + the
  three PASS/FAIL gates into `04-ui-poc/results/`, and confirm the raw-vs-component
  recommendation (or surface a surprise, per E's provisional note).
- **Mature the raw-gpui grid** toward a real spreadsheet: in-cell editing (an editor
  overlay — `gpui-component` has no built-in inline edit, per Sub-project A), multi-cell
  **selection ranges** (drag/shift/ctrl), and **frozen panes** (frozen rows *and*
  columns — note `gpui-component`'s freeze is left-columns-only, another reason the grid
  is raw-gpui). Re-run the Run Test with these features live to confirm the frame budget
  survives the extra per-frame work, especially text-shaping under fast horizontal pan
  (E's named frame-time risk).
- **Rendering-correctness PNG baselines:** stand up golden-PNG comparison for
  representative cells (highlight/bold/italic/alignment/number-format combinations) —
  the product's rendering-regression strategy in miniature.

**What it de-risks.** Whether GPUI actually hits the §5.4 render bar (still *measured,
not vibes*, and still unrecorded), whether the real grid features fit the frame budget,
and it bootstraps the rendering-regression testing the product is committed to.

### 8. Residual A–E gaps (rolled up)

Smaller items surfaced across A–E, worth a Round-2 pass but below the majors above:

- **GPUI/Zed coupling + GPL-linkage sign-off (Sub-project A).** The real ecosystem
  **git-pins a specific Zed commit** (pre-1.0, no semver, ~8-month crate lag); pin a
  known-good rev and budget periodic rev-bump upgrades. Separately, **issue #55470**:
  a default build statically links **GPL-3.0** object code via `gpui → sum_tree →
  ztracing` (a runtime no-op with a trivial fix, but present) — **needs legal sign-off
  and a merged-fix check on the pinned rev before shipping a proprietary binary.**
- **CSV bridge (Sub-project B).** IronCalc ships **no CSV**; Phase 1 wrote a ~40-line
  RFC-4180 bridge over `set_user_input` / `get_formatted_cell_value`. Round 2 should
  harden it (quoting/encoding/large-file streaming) into a FreeCell-owned CSV layer.
- **IronCalc load-API friction (Sub-project B).** The two-step load takes **four
  locale/tz/language args** that must be threaded consistently across build/reload
  (a mismatch shifts formatting/parse behavior), and the path-based `save_to_xlsx`
  **refuses to overwrite** (must use the writer form). Nail down the canonical
  save/load policy so formatting doesn't drift.
- **Recompute shapes not yet exercised (Sub-project C).** Phase 1's shapes were serial
  chain + SUM fan-out. Round 2 should also probe **cross-sheet cascades, volatile
  functions** (`NOW`/`RAND`/`OFFSET`), and — since IronCalc is single-threaded — confirm
  there is no parallel-eval escape hatch (there isn't, per C), which reinforces #1's
  off-thread strategy.
- **IronCalc storage-density headroom (Sub-project C/decision).** At ~162 B/cell
  (~9× less dense than the Arrow alternative), the memory envelope toward true
  Excel-max (1,048,576 × 16,384) is the structural cost we accepted; #2's peak-RSS
  numbers should be extrapolated to that ceiling to confirm it stays within a sane RAM
  envelope on target hardware.

---

## Captured notes & resolved history (from the Phase-1 gate, 2026-07-01)

Preserved from the human review at the stack-decision gate, updated with their
Round-2 disposition.

- **[Round 2 — LIVE, now exploration #6] Style read → write roundtrip fidelity
  experiment.** Explicitly a *next-round* experiment (per human): load a styled
  `.xlsx`, edit formatting, save, reload, and verify which styles survive (bold/italic/
  fills/borders/number formats/merges/conditional formatting/themes). Phase 1
  (Sub-project D) did the *design* and a representative capability probe; the hands-on
  **roundtrip-fidelity** validation was deferred — it is **exploration #6** above
  (now anchored on the chosen engine, IronCalc, whose native styled path replaces
  Formualizer's umya-double-load model). Kept, not lost.

- **[Was "proposed for this round" — now DONE] Parallel IronCalc engine evaluation
  (the engine bake-off).** At the gate the human leaned toward adding a parallel
  hands-on evaluation of **IronCalc** alongside Formualizer. That was **confirmed and
  executed**: it became the two-engine bake-off across Sub-projects B/C/D and the
  synthesis in **Sub-project G** (`06-engine-bakeoff/decision.md`), which the human
  signed off with **engine = IronCalc**. So this note is **resolved history, not a
  live Round-2 item** — its output is precisely why this Round-2 list is
  IronCalc-specific. Recorded here so the provenance isn't lost.
