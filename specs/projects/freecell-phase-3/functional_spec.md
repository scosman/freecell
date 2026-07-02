---
status: complete
---

# Functional Spec: FreeCell — Phase 3 (Pre-Build De-risking / "Round 3")

> Read `project_overview.md` first — it is a self-contained handoff carrying the locked
> Phase 1–2 decisions (engine = IronCalc, UI = GPUI raw-gpui grid, formatting =
> IronCalc-native styles, the SP1 worker seam, ~3× overscan, the always-resident
> style+geometry cache), the inherited evidence, the four investigations **already
> scoped with pass criteria (§5)**, the file map, and the hard-won working conventions.
> This spec formalizes those four investigations into gated experiments. It is
> **deliberately light** — it does not re-derive the overview; it turns §5 into a
> runnable plan. **This is not a plan to start building the app** — anything that is
> "build the feature" is the real build, not Phase 3.

## 1. Purpose

Phase 2 (Round-2) returned **BUILD** (`experiments/round-2/SYNTHESIS.md`): IronCalc
cleared every bar; the remaining work is a well-scoped engineering agenda, not a set of
blockers. But Phase 2 validated only **reading, recomputing, and rendering**. It never
touched the **interactive editing model** (insert/delete row/col, undo/redo,
copy/paste) — and that is exactly the operation the just-adopted **always-resident
style+geometry cache** must stay in lockstep with. Phase 3 closes that gap plus three
smaller, cheap-to-answer pre-build unknowns.

The governing filter (same as Phase 2): **is this a real architectural unknown that
could force a redesign before we commit to the build?** If yes, it's an investigation.
If it's "write the feature," it's out — that's the real app.

**Success = each investigation (§6) answered with reproducible evidence + a `findings.md`,
and a round-level verdict: "clear to build" or a precise list of "what must change
first" (and how).** A well-evidenced "this must change" is a successful Phase 3.

## 2. Scope

### In scope — four investigations (§6), each closing a pre-build unknown
1. **A — Style/geometry cache sync + structural editing** *(highest-stakes; the crux)*.
2. **B — Needed-API audit** (present / absent / workaround matrix for everything the
   build needs).
3. **C — CI snapshot rendering** (confirm a "snapshot the grid in CI" mechanism exists;
   fuzzy/perceptual match acceptable).
4. **D — Engine robustness** (circular refs don't hang; malformed input doesn't panic;
   async-seam worker survives a bad eval).

Plus a **Phase-3 synthesis** producing the "clear to build" recommendation.

### Out of scope (explicitly — building, or already settled)
- **Building the real app / real GPUI feature work** — grid selection, inline editing,
  the formula bar, the actual resident-cache implementation shipped in the product.
  Phase 3 builds *throwaway probes/prototypes to answer unknowns*, not product code.
- **Relitigating locked decisions (overview §2):** engine, UI, native-styles, the SP1
  seam, overscan, the resident-cache adoption. Phase 3 *stresses* the cache design; it
  does not reopen whether to have one.
- **The known-and-accepted items (overview §2), which are NOT unknowns:** dynamic
  arrays 0/17 (a pending **product** decision, not a technical one), recompute
  staleness, ~18s single-threaded 100 MB parse, minor SP5 fidelity losses. Phase 3
  re-confirms merges/CF and dynamic arrays only as part of the B matrix — it does not
  design them.
- **Merges + conditional formatting design** — OPEN (no IronCalc API; would force
  owning `.xlsx` writing, ~10× scope). Recorded in B as a known gap, not designed.
- **GPL #55470** — a pre-distribution packaging/legal task, tracked; not a technical
  unknown → not a Phase-3 experiment.
- Production import/export, persistence, packaging, error UX polish.

