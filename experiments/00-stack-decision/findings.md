# Sub-project A — Stack Decision (GATE)

> Status: **complete** — this is the gating sub-project (functional_spec §6.A,
> architecture §4). It removes the biggest up-front unknown: *is
> **Formualizer + GPUI** (or a better-ranked alternative) a stack we can
> confidently build FreeCell on?* The output is a **ranked recommendation** for a
> human to sign off (go / pivot). Structure follows functional_spec §5.2.
>
> Environment for all in-container work: Rust 1.94.1, 4 cores / ~15 GB RAM, no GPU,
> no display. Date: 2026-07-01. Formualizer version probed: **0.7.0** (sub-crates
> `formualizer-eval 0.7.0`, `formualizer-workbook 0.7.0`, `formualizer-parse 2.0.0`,
> `formualizer-common 2.0.0`, `formualizer-sheetport 0.7.0`).

## Questions

1. Is **Formualizer** a real, usable spreadsheet engine — does it build in our
   environment, load a file, evaluate a formula, mutate a cell? What is its **actual
   API surface** for the operations FreeCell depends on (single-cell read/eval,
   range/bulk reads, parallel evaluation, update-subscription / dirty-tracking,
   styles/formatting exposure, and how Apache Arrow is surfaced)? This captured
   surface is the input that unblocks the later phase plans (B–E).
2. Is Formualizer a good **engine** base for FreeCell (maturity, function coverage
   vs Excel's ~500, file fidelity, license, maintenance / bus-factor, performance
   ceiling, Arrow model)? What credible **engine** alternatives exist?
3. Is **GPUI** a sane **UI** base (viable as a standalone dependency given its
   coupling to Zed; `gpui-component` as the practical component layer)? What
   credible **UI** alternatives exist?
4. Synthesizing all of the above: what are the 2–4 best **full stacks**, ranked,
   with reasoning and explicit risks?

## What was done

### Hands-on Formualizer smoke test (in-container, code committed)

A minimal, **correctness-focused** Cargo crate at
[`00-stack-decision/smoke/`](smoke/) that both exercises and **documents** the real
0.7.0 API. It is not a benchmark (that is Sub-project C). Reproduce with:

```sh
cd experiments/00-stack-decision/smoke
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo build
cargo test        # 7 probes, all passing
```

Dependencies: `formualizer = "0.7"` with
`features = ["eval","parse","workbook","calamine","csv","umya","json","system-clock"]`,
`anyhow`, and (dev) the shared `datagen` crate (via `../../shared/datagen`) to
generate a tiny CSV input from committed code — no hand-made binary fixtures
(functional_spec §5.3). Formualizer fetched from crates.io and compiled cleanly
in-container (Arrow + umya + calamine included). Code pointers:

- [`smoke/src/lib.rs`](smoke/src/lib.rs) — thin documented helpers; its module docs
  are the captured API-surface reference.
- [`smoke/tests/smoke.rs`](smoke/tests/smoke.rs) — the seven probes.

### Web research (two parallel helper agents)

Helper sub-agents researched, from **primary sources** (crates.io API, GitHub raw
source + repo trees, project READMEs, official framework docs): (a) the
spreadsheet-engine landscape (Formualizer + alternatives), and (b) the GPU/native-UI
landscape (GPUI, `gpui-component`, alternatives). Findings synthesized below.

## Results / evidence

### A. Captured Formualizer 0.7.0 API surface (the key deliverable)

All verified by the passing smoke probes and by reading the crate source in
`~/.cargo/registry`. **Row/column indices are 1-based everywhere** in this API
(`RangeAddress::new` rejects 0). Core type is `formualizer::Workbook` (re-exported
from `formualizer-workbook`).

**Build & mutate**
- `Workbook::new()` (implicit `Sheet1`); `Workbook::new_with_config(WorkbookConfig)`.
- `add_sheet(&str)`, `has_sheet(&str)`, `sheet_names() -> Vec<String>`,
  `sheet_dimensions(&str) -> Option<(u32,u32)>`.
