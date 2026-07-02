# Sub-project E — UI Technical Test (GPUI Proof-of-Concept, macOS)

> Status: **complete — measured verdict recorded.** The human ran `run_test.sh both` on
> macOS/Metal on **2026-07-02**: **both variants PASS all three §5.4 gates** (see
> *Results / evidence*; transcript at `results/human-run-2026-07-02-runtest.txt`).
> Original status text follows. Sub-project E
> is the only UI sub-project and is engine-neutral — no spreadsheet engine, just a static
> datamodel provider (functional_spec §6.E, architecture §7). The GPUI crates target
> macOS/Metal and **cannot build in the headless Linux container**, so the numbers come
> from the human running it on a Mac. See **"HUMAN RUN REQUIRED"** at the bottom for the
> exact ask.

## Questions

1. Can GPUI render an **Excel-max** spreadsheet grid (1,048,576 rows × 16,384 cols) at
   the functional_spec §5.4 perf bar — **measured**, not vibes?
   - Frame render p99 ≤ 8.33 ms (sustained 120 fps), never worse than 16.67 ms (60 fps)
     under fast scroll / jump.
   - Newly-visible-cell load (values + formatting) p99 < 2 ms.
2. How does a **custom grid on raw `gpui`** compare to **`gpui-component`'s virtualized
   `DataTable`** — perf and ergonomics?
3. Does it *feel* smooth interactively?

## What was done

One macOS/Metal app in **two variants over the same static provider**, plus a shared
engine-neutral core that is CI-checked in-container:

### `poc-core/` — the load-bearing, GPUI-free core (CI-checked on Linux)

Because GPUI can't build here, all the logic that *can* be verified was isolated into a
`poc-core` library crate with **no gpui dependency**. It `cargo check`s, `clippy -D
warnings` cleans, and `cargo test`s green in-container (20 tests). Modules:

- **`layout::Axis`** — variable-size virtualization. Maps scroll offset → visible index
  range and index → pixel offset over variable row heights / column widths via
  **segment-summed prefix sums + binary search** (architecture §7). Critically, it does
  **not** materialize a per-row array: block sums of size 512 keep memory at O(n/512)
  (~2,048 `f64` for the full Excel-max row axis, ~16 KB), so 1M+ rows build and query
  cheaply. Tested incl. an Excel-max-rows no-OOM/roundtrip case.
- **`style::RenderCell`** — GPUI-free conversion of a `datagen::CellData` into
  render-ready text + `0xRRGGBB` fill/text colours + bold/italic/align, so both variants
  draw cells the same way (apples-to-apples look).
- **`harness`** — the scripted "Run Test" viewport sequence: steady scroll, fast scroll,
  horizontal pan across the wide variable-width columns, deterministic jump-to-cell, and
  seeded random jumps. Deterministic (pure function of config + seed), advanced one frame
  at a time; records a `FrameSample { frame_render_ns, cell_load_ns, newly_visible }` per
  frame.
- **`report`** — turns samples into `bench_util::LatencyStats` (p50/p99/max) for frame
  time and cell-load, builds the three §5.4 `GateResult`s (PASS/FAIL), assembles a
  `bench_util::BenchResult` stamped with environment + a passed-in date/commit (no wall
  clock in recording code; architecture §3), prints a human summary, and writes
  `results/<variant>-runtest.json`.

The **static datamodel provider** is the frozen `shared/datagen::SyntheticSheet`
(`trait CellSource { fn cell(row,col) -> CellData }`): deterministic, varied text lengths,
numbers, ~15% highlighted cells, scattered bold/italic, variable row heights and column
widths incl. very-wide columns — a proxy for a big, difficult sheet. Consumed read-only by
relative path; no engine connected.

### `raw-gpui/` — custom virtualized grid on raw gpui

A `Grid` view that owns two `poc_core::Axis`es and the scroll offset. Each frame it
computes the visible row/col range from the live `window.viewport_size()` and draws only
those cells (+ overscan) as **absolutely-positioned** `div`s at `Axis::offset_of(index)`.
White background, grey `border_1` gridlines, per-cell fill/bold/italic/alignment, column
letters + row numbers as headers, honouring **per-row heights and per-column widths**.
`on_scroll_wheel` drives interactive scrolling; the menu "Run Test" / `--run-test` flag
flips it into scripted mode, where it times its own render + newly-visible provider pulls,
records a `FrameSample`, and `window.request_animation_frame()`s the next frame until the
script ends — then finalizes, prints PASS/FAIL, and quits.

### `gpui-component/` — virtualized `DataTable`

A `SheetDelegate: TableDelegate` over the same provider, rendered by `gpui-component`'s
virtualized `DataTable`. Same `poc_core::RenderCell` styling path. Scripted mode drives
the component via `TableState::scroll_to_row` / `scroll_to_col` per frame and measures the
same way, so the two `results/*.json` are directly comparable.

