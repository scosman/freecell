# Sub-project G — Engine Bake-off Decision (Formualizer vs IronCalc)

> Status: **complete** — the engine-choice document for **human sign-off**
> (functional_spec §6.G; architecture §1.1; implementation_plan Phase 7). This is a
> **synthesis** of already-committed bake-off evidence, **not** a new experiment: no
> benchmarks were run here. Every quantitative claim is cited from Sub-projects A
> (`00-stack-decision`), B (`01-file-support`), C (`02-datamodel-binding-perf`), and
> D (`03-formatting`). The one piece of new evidence is a **read-only source count**
> of each engine's registered function set (method in *Function coverage*, below).
>
> Environment for all cited numbers: Linux x86_64, 4 logical cores, ~15 GB RAM, no
> GPU/display, Rust 1.94.1, `Intel(R) Xeon(R) @ 2.80GHz`. Engines: **Formualizer
> 0.7.0**, **IronCalc / ironcalc_base 0.7.1**. Perf figures are the C sub-project's
> **fairness-corrected** `full`-profile run (commit stamp `783a515`).
>
> **Scope.** UI is already settled (**GPUI**, no competitor). This document decides
> only the **engine**. Its output feeds Sub-project H (`SYNTHESIS.md`).

---

## The question

**Formualizer or IronCalc — which engine should FreeCell build on?**

FreeCell's north star is a spreadsheet that is *stupid-fast on huge sheets*
(Excel-max: 1,048,576 rows × 16,384 cols; functional_spec §5.4). The bake-off was
run to find out whether Formualizer's Apache-Arrow columnar core actually delivers
that advantage, and whether it does so at an acceptable maturity/feature cost versus
the more-established IronCalc. The corrected evidence makes this a **genuine
trade-off**, not a one-sided call: perf on the headline axis leans Formualizer;
maturity, formatting, and formula-graph build lean IronCalc.

---

## 1. Dimension-by-dimension comparison

All numbers are recorded evidence from the cited sub-project. "§5.4" verdicts are
against the functional-spec targets. Winner column marks the engine each dimension
favors (— = tie / decision-neutral).

