---
status: complete
---

# Architecture: FreeCell — Phase 3 (Pre-Build De-risking / "Round 3")

Like Phase 1–2, this is a **research / de-risking** effort, so the "architecture" is:
the experiment workspace + reuse strategy, the **agent-swarm orchestration**, the
**measurement methodology**, and — because it is the load-bearing hard problem — the
**resident-cache shift-on-structural-edit design space (Investigation A)**. Remaining
per-investigation detail (exact probe lists, the B checklist, the C harness shape) is
deferred to **phase plans** (§7), matching Phase 1–2. Read the Phase-1/2
`architecture.md` first; this doc only adds/changes. The overview (`project_overview.md`)
is the self-contained handoff and is authoritative for decisions/evidence.

## 0. Grounding facts (inherited + Phase-3-relevant)

- Rust 1.94+; container **4 cores / ~15 GB RAM, no GPU / no display**; crates.io works.
  GitHub scoped to `scosman/freecell`; branch work updates **PR #1**. **A, B, D are
  fully in-container** and autonomous; **C's demonstrable half is macOS/human-run**
  (its in-container half is the "is headless even possible?" finding).
- **Engine = IronCalc 0.7.x**, pinned to the **same version the round-2 harness uses
  (0.7.1)** for comparability. Known shape (from Phase 1–2): full-workbook `evaluate()`
  (no incremental recalc, `&mut self`, not interruptible, no change-stream — edit-sites
  only); native styled `.xlsx` I/O; persists cached results; style API covers per-cell +
  row/col band + empty-cell (SP4). **What Phase 3 adds to this picture is `UserModel`**
  — the interactive API (undo/redo + diff-list) the app needs but SP1 never used.
- **The just-adopted always-resident style+geometry cache** (`projects/style-cache.md`)
  is the thing under test. Its sync against structural edits is the top open question
  (Investigation A). §4 gives the *design space* (data structures + shift primitives +
  what "correct" means), **not** a prescribed, locked implementation — A measures and
  locks it, the same way Phase-2 §4 framed the SP1 seam as a design space that SP1
  closed.

## 1. Repository Layout & Reuse Strategy

Round-3 work lives under **`experiments/round-3/`**. Phase-1/2 folders are **frozen**
(read-only); Round-3 never edits them.

```
experiments/
  shared/                          # Phase-1, FROZEN (datagen, bench_util)
  round-2/
    harness/                       # Phase-2, FROZEN — SpreadsheetEngine (Model) adapter,
                                    #   scenarios, peak_rss(). Reused read-only by A/B/D.
    01-async-interop/              # Phase-2, FROZEN — the SP1 seam; A checks it vs UserModel
    04-styled-read/  05-…          # Phase-2, FROZEN — SP4/SP5 style-API + fidelity evidence
  round-3/
    A-cache-sync/                  # Investigation A (independent Cargo project) — the crux
    B-api-audit/                   # Investigation B
    C-ci-rendering/                # Investigation C (pins GPUI on macOS; human-run half)
    D-robustness/                  # Investigation D
    SYNTHESIS.md                   # Phase-3 "clear to build" verdict → Stage 3
```

**No new frozen harness.** The round-2 `harness/` wraps the low-level `Model` behind the
`SpreadsheetEngine` trait; A and B need the **`UserModel`** interactive API (undo/redo,
diff-list, structural edits) that the trait doesn't expose. Rather than widen or fork the
frozen harness, **A and B probe `ironcalc`'s `UserModel` directly in their own crates**
(depending on `ironcalc`/`ironcalc_base` at the pinned 0.7.1, plus the harness read-only
for its adapter/scenarios/`peak_rss` where useful). D reuses the harness `Model` adapter
as-is (robustness is a `Model`-level property). This keeps the frozen crate untouched;
a genuinely-needed shared change → **escalate**, don't edit in place.

Each investigation is an **independent Cargo project** (NOT a workspace — Phase-1
isolation rationale) depending by **relative path, read-only** on `../../round-2/harness`
and `../../shared/*`. `target/` gitignored repo-wide. Repeated IronCalc compiles accepted.

## 2. Agent-Swarm Orchestration

Same structure as Phase-1/2 `architecture.md` §2: a **coordinator** spawns, per phase, a
**manager → coding sub-agent → CR sub-agent** running the attestation → CR → commit
loop; the manager never writes code itself. Parallel phases run concurrently in their own
worktrees/folders.

