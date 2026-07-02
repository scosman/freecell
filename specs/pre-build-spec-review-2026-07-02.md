---
status: complete
type: review
date: 2026-07-02
scope: Full planning corpus — specs/projects/freecell-phase-{1,2,3}, experiments/ (Phase 1, round-2, round-3), PROJECTS.md, projects/. Code excluded (throwaway).
---

# Pre-Build Spec Review — Did We End Up in a Good Place?

Independent review of the FreeCell de-risking corpus at the "CLEAR TO BUILD"
checkpoint. Four parallel review passes were run over the documents (decision audit,
spec coherence, blind-spot hunt, over-engineering hunt); this report is their
synthesis. Claims about specific documents were spot-verified against the sources.

## 1. Verdict

**Yes — you ended up in a good place, with one systematic weakness and one real gap.**

- **The engine-side de-risking is genuinely excellent.** Almost every adopted
  architectural decision is backed by direct measurement, often with negative
  controls, and the process repeatedly chose the *boring* option after measuring
  (dense prefix-sum over Fenwick, D2 bulk-read over the D3 cache, overscan over
  snapshots, no `FormatStore`). Over-engineering is minimal (~80/20
  useful-derisking to waste), and the waste sits almost entirely in cheap throwaway
  experiments, not in adopted architecture — the right place for it.
- **The systematic weakness: everything requiring a Mac decayed from "measured, not
  vibes" to vibes.** The product's headline claim — 120 fps GPU rendering at scale —
  has **never produced a recorded number in three rounds**, and later documents
  progressively inflated "not yet recorded" into "GPUI is validated" into
  human-validated "stellar." This is the inverse of the project's own methodology.