**Structural finding (already known from the API, pre-run):**
- **Variable column widths: supported** — each `Column` carries its own width (from the
  provider's `col_width`) + min/max + interactive resize.
- **Variable row heights: NOT supported** — `DataTable`'s vertical virtualization is
  `uniform_list`-based (fixed row height). So this variant renders a **uniform** row
  height, while raw-gpui honours per-row heights. That's a real ergonomics gap for a
  spreadsheet (Excel has variable row heights), and it means the component would need a
  patch or a different vertical-virtualization primitive to match a spreadsheet exactly.
- `DataTable` also builds a `Column` object per column up front; at 16,384 columns that's
  a materialization cost the raw-gpui variant avoids (it virtualizes columns with no
  per-column object). Whether this matters at Excel-max width is part of what the Mac run
  should reveal.

### Scripts

`scripts/build_and_run.sh <variant>` (interactive) and `scripts/run_test.sh
<variant|both>` (one-shot measured run → `results/`), plus `scripts/README.md` with the
Mac prerequisites and the report checklist.

### Cargo pinning (studied from real examples)

Both variants pin `gpui` + `gpui_platform` to the Zed rev that `gpui-component`'s own
workspace pins: **`1d217ee39d381ac101b7cf49d3d22451ac1093fe`**. This rev is **post the
gpui Window/App split**: the app is bootstrapped with `gpui_platform::application().run(...)`
(not the older `Application::new().run(...)`), the render signature is
`fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement`,
and gpui-component's top-level window element must be a `gpui_component::Root`. The
`gpui-component` dependency itself floats `main`; the human should lock it to a known-good
`main` SHA (`git ls-remote`) before recording numbers, keeping the gpui rev and the
gpui-component SHA a known-good pair (the container proxy blocked resolving the SHA here).

**Reproduce (Mac):** `experiments/04-ui-poc/scripts/run_test.sh both`.

## Results / evidence

**Mac run recorded 2026-07-02** (human run, macOS/Metal; verbatim console transcript
committed at `results/human-run-2026-07-02-runtest.txt`; the two `*-runtest.json`
files with the env stamp were written on the human's machine — commit them from there
when convenient). 348 frames measured per variant. What is verified in-container:

- `poc-core`: `cargo test` → **20 passed**; `cargo clippy --all-targets -- -D warnings`
  → clean; `cargo fmt --check` → clean. This exercises the virtualization math, the
  cell-styling, the scripted-harness determinism/termination, the newly-visible-cell
  computation, and the gate/JSON reporting (incl. an Excel-max-rows no-OOM case and
  PASS/FAIL-at-threshold cases).

Mac run results (2026-07-02):

| Variant | build? | feels smooth? | frame p50 | frame p99 | frame max | cell-load p99 | frame-p99 gate | frame-max gate | cell-load gate | VERDICT |
|---|---|---|---|---|---|---|---|---|---|---|
| raw-gpui | yes | (Phase-1: "worked great"; not re-reported) | 921.17 µs | **1.98 ms** | 2.31 ms | 120.25 µs | PASS (≤8.33 ms) | PASS (≤16.67 ms) | PASS (≤2 ms) | **PASS** |
| gpui-component | yes | (not re-reported) | 17.00 µs | 37.21 µs | 44.17 µs | 41.83 µs | PASS | PASS | PASS | **PASS** |

**Reading the numbers.** The load-bearing result: **raw-gpui — the adopted grid — clears
the 120 fps gate with ~4.2× margin on p99** (1.98 ms vs 8.33 ms) and cell-load with ~16×
margin, closing the §5.4 "measured, not vibes" question in the affirmative.

**⚠ Adversarially flag before trusting the comparison:** gpui-component's frame numbers
(p50 **17 µs**, ~54× faster than raw-gpui) are surprisingly low for any real GPU frame —
plausibly its `uniform_list`-based `DataTable` does far less measured work per scripted
frame (uniform row heights, different scene/virtualization path), i.e. the two variants
are **not an apples-to-apples frame comparison**. Per the repo convention, do not use
this delta to rank the variants without reviewing what each harness actually measures.
It does **not** affect the §5.4 verdict (both PASS; raw-gpui passes on its own numbers)
or the architectural recommendation below (the disqualifiers are structural, not perf).

## Conclusion

**Measured §5.4 verdict (2026-07-02): PASS on both variants — GPUI hits the bar.** The
raw-gpui grid renders at frame p99 1.98 ms (~4.2× inside the 120 fps budget) with
cell-load p99 120 µs (~16× inside). What was stated pre-run still stands:

- The **engine-neutral core is correct and reproducible** (tested in-container): the
  virtualization scales to Excel-max without O(n) memory, the harness is deterministic,
  and the gates/recording match the §5.4 targets and the shared `bench_util` shape.
- **Architecturally, the raw-gpui variant is the better fit for a real spreadsheet**
  regardless of the perf numbers, because `gpui-component`'s `DataTable` cannot do
  variable row heights (uniform_list-based) and materializes a per-column object — both
  are things a spreadsheet needs to avoid. The `gpui-component` variant is valuable as a
  fast-to-stand-up baseline and a perf reference point.
- The go/no-go on "GPUI hits the bar" is a **measured** question the human's run answers.

## Recommended design + next-best alternative

- **Recommended: raw `gpui` custom grid** (the `raw-gpui/` variant) — full control over
  2D virtualization, variable row heights *and* column widths, no per-column
  materialization, and a render path we can profile and tune. This matches the
  spreadsheet's needs and is where the product grid should be built.
- **Next-best: `gpui-component` `DataTable`** — faster to stand up, good for tables, and
  a useful perf baseline; but its uniform-row-height virtualization and per-column
  `Column` objects make it a poorer fit for an Excel-fidelity spreadsheet without
  upstream changes. *(This ranking is provisional until the Mac run's perf numbers either
  confirm it or surface a surprise — e.g. if `DataTable` is dramatically faster, that
  reopens the trade-off.)*
  **Post-run (2026-07-02):** `DataTable`'s numbers *are* dramatically lower, but the
  delta is flagged as likely not apples-to-apples (see *Results / evidence* ⚠ note) —
  and both variants pass §5.4 with wide margins, so perf does not reopen the trade-off;
  the structural disqualifiers (uniform row heights, per-column materialization) decide
  it. **Ranking confirmed: raw-gpui custom grid.**

## Risks / open questions

- **GPUI API churn.** The pinned Zed rev is post the Window/App split; the code mirrors
  current examples for that rev. If the human bumps the rev, small API shifts are likely
  (e.g. `UniformListScrollHandle` internals, exact styling-helper names). Keep the rev
  pinned when recording numbers.
- **gpui-component SHA not locked here** (proxy blocked `git ls-remote`); the human must
  lock it to a known-good `main` SHA paired with the gpui rev before recording numbers.
- **Column materialization at 16,384 cols** in the `DataTable` variant is an unknown cost
  until measured; the raw-gpui variant sidesteps it.
- **Text-shaping cost** (bold/italic, long strings, wide columns) is the most likely
  frame-time risk and only shows on real Metal; the scripted horizontal pan across
  wide/variable columns is designed to surface it.
- **First-run compile time** on the Mac will be long (Cargo builds the pinned Zed gpui
  from source). This is a one-time cost, noted so it isn't mistaken for a hang.

---

## HUMAN RUN REQUIRED

> **COMPLETED (run_test) 2026-07-02.** Step 2 was run (`run_test.sh both`) — both
> variants built and PASS; results recorded above and in
> `results/human-run-2026-07-02-runtest.txt`. Still open from this checklist: commit the
> two `results/*-runtest.json` files from the Mac (they carry the env stamp), and the
> optional step-3 visual sanity/screenshot. Original ask follows.

The measured verdict and the raw-vs-gpui-component recommendation can only be completed
after a Mac run. Please do the following on a **macOS/Metal** machine and report back.

**0. One-time setup.**
   - Ensure Rust (edition 2024, recent stable) + Xcode CLT (`xcode-select --install`).
   - Lock the `gpui-component` pin for reproducibility:
     ```sh
     git ls-remote https://github.com/longbridge/gpui-component refs/heads/main
     ```
     Add `rev = "<that sha>"` to the two `gpui-component` / `gpui-component-assets` lines
     in `experiments/04-ui-poc/gpui-component/Cargo.toml`. (raw-gpui's gpui pin is fixed.)

**1. Build + feel each variant** (from `experiments/04-ui-poc/`):
   ```sh
   ./scripts/build_and_run.sh raw-gpui
   ./scripts/build_and_run.sh gpui-component
   ```
   Report, per variant: **did it build?** (paste any error), and **does it scroll/pan
   smoothly** (buttery / occasional hitching / choppy)?

**2. Run the measured perf test** (from `experiments/04-ui-poc/`):
   ```sh
   ./scripts/run_test.sh both
   ```
   Each run prints `frame render p50/p99/max`, `cell load p50/p99/max`, the three §5.4
   gate lines (PASS/FAIL), and a final `VERDICT`. Report:
   - the printed output for **both** variants, and
   - the contents of `results/raw-gpui-runtest.json` and
     `results/gpui-component-runtest.json`.

**3. (Optional) Correctness sanity.** Eyeball that cells show varied text/numbers, ~15%
   highlighted fills, scattered bold/italic, column letters + row numbers, and visibly
   variable column widths (and variable row heights in **raw-gpui**; uniform in
   gpui-component — expected). A screenshot is a nice-to-have.

**What we'll conclude with your report:** the §5.4 PASS/FAIL verdict (does GPUI hit the
bar?), the raw-vs-gpui-component perf comparison, and the final recommendation — filled
into the *Results*, *Conclusion*, and *Recommended design* sections above.

**If a variant does not build**, paste the first Cargo error; the likely culprits are the
`gpui-component` SHA pairing (step 0) or a minor API drift on a bumped gpui rev (see
Risks). The `poc-core` crate is independently verified, so a build issue is isolated to
the thin gpui shell.
