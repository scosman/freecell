---
status: draft
---

# FreeCell — Phase 3 (Pre-Build De-risking / "Round 3")

> **Self-contained handoff.** Written for an agent starting with **no prior
> conversation context**. It carries the decisions, evidence, the four investigations
> (with pass criteria), the file map, and working conventions you need. Read it fully
> before doing anything. Detail lives under `experiments/` and `specs/projects/` —
> pointers in §6.

---

## 0. How to start (for the picking-up agent)

1. Read this whole file.
2. Skim the capstones: `experiments/round-2/SYNTHESIS.md` (the Stage-3 recommendation +
   **adopted baseline decisions** + Round-3 agenda), `CLAUDE.md`, `PROJECTS.md` +
   `projects/style-cache.md` + `projects/viewport-cache.md` (the cache plan this round
   validates).
3. Skim the prior specs for conventions/evidence: `specs/projects/freecell-phase-2/`
   (esp. `architecture.md` §4 = the SP1 seam) and `specs/projects/freecell-phase-1/`.
4. This is the active project. Run the spec flow for `freecell-phase-3` (functional
   spec → architecture → implementation plan → implement). The four investigations in
   §5 are already scoped with pass criteria, so the functional spec is mostly
   formalizing §5; keep it light.
5. Honor the **locked decisions (§2)** and **working conventions (§7)** — several were
   learned the hard way and ignoring them wastes large amounts of time/tokens.

---

## 1. What this is

FreeCell is a **GPU-rendered (GPUI), Rust, Excel-compatible spreadsheet** built to be
**stupid-fast on huge sheets** (Excel-max = 1,048,576 × 16,384). Engine = **IronCalc**;
UI = **GPUI** (custom raw-gpui grid + gpui-component chrome). Built agentically in
**staged de-risking rounds**; **no production app exists yet.**

- **Stage 1 (Phase 1)** validated the core tech → GO.
- **Stage 2 (Phase 2 / Round-2)** validated the conditions on that GO → **verdict:
  BUILD** (`experiments/round-2/SYNTHESIS.md`). No off-ramp fired.
- **This round (Phase 3)** closes the **last architectural unknowns before committing
  to the real build.** Phase 2 validated *reading, recomputing, rendering*. This round
  validates the parts those didn't touch — the **interactive editing model** and a few
  targeted checks. After this it is **building, not de-risking.**

**Success = each investigation (§5) answered with reproducible evidence + a findings
doc, and a clear "clear to build" or "these must change first" verdict.** A
well-evidenced "this needs to change" is a successful Phase 3.

## 2. Locked decisions from Phase 1–2 (do NOT relitigate; build on these)

- **Engine = IronCalc 0.7.x** (`ironcalc` / `ironcalc_base`, MIT/Apache-2.0). Pin the
  **same version the round-2 harness uses (0.7.1)** for comparability.
- **UI = GPUI** (Apache-2.0), macOS/Metal primary; **grid = custom raw-gpui** widget
  (gpui-component can't do variable row heights / materializes at 16k cols);
  gpui-component kept for chrome. GPUI grid rendering/scrolling was human-validated
  ("stellar") in Phase 1.
- **Formatting = IronCalc-native styles**, **no FreeCell side-table.** SP4 proved the
  public style API covers per-cell + row/column band + empty-cell resolution.
- **Recompute seam (SP1, `experiments/round-2/01-async-interop/`):** a worker thread
  owns the (`Send`) `Model` and runs all `evaluate()`s; edits arrive over a channel and
  **coalesce** (drain-then-one-eval); the worker **publishes** the visible viewport on
  eval completion; the render loop watches a **generation counter** for cheap on-demand
  re-pulls and never touches the model. `evaluate()` is `&mut self`, full-workbook,
  non-incremental, non-interruptible, and IronCalc exposes **no evaluated-cell change
  stream** (edit-sites only).
- **~3× overscan published viewport** so scrolling stays live during a multi-second
  recompute (free; SP1 scroll-during-eval probe). A `to_bytes()` snapshot serves stale
  mid-eval reads but costs more to build than the eval, so it's on-demand-only.
- **Always-resident style + geometry cache (near-MVP)** — `projects/style-cache.md`.
  Caches ALL row/col sizes + fills/borders/fonts/number-format in the frontend; styles
  don't change on recompute, so the grid renders fully-styled during an eval (only
  values lag). **Its sync design is Investigation A below — the highest-stakes item.**
