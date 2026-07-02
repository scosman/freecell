# FreeCell — Phase 1 Final Synthesis (Sub-project H)

> Status: **complete.** This is the capstone of Phase 1 (Stage 1 of FreeCell): the
> **go / no-go** artifact that feeds the human's Stage-3 "do we keep going" decision.
> It is a **synthesis** — no benchmarks were run here. Every quantitative claim is
> cited from a committed Phase-1 findings doc (Sub-projects A–G) and traceable to the
> recorded results under `experiments/*/results/`.
>
> **Environment for all cited engine/perf numbers:** Linux x86_64, **4 logical cores**
> (`Intel(R) Xeon(R) @ 2.80 GHz`), ~15 GB RAM, **no GPU / no display**, Rust 1.94.1.
> Engines: **Formualizer 0.7.0**, **IronCalc / ironcalc_base 0.7.1**. Perf figures are
> Sub-project C's **fairness-corrected** `full`-profile run (commit stamp `783a515`),
> dated 2026-07-01. UI perf is **not** measured in-container (no GPU); the GPUI PoC is
> code-complete and human-validated on macOS/Metal (see §4, §7).
>
> **The stack decided in Phase 1:** engine = **IronCalc** (human sign-off, Sub-project
> G); UI = **GPUI** — a **raw-gpui custom grid** for the sheet + **gpui-component** for
> the surrounding chrome (human-validated on macOS: "worked great").

---

## 1. Verdict

**GO, with conditions.** Phase 1 de-risked the FreeCell thesis — a GPU-rendered,
Rust, Excel-compatible spreadsheet that is stupid-fast on huge sheets — hard enough
to justify investing in Stage 2 on the **IronCalc + GPUI** stack: viewport reads clear
the frame budget by ~4× on both engines, both engines reach 10⁷ cells, GPUI is a
proven-fast renderer with a working custom virtualized grid, and file/format I/O
round-trips. **Confidence: moderate-to-high on the stack being buildable; lower on the
performance ceiling** — because the single hardest target (a 1M-cell dependency
cascade in <100 ms) **failed on both engines** (~2 s), IronCalc has **no incremental
recalc** so a full `evaluate()` fires on every edit, and two §5.4 items (end-to-end
large-`.xlsx` open; a real 120-fps grid run at Excel-max) are **not yet measured**.
The conditions are the Round-2 agenda (§5, Sub-project F): they are validations, not
known blockers. A well-evidenced "proceed" — not a hedge, and not a claim that the
performance bar is already in hand.

---

## 2. What Phase 1 set out to de-risk, and the answers

Phase 1's job (functional_spec §1, §5.5) was to answer *"can we build FreeCell on this
stack and hit the bar?"* — with reproducible evidence, not vibes. The core
uncertainties and where they landed:

| Core uncertainty | What we found | Status |
|---|---|---|
| **Engine viability** — is there a real, usable Rust calc engine? | Both Formualizer and IronCalc build, load `.xlsx`/CSV, evaluate, mutate, and recompute; head-to-head bake-off run across file/perf/formatting (A, B, C, D). Human chose **IronCalc** for delivery risk (G). | **PROVEN** |
| **Huge-sheet perf (load + memory)** — does the stack scale toward Excel-max? | Both engines reach the **10⁷-cell** target; IronCalc: **6.13 s / 1.63 GB** (~162 B/cell) via native `set_user_input`; Formualizer (the engine we passed on) is ~3.5× faster / ~9× denser via Arrow ingest. IronCalc is not disqualified but structurally cedes density (C). | **PARTLY PROVEN** (in-memory ingest at 10⁷; not end-to-end file open) |
| **Viewport read latency** — can we pull a scrolling viewport inside a frame? | **Yes, comfortably.** Best design (D2 bulk) p99 = **392 µs on IronCalc** / 222 µs on Formualizer, vs the **<2 ms** target — passing by ~4–9× on an ~1,800-cell window (C). | **PROVEN** (at Phase-1 scale, values-only) |
| **Dependency cascade recompute** — is a big recompute fast? | **No.** 1M-cell `=PREV+1` chain recomputes in **~2.11 s (IronCalc) / ~1.87 s (Formualizer)** vs the **<100 ms** target — **both FAIL by ~19–21×**. Inherently serial + linear; async off-thread recompute is mandatory regardless of engine (C). | **UNPROVEN / gap** |
| **GPU UI at scale** — can GPUI render an Excel-max grid at 120 fps? | Custom virtualized raw-gpui grid built (2D variable-size virtualization, O(n/512) memory); engine-neutral core CI-tested (20 tests); human confirmed it renders/scrolls well on macOS ("worked great"). **Numeric §5.4 "Run Test" gates not yet recorded** (GPUI can't build headless) (E). | **PARTLY PROVEN** (built + felt fast; not yet numerically gated) |
| **File I/O** — read/write modern `.xlsx` + CSV, round-trip? | Both engines round-trip values, formulas, multi-sheet, cross-sheet refs, dates, booleans (asserted by tests). IronCalc: native styled writer, **persists cached results**, but **no CSV** (40-line bridge). Formualizer: first-class CSV but drops cached results + no styles on read (B). | **PROVEN** (structure/values/formulas) |
| **Formatting foundation** — can we read/preserve styles? | IronCalc surfaces styles **natively** (read+write+round-trip: bold/italic/size/fill/border/align/num-fmt, probe-backed). Gaps: **no merges API, no conditional-formatting API**. Design: engine-neutral **`FormatStore`** side-table as render source of truth (D). | **PARTLY PROVEN** (representative attributes; long-tail fidelity + merges/CF deferred) |
| **Excel-compat foundation** — is function coverage credible? | IronCalc: **345** registered builtins (source-counted); Formualizer: 410. Both below Excel's ~500; **neither publishes a per-function parity matrix**. Raw count ≠ verified parity (G). | **UNPROVEN** (count known, parity un-audited) |

**Reading the table:** the everyday interaction (scroll → read a viewport) is proven
fast; the extreme worst case (a million-deep instant cascade) is proven *not*
instant; and the two biggest still-open questions — a real 120-fps grid run and an
end-to-end big-file open — have credible-by-design paths but **no recorded number
yet**. That is exactly the shape of a "go-with-conditions," not a "done."

---

## 3. The stack we're recommending to build on

### Engine — IronCalc (native styled I/O + funded team; nested-HashMap storage)

**Why.** The engine call was a genuine toss-up (G ends on a *soft* lean toward
Formualizer; the human overrode to IronCalc). IronCalc wins on the axes that most
reduce **delivery risk**: it is a funded, multi-contributor project (~3,978 stars, 29
contributors, NLnet-funded, shipping since 2023) versus Formualizer's effectively
single-author 0.x crate (~775 of ~805 commits by one owner, ~944 downloads). It has a
**native styled `.xlsx` reader/writer** that preserves styles and number formats
through a real round-trip and **persists cached formula results** (so a reloaded
workbook shows values before any recompute — B, D). Its formula-graph build is ~4.4×
faster (~6 s vs ~27 s for a 1M-node chain) and its wide `=SUM(range)` fan-out
recomputes ~45× faster (77.5 ms vs 3.51 s) than Formualizer's (C). It still reached
the 10⁷-cell target. **What it costs:** its storage is a nested `HashMap<i32,
HashMap<i32, Cell>>` — **~9× less memory-dense (~162 B/cell)** and ~3.5× slower to
load 10⁷ cells than the Arrow columnar alternative FreeCell was scoped to prefer, so
we deliberately ceded some of the "stupid-fast on huge sheets" headroom. It has **no
incremental recalc** (every edit = full-workbook `evaluate()`), **no native range
read** (per-cell viewport loop — still passed <2 ms), **no CSV**, and **no merges /
conditional-formatting API**. We accepted a more finished engine over a faster storage
model — a defensible optimization for a Stage-1 de-risking project, but one whose
huge-sheet cost is real and must be watched (§5).

### UI — GPUI (raw-gpui grid + gpui-component chrome)

**Why.** GPUI is the only surveyed option that has *already shipped* 120-fps GPU UI at
scale (Zed 1.0, "renders like a videogame" over 200k-line files) — the strongest
evidence in the whole survey that a Rust GPU UI can hold the frame budget (A). Both
crates are permissively licensed (Apache-2.0). The PoC settled the internal split: the
**grid itself is raw-gpui** because gpui-component's `DataTable` can't do variable row
heights (`uniform_list`-based) and materializes a per-column object at 16,384 columns —
both wrong for a spreadsheet — while **gpui-component is kept for the surrounding
chrome** (menus/dialogs/panels), where it's the fast path (E, F). The human ran both
variants on macOS and confirmed it "worked great." **What it costs:** GPUI is coupled
to Zed — the real ecosystem **git-pins a specific Zed commit** (pre-1.0, no semver,
sparse docs, the crates.io `gpui` lags `main` ~8 months), so we carry periodic
rev-bump maintenance. There is also a **GPL-linkage caveat** (issue #55470: a default
build statically links GPL-3.0 object code via `gpui → sum_tree → ztracing` — a runtime
no-op with a trivial fix, but present) that **needs legal sign-off** before shipping a
proprietary binary (A). And building a real grid (inline editing, selection ranges,
frozen panes) on raw-gpui is more from-scratch work than adopting a table component
would be — a cost we took on purpose for control over the render path.

---

## 4. What is PROVEN (with the key numbers)

Everything here is backed by a passing test or a recorded result under
`experiments/*/results/`.

- **Viewport reads clear the frame budget on both engines.** Best design (D2 bulk
  read) p99 = **392 µs (IronCalc)** / **222 µs (Formualizer)** vs the **<2 ms** target;
  every binding design (D1/D2/D3) on both engines came in ≤ 585 µs p99 for an
  ~1,800-cell window. **IronCalc clears it without a native range API** (per-cell loop).
  This is the everyday scroll-and-read path, and it passes with ~4–9× margin (C).
- **Both engines reach the 10⁷-cell target.** IronCalc loads 10M `f64` literals in
  **6.13 s at 1.63 GB peak RSS** (fresh process, ~162 B/cell) via `set_user_input`;
  Formualizer via Arrow bulk-ingest in **1.73 s at 0.18 GB** (~17 B/cell). Both are
  sane multiples of the raw data; the Arrow store is ~9× denser (C).
- **GPUI renders the grid fast** — human-confirmed on macOS/Metal ("worked great"),
  and the **engine-neutral PoC core is CI-verified in-container** (`poc-core`: 20
  tests green, `clippy -D warnings` clean). The virtualization is proven to scale to
  **Excel-max rows without O(n) memory** (segment-summed prefix sums + binary search,
  ~16 KB for the full 1,048,576-row axis) and the scripted "Run Test" harness is
  deterministic and gate-wired to the §5.4 targets (E).
- **`.xlsx` + CSV round-trip works.** Both engines round-trip number/text literals,
  booleans, formula text, multiple sheets, cross-sheet formulas, and dates-as-serials
  — each asserted by a passing test (B).
- **IronCalc native styled I/O + persists cached results.** Styles (bold, italic, font
  size, fill, border, alignment, number format) and row/col sizes read, write, and
  **survive a real `.xlsx` round-trip** (probe-backed, D). Cached formula results
  **persist** in the file, so a reloaded workbook displays values *before* any
  `evaluate()` (B) — a real open-time-UX win over Formualizer, which drops them.
- **File / format capability matrix** (from B + D):

  | Capability | IronCalc 0.7.1 | Formualizer 0.7.0 |
  |---|---|---|
  | `.xlsx` read / write | Native (styled) | calamine read / umya `to_xlsx_bytes` write |
  | CSV read / write | **None** (40-line RFC-4180 bridge) | First-class `CsvAdapter` |
  | Cached formula results in file | **Persisted** | Dropped (recompute-on-open mandatory) |
  | Styles on read path | **Native** `get_style_for_cell` | `style: None` everywhere (needs umya side-load) |
  | Style round-trip (representative attrs) | **Survives** (probe-backed) | Only via a directly-owned umya workbook |
  | Merges / conditional formatting | **No API** (side-store needed) | Via umya (merges probed; CF unverified) |
  | Registered function count (source) | 345 | 410 |
  | Storage model | Nested `HashMap` (~162 B/cell) | Apache Arrow columnar (~17 B/cell) |

---

## 5. What is UNPROVEN / the top risks carried into Stage 2

These are the conditions on the GO. **Sub-project F (`05-round-2-proposal/`) is the
agenda that closes them**, and its ranking mirrors this list.

1. **IronCalc's no-incremental-recalc, full-`evaluate()` cost — the #1 risk.**
   IronCalc has **no incremental recalc**: every committed edit fires a
   **full-workbook `evaluate()`**, which is **O(all cells)**. The measured failure
   envelope is already on record: the 1M-cell `=PREV+1` cascade recomputes in
   **~2.11 s p50** (vs the §5.4 **<100 ms** target — FAIL by ~21×), and even a 100k
   "cascade → visible" is **107 ms** (FAIL vs the 16.6 ms frame). This fires on *every
   keystroke-commit*, so **async / off-thread recompute with a "recalculating" state is
   mandatory** — and it is **unvalidated at scale**: we have a ~2 s point sample, not a
   size×density sweep, and no prototype of the debounced, cancellable, off-thread
   binding. This is a daily-papercut risk, not just a worst-case one. (F #1.)

2. **End-to-end large-`.xlsx` open — §5.4's one un-run target.** Nobody measured
   *"open a 100 MB+ `.xlsx`; record load time and peak memory."* Our 10⁷-cell numbers
   are **in-memory `set_user_input` ingest (6.13 s / 1.63 GB)** — not a real file open,
   which additionally pays OOXML unzip + XML parse + shared-strings cost and a
   parse-time peak-RSS spike, **with styles loaded** (IronCalc reads styles on the same
   path). At ~162 B/cell (~9× less dense than the alternative we passed on), the memory
   answer at Excel-max is genuinely uncertain (B, C, G both flag this). (F #2.)

3. **Function-parity audit — 345 registered vs Excel's ~500.** "Excel compatibility
   required" is a headline product promise, but all we have is a **raw registry count**
   (345 IronCalc builtins), not a parity audit. A registered function can still diverge
   from Excel on edge cases, error semantics (`#DIV/0!`/`#N/A`/`#VALUE!` propagation),
   locale/date behavior, or array/spill semantics. This axis was Formualizer's main
   non-perf edge (410), traded away on maturity grounds — that trade must be *validated*,
   and G explicitly names it as the kind of finding that could reopen the engine choice.
   (F #3.)

4. **GPUI/Zed git-pin maintenance + GPL #55470 legal sign-off.** The stack git-pins a
   specific Zed rev (pre-1.0, no semver, ~8-month crate lag) → budget periodic rev-bump
   upgrades. Issue **#55470** statically links GPL-3.0 object code via `gpui → sum_tree
   → ztracing` (runtime no-op, trivial fix, but present) → **needs legal sign-off and a
   merged-fix check on the pinned rev before shipping a proprietary binary** (A, F #8).

Also carried forward (lower rank, in F): the IronCalc-specific binding layer under
*real formatting* at real scale (#4); the engine-neutral `FormatStore` prototype +
its unsolved row/col insert-delete shift (#5); long-tail style-roundtrip fidelity +
a merges / conditional-formatting side-store (#6); GPUI grid maturation (inline
editing, selection, frozen panes) + recording E's still-pending §5.4 "Run Test"
numbers + PNG rendering baselines (#7); and residual A–E gaps — CSV hardening,
IronCalc load-API friction, untested recompute shapes (cross-sheet/volatile),
storage-density extrapolation to true Excel-max (#8).

---

## 6. Meta-outcome — the north star is tractable

Two Phase-1 by-products matter beyond any single number, because they are direct
evidence that the product's "deep testing" north star (project_overview: *"we assume
we're going to find bugs… a test suite that grows over time"*) is achievable:

- **A reusable, engine-neutral test/benchmark harness.** Phase 1 stood up a frozen
  `shared/datagen` (committed generators — no hand-made binary fixtures), a shared
  `shared/bench_util` (environment-stamped `LatencyStats` p50/p99/max + `GateResult`
  PASS/FAIL wiring), and a **`SpreadsheetEngine` trait** that let *both* engines run
  the *same* scenarios through one driver — which is precisely what made the bake-off a
  fair, apples-to-apples comparison and what makes every Round-2 measurement (F #1–#4)
  drop-in comparable to Phase-1 baselines. The rig is engine-swap-proof by design.

- **A process that catches its own mistakes.** An earlier draft of Sub-project C
  reported a **backwards** conclusion — that Formualizer "couldn't reach 10⁷ cells" and
  was "~5× less dense" — which was an artifact of routing its load through the wrong
  (interactive `write_range`) API instead of Arrow bulk-ingest. **Adversarial review
  caught it and the corrected numbers reversed the build/memory/scale conclusions
  before they drove the engine decision.** A perf methodology that surfaces and
  self-corrects a load-bearing error *before* it reaches a decision is exactly the
  discipline a decades-long Excel-compatibility effort will live or die on.

---

## 7. Honest caveats

Read the GO with these firmly in mind — they bound how far the evidence stretches:

- **All engine/perf numbers are from a 4-core Linux box with no GPU.** A Mac is
  faster on both CPU and GPU, so the container numbers are, if anything, a floor for
  the engine work — but they are **not** the target platform for UI perf, and the
  authoritative §5.4 render/cell-load gates **have not been recorded** yet (E is
  code-complete; the human confirmed *feel*, not measured numbers).
- **Several conclusions rest on Phase-1-scale point samples, not sweeps.** The
  cascade cost is one ~2 s sample at 1M cells; viewport reads are one ~1,800-cell,
  **values-only** window (no per-cell style resolve); the fan-out ceiling was capped at
  1,000×1,000 (5,000² was ~100 s). The *shape* of the answers is trustworthy; the exact
  cost surface at Excel-max × formula-density × real formatting is a Round-2 sweep, not
  a Phase-1 fact.
- **The engine call was a genuine toss-up decided on delivery risk.** G's own lean was
  a *soft* go-with-conditions toward Formualizer (which wins the huge-sheet axis the
  project was scoped around); the human overrode to IronCalc, prioritizing a funded
  team + native styled I/O over peak storage density. That is a defensible optimization,
  **not** a verdict that IronCalc is faster on the north-star axis — it structurally is
  not. If Round-2's large-file-open or parity results disappoint, the decision doc's own
  "what would change the recommendation" section (G §4) is the pre-agreed off-ramp.
- **Excel compatibility — the headline promise — is the least-proven axis.** We know
  the function *count*; we have not audited parity. Nothing in Phase 1 falsified the
  promise, but nothing yet *proves* it either.

**Bottom line for Stage 3:** the stack is buildable and the everyday-fast case is
proven; the extreme-scale and full-compat cases are credible-by-design but unmeasured.
Proceed to Stage 2, treating the §5 conditions — led by IronCalc's async-recompute
validation — as the gates that would trigger a re-think if they don't hold. The
Round-2 list (F) is the plan.