- `set_value(sheet, row, col, LiteralValue) -> Result<(), IoError>`.
- `set_formula(sheet, row, col, &str) -> Result<(), IoError>`.
- Bulk writes: `set_values(..)`, `set_formulas(..)`,
  `write_range(sheet, start, BTreeMap<(u32,u32), CellData>)` (uses a deferred-dirty
  scope: one propagation for the whole batch, not a BFS per cell).

**Single-cell read / eval**
- `get_value(sheet,row,col) -> Option<LiteralValue>` (stored/cached value).
- `get_formula(sheet,row,col) -> Option<String>` (canonical formula text).
- `evaluate_cell(sheet,row,col) -> Result<LiteralValue, IoError>` — pulls precedents;
  **recomputes after an edit to a precedent** (probe
  `builds_and_evaluates_dependent_cell`: `A3==A1+A2` = 3, then `A1:=10` → `A3` = 12).

**Range / bulk / batch read**
- `read_range(&RangeAddress) -> Vec<Vec<LiteralValue>>` — 2D range read backed by a
  columnar **range view** (`sheet.range_view(...)` over the Arrow store).
- `evaluate_cells(&[(&str,u32,u32)]) -> Result<Vec<LiteralValue>>` — batch eval,
  order preserved; `evaluate_cells_cancellable(targets, Arc<AtomicBool>)` for a
  cancel flag.
- `evaluate_all() -> Result<EvalResult>`; `build_recalc_plan()` + `evaluate_with_plan()`;
  `get_eval_plan(targets) -> EvalPlan` for inspecting/reusing a recompute plan.
- (Probe `range_bulk_read_returns_grid`: `read_range(A1:A3)` → `[[1],[2],[3]]`;
  `evaluate_cells` returns the batch in order.)

**`LiteralValue`** (from `formualizer-common`): `Int(i64)`, `Number(f64)`, `Text`,
`Boolean`, `Array(Vec<Vec<..>>)`, `Date`, `DateTime`, `Time`, `Duration`, `Empty`,
`Pending`, `Error(ExcelError)`.

**Parallel evaluation** — first-class and reachable:
`WorkbookConfig.eval` is a public `EvalConfig { enable_parallel: bool,
max_threads: Option<usize>, .. }`. The scheduler evaluates independent graph
vertices in layers; per the engine's own design notes, parallelism is applied in
the compute phase with a single-threaded apply phase for determinism. (Probe
`parallel_eval_config_is_exposed`: a workbook built with `enable_parallel=true,
max_threads=Some(4)` evaluates correctly.)

**Update subscription / dirty tracking / change notification** — via an append-only
`ChangeLog`:
- `Workbook::set_changelog_enabled(true)`; `Workbook::changelog() -> &ChangeLog`.
- `ChangeLog`: `events() -> &[ChangeEvent]`, `len()`, `take_from(index) -> Vec<ChangeEvent>`,
  compound grouping (`begin_compound`/`end_compound`, `last_group_indices()`).
- `ChangeEvent` variants carry old+new state: `SetValue { addr, old_value,
  old_formula, new }`, `SetFormula { .. }`, plus `SpillCommitted/Cleared`,
  `EdgeAdded/Removed`, named-range events, etc.