- **Values** are the only eval-dependent render input → optional viewport value-delta
  cache (`projects/viewport-cache.md`).
- **Known-and-accepted (NOT unknowns; don't re-open):** dynamic arrays / spilling
  = 0/17 (a **product decision** pending — accept v1 / build spill / contribute
  upstream); recompute staleness ≈ one eval (seconds on huge edits, mitigated);
  ~18s single-threaded parse to open 100 MB; minor SP5 fidelity losses; **merges +
  conditional formatting have no IronCalc API (OPEN** — would force owning `.xlsx`
  writing); **GPL #55470** GPL-3.0 transitive dep (pre-distribution fix, tracked).

## 3. Inherited evidence (headline numbers; 4-core Linux container floor)

- **Viewport read:** value-only p99 392 µs; **value+style ~10× the value cost** →
  ~1,800-cell viewport <2 ms PASS, but a 10k-cell overscan exceeds the 2 ms budget
  (SP4). The styled read runs on the SP1 worker, off the frame budget.
- **Recompute:** 1M `=PREV+1` chain ~1.2–2 s (non-incremental; the point is the
  non-blocking seam, not the raw number). Render tick stays <8.3 ms during a
  multi-second eval (SP1, gate-proven with a negative control).
- **Open 100 MB styled `.xlsx`:** ~22 s, peak RSS ~2.5 GB (~5× uncompressed); dominated
  by a single-threaded ~18 s parse (SP2).
- **Function parity:** 96.4% golden-correctness, 81.5% common-function coverage;
  **dynamic arrays 0/17** (SP3).
- **Style fidelity:** 50/59 attributes survive an `.xlsx` round-trip; losses minor/edge
  (SP5).
- **Model threading:** `Model<'static>` is `Send`; `to_bytes()` snapshot ~13 B/cell,
  ~3.2 s to build+load @1M.

**Process lesson (keep it):** an early Phase-2 draft shipped a *backwards* conclusion
from using the wrong API; **adversarial review caught it.** Adversarially review any
surprising number before it drives a decision.

## 4. Goal

Close the remaining **pre-build architectural unknowns** so we can commit to building
with confidence. The highest-stakes is **Investigation A** — it stresses the
just-adopted always-resident style/geometry cache against the one operation that shifts
everything (insert/delete row/col). Deliverable of the round: a **"clear to build"**
recommendation, or a precise list of **what must change first** (and how).

## 5. The investigations

Format per item: **Questions / Approach / Deliverables / Pass criteria.** Pass criteria
are **GATE** (hard, measured/asserted) or **DISCOVERY** (record + judge). Findings-doc
headings per Phase-1 `functional_spec.md` §5.2.

---

### A — Style/geometry cache sync + structural editing  *(highest-stakes)*

**Why it's the crux.** We adopted an **always-resident style+geometry cache** as
near-MVP. **Insert/delete row/column shifts every downstream row/col's geometry, style
band, and formula references** — so the cache must shift in lockstep, and it must be
undoable. If IronCalc's structural edits are wrong/slow, or the cache-shift is
intractable, the adopted architecture needs rework. This is the thing most likely to
force a redesign if skipped.

**Questions.**
- **`Model` vs `UserModel` — which does the app build on?** IronCalc has a low-level
  `Model` (what SP1 used, `&mut evaluate()`, no undo) and an interactive **`UserModel`**
  (undo/redo + an edit **diff-list**/history). The real app almost certainly needs
  `UserModel`. **Does the SP1 seam still hold for `UserModel`?** Is `UserModel` `Send`
  (movable to the worker)? Does its evaluate/apply block the same way? Capture its API.
- **Structural edits.** Do `UserModel` (or `Model`) insert/delete **rows** and
  **columns** correctly shift: (a) formula references, (b) row/column **band styles**,
  (c) row heights / column widths, (d) merged regions if present? Round-trip through
  `.xlsx`?
- **Undo/redo.** Present? Coverage (value / style / structural / size edits)?
  Granularity? Does undo of a structural edit fully un-shift everything?
- **Copy/paste of a range.** Does IronCalc translate **relative references** on paste?
  Paste values vs formulas vs styles?
- **The cache-sync design.** Given the resident cache (default + sparse override maps
  for sizes and styles, interned `StyleId`s, + cumulative-size **prefix sums** for
  scroll math): how does it shift on insert/delete-row/col — incrementally
  (O(overrides ≥ R) + a prefix-sum patch from R to end) or via a bounded rebuild? What
  does it cost at scale? Is it reversible for undo?

**Approach.**
- Probe IronCalc's `UserModel` API surface (insert/delete rows/cols, undo, redo,
  copy/paste, the diff-list) and its `Send`-ness; write it down (a mini "smoke" like
  Phase-1 did for `Model`).