- **The real gap: the corpus proves a fast *viewer-recalculator* of synthetic files
  is buildable. It says almost nothing about the *product*.** No target user, no MVP
  cut-line, no platform/license posture — and, most acutely, no examination of the
  **write path** (saving a real Excel user's file is silently destructive) or the
  **input path** (IME, Excel clipboard interop).

None of this invalidates the three rounds. The blind spots are each **days, not
weeks** to settle, and three of them can reshape the build plan — so settle them
before the build commit, not during.

---

## 2. Question 1 — Blind spots (make-or-break before starting)

What IS covered, credit first: engine correctness/perf/robustness, style fidelity,
function parity, the recompute seam, cache-sync under structural edits,
merges/CF/GPL/bus-factor (tracked), dynamic arrays (flagged, pending). The blind
spots cluster on the **product, file-write, input, and operational** sides.

### Must settle before the build commit (ranked)

1. **No product definition exists.** Across ~40 documents the entire product spec is
   one line ("build a better Spreadsheet app", phase-1 `project_overview.md`) plus
   adjectives. No target user, no reason-to-switch, no MVP cut-line, no platform
   targeting (macOS-primary is an experiment convenience that silently became
   product posture — Excel's base is overwhelmingly Windows), no free-vs-commercial
   posture (the GPL fix says "before shipping a proprietary binary," but nothing
   states the product IS proprietary). **Every deferred product decision the
   syntheses punt on — dynamic arrays, merges/CF, comments, charts/pivots in-or-out
   — is unanswerable without this.** "Clear to build" currently means "clear to
   build something undefined."
   *De-risk: a one-page product spec (user, top-20 workflows, v1 in/out, platform,
   license posture), then re-score the round-3 B gap matrix against it.*

2. **Saving a real user's file is silently destructive — and no real Excel-authored
   file was ever opened.** IronCalc drops on save everything it doesn't model:
   comments (probe-confirmed), data validation, hyperlinks, merges, CF — and never
   examined for IronCalc's save path: **charts, pivot tables, images/drawings,
   tables/autofilter, external links, VBA** (drawings/autofilter/external
   links/VBA appear nowhere in the corpus at all; charts/pivots only in passing,
   never as a drop-on-save analysis). "Open a colleague's workbook, fix one
   cell, save" destroys their charts and pivots without warning. The corpus frames
   merges/CF as *features FreeCell can't offer*, never as *data loss on re-save*.
   Compounding it, every fixture in three rounds is synthetic (SP2's 105 MB file
   was written by IronCalc's own writer — acknowledged in SP2 §5). The deferred
   "own `.xlsx` writing (~10× scope)" decision is treated as optional ("if
   pursued"), but destructive save arguably makes *some* preservation strategy
   (e.g. zip-level unknown-part pass-through) **mandatory for v1** — and that
   changes the file-layer architecture.
   *De-risk: 30–50 real Excel-authored files; open→save→diff the OOXML part
   inventory; prototype unknown-part pass-through; make the explicit v1 policy call.*

3. **The headline UI number was never recorded** (also the decision audit's top
   finding — see §4). The already-built Phase-1 `run_test.sh both` harness and the
   round-3 C snapshot harness have simply never been run on a Mac.
   *De-risk: one afternoon on a Mac; commit the JSON. If the numbers disappoint,
   far better to know now.*

4. **The editing input stack is unexamined: text input/IME and Excel clipboard
   interop.** The grid is a from-scratch raw-gpui widget, but whether GPUI gives a
   non-Zed app a production text-input path (IME/CJK composition, dead keys,
   layouts, decimal-comma locales) appears nowhere in the corpus — and neither does
   OS **clipboard interop with Excel** (TSV/HTML/XML-Spreadsheet formats), the
   single most-used bridge for anyone trialing a new spreadsheet. These are
   de-riskable unknowns, not build work.
   *De-risk: 2–3 day probe — GPUI editor overlay with CJK IME at the pinned rev; a
   two-way TSV/HTML clipboard bridge tested against real Excel.*

5. **Save/autosave/crash recovery were never treated as product operations.** Open
   was measured (22 s); save never was (the only datum is an incidental ~28 s
   generator write). Undesigned: save-during-eval (the model is `&mut` during a
   possibly-7 s evaluate — does save queue behind it?), atomic write/temp-rename
   (Phase-1 B recorded the workaround — use the in-memory writer form so
   "FreeCell owns temp-file and atomic-save policy" — but that policy was never
   designed), autosave and crash recovery (**zero mentions**) — for an app
   holding gigabytes
   of un-persisted state whose one known crash mode is an *uncatchable abort*
   (round-3 D).
   *De-risk: measure save at 10⁵–10⁷ cells; design save-on-worker + atomic replace
   + journaled autosave; ~2 days.*

6. **Untrusted-file robustness: D's input cap has a hole at the open path.** The
   pre-eval formula cap is framed around *user-typed* edits, but formulas also
   arrive via `load_from_xlsx` → `parse_formulas()` — one opaque engine call
   FreeCell can't intercept — so a crafted or merely pathological file with a
   ~500-deep formula aborts the app **at open**. Zip bombs / entity expansion /
   absurd dimensions were never probed. Spreadsheets are files people download.
   *De-risk: adversarial-open suite in a subprocess (D's harness pattern already
   exists); decide open-in-subprocess vs pre-scan vs upstream depth cap.*

7. **No upstream/fork strategy for IronCalc, despite the roadmap leaning on it.**
   A dozen agenda items resolve to "fix upstream or shim" (the round-2 proposal
   asked whether an upstream incremental-recalc contribution is "the only real
   fix" for the #1 risk — a question the corpus never answers; the 18 s parse;
   spill; hidden-row setter; fidelity fixes) — yet no one has assessed whether
   IronCalc accepts
   contributions at pace, what its roadmap holds, or FreeCell's fork/pin/upgrade
   policy. The product's perf ceiling is mortgaged to a 0.7.x project's velocity
   with no contact ever made.
   *De-risk: file 2–3 already-diagnosed small PRs (TRIM bug, hidden-row setter);
   measure responsiveness; write a one-page fork/track policy.*

8. **Accessibility posture is undecided, and the architecture is unforgiving.**
   A11y appears exactly once in the corpus — as a PoC out-of-scope line. A custom
   GPU-immediate grid has no free screen-reader story; retrofitting an AX tree onto
   a hand-rolled renderer approaches a view-layer rewrite, and a11y is frequently a
   hard procurement requirement for a commercial spreadsheet. The *posture* (and
   the fact of what GPUI exposes at the pinned rev) must be known before the grid
   hardens — the feature itself needn't be pre-MVP.
   *De-risk: one-day survey of GPUI/Zed's a11y surface + a recorded product decision.*

### Real but deferrable (record, don't block on)

Windows/Linux port (fine *if* §1 explicitly decides macOS-first; note the CI
snapshot strategy is Metal/macOS-only, which quietly couples testing to one
platform) · distribution/updates/code-signing/crash reporting/telemetry ·
i18n/localized function names & decimal-comma entry · FreeCell-side undo spanning
view-model state (zoom, hidden cols, sheet order) · memory ceiling beyond
~60–75 M cells (knowingly accepted, documented) · collaboration · print,
find/replace, sort/filter UI (all part of the §1 MVP exercise).

---

## 3. Question 2 — Over-engineering / premature optimization

**Overall: ~80/20 useful-derisking to waste, and the waste is in cheap throwaway
experiments, not adopted architecture.** The corpus repeatedly measured before
committing and then chose the boring option; it killed its own speculative
abstraction (the Phase-1 `FormatStore` side-table) in writing before building it;
and it parked optimizations in `PROJECTS.md`/`projects/` with explicit
"measure first" conditions instead of building them. That mechanism worked exactly
as intended.

| Area | Verdict | Notes |
|---|---|---|
| Excel-max grid envelope (1M×16k) | **Justified** | Virtualization math, sparse addressing, and the styled-read benchmarks only work if designed against max dimensions; retrofitting is a rewrite. But note: it's now a *stress envelope*, not a reachable product target — see §4 on the density concession. |
| <100 ms 1M-cascade target | **Overdone as a target** | Never realistic (both engines fail ~20×; the spec's own note anticipated ~2 s). Correctly converted into the async-recompute + staleness-UX answer, but never formally re-baselined — it survives three phases of docs as a permanently-failing gate. |
| Benchmark harness rigor | **Justified — best ROI in the corpus** | The "gold-plating" (force+assert, fresh-process RSS, negative controls, adversarial review) caught a **backwards** Sub-project-C conclusion before it drove the engine decision. Without it, you'd plausibly be on the wrong engine for wrong reasons. |
| Cache/sync architecture | **Justified / deferred-correctly** | Seven cache-ish designs appear; dispositions are uniformly evidence-driven. The one adopted cache (resident style+geometry) is load-bearing, and validating its shift-sync *before* the build (round-3 A) de-risked the classic expensive retrofit. Fenwick/chunked explicitly rejected as "not needed for v1"; viewport value cache parked with a measure-first condition. |
| Style fidelity (SP5, 59 attributes) | **Mildly overdone** | Run after the engine was locked; near-zero decision-changing power. ~15 attributes would have sufficed, long tail as build-time regression tests. Venial: the matrix seeds the product fidelity suite. |
| Function parity (SP3) | **Justified, well-sized** | Wired to a real off-ramp; 138 golden cases (not 5,000); the decomposition of the 35 missing functions into structural-capability vs aliases vs contributable scalars is exactly build-plan grade. |
| Async/interop seam (SP1, round-3 A) | **Justified — the foundational item** | The seam determines the app's whole concurrency shape and IronCalc's `&mut`/non-incremental/no-change-stream architecture forced it. Design-space framing ("mechanism is an output") was right; the 3.2 s snapshot-build vs 1.4 s eval number is precisely what kills a plausible design. |
| Round-3 C (CI rendering) | **Overdone / wrong-shaped** | Landed at "strategy confirmed, demo never executed, deferred until a real grid exists" — where a source-reading memo would have landed for a fraction of the cost. The authored-but-never-run harness is speculative work. |
| Three rounds of de-risking | **Justified, not paralysis** | Scope contracted each round (6→5→4 investigations); every round had pre-registered off-ramps and human checkpoints; each round answered questions the prior round *created*. Round-3 A alone justified the round. |
| Process boilerplate in specs | **Mildly overdone** | Agent-orchestration/isolation protocol repeated near-verbatim across three architecture docs; earned by real incidents, but a shared process doc referenced three times would have been leaner. |

**Bottom line for Q2: this was good architecture to cover before launching, not
over-engineering.** The identifiable waste (SP5's long tail, round-3 C, some 10⁷
sweep cells, repeated boilerplate) totals days. The failure mode the corpus
actually has is the opposite one: **under-measurement** of the headline UI claim.

---

## 4. Decision audit — are the adopted decisions justified?

Full inventory: 22 adopted decisions traced to evidence. The load-bearing ones:

| Decision | Rating |
|---|---|
| Recompute seam (worker-owned `Send` model, coalesce, publish-on-completion, generation counter) | **Well-supported** (gate + negative control) |
| ~3× overscan over snapshot-publish | **Well-supported** (both alternatives measured) |
| Cache-sync design (dense prefix-sum, BTreeMap re-key, mirror-the-primitive undo) | **Well-supported** (17 GATE tests + negative control) |
| Build on `UserModel`; display formatting engine-owned | **Well-supported** |
| Pre-eval input cap (+stack, +catch_unwind) | **Well-supported** (limits themselves are reasoned, not tuned) |
| Function-parity "credible" verdict | **Well-supported** for coverage; **thin** for per-function correctness (~1 golden case per common function — mitigated by cases-as-data design) |
| Engine = IronCalc | **Partially at adoption** (human override of the measured Formualizer lean, on delivery-risk grounds; the deferred validations later passed) — sound *process*, but see the density caveat below |
| Formatting = native styles (no side-table) | **Asserted at adoption → well-supported after SP4/SP5** (gamble that paid off, and SP4 was explicitly empowered to reopen it) |
| Resident style/geometry cache | **Partially** — sync proven; memory footprint on a heavily-styled huge sheet never measured (asserted via interning/sparsity reasoning) |
| CI snapshot strategy | **Partially** — perceptual diff proven; the render→PNG half never executed anywhere |
| UI = GPUI; "GPUI is validated" | **Partially / asserted** — the standout gap, below |

### The two findings that matter

**(a) GPUI validation-claim inflation — the one place the corpus violates its own
methodology.** Traceable drift with no new evidence between steps:

- Phase-1 SYNTHESIS: "the authoritative §5.4 render/cell-load gates **have not been
  recorded**" (honest);
- Phase-2 docs: "**GPUI is validated**" (`freecell-phase-2/project_overview.md:168`,
  `functional_spec.md:317`, `architecture.md:17`) — and the re-measurement was cut
  from scope, against round-2-proposal #7's own "still measured, not vibes, and
  still unrecorded";
- Phase-3: "human-validated (**'stellar'**)"
  (`freecell-phase-3/project_overview.md:59`) — the Phase-1 quote was "worked
  great"; "stellar" appears nowhere else in the corpus.

Phase-1 E itself named text-shaping under horizontal pan "the most likely
frame-time risk… only shows on real Metal" — unmeasured. The whole product thesis
is GPU render speed; it is the only load-bearing claim with zero recorded numbers,
and the named fallbacks (egui_table, raw wgpu) get more expensive to reach the
deeper the raw-gpui grid gets built.

**(b) The Excel-max north star was quietly re-scoped and never re-stated.** Choosing
IronCalc knowingly ceded ~9× memory density (~162 B/cell); SP2's own extrapolation
puts the ceiling at ~60–75 M cells on a 15 GB machine, while a full Excel-max sheet
(17.2 B cells) is arithmetically unreachable at that density. The concession was
accepted *by implication* across documents; no document states the achievable
envelope (~10⁷–10⁸ cells) against the "stupid-fast on huge sheets / Excel-max"
framing. Cheap documentation fix; worth doing before the build inherits an
impossible implicit requirement.

### Smaller evidence-quality notes

- **Round-2 headline vs its own table**: "IronCalc cleared **every** Phase-2 bar"
  sits above SP4 marked **PARTIAL PASS** (the 10k-cell styled overscan failed the
  frame budget as spec'd). The binding-layer reframe is defensible; the headline
  overstates.
- **Unreconciled marquee-number swing**: Phase-1 C measured the 1M cascade at
  ~2.11 s; SP1, same box and engine pin, ~1.2 s — a ~43% swing waved through
  without the adversarial review the project's own convention demands (likely
  benign — different shape construction — but unexamined).
- **SP2 measured IronCalc reading its own output**, and the memory gate's
  denominator (≤8× *uncompressed*) fits the observed number a little conveniently.
  Both acknowledged in-doc.
- **Round-3 C's GATE was waived in flight**: the spec required "demonstrated
  end-to-end"; the findings honestly say it wasn't, but the synthesis lists the
  mechanism under "Adopted decisions **confirmed**" and the round verdict absorbed
  an unmet gate without formally revising the criterion.

---

## 5. Coherence & hygiene issues (cheap fixes, high leverage)

1. **`CLAUDE.md` is stale — it predates Round 3.** It still names
   `round-2/SYNTHESIS.md` as "the closest thing to a real-app plan of record" and
   never mentions `round-3/` or `freecell-phase-3/`. It's the repo's orientation
   file; a fresh reader is pointed at the wrong plan of record. Same for
   `experiments/README.md` (still "Phase 1 only").
2. **One dishonest checkbox**: `freecell-phase-3/implementation_plan.md:59` marks
   Phase C `[x]` against a GATE ("demonstrated end-to-end", human macOS run) that
   was not met, and the build-readiness checkpoint (line 70) is a bare `[x]` with
   no outcome annotation — a regression from phase-2's exemplary dated
   "**CLEARED (2026-07-01)**" note.
3. **The plan of record is real but scattered.** Reconstructing "how we build the
   app" requires merging round-3 SYNTHESIS + round-2 SYNTHESIS (§adopted +
   §agenda) + `projects/*.md` + details buried in round-3 A/B findings. There is no
   consolidated build-architecture doc and no phase-4 spec project; "clear to
   build" currently dead-ends with the build unspecced.

Things handled *well*, for the record: the FormatStore reversal documented with
reasoning; the engine-override candor (Phase-1 SYNTHESIS §7); dated post-gate
annotations; pre-registered off-ramps written before results; the adversarial
review that caught and reversed the backwards bake-off conclusion; PROJECTS.md as
a working anti-scope-creep valve.

---

## 6. Recommended pre-build punch list

In order. Items 1–3 can each reshape the build plan; the rest are cheap insurance
and hygiene.

| # | Item | Cost |
|---|---|---|
| 1 | Write the one-page product spec (user, v1 cut-line, platform, license posture); then make the three pended product calls: dynamic arrays, merges/CF path, comments/validation/hyperlinks | 1–2 days + decisions |
| 2 | Real-file corpus test (30–50 Excel-authored files): open→save→OOXML-part diff; decide the destructive-save / unknown-part-pass-through policy | 2–3 days |
| 3 | ~~Run the already-built macOS harnesses~~ **CLOSED 2026-07-02: both harnesses run on macOS. §5.4 Run Test — both variants PASS all three gates (raw-gpui frame p99 1.98 ms vs the 8.33 ms budget; JSON committed). C render→diff demo — GATE closed end-to-end (re-render pixel-identical PASS; changed scene 3.47% FAIL; PNGs committed). Recorded in `experiments/04-ui-poc/findings.md` and `experiments/round-3/C-ci-rendering/findings.md`; syntheses and phase-3 plan annotated. This resolves finding (a) in §4 and the §5.2 checkbox issue.** | ~~an afternoon~~ done |
| 4 | IME + Excel-clipboard probe on the pinned GPUI rev | 2–3 days |
| 5 | Save-path design + measurement (save-on-worker, atomic replace, autosave/journal) | ~2 days |
| 6 | Adversarial file-open suite (subprocess open; depth-bomb, zip-bomb, dimensions) | 1–2 days |
| 7 | IronCalc upstream engagement (file the 2–3 diagnosed small PRs; gauge velocity) + one-page fork/pin policy | 1 day + calendar time |
| 8 | GPUI a11y surface survey + recorded posture decision | 1 day |
| 9 | Hygiene: update `CLAUDE.md` + `experiments/README.md` for round-3; annotate the phase-3 Phase-C checkbox and checkpoint honestly; write the explicit sheet-size-envelope statement (~10⁷–10⁸ cells vs Excel-max) | hours |
| 10 | Open the phase-4 (build) spec project consolidating the scattered plan of record (round-2 §adopted + round-3 §confirmed + carry-forwards + `projects/*.md`) | with #1 |

## 7. Bottom line

The three rounds did what staged de-risking is supposed to do: every
cheap-to-reverse decision was measured, every expensive-to-retrofit mechanism
(recompute seam, cache sync) was validated before adoption, speculative
abstractions were killed in writing, and off-ramps were pre-registered and honored.
**Not over-engineered** — the detail was overwhelmingly the load-bearing kind, and
the exceptions were throwaway-cheap.

The two things to internalize before the build: **the corpus systematically
under-validated everything that needed a Mac** (the headline render claim above
all), and **it never looked at the product** — who it's for, what v1 is, and what
happens when a real person saves a real file. Those are the make-or-break items,
they're all days-not-weeks, and three of them (product spec, destructive-save
policy, the macOS numbers) genuinely gate the build plan's shape.