### Deliberately deferred
- Anything the Phase-3 synthesis tags forward as "build-time" work.
- Implementing the shipped resident cache (A *validates its sync design*; the real
  implementation is the build). Implementing dirty-tracking / the value cache
  (SYNTHESIS Round-3 #1) — measured/designed if it surfaces, but its build is the app.

## 3. Environment & Division of Labor

**A, B, D run entirely in the headless Linux container (4c / ~15 GB, no GPU),
autonomously, and are authoritative** (they exercise the real IronCalc 0.7.1 API +
benchmarks). **C is the one cross-environment piece:** GPUI needs a real GPU, so the
in-container half of C investigates *whether headless capture is even possible* (a
finding in itself), and the demonstrable render→PNG→diff harness is **written
in-container and run by the human on macOS**, who reports back — exactly the split
Phase 1 used for the GPUI PoC.

Binding conventions from Phase 1–2 hold verbatim (overview §7): **all benchmarks run
FOREGROUND with `timeout`** — never `nohup`/`&`/background monitors (a Phase-1 agent
burned ~600k tokens on a background poller). If a run is too slow, **cap the scale and
record the ceiling as a finding.** Separate build/load time from the measured op;
**force + assert** the measured op; **adversarially review any surprising number**
before it drives a conclusion.

## 4. Sequencing, Gating & the Build-Readiness Checkpoint

No up-front human gate (the engine is chosen and Phase 2 cleared it). One **checkpoint**
near the end, analogous to Phase 2's off-ramp:

1. **In-container cohort (A, B, D)** — run in parallel (disjoint folders). **A is the
   highest-stakes** and is the most likely single item to force a redesign.
2. **C** — the in-container investigation runs alongside; its demonstrable half is
   handed to the human for the macOS run and folded in when reported.
3. **Build-readiness checkpoint (human review of A–D).** Present findings against each
   investigation's pass criteria. If any GATE fails or an off-ramp fires — structural
   edits broken/absent/prohibitively slow, undo/redo missing, the cache-shift
   intractable, a surprise load-bearing API gap, no viable CI-snapshot mechanism, or a
   circular ref that hangs — **surface it for a human "change first vs accept" decision
   before the build commit.** Clean → "clear to build."
4. **Phase-3 synthesis (last).** Consumes all findings → the Stage-3 "clear to build"
   recommendation (or the must-change list).

## 5. Cross-Cutting Conventions

Phase-1/2 conventions carry over verbatim (overview §7; Phase-2 `functional_spec.md`
§5, `architecture.md` §2–§3 — **read them**). This only notes what's specific to Phase 3:

### 5.1 Layout
Round-3 work lives under **`experiments/round-3/{A,B,C,D}-*/`**, one self-contained,
**independent Cargo project** per investigation (NOT a workspace — Phase-1 isolation
rationale). Each depends by **relative path, read-only** on the frozen
`experiments/round-2/harness/` (the IronCalc 0.7.1 `SpreadsheetEngine` adapter +
scenarios + `peak_rss()`) and `experiments/shared/*` (datagen, bench_util). **A and B
also probe IronCalc's `UserModel` directly** (the harness wraps the low-level `Model`,
not `UserModel`); that probing code is local to the investigation's own crate — **no
new frozen harness is created**, and the round-2 harness is never edited (escalate if a
shared change seems needed). The C rendering crate pins GPUI on macOS like Phase-1
`04-ui-poc`.

### 5.2 Findings & standards
Same headings as Phase-1 §5.2; same benchmark discipline as §5.3 and overview §7. A
**"present" API claim needs a probe or a source citation** (the crate source under
`~/.cargo/registry/.../ironcalc*-0.7.1/`); an **absent** claim needs a documented search.
Grade honestly against the pass criteria; call out anything un-measured or capped.

### 5.3 Review altitude (per the human — hard-won)
These are **throwaway experiments for functional lessons.** Code-review **only for
things that impact the answer** (is a measurement real; does the evidence support the
claim; is the cache-sync design actually validated against IronCalc) — **not** typos,
style, or prose. Accept on first pass unless something answer-impacting surfaces; no
cosmetic round-trips.

## 6. The Investigations

Format: **Questions / Approach / Deliverables / Pass criteria**, where criteria are a
**GATE** (hard, measured/asserted), a **DELIVERABLE** (must be produced), or a
**DISCOVERY** (record + judge). Off-ramp = a finding significant enough to pause for a
human decision before the build commit. The overview §5 is the authoritative scoping;
what follows is the same, formalized, and is what the phase plans execute against.