| Dimension | Formualizer 0.7.0 | IronCalc 0.7.1 | Leans | Source |
|---|---|---|---|---|
| **Huge-sheet load** (10M `f64` literals, native loader) | **1.73 s** | 6.13 s | **FZ** (~3.5×) | C |
| **Huge-sheet memory** (10M cells, peak RSS, fresh process) | **0.18 GB** (~17 B/cell) | 1.63 GB (~162 B/cell) | **FZ** (~9× denser) | C |
| **Storage model** | Apache **Arrow** columnar lanes (typed per-column, chunked; delta/computed overlays) | Nested `HashMap<i32, HashMap<i32, Cell>>` (boxed cells) | **FZ** for scan/density at scale | A, C |
| **Formula-graph build** (1M `=PREV+1` chain) | 27.3 s (≈2.4 s stage + ~24 s first `evaluate_all`; no bulk path avoids it) | **6.15 s** | **IC** (~4.4×) | C |
| &nbsp;&nbsp;Fan-out graph build (1,000×1,000) | 3.41 s | **0.085 s** | **IC** (~40×) | C |
| **Cascade recompute** — 1M serial `=PREV+1`, p50 | 1.87 s (p99 3.24 s) | 2.11 s (p99 2.15 s) | — (**both FAIL** <100 ms by ~19–21×) | C |
| &nbsp;&nbsp;Wide fan-out recompute (1,000×1,000), p50 | 3.51 s | **77.5 ms** | **IC** (~45×) | C |
| &nbsp;&nbsp;Cascade→visible (100k chain) D2 p50 | 138 ms | **107 ms** | IC (both FAIL 16.6 ms frame) | C |
| **Viewport read** (~1,800-cell window), best design p99 | **222 µs** (D2, native `read_range`) | 392 µs (D2, per-cell loop) | — (**both PASS** <2 ms; FZ mild edge) | C |
| **Single write** (mean, incl. recompute) | 149 µs | **32 µs** | IC | C |
| **Batch write** (1,000 cells, total) | 191 ms | **534 µs** | **IC** (batches 60.5×; FZ interactive path ~0.84×, no win) | C |
| **`.xlsx` read/write** | Yes (calamine read / umya `to_xlsx_bytes` write) | Yes (native styled reader + writer) | — (both viable) | B |
| **Cached formula results in file** | **Dropped** on save; recompute-on-open **mandatory** to show values | **Persisted**; values display on reload before any `evaluate()` | **IC** | B |
| **CSV** | First-class (`CsvAdapter` read; export via `get_value`) | **None** (DIY ~40-line RFC-4180 bridge required) | **FZ** | B |
| **`.ods` read** | Yes (calamine; not exercised) | No | FZ | B |
| **Formatting on read** | **`style: None`** everywhere (0.7 hard-codes it, both backends) | **Native** `get_style_for_cell` (bold/italic/size/fill/border/num_fmt/align) | **IC** | A, B, D |
| **Formatting write / round-trip** | Only via a **directly-owned umya workbook** (engine's own umya adapter is unreachable) | Native, survives a real `.xlsx` round-trip | **IC** | B, D |
| &nbsp;&nbsp;Merges / conditional formatting | Via umya (merges probed; CF inferred, write fidelity unverified) | **No public API** for either | mixed (both need a side-store) | D |
| **Function coverage** (registered builtins, source count) | **410** distinct (name() literals in `builtins/`) | **345** distinct (`Function` enum variants) | FZ on raw count; **neither has a public per-function parity matrix** | this doc |
| **Incremental recalc** | Yes (dirty-set `evaluate_all`) | **No** — full-workbook `evaluate()` only | **FZ** | C |
| **Parallel eval** | Yes (`EvalConfig.enable_parallel`, layered scheduler) | No (single-threaded) | FZ | A, C |
| **Change feed for binding cache** | Append-only `ChangeLog` (poll); edit-sites only, no downstream-dirty | `UserModel` diff-list (poll); edit-sites only | — (both lack downstream-dirty; D3 can't beat D2 on cascade) | A, C |
| **License** | MIT OR Apache-2.0 | MIT OR Apache-2.0 | — (both permissive, forkable) | A |
| **Maturity / adoption** | 0.x, first publish 2026-01-30, 21 releases in ~4.5 mo, ~146 stars, **~944 downloads** | 0.x, since 2023, **~3,978 stars**, 29 contributors, **NLnet-funded**, ~23.5k downloads | **IC** | A |
| **Bus-factor** | Effectively **single-author** (~775 of ~805 commits by owner `PSU3D0`) | Funded team, 29 contributors | **IC** | A |
| **Multi-target** | Rust core → Python (PyO3) + WASM | Rust core (+ WASM/JS bindings, Python) | — (both multi-target; FZ's is first-class in-repo) | A |
| **API suitability for binding** | Richer: native columnar `read_range`, Arrow bulk-ingest, incremental+parallel eval, changelog — **but** two ingest APIs and the interactive `write_range` is a super-linear trap | Thinner: no range read (per-cell loop), no incremental recalc, no parallelism — but simple primitives + **styles on read free** | — (each wins different needs) | C |

**Two verdicts worth pulling out of the table:**

- The **1M serial cascade fails on both engines** (~1.9 s FZ / ~2.1 s IC vs the
  <100 ms target). A million-deep serial `=PREV+1` chain is inherently linear *and*
  unparallelisable — every engine must touch all 10⁶ cells in order. This gap is
  **engine-independent** and does **not** discriminate between the candidates; it
  makes an **async / off-thread recompute UX** mandatory regardless of choice.
- Viewport reads (<2 ms) **pass comfortably on both** (p99 ≤ 585 µs across all
  binding designs). Not a differentiator, and notably IronCalc clears it **without**
  a native range-read API.

---

## 2. The case FOR Formualizer

**It wins the axis the whole project was scoped around.** With each engine loaded
through its *fastest native path* (the fairness correction in Sub-project C), the
Arrow-columnar bet is **demonstrated, not just asserted**:

- **Huge-sheet load & memory:** 10M literals in **1.73 s at 0.18 GB** (~17 B/cell,
  close to the ~9 B/cell f64 floor) vs IronCalc's **6.13 s / 1.63 GB** (~162 B/cell).
  That is **~3.5× faster load and ~9× denser memory** — the decisive numbers at
  Excel-max scale, and exactly the "stupid-fast on huge sheets" thesis the product
  exists to deliver. Ingest is validated **O(cells)** (1M→0.23 s, 10M→1.85 s,
  20M→3.42 s), scaling linearly to 20M cells / 0.32 GB with headroom in the 15 GB box.
- **Native columnar range read** (`read_range`) is the fastest viewport read measured
  (222 µs p99), and it is the primitive a scrolling grid pulls on every frame.
- **Richer binding substrate:** incremental (dirty-set) recompute, first-class
  parallel eval, and an append-only `ChangeLog` for cache invalidation — capabilities
  IronCalc simply does not have.
- **Permissive license** (MIT OR Apache-2.0, forkable) and **first-class multi-target**
  (Rust → Python + WASM in-repo) — good for a commercial product and future surfaces.
- **Function count is competitive-to-higher:** 410 registered builtins by source
  count (see below), including dynamic arrays (FILTER/UNIQUE/SORT with spill),
  LET/LAMBDA, XLOOKUP/XMATCH.

**Its risks (all real, several material):**

- **Maturity / bus-factor:** 0.x, ~5 months old, **effectively single-author** (~775
  of ~805 commits by the owner), **~944 total downloads**. No semver stability; the
  Arrow storage contract is still a "Draft" and may shift across 0.x. Continuity is
  unguaranteed (mitigation: the permissive license makes it forkable; pin the version).
- **No styles on the read path** (0.7 hard-codes `style: None` on both backends), so
  FreeCell must run a **parallel umya workbook** as the style source of truth and keep
  row/col identity in sync across edits — an extra moving part and a memory/sync cost
  that is **not yet measured** (flagged by Sub-project D for Round 2).
- **Slow formula-graph build:** ~20–27 s for a 1M-node graph, and **no bulk path
  avoids it** (it is inherent graph construction, not the load artifact). Real
  open-time cost on formula-dense workbooks; ~4.4× slower than IronCalc here.
- **The `write_range` trap:** there are *two* ingest APIs, and the interactive one is
  super-linear (per-cell vertex + whole-chunk recompaction). Large literal loads
  **must** use `begin_bulk_ingest_arrow`; getting this wrong reproduces the earlier
  (wrong-API) "Formualizer can't scale" artifact. A hard, easy-to-miss binding rule.
- **No public bulk *formula*-ingest fast path** (unlike literals) and **no
  downstream-dirty subscription** — so a cache can't beat a plain bulk re-read on a
  cascade.
- **Drops cached formula results on save** → recompute-on-open is mandatory just to
  show values.
- **Function parity is unverified:** the 410 count is a raw registry count, not a
  correctness/parity guarantee vs Excel; there is no public per-function parity matrix.

---

## 3. The case FOR IronCalc

**It is the more finished, lower-risk engine, and it wins several concrete axes.**

- **Maturity & adoption:** ~3,978 GitHub stars, 29 contributors, **NLnet-funded**,
  shipping since 2023, ~23.5k downloads. A real team, not one person — the strongest
  answer to the single biggest non-technical risk in the whole survey.
- **Native, styled file I/O:** its own xlsx reader/writer **preserves styles and
  number formats** (bold, fills, borders, alignment, date formats all survive a real
  `.xlsx` round-trip — probe-backed in Sub-project D) and it **persists cached formula
  results**, so a reloaded workbook shows values **immediately, before any recompute**.
  Formatting is `get_style_for_cell` / `set_cell_style` on the same `Model` — load →
  edit → save is **easy**, entirely on the engine's side of the seam.
- **Faster formula-graph build:** ~6.15 s for the 1M-node chain vs ~27 s
  (~4.4×), and **~40× faster** fan-out build. Matters for open-time on formula-heavy
  sheets.
- **Dramatically faster non-serial recompute:** the 1,000×1,000 fan-out recomputes in
  **77.5 ms vs Formualizer's 3.51 s** (~45×). For realistic wide `=SUM(range)` shapes
  (far more common than million-deep serial chains), this is a large real advantage.
- **Cheaper single/batch writes** (32 µs single; 60.5× batching win) and a **simpler
  mental model** (a HashMap of cells; primitives that are easy to reason about).
- **Permissive license** (MIT OR Apache-2.0), same as Formualizer.

**Its risks:**

- **Non-columnar storage is the weaker huge-sheet bet** — ~9× less memory-dense
  (~162 B/cell) and ~3.5× slower to load 10M cells. It **did still reach 10⁷ cells**
  (6.13 s / 1.63 GB), so it is not disqualified, but at Excel-max densities this is
  exactly the axis FreeCell was scoped to win, and IronCalc structurally cedes it.
- **No incremental recalc:** every edit triggers a **full-workbook `evaluate()`**.
  Cheap on small sheets, but O(all cells) on Excel-max sheets — a real per-edit and
  open-time cost that the binding must hide with batching + off-thread eval.
- **No native range read** (per-cell loop) and **no parallel eval** — thinner binding
  surface; the viewport target is still met, but there is less headroom.
- **No CSV** (FreeCell owns a ~40-line RFC-4180 bridge) and **no merges /
  conditional-formatting API** at all (both are `None` in 0.7 — need a FreeCell
  side-store or upstream work).
- **Load-API friction:** two-step load with four locale/tz/language args to thread
  consistently, and the path-based `save_to_xlsx` **refuses to overwrite** (must use
  the writer form).
- **Also pre-1.0** (0.7.1) — its `Style` API can churn too.
- **Function count is lower** (345 vs 410 by source count), though it targets ~90%
  Excel parity and, like Formualizer, has **no public per-function parity matrix**.

---

## Function coverage / missing features (source-counted, this doc)

The project materials leave this axis fuzzy (Formualizer's own docs say "320+" in
some places and "400+" in others; IronCalc advertises "~300+ targeting ~90% Excel
parity"), and **neither project publishes a per-function parity matrix**. To put a
defensible number on it, I counted the registered function set directly in each
crate's committed source under `~/.cargo/registry` (read-only):

- **Formualizer 0.7.0 → 410 distinct builtins.** Counted from the `name() -> &'static
  str` string literals across `formualizer-eval-0.7.0/src/builtins/` (each registered
  `Function` impl returns its canonical Excel name). This **corroborates the "400+"
  claim** and puts the earlier "~320 floor" concern to rest for the *registered* set.
- **IronCalc 0.7.1 → 345 distinct functions.** Counted from the `Function` enum
  variants in `ironcalc_base-0.7.1/src/functions/mod.rs`, **cross-checked** against
  the name-mapping block and the eval-dispatch `match` (both independently yield 345
  arms). Consistent with its "~300+" claim.

**Caveats (do not over-read these):** both are **counts of registered functions, not
audits of Excel-correctness or parity coverage.** A registered function can still
differ from Excel on edge cases, error semantics, locale, or array behavior. Both
sit below Excel's ~500. **A per-function parity audit is an open follow-up** (Round
2) and is the kind of finding that could move this recommendation (below). On the raw
count Formualizer is ahead (410 vs 345); on *verified* parity, **it is currently
unquantified for both**.

---

## 4. Recommendation *(input for the human's decision — not the decision)*

> This section is a **reasoned lean for the human to ratify or overrule**, per the
> §6.G brief ("→ HUMAN SIGN-OFF on the engine choice"). It is genuinely close.

### What's decision-neutral (don't let it sway the call)

- **Formatting model is engine-neutral either way.** Sub-project D's recommendation is
  a FreeCell-owned **`FormatStore` side-table** *regardless* of engine — mandatory on
  Formualizer (surfaces nothing), still advisable on IronCalc (isolates FreeCell from
  0.x `Style` churn and keeps the model swappable). IronCalc's native styles are a
  *convenience that reduces the load/save adapter work*, not an architectural
  requirement. So "IronCalc has styles" is a real but **bounded** advantage, not a
  decider.
- **Both engines need async / off-thread recompute.** The 1M serial cascade is ~2 s on
  *both*; the product needs a "recalculating" UX either way. Not a differentiator.
- **UI is settled (GPUI).** Nothing here touches the UI decision.
- **License is a wash** (both MIT OR Apache-2.0, forkable).
- **Viewport reads pass on both.** Not a differentiator.

### The lean

**Lean: Formualizer — as a "go-with-conditions", not a firm commitment.** The reasoning:

1. **It wins the one axis FreeCell was built to win.** "Stupid-fast on huge sheets" is
   the product thesis, and the corrected numbers show the Arrow columnar store is
   **~3.5× faster to load and ~9× denser** at 10M cells — a *structural* advantage
   from the storage model, not a tuning detail. IronCalc reached 10⁷ cells but cedes
   this axis by design (nested HashMap, ~162 B/cell). If the north star means what it
   says, the engine that structurally wins it should be the default.
2. **IronCalc's real edges are mostly recoverable or bounded.** Native styles →
   replaced by the engine-neutral `FormatStore` we're building anyway. Faster
   graph-build and fan-out recompute are real, but (a) both engines already fail the
   headline cascade target, so recompute needs an async UX regardless, and (b) the
   fan-out gap, while large, is on a *discovery* metric, not a §5.4 gate. Cached-result
   persistence and CSV/merges are convenience/coverage items, not thesis-defining.
3. **The binding substrate is richer on Formualizer** (native range read, incremental
   + parallel eval, changelog) — more headroom to hit the frame budget as the app
   grows, versus IronCalc's full-workbook re-eval on every edit.

**But the lean is deliberately soft, because the risks are concentrated and correlated
on Formualizer:** single-author bus-factor, 0.x storage churn, the slow formula-graph
build, the `write_range` trap, and no styles on read all land on the *same* engine.
IronCalc's counter-case — a funded team with ~4k stars, native styled I/O, and a
simpler model — is a legitimate "de-risk the project by taking the more finished
engine, and treat the huge-sheet axis as good-enough (it did reach 10⁷)." A reasonable
human optimizing for **delivery risk over peak huge-sheet performance** could
rationally sign off on IronCalc instead, and this document should not pretend that is
wrong.

**Net recommendation to the human:** **adopt Formualizer, conditionally**, because it
delivers the project's defining advantage and its costs are largely mitigable with the
architecture Phase 1 already specified (engine-neutral `FormatStore`, async recompute,
mandatory bulk-ingest path). Treat the conditions below as the things that would flip
the call to IronCalc. If the project's priority is *maturity and delivery risk* over
*peak huge-sheet performance*, **IronCalc is the defensible alternative** and this is
a close enough call to choose it on those grounds.

### What would change the recommendation (→ IronCalc, or firm up Formualizer)

- **A function-parity audit that favors IronCalc.** The 410-vs-345 raw count leans
  Formualizer, but neither is *verified*. If a Round-2 parity audit shows Formualizer's
  functions are materially less Excel-correct (errors/edge cases/locale), that erodes
  its main non-perf edge and strengthens IronCalc.
- **Formualizer's formula-graph build stays ~20–27 s with no path down.** If real
  target workbooks are formula-dense, that open-time cost is a daily papercut IronCalc
  (~6 s) avoids. A found config or upstream fix that parallelises graph build would
  *firm up* the Formualizer lean; its continued absence pushes toward IronCalc.
- **The umya double-load (Formualizer's style workaround) proves expensive.** D flagged
  the memory/sync cost of holding the sheet twice (Arrow engine + umya for styles) as
  unmeasured. If Round 2 shows it meaningfully erodes the ~9× memory-density win or
  complicates edits (row insert/delete must shift both), IronCalc's native styles
  become materially more attractive.
- **Formualizer's bus-factor materialises as risk** (stalled releases, a breaking
  storage-contract change on the pinned version) — the single-author dependency is the
  most likely reason a human overrules toward the funded team.
- **Conversely, what would firm up Formualizer:** a published bulk *formula*-ingest
  path (removing the graph-build cost), styles surfaced on the read path (removing the
  umya double-load), or a downstream-dirty subscription (enabling a true incremental
  cascade) — any of these would make the lean decisive rather than conditional.

---

## Appendix — reproducing the function counts (read-only)

```sh
# Formualizer 0.7.0 — distinct registered builtin names (410)
FZ=~/.cargo/registry/src/index.crates.io-*/formualizer-eval-0.7.0
grep -rhA2 "fn name(&self) -> &'static str" "$FZ/src/builtins" \
  | grep -oE '"[A-Z][A-Z0-9_.]*"' | tr -d '"' | sort -u | wc -l   # -> 410

# IronCalc 0.7.1 — distinct Function enum variants (345), cross-checked
IC=~/.cargo/registry/src/index.crates.io-*/ironcalc_base-0.7.1
awk '/^pub enum Function \{/{f=1;next} f&&/^\}/{exit} f{print}' "$IC/src/functions/mod.rs" \
  | grep -vE '^\s*//|^\s*$' | grep -cE '^\s*[A-Za-z][A-Za-z0-9]*,'   # -> 345
grep -oE 'Function::[A-Za-z0-9]+ =>' "$IC/src/functions/mod.rs" | sort -u | wc -l  # -> 345 (dispatch)
```

Both are counts of the **registered** function set on the probed versions, not
parity audits. See *Function coverage* above for caveats.