- **Correctness harness:** build a sheet with cross-referencing formulas + row/col band
  styles + custom sizes (+ a merge if the API allows); insert/delete a row and a
  column; **assert** references, band styles, sizes shifted correctly; `.xlsx`
  round-trip. Same for undo/redo (edit → undo → assert reverted → redo → assert
  reapplied) across value/style/structural edits. Same for paste-range reference
  translation.
- **Cost at scale:** measure insert/delete-row on 10⁵–10⁶ populated rows (foreground,
  force+assert). Is it O(shifted) or worse?
- **Cache-sync prototype:** implement the resident cache's shift-on-insert/delete
  (sparse-map key shift + prefix-sum patch), measure its cost, and **verify it agrees
  with IronCalc after the edit** (re-read IronCalc sizes/styles == shifted cache).

**Deliverables.** `experiments/round-3/A-cache-sync/` (or a chosen path) with the probe
+ correctness/cost harness + a **locked cache-sync design** in `findings.md`, and a
recommendation on **`Model` vs `UserModel`** for the app (and whether the SP1 seam
carries over).

**Pass criteria.**
- **GATE:** insert/delete row/col correctness proven (references + band styles + sizes
  shift correctly; probe-backed); undo/redo covers value+style+structural edits.
- **GATE:** a validated cache-sync design — the resident cache shifts correctly on
  insert/delete (agrees with IronCalc) at an acceptable measured cost, and is reversible
  for undo.
- **DISCOVERY:** `UserModel` `Send`-ness + whether the SP1 worker seam holds for it;
  structural-edit cost at 10⁵–10⁶.
- **Off-ramp:** structural edits broken/absent/prohibitively slow, or undo/redo missing,
  or the cache-shift intractable → a significant finding (FreeCell-side implementation
  or an architecture change) surfaced before the build commit.

---

### B — Needed-API audit  *(breadth: do we have everything the build needs?)*

**Questions.** Beyond the structural APIs in A, does IronCalc's public API expose
everything the real app needs — or are there load-bearing gaps to plan around **now**?
Audit at least:
- **Display formatting** *(load-bearing for rendering)* — does IronCalc produce the
  **display string** for a cell (value + number-format → e.g. `"1,234.50"`, `"50%"`, a
  formatted date), or must FreeCell implement number-format rendering itself? *(We
  render displayed text, not raw values — confirm which side owns it.)*
- **The edit diff-list** (`UserModel`) for **surgical UI updates** — SP1 found it
  carries edit-sites only (no downstream-dirty). Confirm shape + how FreeCell uses it.
- **Sheet ops:** add / rename / delete / reorder sheets; enumerate sheets.
- **Defined names / named ranges** (read + write).
- **View/UI state in `.xlsx`:** freeze panes, hidden rows/cols, gridline/zoom — does
  IronCalc persist/expose these, or does FreeCell own them?