### 2.1 Topology
```
Phase-3 Coordinator
├─ Phase 3.0: Scaffolding (serial) ── create round-3/{A,B,C,D}-*/ skeletons;
│                                       wire read-only deps on round-2/harness + shared/*
├─ In-container cohort (PARALLEL):     A (crux), B, D
│     └─ C: in-container half (headless-possible? investigation) runs alongside;
│           its demonstrable render→PNG→diff harness is authored here, HUMAN-RUN on macOS
├─►  BUILD-READINESS CHECKPOINT ── human review of A–D vs each investigation's criteria
└─ Phase-3 Synthesis (serial; last) ── SYNTHESIS.md ("clear to build" or must-change)
```
A, B, D build/run in-container; numbers are authoritative there. C is the one
cross-environment piece (§3, overview §7).

### 2.2 Parallel-editor isolation (REQUIRED — inject verbatim into every parallel agent)
Reuse Phase-1/2 `architecture.md` §2.2 **exactly**, folder token =
`experiments/round-3/<X>-<name>/`. Recap: operate only inside your folder (plus
read-only `round-2/harness/`, `shared/`, `specs/`); **never** edit the repo root, another
investigation, or a frozen crate; git-scope every command to your folder
(`git add experiments/round-3/<X>-<name>/`; **never** `git add -A` / `.` / `commit -a`);
CR + attestation cover only your folder's diff; must touch a shared/frozen file →
**stop and escalate**.

### 2.3 Commit safety
Serialized, path-scoped commits (Phase-1/2 §2.3): coordinator admits one commit at a
time; disjoint folders → conflict-free. **Worktree isolation (`isolation: "worktree"`)
recommended** for the parallel cohort. (Learned Phase-2 bug: a stray `cd` into a
sub-folder poisoned later git CWD — prefer `git -C <repo-root>` in commands.)

### 2.4 Build-readiness checkpoint
After A–D land (C folded in when the human reports the macOS run), the coordinator
**pauses and presents** findings against each investigation's pass criteria (§6 of the
functional spec). Any GATE fail or off-ramp — structural edits broken/absent/slow,
undo/redo missing, cache-shift intractable, a surprise load-bearing API gap, no viable
CI-snapshot mechanism, or a circular ref that hangs — is **surfaced for a human
"change-first vs accept" decision before the build commit.** Clean → "clear to build."

## 3. Measurement Methodology (additions to Phase-1/2 §3)

Phase-1/2 §3 stands (Criterion + `bench_util` timers; p50/p99/max; committed
code-generated inputs; env-stamped results; PASS/FAIL asserts; **foreground-only** runs
with `timeout`; adversarial review of surprising numbers; fresh-child-process peak-RSS).
Round-3 adds:

- **Structural-edit cost (A).** Time a single `insert_row` / `delete_row` /
  `insert_column` / `delete_column` on a populated sheet at 10⁵–10⁶ rows, **foreground,
  force+assert** the shift actually happened (assert a reference/size/style at an index
  past the edit moved). Report p50/p99. Separate this IronCalc-side cost from the
  **cache-shift** cost (measured independently in the prototype).
- **Cache-vs-engine agreement (A) — the load-bearing assertion.** After each structural
  edit, apply the same shift to the resident-cache prototype, then **re-read IronCalc's
  authoritative sizes/band-styles/cell-styles for a sample of indices spanning the edit
  point and assert they equal the cache's shifted values.** A design that's fast but
  disagrees with IronCalc is a FAIL. Do the same after undo/redo.
- **Perceptual image diff (C).** Not bit-exact: a tolerance-based metric (per-pixel
  distance with a threshold, or a structural/perceptual metric) so anti-aliasing / font
  rendering differences don't cause false failures. The GATE is that a re-render of the
  same scene passes within tolerance and a deliberately-changed scene fails — i.e. the
  diff has real discriminating power, not a rubber-stamp.
- **Hang detection (D).** A circular-ref or pathological input that hangs shows up as a
  `timeout` expiry — **record the timeout as the finding**, don't let it wedge the run.

## 4. Investigation A — Resident-cache shift-on-structural-edit (design space, not a locked mechanism)

