# FreeCell — Phase 3 (Round-3) Synthesis

> Stage-3 **pre-build** input. Consolidates the four Round-3 investigations (A–D) into a
> **clear-to-build / must-change-first** recommendation for FreeCell. Evidence lives under
> `experiments/round-3/{A,B,C,D}-*/findings.md`; the plan and pass criteria are in
> `specs/projects/freecell-phase-3/`. This round closes the **last architectural unknowns
> before committing to the real build** — Phase 2 validated *reading, recomputing,
> rendering* (`round-2/SYNTHESIS.md`, verdict BUILD); Round-3 validates the parts those
> never touched: the **interactive editing model** and a few targeted checks. All A/B/D
> numbers are a 4-core / ~15 GB Linux container **floor** (real hardware is faster); C's
> in-container half is authoritative for the diff, its render half is macOS/human-run.

## Verdict: **CLEAR TO BUILD**

**No off-ramp fired in any investigation.** The build-readiness checkpoint is clean.

A — the crux (style/geometry cache-sync vs structural editing) — passed every GATE:
`UserModel` is `Send`, the SP1 worker seam carries over unchanged, insert/delete row/col
shift references + band styles + sizes correctly, undo/redo fully un-shifts, and the
resident-cache shift design is **locked and validated to agree with IronCalc** at a
negligible measured cost. B found **no surprise load-bearing gap** — the one
renderer-critical question (who owns number-format display) resolves in the engine's
favor. C **confirmed the CI-snapshot strategy is buildable** (the perceptual-diff harness
is proven in-container; the offscreen-capture mechanism is source-confirmed) with the
end-to-end empirical demo **not yet demonstrated** — GPUI could not run in this no-GPU /
proxy-blocked container, so the authored render harness is **pending a macOS run**, and the
team additionally chose not to run the throwaway-stub demo now (no real grid to snapshot
yet), deferring it to when the real grid exists — not a failure, not an off-ramp, not a
full GATE pass. D proved circular refs and malformed input are handled gracefully, and
surfaced one well-understood, cheaply-mitigated crash mode (a deep-recursion parser abort)
as a FreeCell-side input-cap requirement.

After Phase 3 it is **building, not de-risking.** A well-evidenced "the last unknowns are
closed" is the successful outcome here — and they are.

## What each investigation established