- **Cell extras:** comments/notes, data validation, hyperlinks (present? workaround?).
- **Formula-editing helpers:** the function list (SP3 = 345) + a tokenizer/parser
  usable for the formula bar / reference highlighting.
- Re-confirm the **known OPEN gaps**: merges, conditional formatting (no API);
  dynamic arrays (0/17).

**Approach.** Turn the above into a **checklist**; probe each against IronCalc 0.7.1's
public API with a short assertion or a documented "not present"; mark
**present / absent / workaround**. Cite the source location for each (the harness's
adapter + the crate source under `~/.cargo/registry/.../ironcalc*-0.7.1/`).

**Deliverables.** `experiments/round-3/B-api-audit/findings.md` — a present/absent/
workaround matrix for every needed API, each entry backed by a probe or a source
citation, with a **plan for each gap** and a flag on any load-bearing absence.

**Pass criteria.**
- **DELIVERABLE:** the coverage matrix, reproducible.
- **GATE (judgment):** no *surprise* load-bearing gap (e.g. if display formatting isn't
  provided, that's a real scope item — surface it, don't bury it).

---

### C — CI snapshot rendering  *(confirm the rendering-test strategy is buildable)*

**Why.** FreeCell's north star includes **rendering tests vs known-good PNGs.** That
whole strategy depends on being able to **capture a snapshot of the GPUI grid in CI.**
**Fuzzy / perceptual image match is acceptable** (anti-aliasing / font differences are
fine) — but "capture a snapshot in CI" must be **confirmed** with a working mechanism.

**Questions.**
- Can GPUI render the grid **offscreen / headless** to an image (a texture/framebuffer
  → PNG) **without a window/display**? (Zed has test infra — investigate what's
  reachable.)
- If not headless, what's the CI path — a **macOS CI runner** with a real/virtual
  display? Confirm at least ONE viable mechanism.
- Build a minimal **render → PNG → perceptual-diff-vs-baseline** harness (tolerance-
  based match, not bit-exact).