- `undo()` / `redo()` and `action(desc, ..)` transactions build on the same log.
- This is the substrate a **binding cache** (Sub-project C's D3 design) can poll to
  invalidate exactly the visible cells after an edit. (Probe `changelog_tracks_edits`:
  edits append a `SetValue { new: Int(42), .. }`.) **Note:** it is a *poll* model
  (read the event list), not a push/callback subscription; there is a `CalcObserver`
  trait in the engine for hooking calc phases, but the changelog is the documented
  edit-notification surface.

**Apache Arrow** — real and central:
- `formualizer-eval` depends on the official Apache `arrow`, `arrow-array`,
  `arrow-buffer`, `arrow-cast`, `arrow-schema`, `arrow-select` crates.
- Cell "truth" is an **Arrow-backed columnar store**: typed per-column lanes
  (`Float64Array` numbers, `BooleanArray`, Utf8 text, `UInt8Array` error codes, a
  per-row type tag), chunked by rows. There is a bulk columnar ingest builder and a
  `range_view` columnar read path (what `read_range` uses).
- The engine journal records `ArrowOp` / `ArrowUndoBatch`; edits land in a **delta
  overlay** and formula outputs in a **computed overlay**, periodically compacted
  into base lanes (read precedence delta → computed → base). The dependency graph
  stays canonical for formulas/edges/scheduling; values live in Arrow.
- Access-model descriptors: `AccessGranularity { Cell, Range, Sheet, Workbook }`
  and `LoadStrategy { EagerAll, EagerSheet, LazyRange { row_chunk, col_chunk },
  LazyCell, WriteOnly }`.
- **Caveat:** raw Arrow `RecordBatch`es are **not** exposed as a public
  read-your-own-columns API in 0.7.0; columnar access goes through `read_range` /
  the range view. Zero-copy column pulls for the UI would need either those APIs to
  suffice or an upstream addition — flagged for Sub-project C.

**File I/O**
- Read `.xlsx`: `CalamineAdapter::open_bytes(Vec<u8>)` / `open_path(P)`, then
  `Workbook::from_reader(adapter, LoadStrategy, WorkbookConfig)`.
- Read/write CSV: `CsvAdapter` (options for delimiter/headers/etc.).
- Write `.xlsx`: `Workbook::to_xlsx_bytes()` (uses the umya backend under the hood).
- `.xlsx` **round-trip fidelity observed** (probe `xlsx_roundtrip_via_umya_and_calamine`,
  build → `to_xlsx_bytes` → reload via calamine): **literal cells survive as values**;
  a **formula cell survives as its formula text, NOT its cached value** (calamine
  reads `value=None, formula=Some("=A1 + A2")`); after reload + `prepare_graph_all()`
  the formula **re-evaluates correctly** to the original result. So values+formulas
  survive; cached results do not through this path. (Detailed fidelity — styles,
  merges, number formats, multi-sheet, large files — is Sub-project B's job.)

**Styles / formatting (KEY GAP — primary input to Sub-project D)**
- `traits::CellData` carries only `style: Option<StyleId>` where `StyleId = u32`
  (opaque). `BackendCaps.styles` is `true` for the umya backend and **`false`** for
  calamine.
- **However, both backends' read paths hard-code `style: None` in 0.7.0** — the
  calamine reader sets `styles: false` and umya's `read_sheet`/`read_range`/`read_cell`
  always emit `CellData { style: None, .. }`. So bold/italic/fills/number-formats
  are **not surfaced through the standard `CellData` read path**. (Probe
  `styles_not_surfaced_through_celldata` regression-locks this: calamine
  `capabilities().styles == false`, and a read-back `CellData.style` is `None`.)
- Consequence for FreeCell: formatting must be read from the underlying
  `umya_spreadsheet` workbook directly (umya *does* preserve styles for round-trip),
  or held in a **FreeCell-side formatting store**. Sub-project D designs this.

### B. Engine landscape (research, primary sources)

**Formualizer maturity & risk.** Author Frankie Colson (`PSU3D0`), **sole crates.io
owner**; repo `github.com/psu3d0/formualizer` (~146 stars, 8 contributors but **~775
of ~805 commits by the author** — effectively single-author). First publish
2026-01-30 (v0.3.0), **21 releases in ~4.5 months**, current **0.7.0** (2026-06-12),
all **0.x** (no semver stability). **~944 total downloads.** License **MIT OR
Apache-2.0** (ideal for a commercial app; forkable). Multi-target: Rust core →
Python (PyO3) + WASM.

**Function coverage.** Project materials are **internally inconsistent**: the
`-eval` README and GitHub banner say **"320+"**, while the root/workbook READMEs and
docs site say **"400+"**. Treat **~320 as the defensible floor**, 400+ as
unverified/aspirational. Either way it is below Excel's ~500, and there is **no
public per-function parity matrix**. Has dynamic arrays (FILTER/UNIQUE/SORT with
spill), LET/LAMBDA, XLOOKUP, etc.

**File fidelity.** Pluggable I/O behind features: calamine (xlsx/ods **read**), umya
(xlsx **read+write, round-trip**), csv, json. Matches what the smoke test confirmed.
No published enumeration of what styles/charts/pivots/conditional-formatting survive
— bounded by umya (styles) and, on read, the styles gap above.

**Performance ceiling.** **No published benchmark numbers** — the root README says
"Formal benchmarks are in progress." The *architecture* (Arrow columnar lanes, CSR
dependency graph, optional Rayon parallel compute phase) is exactly what a
"stupid-fast on huge sheets" thesis wants, and there is a serious internal benchmark
harness (86+ scenarios; head-to-head adapters for IronCalc/HyperFormula; a
700k-row / 256M-logical-cell / 60s envelope probe) — but **its results are private**.
So the perf ceiling is **credible-by-design but unverified**; Sub-project C must
measure it (1M-cell `=PREV+1` cascade < 100 ms, viewport reads < ~2 ms).

**Alternatives.**
- **IronCalc** (`ironcalc`/`ironcalc-base`, Rust, MIT/Apache) — the credible
  more-mature alternative: **~3,978 stars, 29 contributors, NLnet-funded**, since
  2023, **~23.5k downloads**, ~300+ functions and climbing (goal 90% Excel parity),
  real xlsx read+write with styles/themes/merges/conditional formatting. **But its
  storage is `HashMap<i32, HashMap<i32, Cell>>` (nested row→col hashmaps of boxed
  cells) — NOT columnar/Arrow.** For Excel-max sheets that is a materially weaker
  memory-density / cache-locality / bulk-scan story than Formualizer's Arrow lanes.
  Lower bus-factor risk; weaker perf-architecture fit. **This is the primary pivot
  target.**
- **calamine (read) + rust_xlsxwriter/umya-spreadsheet (write) + build-our-own
  engine** — best-in-class, very mature file I/O for free (calamine 9.4M dls;
  rust_xlsxwriter 2.66M; umya 765k), but **zero calc engine**: you own the
  parser + dependency graph + incremental recalc + ~300–500 functions. That is the
  same multi-year scope Formualizer/IronCalc each represent. The credible **fallback
  engine strategy**, not a quick win.
- **Pure formula-eval crates** (e.g. `xlformula_engine`) — toy-tier (small function
  set, f32-only, effectively abandoned). Not candidates.
- **Non-Rust** (HyperFormula JS — **GPLv3/commercial**, needs a JS runtime;
  LibreOffice Calc — huge embedding cost) — rejected on license and/or integration
  cost for a native Rust GPU app.

### C. UI landscape (research, primary sources)

**GPUI.** GPU-accelerated Rust UI framework by the Zed team; a hybrid
immediate/retained model with an entity → view (`Render`) → element layering driven
through contexts (actions, an async executor, a `#[gpui::test]` macro). Rendering
backends: **native Metal on macOS** (primary, most mature); **Linux migrated from
Blade to `wgpu`/Vulkan in early 2026** (Zed PR #46758; shaders ported to WGSL so one
source targets Metal/Vulkan/DX12); **Windows DX12**. Zed reached **1.0 on
2026-04-29** with Linux/Windows at parity. GPUI powers Zed's "renders like a
videogame" 120-fps pipeline over 200k-line files — the strongest evidence in the
whole survey that a Rust GPU UI can sustain the frame budget at scale. Ships a
virtualized `uniform_list` primitive (uniform-height row virtualization — exactly a
spreadsheet's default row model) plus low-level quad/shadow/glyph/path batching and
first-class scroll handles + bounds/hit-testing.

- **Coupling to Zed is the central risk (nuanced update).** GPUI **is now published
  on crates.io** (`gpui` v0.2.2, 2025-10-22, Apache-2.0, ~136k downloads) — the old
  "git-only name-squat" story is out of date. **But** the crate lags Zed's fast
  `main` by ~8 months, is explicitly **pre-1.0 with frequent breaking changes**, and
  the *real* ecosystem (including gpui-component) still **git-pins a specific Zed
  commit** of `zed-industries/zed`. The Zed team has publicly **declined to commit to
  maintaining GPUI as a standalone library** (discussions #10431 / #30515), and a
  community fork `gpui-ce` exists precisely for that reason. Practical posture: pin a
  known-good Zed rev and budget for periodic rev-bump upgrades.
- **License caveat to flag for legal.** The `gpui` crate is **Apache-2.0** (Zed the
  *app* is GPL; the framework crate is permissive), good for a commercial product —
  **however** open issue **#55470** notes a default release build statically links
  **GPL-3.0** object code via `gpui → sum_tree → ztracing`. It is a **runtime no-op**
  (the code path is never enabled outside Zed) and the fix (`ztracing → tracing`) is
  trivial, but the dependency is present. **Get legal sign-off and verify the fix is
  merged on the pinned rev before shipping a proprietary binary.**
- **Capability fit is strong**; the open question is 2D (row **and** column)
  virtualization with variable sizes at 16k-column width, which the PoC (Sub-project
  E) must build/measure on real Metal.

**gpui-component** (`github.com/longbridge/gpui-component`, by the Longbridge
fintech; Apache-2.0; ~11.9k stars, ~104 contributors, on crates.io **and** git;
first-party production user is Longbridge's own trading terminal). The practical
component layer on top of gpui (60+ components). Its **Table does true 2D
virtualization** — rows via `uniform_list`, columns via a custom `virtual_list` with
leaf-column culling (only the visible column range is built) — plus column
resize/sort/move, left-column freeze, and Row/Column/Cell selection with hit events.
**Concrete spreadsheet gaps found in source (important for Sub-project E/D):**
(1) **uniform row heights only** (no variable row heights); (2) **freeze = left
columns only** (no frozen rows / frozen-right beyond the sticky header); (3) **no
built-in inline cell editing** (you wire your own editor overlay on
`DoubleClickedCell`/`SelectCell`). Its demo shows visible-cell render at ~2.6% of an
~8.8 ms frame, but there is **no benchmark at 1M×16k**. It **git-pins gpui to a Zed
commit** (so it, too, tracks Zed). Whether to adopt-and-extend it or hand-roll a raw
gpui grid is the raw-vs-`gpui-component` decision in Sub-project E.

**Alternatives (ranked by fit as a fallback).**
- **egui + `egui_table`** (immediate-mode; wgpu → Metal default; egui MIT/Apache,
  ~19M downloads, ~6 yrs, the most production-proven Rust GUI — flagship
  large-tabular user is Rerun). `egui_table` (Rerun, MIT/Apache) does **true 2D
  virtualization** (rows over a `u64` range; columns via prefix-sum binary search),
  advertises "**millions of rows**," sticky rows+cols, resizable + heterogeneous
  row heights — the **best high-level fallback**. Caveat: immediate-mode redraws the
  full frame on every scroll, and there's **no public proof of 120 fps on a
  text-dense grid**; "batteries not included" (you draw cells/selection/editing).
- **Raw wgpu custom renderer** (native Metal; very mature — underpins Firefox/Bevy/
  Iced/Vello; MIT/Apache) — build winit (event loop) + glyphon/cosmic-text (text) +
  your own layout/hit-testing/virtualization. For a **uniform** grid, virtualization
  and hit-testing are just arithmetic and per-frame cost scales with the *viewport*,
  not the data — arguably the **lowest architectural risk at extreme scale**, at the
  cost of building text/selection/editing yourself. The **strongest low-level
  fallback**.
- **Iced** (wgpu → Metal; MIT; pre-1.0 but real production in System76 COSMIC) — no
  first-class million-row table; you'd build a custom grid widget.
- **Xilem / Masonry / Vello** (Linebender, wgpu/GPU-compute vector; Apache/MIT) —
  **experimental / pre-1.0**; Vello is alpha with glyph caching unfinished (bad for
  text-heavy grids), Masonry's virtual-scroll is a young 1-D MVP with a
  focus-during-scroll bug. **More from-scratch work than GPUI; a multi-year bet.**
- **Slint** — capable Metal rendering, but the **licensing is the gate**: GPL /
  royalty-free-with-attribution / paid-commercial; declarative `.slint` markup is a
  poor fit for a fully custom GPU spreadsheet, and its table has a large-model perf
  history.
- **Freya (Dioxus+Skia)** — ships a `VirtualScrollView` out of the box but is
  **pre-1.0**, small/largely single-maintainer, unproven at scale. **Dioxus/Blitz**
  renders an HTML/CSS box model (wrong abstraction; Blitz alpha). **Flutter + FFI** —
  mature grid UI but Dart UI + a per-cell FFI marshaling seam.

## Conclusion

**Formualizer is real and usable.** It builds in-container with Arrow + file
backends, and the smoke test confirms the operations FreeCell needs: build a
workbook, set a value, evaluate a dependent cell **with correct recalc on mutate**,
bulk/range reads via a columnar range view, batch + parallel evaluation, an
append-only change log for dirty tracking, and `.xlsx`/CSV load. Its **Apache Arrow
columnar core is genuine** and is the single best architectural match to FreeCell's
"stupid-fast on huge sheets" thesis among all engines surveyed.

Two concrete gaps were pinned (and are exactly why the gate exists):
1. **Styles/formatting are not surfaced through the `CellData` read path** in 0.7.0
   (calamine `styles=false`; umya read returns `style: None`). FreeCell will need to
   read formatting from umya directly and/or keep its own formatting store
   (→ Sub-project D). *We could not determine the full style-read story from the
   public read API because it simply isn't wired through in 0.7.0.*
2. **Raw Arrow `RecordBatch`es are not a public read API**; columnar reads go through
   `read_range`. Whether that is fast enough for viewport pulls, or we need an
   upstream zero-copy column API, is for Sub-project C to measure. *We could not
   determine the viewport-read perf ceiling here because this phase is
   correctness-only, not a benchmark.*

The dominant **non-technical** risk is Formualizer's **maturity/bus-factor**: a
0.x, ~5-month-old, effectively single-author crate with ~944 downloads and **no
published performance numbers**. Its permissive MIT/Apache license (forkable) and
strong internal engineering signals (design docs, benchmark governance) partly
offset this, but continuity is unguaranteed.

**GPUI** is a capable, proven-fast UI base (it is the only surveyed option that has
*already shipped* 120-fps GPU UI at scale, in Zed 1.0). Its main risk is **coupling
to Zed** (the real ecosystem git-pins a Zed commit; pre-1.0, sparse docs, no semver)
plus a **GPL-linkage caveat (issue #55470)** that needs legal sign-off — not
capability. The `gpui-component` table already does true 2D virtualization but has
concrete spreadsheet gaps (uniform row heights only, left-only column freeze, no
inline editing); raw-gpui vs `gpui-component` is a Sub-project E question. Strong,
credible fallbacks exist end-to-end (**egui + `egui_table`** high-level; **raw
wgpu** low-level), so a UI pivot is not a dead end.

Net: **the original Formualizer + GPUI direction is sound enough to proceed**, with
eyes open on the risks above and named fallbacks ready.

## Recommended design + next-best alternative

Ranked full stacks (engine × UI):

### 1 (RECOMMENDED). Formualizer + GPUI (with `gpui-component`) — "go"
- **Why:** best architectural fit end-to-end. Formualizer's Arrow columnar core is
  purpose-built for huge-sheet perf; GPUI is a proven 120-fps GPU renderer (Zed) on
  the primary target (macOS/Metal). Both are permissively licensed. This is the
  direction the project was scoped around, and nothing in the smoke test or research
  contradicts it.
- **Conditions / how we de-risk before committing hard:** Sub-project C must show
  Formualizer hits the cascade (< 100 ms for 1M `=PREV+1`) and viewport-read
  (< ~2 ms) targets on our hardware (its perf is unbenchmarked publicly); Sub-project
  E must show GPUI (raw vs `gpui-component`) sustains the frame budget with 2D
  virtualization on real Metal; Sub-projects B/D must confirm the file-fidelity and
  formatting-store plan around the styles gap. Pin GPUI to a specific Zed commit.

### 2 (NEXT-BEST). IronCalc + GPUI — the engine pivot
- **Why:** if Formualizer's perf doesn't materialize, or its bus-factor is judged
  unacceptable, IronCalc is the more-mature Rust engine (real team + funding, more
  adoption, solid xlsx r/w). Keep GPUI. **Cost:** IronCalc's `HashMap` storage is a
  weaker huge-sheet perf/memory story, so the "stupid-fast" bar is at more risk and
  the binding layer must work harder. Same UI, so Sub-project E is unaffected.

### 3. Formualizer + egui (`egui_table`), with raw-wgpu as the escape hatch — the UI pivot
- **Why:** if GPUI's Zed coupling / API churn / GPL-linkage proves too costly, egui
  is the mature, well-documented, broadly-portable fallback, and `egui_table`
  (Rerun) already does true 2D virtualization at "millions of rows". If even that
  can't hit the frame budget on a text-dense grid, a **raw-wgpu** custom grid is the
  lowest-architectural-risk option at extreme scale (viewport-bound cost; build
  text/selection/editing yourself). **Cost:** immediate-mode at a locked 120 fps is a
  harder fit than GPUI; raw-wgpu is the most control but the most from-scratch UI
  work. Engine unchanged in all cases.

### 4. calamine + rust_xlsxwriter/umya + **own engine**, on GPUI — the "own it all"
- **Why:** only if no existing engine meets the bar *and* we choose to make the calc
  engine a core competency. Best-in-class file I/O for free; full control of the data
  model (columnar from day one). **Cost:** by far the most work (parser + graph +
  recalc + hundreds of Excel-compatible functions + fidelity testing). This is the
  long-term fallback, not a starting point.

**Recommendation to the human: sign off on Stack #1 (Formualizer + GPUI) to
proceed**, treating Sub-projects C (engine perf) and E (UI perf) as the hard gates
that would trigger a pivot to #2 or #3 respectively if their measured targets aren't
met (or a credible path to them isn't shown).

## Risks / open questions

- **Formualizer perf is unverified** (no public benchmarks). Biggest technical
  unknown; owned by Sub-project C (cascade, viewport-read, memory).
- **Formualizer bus-factor / 0.x churn.** Single-author, no semver, Arrow-canonical
  storage doc still "Draft" — the storage contract may shift across 0.x. Mitigation:
  permissive license (forkable), pin the version, watch releases.
- **Formualizer function coverage** (~320 confirmed floor vs 500 in Excel; 320-vs-400
  claim inconsistency unresolved). Owned by B/D and future coverage work; count the
  registry directly before relying on a number.
- **Styles not surfaced via `CellData`** in 0.7.0 → FreeCell needs a formatting-read
  path (umya-direct) and/or its own store. Owned by Sub-project D.
- **No public raw-Arrow read API** → viewport reads go through `read_range`; may or
  may not be fast enough / zero-copy enough. Owned by Sub-project C.
- **`.xlsx` cached formula results are dropped** by the umya write path (formulas +
  literals survive; cached values don't). Owned by Sub-project B; matters for
  open-time perf (recompute-on-load).
- **Change notification is poll-based** (read the changelog), not push/callback. Fine
  for a binding cache that ticks per frame, but note it for Sub-project C's design.
- **GPUI ↔ Zed coupling**: on crates.io now (v0.2.2) but the crate lags `main` ~8
  months and the real ecosystem git-pins a Zed commit; pre-1.0, sparse docs, no
  semver. Mitigation: pin a known-good rev; budget periodic rev-bump upgrades; the
  PoC (E) validates capability.
- **GPUI GPL-linkage (issue #55470)**: a default build statically links GPL-3.0
  object code via `gpui → sum_tree → ztracing` (runtime no-op; trivial fix). **Needs
  legal sign-off** and a check that the fix is merged on the pinned rev before
  shipping a proprietary binary.
- **`gpui-component` spreadsheet gaps**: its Table 2D-virtualizes, but rows are
  uniform-height only, freeze is left-columns-only, and there's no inline cell
  editing — so adopting it means fork/extend work vs. a raw-gpui grid. No public
  benchmark at 1M×16k. Owned by Sub-project E.
- **No public 120-fps benchmark at 1,048,576 × 16,384 in any framework** — GPUI
  proves 120 fps in an *editor*, not a 16k-wide grid. The **column-virtualization
  axis** is the single most important thing for the PoC (E) to spike-test.
- **UI perf is only authoritative on real macOS/Metal**, run by the human — not
  measurable in this container.