---

### A — Style/geometry cache sync + structural editing  *(THE key investigation)*

**Why it's the crux.** We adopted an **always-resident style+geometry cache** as
near-MVP (`projects/style-cache.md`). **Insert/delete row/column shifts every downstream
row/col's geometry, style band, and formula references** — so the cache must shift in
lockstep, and it must be **undoable**. If IronCalc's structural edits are wrong/slow, or
the cache-shift is intractable, the adopted architecture needs rework. This is the item
most likely to force a redesign if skipped.

**Questions.**
- **`Model` vs `UserModel` — which does the app build on?** IronCalc has the low-level
  `Model` (what SP1 used: `&mut evaluate()`, no undo) and the interactive **`UserModel`**
  (undo/redo + an edit **diff-list**). The real app almost certainly needs `UserModel`.
  **Does the SP1 worker seam still hold for `UserModel`?** Is `UserModel` `Send`
  (movable to the worker)? Does its evaluate/apply block reads the same way? Capture its
  API surface.
- **Structural edits.** Do insert/delete **row** and **column** correctly shift: (a)
  formula references, (b) row/column **band styles**, (c) row heights / column widths,
  (d) merged regions if the API exposes them? Survive an `.xlsx` round-trip?
- **Undo/redo.** Present? Coverage (value / style / structural / size edits)?
  Granularity? Does undo of a structural edit fully un-shift everything?
- **Copy/paste of a range.** Does IronCalc translate **relative references** on paste?
  Paste values vs formulas vs styles?
- **The cache-sync design.** Given the resident cache (default + sparse override maps for
  sizes and styles, interned `StyleId`s, + cumulative-size **prefix sums** for scroll
  math): how does it shift on insert/delete-row/col — incrementally (sparse-map key
  shift ≥ the edit index + a prefix-sum patch from there to the end) or a bounded
  rebuild? Cost at scale? Reversible for undo?

**Approach.**
- Probe `UserModel`'s API (insert/delete rows/cols, undo, redo, copy/paste, the
  diff-list) and its `Send`-ness; write it down (a mini "smoke" like Phase-1 did for
  `Model`), backed by compile-time + runtime probes.
- **Correctness harness:** build a sheet with cross-referencing formulas + row/col band
  styles + custom sizes (+ a merge if the API allows); insert/delete a row and a column;
  **assert** references, band styles, and sizes shifted correctly; `.xlsx` round-trip.
  Same for undo/redo (edit → undo → assert reverted → redo → assert reapplied) across
  value/style/structural edits. Same for paste-range reference translation.
- **Cost at scale:** measure insert/delete-row on 10⁵–10⁶ populated rows (foreground,
  force+assert). Is it O(shifted) or worse?
- **Cache-sync prototype:** implement the resident cache's shift-on-insert/delete
  (sparse-map key shift + prefix-sum patch), measure its cost, and **verify it agrees
  with IronCalc after the edit** (re-read IronCalc sizes/styles == the shifted cache).

**Deliverables.** `experiments/round-3/A-cache-sync/` with the `UserModel` probe + the
correctness/cost harness + a **locked cache-sync design** in `findings.md`, and a
recommendation on **`Model` vs `UserModel`** for the app (and whether the SP1 seam
carries over).

**Pass criteria.**
- **GATE:** insert/delete row/col correctness proven — references + band styles + sizes
  shift correctly (probe-backed); undo/redo covers value + style + structural edits.
- **GATE:** a validated cache-sync design — the resident cache shifts correctly on
  insert/delete (**agrees with IronCalc**) at an acceptable measured cost, and is
  reversible for undo.
- **DISCOVERY:** `UserModel` `Send`-ness + whether the SP1 worker seam holds for it;
  structural-edit cost at 10⁵–10⁶.
- **Off-ramp:** structural edits broken/absent/prohibitively slow, or undo/redo missing,
  or the cache-shift intractable → surface as a significant finding (FreeCell-side
  implementation or an architecture change) before the build commit.