| # | Investigation | Result vs pass criteria | The single load-bearing lesson |
|---|---|---|---|
| **A** | Style/geometry cache-sync + structural editing *(the crux)* | **GATE PASS** ×3 (structural shift; undo/redo covers value+style+structural; validated cache-sync design agrees with IronCalc, reversible) + DISCOVERY recorded. No off-ramp. (`A/findings.md`, 17 runtime GATE tests (incl. a negative control) + a compile-time `Send` proof) | **The SP1 seam carries over to `UserModel` and the resident cache is buildable as designed.** `UserModel<'static>` is `Send`; its structural edits + `set_user_input` auto-run the same full, `&mut self`, non-incremental `evaluate()` SP1 characterized — so worker-owns-the-model + coalesce + publish-viewport holds verbatim. Cache-sync design **LOCKED**: dense prefix-sum cumulative (candidate (a) — the simple option, measured good-enough), `BTreeMap` sparse-override re-key (O(overrides shifted)), mirror-the-primitive undo — **provably agrees with IronCalc** after every insert/delete/undo/redo. Cache shift is **sub-ms even at 1M** (0.44 ms) vs IronCalc's own `insert_row` at **~4.6 s @1M** (engine-side, O(cells moved), absorbed by the worker seam). |
| **B** | Needed-API audit *(breadth)* | **DELIVERABLE PASS** (27-row present/absent/workaround matrix, reproducible) + **GATE (judgment) PASS** (no surprise load-bearing gap). (`B/findings.md`, 14 tests) | **Display formatting is ENGINE-OWNED — FreeCell does NOT implement number-format rendering.** `get_formatted_cell_value` / the public `format_number(value, fmt, locale) -> Formatted{text,color,error}` produce the display string (`1234.5`→`"1,234.50"`, `1.0`→`"100.00%"`, serial `44197`→`"2021-01-01"`) + color for `[Red]` formats. The prime-suspect renderer-scope risk is **cleared, no scope**. The rest of the matrix holds no surprises: 14 present, 4 workaround, 9 absent — every absence is either small FreeCell-side view-model work or a **pre-known** product-scope item. |
| **C** | CI snapshot rendering | **Strategy confirmed buildable; end-to-end empirical demo NOT DEMONSTRATED (pending a macOS run + deferred by choice).** DISCOVERY (Q1) ANSWERED; GATE (Q3, perceptual-diff discriminating power) MET in-container; GATE (Q2, end-to-end render→PNG) authored + source-confirmed but **not demonstrated** (not a full GATE pass). No off-ramp. (`C/findings.md`, 6 tests) | **The rendering-test north star is buildable, on a macOS runner.** GPUI *can* capture windowless/offscreen, but the headless renderer is **Metal/macOS-only** at our pinned Zed rev (`current_headless_renderer()` is `None` off-macOS; no Linux/CPU/Vulkan path) → CI = a `macos-*` runner doing **offscreen** (`show:false`, no display needed) Metal capture, **the exact path Zed's own `visual_test_runner.rs` uses**. The perceptual diff (tolerance + fraction, Zed's metric shape) is **proven** in-container. The end-to-end render→PNG→diff demo is **not yet demonstrated for two compounding reasons, both true:** (a) *environment-forced* — GPUI could not run in this container at all (no GPU, and the Zed source is proxy-blocked / HTTP 403), so the render harness — though fully authored and source-confirmed against Zed's own `visual_test_runner.rs` — was **never runnable in-container** and is **pending a human macOS run**; and (b) *deliberate deferral* — at the build-readiness checkpoint the team chose not to run the throwaway-stub macOS demo now (there is no real FreeCell grid to snapshot yet, so it would validate a stub), deferring the empirical demo to when the real grid exists. The strategic question is answered; the empirical demonstration remains outstanding. |
| **D** | Engine robustness *(cheap)* | **GATE PASS** ×2 (circular refs → typed error, no hang; malformed → typed error, no panic) + DISCOVERY + DELIVERABLE (worker-recovery recommendation). No off-ramp. (`D/findings.md`, 9 tests) | **Cycles and bad input are safe; deep nesting is the one crash mode, and it's a FreeCell-side input cap.** Self / mutual / 1000-ring cycles return `#CIRC!` (`CellType::ErrorValue`) in single-digit ms via a marker guard (not deep recursion) — no hang. Malformed input is handled gracefully — a typed error (or, in one case — `="unterminated` — graceful string recovery) — **never a panic**. **DISCOVERY:** IronCalc's recursive parser has **no depth cap** → a deeply-nested formula OR a long flat operator chain **stack-overflows into a process ABORT** (~490-depth / ~2832-term on the worker's default ~2 MiB stack), which `catch_unwind` **cannot** catch. The worker (a spawned thread, smaller stack) is *more* exposed than the main thread. Not an IronCalc defect that blocks the build — a cheap, well-understood pre-eval input cap. |

## The cross-cutting picture

Round-2 established that IronCalc's **correctness and file fidelity are strengths** and its
**single-threaded / non-incremental / no-change-stream architecture is the source of every
caveat.** Round-3 extends that same picture to the **interactive** layer and finds it
holds:

- **The editing model reuses the recompute seam.** `UserModel` is just `Model` + an
  undo/redo history + a diff-list, all `Send`; its structural edits auto-run the same
  blocking full `evaluate()`. So Round-3 adds **no new concurrency model** — the SP1
  worker seam (worker owns the model, edits coalesce, viewport publishes on completion,
  render loop watches a generation counter) is the one seam, now carrying `UserModel`. The
  multi-second structural-edit latency at 1M rows is the **already-accepted** "staleness ≈
  one eval" applied to insert/delete, not a new blocker (A §6).
- **The engine owns more than expected, in FreeCell's favor.** Display formatting (B) is
  engine-owned — the single biggest potential renderer-scope item evaporates. The resident
  cache holds only sizes + styles (which don't change on recompute) + a cumulative-size
  lookup; A proved it stays in lockstep across structural edits cheaply, so the grid
  renders **fully styled and correctly laid-out during a recompute — only values lag**, as
  the round-2 adoption intended.
- **The diff-list is a replica-sync transport, not a surgical-update channel.** A and B
  both confirm it is **opaque bitcode** (`pub(crate)` `Diff` enum), edit-sites only.
  FreeCell drives surgical UI updates by **mirroring the op it issued** (it originates
  every edit, so it knows `kind/at/count`) — which is exactly the locked cache-sync
  strategy. Consistent across both investigations.
- **The remaining rough edges are small and known.** A few view-model responsibilities
  land on FreeCell (sheet reorder, hidden columns, zoom, its own function list) and the
  pre-known xlsx-writing-forcing features (merges, conditional formatting,
  comments/validation/hyperlinks) are re-confirmed absent — all already on the record, none
  a surprise. The one genuinely new hazard (D's parser abort) is a cheap input cap.

## Adopted decisions confirmed by Phase 3 (fold into the plan of record)

Round-2 adopted a set of baseline decisions; Phase 3 stress-tested the load-bearing ones
and **confirms them** — they move from "adopted, pending validation" to "validated." These
are decided for the real build, not "maybe later":

- **Build the app on `UserModel`** (per A). It is the only public path to undo/redo +
  structural edits + the collaborative diff-list, it is `Send`, and it **does not change
  the SP1 seam**. `Model` remains reachable read-only via `get_model()` for the style/size
  getters and the `.xlsx` export path.
- **The resident style/geometry cache-sync design is LOCKED** (per A): dense prefix-sum
  cumulative-size array (candidate (a) — measured good-enough, so no Fenwick/chunked
  needed for v1); `BTreeMap` sparse-override maps re-keyed on shift (O(overrides shifted));
  **mirror-the-primitive** undo (the cache applies the inverse structural op, restoring
  saved overrides). Proven to agree with IronCalc after every insert/delete/undo/redo, with
  a negative control confirming the agreement check has real discriminating power. The
  round-2 adoption of "an always-resident full style + geometry cache" now has a validated
  sync mechanism.
- **Native display formatting is engine-owned** (per B) — reaffirms and *strengthens*
  round-2's "formatting = IronCalc-native styles, no side-table" decision: not only are
  styles engine-owned, but **the rendered display string itself is too.** FreeCell calls
  `get_formatted_cell_value` (or `format_number` from a display-cache) per visible cell and
  renders that text + optional color. **No Excel number-format engine on the FreeCell
  side.**
- **CI snapshot tests run on a macOS runner via GPUI offscreen Metal capture + a
  perceptual diff** (per C): `VisualTestAppContext` + `current_headless_renderer()` →
  `capture_screenshot` → PNG → the tolerance+fraction perceptual diff. `show:false`, so no
  display/virtual framebuffer is needed — it fits an unattended CI job, and it is the path
  Zed itself uses upstream. (The empirical end-to-end demo is **not yet demonstrated** —
  unrunnable in this no-GPU/proxy-blocked container, pending a macOS run, and deferred until
  a real grid exists; see carry-forward #2.)

## Build-time carry-forward agenda (ranked — this is the real output)

New items from Round-3 first, then the still-OPEN round-2 items carried forward so they
are not lost.

1. **D's pre-eval input cap — do this early in the build (load-bearing).** Before handing a
   formula to IronCalc, reject (surface as `#ERROR!` in the UI) any formula exceeding a
   **nesting-depth** and **length** bound. Match Excel's own limits as a conservative,
   spec-compatible ceiling: **depth ≤ 64, length ≤ 8192 chars** — both well under the
   measured ~490-depth / ~2832-term worker ceilings. Pair with **(2)** a larger worker
   stack (`thread::Builder::stack_size(~64 MiB)`, raising the ceiling ~30×) and **(3)** a
   `catch_unwind(AssertUnwindSafe(...))` around the worker's apply+evaluate
   (belt-and-braces — catches nothing today but degrades a future IronCalc `panic!` to a
   recoverable per-edit error). The cap is the real fix (it eliminates the only crash mode,
   the uncatchable stack-overflow abort, at its source); stack + `catch_unwind` are cheap
   defense-in-depth. Exact limits are a build/product tuning call (`D/findings.md` §Risks).
2. **C's outstanding empirical snapshot demo — run it, then close it against the real
   grid.** The strategy is confirmed and the harness fully authored, but the end-to-end
   render→PNG→perceptual-diff has **never been executed** — it was unrunnable in this
   no-GPU / proxy-blocked (HTTP 403 on the Zed source) container, so it is **pending a human
   macOS run**; and the team additionally chose not to run the throwaway-stub demo now (the
   current demo would validate a stub, not a real grid). What remains is running it
   **end-to-end on a macOS runner once the real grid exists.** When closing it: capture baselines
   on the **same CI runner class** you validate against (Metal AA / font versions vary by
   machine), and tune the tolerance/fraction from the first real re-render-vs-baseline PASS
   + changed-scene FAIL. Watch for small GPUI API drift vs the pinned Zed rev
   (`C/findings.md` §Risks). A non-Mac headless path would need an upstream GPUI
   blade/Vulkan `PlatformHeadlessRenderer` (not present at our rev) — stick with the macOS
   runner.
3. **B's small view-model / FreeCell-side gaps (not load-bearing).** FreeCell owns, in its
   own view model: **sheet reorder** (no `move_sheet`; reorder the worksheet vector on
   export), **hidden columns** (`Col` has no `hidden` field) and **zoom** (view-only,
   neither round-trips through IronCalc), and its **own function-name list** (the 345-variant
   `Function` enum is a private module — ship a small static table for autocomplete, validate
   typed names via parse → `Node::InvalidFunctionKind`). **Hidden rows** are a WORKAROUND
   (public `Row.hidden` field, xlsx round-trips, but no `UserModel` setter — set via the
   field/`Model` path or upstream a setter). Freeze panes / gridlines / selection /
   row-visibility use IronCalc and *do* round-trip. Formula bar uses `get_cell_content` +
   the public `Lexer` + `Parser`/`Node` (all present) for tokenizing / reference
   highlighting.
4. **Still-OPEN items carried forward from round-2 (re-confirmed by Phase 3, not
   designed).** These were OPEN at the round-2 checkpoint and remain so — carried here so
   the build plan of record does not lose them (see `round-2/SYNTHESIS.md` §"Round-3 /
   real-build agenda"):
   - **Merges + conditional formatting — no IronCalc public API** (A §2(d), B §8 confirm
     `merge_cells`/no-CF have no reachable setter/getter). Persisting either likely forces
     FreeCell to **own `.xlsx` writing** (~10× scope). Major features, each needing its own
     technical design.
   - **Dynamic arrays / spilling = 0/17** (SP3, re-confirmed B §8). A **product decision**,
     not a technical unknown: accept for v1 / build spill / contribute upstream. Deserves an
     explicit call before or early in the build.
   - **Comments (lossy on save) / data validation / hyperlinks** (B §6) — fidelity features
     that also force owning `.xlsx` writing if pursued; product-scope, pre-known.
   - **GPL #55470** — the GPL-3.0 transitive dep, a **pre-distribution** packaging/legal fix
     before shipping a proprietary binary (tracked; not a technical unknown).
   - The rest of the round-2 build agenda still stands (FreeCell-owned dirty-tracking +
     viewport value cache; large-file open latency; styled-read binding layer; minor SP5
     fidelity losses) — see `round-2/SYNTHESIS.md`.

## Bottom line

**Proceed to build FreeCell on IronCalc + GPUI.** The last pre-build architectural
unknowns are closed: the interactive editing model reuses the proven SP1 worker seam
(`UserModel` is `Send`), the resident style/geometry cache has a **validated, locked**
sync design that provably tracks IronCalc across structural edits and undo/redo at sub-ms
cost, display formatting is engine-owned (no renderer number-format scope), the CI
rendering-test strategy is confirmed buildable, and the engine is robust to cycles and
malformed input. No off-ramp fired.

Two things to do early in the build: **(1) implement D's pre-eval formula input cap**
(depth ≤ 64 / length ≤ 8192) + larger worker stack + `catch_unwind` — the one genuinely new
hazard, cheaply closed; and **(2) make the explicit product decision on dynamic arrays**
(accept v1 / build spill / upstream). Then close C's empirical snapshot demo against the
real grid on a macOS runner when it exists. Everything else is an ordered engineering
agenda, most of it already framed by the round-2 synthesis.