The Phase-3 crux. The **goal is fixed; the mechanism is an output of the investigation.**
Goal: the always-resident style+geometry cache stays **provably in sync** with IronCalc
across insert/delete row/col and undo/redo, at an acceptable cost at Excel scale. A picks
+ justifies + **validates against IronCalc** one design from the space below — it does
not adopt a pre-chosen implementation. (Same discipline as Phase-2 §4's SP1 seam.)

### 4.1 What the cache holds (from `projects/style-cache.md`)
Per axis (rows, cols): a **default size** + a **sparse override map** (index → size); a
**default band style** + a **sparse override map** (index → interned `StyleId`). Per
cell: a **sparse override map** ((row,col) → `StyleId`), styles interned/deduped (SP4/SP5
show styles are highly repetitive). Plus a **cumulative-size lookup** the renderer needs
for scroll math (pixel↔index in both directions), currently conceived as prefix sums.

### 4.2 The two operations a structural edit forces
Insert/delete of `k` rows at index `R` (columns symmetric) must do two things, and both
must end up **agreeing with IronCalc's own post-edit state**:

1. **Re-key the sparse maps.** Every override keyed by an index `≥ R` shifts by `±k`;
   overrides on deleted rows are removed (and must be restorable for undo). Cost target:
   **O(overrides shifted)**, not O(rows) — the maps are sparse, so this is small even at
   Excel-max *if* the structure supports a cheap tail re-key.
2. **Patch the cumulative-size lookup** so scroll math stays correct from `R` to the end.

### 4.3 The design space (A picks + justifies + validates one)
- **Cumulative-size structure — three candidates, measure which is needed:**
  - **(a) Dense prefix-sum array** (one entry per row/col): O(1) scroll lookup, but an
    insert/delete is an **O(rows) splice/memmove** and O(rows) memory. At 1M rows a
    `u32`/`f64` array is a few MB and a memmove may still be sub-millisecond — **quite
    possibly good enough; measure before rejecting.** Columns (≤16k) are trivial either
    way.
  - **(b) Derived cumulative from default + sparse deltas:** store only
    `delta = size − default` for overrides; then
    `offset(i) = i·default + Σ_{override j < i}(delta_j)`, with the delta-prefix served
    by a **Fenwick/BIT over the sparse deltas**. A structural edit becomes **O(overrides
    shifted)** (re-key the sparse deltas; the BIT follows), and memory is O(overrides).
    More code; wins only if (a)'s splice is too slow at scale.
  - **(c) Chunked / segment-tree** middle ground: O(√n or log n) both ways. Only if
    (a) fails and (b)'s exactness is awkward.
  Store the sparse override maps in a structure that makes the tail re-key cheap (e.g. a
  `BTreeMap` split at `R`, or an index-remap); the point A settles is whether re-keying +
  cumulative-patch together stay well under a frame at 10⁶.
- **Undo/redo — two candidate strategies:**
  - **Mirror-the-primitive:** the cache applies the inverse structural primitive on undo
    (undo-insert = delete at `R`, undo-delete = insert at `R` restoring the saved
    overrides). Fully local, but the cache must **retain the removed overrides** from a
    delete to restore them.
  - **Re-sync-from-engine:** on undo/redo (rarer than edits), **re-pull the affected band
    from IronCalc** (which owns the authoritative undo stack) instead of maintaining an
    inverse log. Simpler cache; costs a bounded re-read. A measures both and picks by
    cost/complexity.
- **`Model` vs `UserModel`:** A first establishes whether the app builds on `UserModel`
  (needs undo/redo + diff-list) and **whether the SP1 worker seam still holds** for it
  (Is `UserModel` `Send`? Does apply/evaluate block reads the same way?). The cache-sync
  design must fit whichever model the app uses; if `UserModel` isn't `Send`, that's a
  seam-adjustment finding surfaced at the checkpoint.

### 4.4 What "correct" means (the validation contract)
For each of {insert row, delete row, insert col, delete col} and each of {undo, redo of
each}: apply to **both** IronCalc and the cache prototype, then assert, over a sample of
indices spanning the edit point:
- formula **references** shifted correctly (IronCalc-side; A asserts IronCalc does this),
- row/col **band styles** and **cell styles** in the cache == IronCalc's re-read values,
- row/col **sizes** in the cache == IronCalc's re-read values, and
- the cumulative-size lookup yields offsets consistent with the shifted sizes.
Plus an **`.xlsx` round-trip** of a structurally-edited sheet to confirm persistence.

### 4.5 What A builds & asserts
A headless prototype (no GPUI): the `UserModel` probe + a correctness harness that runs
the §4.4 contract (GATE: correctness proven + cache agrees with IronCalc + undo/redo
covers value/style/structural) and a cost harness (DISCOVERY: structural-edit +
cache-shift cost at 10⁵–10⁶). Output: the **locked cache-sync design** (chosen
cumulative structure + undo strategy, justified by the measured numbers) and the
**`Model`-vs-`UserModel` recommendation** (+ whether the SP1 seam carries over).

## 5. Investigations B / C / D — design-level (detail → phase plans)

- **B (api-audit):** a `UserModel`/`Model` probe crate that walks a **checklist** (spec
  §6 B) and marks each API **present / absent / workaround**, each entry backed by a
  runtime probe or a source citation (`~/.cargo/registry/.../ironcalc*-0.7.1/`). The
  headline is **display formatting** — a probe that asks IronCalc for a cell's displayed
  string under a number format and records whether the engine formats it or hands back a
  raw value (→ FreeCell owns number-format rendering, a real renderer scope item). Output:
  the coverage matrix + a plan per gap + a flag on any load-bearing absence.
- **C (ci-rendering):** two halves. **In-container:** investigate GPUI's render/test
  surface for an offscreen (windowless) capture path and *attempt* it — expected to fail
  with no GPU; the failure mode is the finding. **macOS (human-run):** a minimal crate
  that renders the raw-gpui grid (evolving Phase-1 `04-ui-poc`) to a **PNG**, commits a
  baseline, and runs a **perceptual diff** (tolerance-based) of a re-render vs baseline
  and of a deliberately-changed scene (must fail) → confirms discriminating power. Output:
  the confirmed CI mechanism (headless if possible, else Mac-CI) + the harness + baseline.
- **D (robustness):** a probe crate on the harness `Model` adapter: feed **cycles**
  (`A1=A1`; `A1=B1,B1=A1`), **malformed/pathological** formulas (giant, deeply nested,
  syntactically invalid), assert IronCalc returns typed errors and **does not hang/panic**
  (foreground `timeout` catches a hang), and test a **worker-panic-recovery** path if
  `evaluate()` can panic (does the SP1-style worker thread survive; is `catch_unwind` /
  restart needed). Output: circular-ref + malformed-input behavior + the worker-recovery
  recommendation.

## 6. Error Handling, Testing, Dependencies

- **Error handling:** `anyhow`; an unmet target / a hang / a missing API is a **recorded
  finding**, never a silent skip or a panic that hides the result. D specifically asserts
  IronCalc's *own* error handling is graceful.
- **Testing:** each investigation ships **correctness assertions + benchmarks**;
  force+assert every measured op; A's cache-vs-engine agreement is the load-bearing test.
- **Dependencies:** `ironcalc`/`ironcalc_base` pinned to **0.7.1** (round-2 harness
  version). A/B additionally use the `UserModel` API from those same crates. `criterion`
  dev-dep. C pins **GPUI** on macOS (git ref per Phase-1 `04-ui-poc`) + a PNG encoder + a
  perceptual-diff crate. **No new engine; no GPUI in the in-container crates** (A/B/D are
  headless IronCalc only).

## 7. Doc Organization (1-phase) & Phase Plans

**Single `architecture.md`, no `components/` dir** (same as Phase 1–2 — the technical
content fits well under ~300 lines of substance and the investigations are independent).
Detailed per-investigation design — A's exact probe/assertion list and chosen data
structures, B's full checklist, C's harness/diff specifics, D's input corpus — is written
into each phase's **`phase_plans/phase_<X>.md` by its lead agent** at implementation time,
against the real IronCalc 0.7.1 / GPUI API. The one problem that couldn't wait (A's
cache-sync design space) is framed above (§4); A *closes* it with measured evidence.