---

### B — Needed-API audit  *(breadth: do we have everything the build needs?)*

**Questions.** Beyond A's structural APIs, does IronCalc's public API expose everything
the real app needs — or are there load-bearing gaps to plan around **now**? Audit at
least:
- **Display formatting** *(load-bearing for rendering)* — does IronCalc produce the
  **display string** for a cell (value + number-format → `"1,234.50"`, `"50%"`, a
  formatted date), or must FreeCell implement number-format rendering itself? *(The grid
  renders displayed text, not raw values — confirm which side owns it.)*
- **The edit diff-list** (`UserModel`) for **surgical UI updates** — SP1 found it carries
  edit-sites only (no downstream-dirty). Confirm shape + how FreeCell consumes it.
- **Sheet ops:** add / rename / delete / reorder sheets; enumerate sheets.
- **Defined names / named ranges** (read + write).
- **View/UI state in `.xlsx`:** freeze panes, hidden rows/cols, gridline/zoom — does
  IronCalc persist/expose these, or does FreeCell own them?
- **Cell extras:** comments/notes, data validation, hyperlinks (present? workaround?).
- **Formula-editing helpers:** the function list (SP3 = 345) + a tokenizer/parser usable
  for the formula bar / reference highlighting.
- Re-confirm the **known OPEN gaps** (record, don't design): merges, conditional
  formatting (no API); dynamic arrays (0/17).

**Approach.** Turn the above into a **checklist**; probe each against IronCalc 0.7.1's
public API with a short assertion or a documented "not present"; mark
**present / absent / workaround**. Cite the source location for each (the round-2
harness adapter + the crate source under `~/.cargo/registry/.../ironcalc*-0.7.1/`).

**Deliverables.** `experiments/round-3/B-api-audit/findings.md` — a present/absent/
workaround matrix for every needed API, each entry backed by a probe or a source
citation, with a **plan for each gap** and a flag on any load-bearing absence.

**Pass criteria.**
- **DELIVERABLE:** the coverage matrix, reproducible.
- **GATE (judgment):** no *surprise* load-bearing gap. If display formatting isn't
  provided, that's a real scope item — **surface it, don't bury it** (it changes what
  the renderer must own).

---

### C — CI snapshot rendering  *(confirm the rendering-test strategy is buildable)*

**Why.** FreeCell's north star includes **rendering tests vs known-good PNGs.** That
whole strategy depends on being able to **capture a snapshot of the GPUI grid in CI.**
**Fuzzy / perceptual image match is acceptable** (anti-aliasing / font differences are
fine) — but "capture a snapshot in CI" must be **confirmed** with a working mechanism.

**Questions.**
- Can GPUI render the grid **offscreen / headless** to an image (texture/framebuffer →
  PNG) **without a window/display**? (Zed has test infra — investigate what's reachable.)
- If not headless, what's the CI path — a **macOS CI runner** with a real/virtual
  display? Confirm at least ONE viable mechanism end-to-end.
- Build a minimal **render → PNG → perceptual-diff-vs-baseline** harness (tolerance-
  based match, not bit-exact).

**Approach.** Investigate GPUI's rendering/test surface for an offscreen capture path.
Try in-container first (**likely fails — no GPU; that's a finding**). Then confirm on the
**Mac** (human-run): render the raw-gpui grid to PNG, commit a baseline, diff a
re-render with a perceptual metric + tolerance. Document the exact CI mechanism.

**Deliverables.** `experiments/round-3/C-ci-rendering/` — the snapshot harness +
`findings.md` documenting the confirmed CI mechanism (headless if possible, else
Mac-CI), a committed baseline PNG + a perceptual-diff pass, and any GPUI APIs used.

**Pass criteria.**
- **GATE:** a **confirmed, working** "snapshot the grid in CI" mechanism, demonstrated
  end-to-end (render → PNG → perceptual diff passes within tolerance).
- **DISCOVERY:** whether headless works or it's Mac-CI-only.
- *(Runs partly on the Mac — GPUI can't render in-container; the human runs the Mac half
  and reports.)*

---

### D — Engine robustness  *(cheap; do it while we're here)*

**Questions.**
- **Circular references** — does IronCalc detect a cycle (`A1=A1`; `A1=B1, B1=A1`) and
  return an error (`#CIRCULAR` / `#REF!`), or hang / stack-overflow? *(Critical: every
  edit triggers a full recompute — a cycle must not lock the app.)*
- **Malformed / pathological input** — giant / deeply-nested / invalid formulas →
  graceful error, not panic.
- **Error propagation through the async seam** — if `evaluate()` panics or errors on bad
  input, does the **worker thread survive** (the SP1 worker owns the model — a panic
  would poison it), or do we need `catch_unwind` / a recovery story?

**Approach.** A small probe crate (reuse the round-2 harness IronCalc adapter): feed
cycles, malformed formulas, and pathological inputs; assert IronCalc returns errors and
does not hang/panic; test a worker-thread panic-recovery path if `evaluate()` can panic.
Foreground with `timeout` (a hang shows up as a timeout — record it).

**Deliverables.** `experiments/round-3/D-robustness/findings.md` — circular-ref
behavior, malformed-input behavior, and the worker-recovery recommendation.

**Pass criteria.**
- **GATE:** circular refs return an error and **do not hang** (or, if they do, a
  documented mitigation — e.g. an iteration cap); malformed input → error, not panic.
- **DELIVERABLE:** a worker-thread robustness recommendation (catch_unwind / restart /
  "evaluate can't panic on user input" — whichever the evidence supports).

---

### Phase-3 Synthesis  *(final; serial)*

**Deliverable.** `experiments/round-3/SYNTHESIS.md`: the **"clear to build"**
recommendation (or the precise **must-change-first** list), citing A–D, stating whether
any off-ramp fired, and listing any carry-forward into the build.

**Pass criteria.** A defensible, evidence-backed pre-build verdict. A well-evidenced
"this must change first" is a successful Phase 3.

## 7. Risks & What Could Invalidate the Approach

- **The SP1 seam may not carry to `UserModel` (A) — the biggest risk.** SP1 proved the
  seam on the low-level `Model`. If `UserModel` (which the app needs for undo/redo)
  isn't `Send`, or blocks differently, the worker-owned seam design needs adjustment.
  A must settle this early.
- **Cache-shift intractable at scale (A).** If insert/delete-row is worse than
  O(shifted), or the sparse-map + prefix-sum shift can't be made to agree with IronCalc
  cheaply, the resident-cache architecture needs rework — a redesign, surfaced before
  the build.
- **A surprise load-bearing API gap (B).** Display formatting is the prime suspect: if
  FreeCell must implement number-format rendering, that's real renderer scope. Surface
  it, don't bury it.
- **No viable CI-snapshot mechanism (C).** If neither headless nor Mac-CI can capture
  the grid, the rendering-test north star needs a different strategy — better to know now.
- **A circular ref that hangs (D).** Every edit recomputes the whole workbook; a cycle
  that locks the app would be a real defect — cheap to check, expensive to discover late.
- **Scope creep.** The pull to start building the real app is strong now that Phase 2
  said BUILD. Phase 3 stops at answering the four unknowns + a pre-build verdict.

## 8. Resolved Decisions
- **Locked decisions from Phase 1–2 are not relitigated** (overview §2): engine =
  IronCalc, UI = GPUI raw-gpui, native styles, the SP1 seam, ~3× overscan, the
  always-resident style+geometry cache. Phase 3 *stresses* the cache; it does not
  reopen it.
- **No new frozen harness.** Reuse `round-2/harness/` + `shared/*` read-only; `UserModel`
  probing is local to A/B's own crates.
- **A, B, D are in-container and authoritative; C's demonstrable half is macOS/human-run**
  (its in-container half investigates *whether* headless is possible — a finding).
- **Merges/CF and dynamic arrays are re-confirmed in B as known gaps, not designed.**
- **One build-readiness checkpoint (human)** after A–D land, not an up-front gate.