**Approach.** Investigate GPUI's rendering/test surface for an offscreen capture path.
Try in-container first (likely fails — no GPU; that's a finding). Then confirm on the
**Mac** (human-run): render the raw-gpui grid to PNG, commit a baseline, and diff a
re-render with a perceptual metric + tolerance. Document the exact CI mechanism.

**Deliverables.** `experiments/round-3/C-ci-rendering/` — the snapshot harness +
`findings.md` documenting the confirmed CI mechanism (headless if possible, else
Mac-CI), a committed baseline PNG + a perceptual-diff pass, and any GPUI APIs used.

**Pass criteria.**
- **GATE:** a **confirmed, working** "snapshot the grid in CI" mechanism, demonstrated
  end-to-end (render → PNG → perceptual diff passes within tolerance).
- **DISCOVERY:** whether headless works or it's Mac-CI-only.
- *(Runs partly on the Mac — GPUI can't render in-container; the human runs the Mac
  half and reports.)*

---

### D — Engine robustness  *(cheap; do it while we're here)*

**Questions.**
- **Circular references** — does IronCalc detect a cycle (`A1=A1`; `A1=B1, B1=A1`) and
  return an error (`#CIRCULAR`/`#REF!`), or hang / stack-overflow? *(Critical: every
  edit triggers a full recompute — a cycle must not lock the app.)*
- **Malformed / pathological input** — giant/deeply-nested/invalid formulas → graceful
  error, not panic.
- **Error propagation through the async seam** — if `evaluate()` panics or errors on
  bad input, does the **worker thread survive** (the SP1 worker owns the model — a panic
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

## 6. Where everything lives (file map)

- **Prior specs:** `specs/projects/freecell-phase-1/` and `-phase-2/` — overviews,
  functional specs, architectures (esp. **phase-2 `architecture.md` §4** = the SP1
  seam; §2 = swarm orchestration + parallel-editor isolation; §3 = methodology),
  implementation plans, `phase_plans/`.
- **Experiments:** `experiments/`
  - `shared/datagen` + `shared/bench_util` and `round-2/harness/` — **FROZEN**,
    read-only (deterministic generators; env-stamped p50/p99 + gating; the IronCalc
    0.7.1 `SpreadsheetEngine` adapter + scenarios + `peak_rss()`). **Reuse these.**
  - `round-2/01-async-interop/` — the SP1 seam (`EvalWorker`, `shapes`) + the
    scroll-during-eval probe. **A/B/D will lean on this + the harness.**
  - `round-2/{02-xlsx-open,03-function-parity,04-styled-read,05-style-fidelity}/` +
    `round-2/SYNTHESIS.md`.
  - Phase-1: `00-stack-decision` … `06-engine-bakeoff`, `04-ui-poc/` (the raw-gpui grid
    PoC — **C evolves from this**), `SYNTHESIS.md`.
- **Backlog:** `PROJECTS.md` + `projects/style-cache.md` (the cache A validates) +
  `projects/viewport-cache.md`. **`CLAUDE.md`** = repo orientation + conventions.
- **New work goes in `experiments/round-3/{A,B,C,D}-*/`** (independent Cargo projects;
  the C rendering piece pins GPUI on macOS like Phase-1 `04-ui-poc`).

## 7. Working conventions (READ — several are hard-won)

- **Environment:** headless Linux container, **4 cores / ~15 GB RAM, no GPU / no
  display**, Rust 1.94+. crates.io works. GitHub scoped to `scosman/freecell`; branch
  work updates **PR #1**. Develop on this session's designated branch; **commit + push
  regularly** (a stop-hook nags; the container is ephemeral).
- **Where work runs:** A, B, D are **in-container and authoritative** (IronCalc API +
  benchmarks). **C needs a real GPU → macOS**: write the code + build scripts, the
  **human runs it** and reports. Don't fight the GPUI build in-container (part of C is
  *whether* headless is even possible — a finding).
- **Reuse, don't rewrite** the frozen `round-2/harness/` + `shared/*`; depend by
  relative path, read-only. Escalate if a shared change is truly needed.
- **Experiments structure:** each in its own folder; **independent Cargo projects**
  (not a shared workspace) so parallel editors never contend; emit a `findings.md`
  (§5.2 headings) + committed `results/`. `target/` is gitignored repo-wide.
- **Spec-driven + agent swarm** (phase-2 `architecture.md` §2): manager → coding
  sub-agent → CR sub-agent per phase; **parallel-editor isolation** (each phase edits
  ONLY its folder; path-scoped git — never `git add -A`; serialize commits; worktree
  isolation recommended).
- **Benchmark discipline:** run **FOREGROUND with `timeout`** — NEVER `nohup`/`&`/
  background monitors (a Phase-1 agent burned ~600k tokens on a background poller). Cap
  scale + record the ceiling if slow. Separate build from the measured op. **Force +
  assert** the measured op. Report p50/p99, env-stamped. **Adversarially review
  surprising numbers.**
- **Review altitude (per the human):** these are **throwaway experiments for functional
  lessons.** Code-review only for **things that impact the answer** (is a measurement
  real / does the evidence support the claim) — **not** typos, style, or prose. Accept
  on first pass unless something answer-impacting surfaces; no cosmetic round-trips.
- **Findings must be honest:** grade against the pass criteria; call out what's
  un-measured or capped; a "present" API claim needs a probe or a source citation.

## 8. Tech references
- IronCalc: https://github.com/ironcalc/IronCalc (crates `ironcalc`, `ironcalc_base`;
  the **`UserModel`** interactive API is the focus of A/B). Source is under
  `~/.cargo/registry/src/index.crates.io-*/ironcalc*-0.7.1/`.
- GPUI: in https://github.com/zed-industries/zed (git-pinned; the raw-gpui grid PoC is
  `experiments/04-ui-poc/raw-gpui`).
- gpui-component: https://github.com/longbridge/gpui-component