## 8. Risks (technical)

- **`UserModel` breaks the SP1 seam (A) — top risk.** SP1 proved the seam on `Model`. If
  `UserModel` isn't `Send` or blocks reads differently, the worker-owned seam needs
  adjustment — surface at the checkpoint, don't paper over it.
- **Cache-shift worse than O(shifted), or can't be made to agree with IronCalc cheaply
  (A).** Then the resident-cache architecture needs rework. §4's dense-array fallback
  exists precisely so "the sparse design is too clever/slow" still has a measured escape
  hatch — but it must be *measured*, not assumed.
- **Display formatting is FreeCell's job (B).** If IronCalc hands back raw values, the
  renderer must implement Excel number-format rendering — real scope; flag it now.
- **No headless GPUI capture (C).** Likely; then CI-snapshot is Mac-CI-only. Confirm at
  least one working mechanism end-to-end, or the rendering-test north star needs rethink.
- **Instrumentation/agreement opacity.** If IronCalc doesn't expose a getter needed to
  *verify* a shift (e.g. re-reading a band style at an index), record the coarsest honest
  check and flag the gap rather than asserting agreement you can't observe.
- **Frozen-crate / version drift.** Keep the 0.7.1 pin so numbers stay comparable to
  round-2; note any forced bump as a finding.
