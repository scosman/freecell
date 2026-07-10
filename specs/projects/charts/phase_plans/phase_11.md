---
status: complete
---

# Phase 11: Line perf + baselines

## Overview

P11 hardens the line-chart pipeline's **performance** and locks in **measured p50/p99** for the
three exit-criterion ops, per `architecture.md §5` challenge 5 and `functional_spec.md §8`:

- **Lazy parse off open's critical path.** Today the worker calls `discover_and_parse_by_sheet`
  **synchronously in `load_and_run`, before the loop runs** — so it blocks the first
  `SetViewport → Published` (first cell-value paint). P11 makes chart discovery **lazy, per-sheet,
  deferred until after that sheet's first paint**: cells publish first; a sheet's charts are
  discovered + bound the first time that sheet is activated. Save forces a full sweep first so a
  never-visited chart sheet is still preserved.
- **Off-screen free.** The app currently deep-copies every chart's render `Chart` into a resident
  `Vec<RenderedChart>` per sheet, and holds them all whether on- or off-screen. P11 makes the grid
  **share** the engine's published `Arc<[ChartSpec]>` (zero app-side duplicate) and hold only a
  tiny per-chart **placement** (`anchor` + derived `fidelity`); the per-frame scan reads only the
  placements (culling off-screen), and the heavy `Chart` is touched **only for the on-screen few**
  (re-materialized from the shared spec when a chart scrolls back in). A huge sheet with K charts
  no longer holds K render pictures resident twice.
- **Coalesced dirty-set recompute.** Already structural since P9 (`apply_edit_batch` drains the
  command queue into one batch, runs **one** `evaluate()`, then **one** `reresolve_charts` over the
  whole batch's edited-cell set → one snapshot + one `Published`). P11 verifies it with tests and
  makes the per-edit snapshot clone cheap by putting the heavy retained source behind `Arc` (so
  `specs_by_sheet` no longer deep-copies chart XML on every intersecting edit — the flagged
  `binding.rs` NOTE).

**Perf targets.** The specs set **no hard numeric targets** — `architecture.md §5` and
`functional_spec.md §8` both say the p50/p99 targets are **"set + measured at the line-chart
checkpoint"** (the BLOCKING human review right after P11). So P11's job is to **measure and report**
p50/p99 for first-paint / edit-rerender / scroll-with-K, environment-stamped, and to note the
targets are ratified at the checkpoint. The scroll-with-K frame budget is compared to the repo's
existing `FRAME_TARGET_NS` (8.33 ms) / `FRAME_WORST_NS` (16.67 ms) as reference.

**Render tests.** These are perf/plumbing changes; an on-screen chart's pixels must not move. The
existing committed `chart_line_*` + `grid_chart_*` perceptual-diff baselines are the guard — P11
confirms they still hold (subset run) and adds **no** new baseline (off-screen-free must be
pixel-identical for an on-screen chart). The manager runs the full pixel suite in the late phase.

## Steps

### A. Cheap, shareable retained source (`freecell-chart-model`)

1. `src/spec.rs`: change `Origin::Loaded { source: SourceXml }` →
   `Origin::Loaded { source: std::sync::Arc<SourceXml> }`. `ChartSpec::loaded` wraps
   `Arc::new(source)`. `source()` returns `Option<&SourceXml>` unchanged (`Some(source)` via
   `Arc` deref). `Clone`/`PartialEq` derive through `Arc`. Result: cloning a `ChartSpec` no longer
   deep-copies the chart XML — only bumps the source refcount (+ clones the render `Chart`, which is
   the value that actually changed on a re-resolve). Add a `loaded` doc note. Existing spec tests
   use `.source().map(...)` and are unaffected.

### B. Lazy, per-sheet chart discovery (`freecell-engine`)

2. `src/chart/load.rs`: add
   `pub fn discover_and_parse_for_sheet(path: &Path, sheet_name: &str) -> Result<SheetCharts>` —
   walks the package (`discover` + the `workbook.xml.rels` name→part map), finds the **one**
   worksheet whose file name equals `sheet_name`, and parses **only that sheet's** chart parts into
   `(part, ChartSpec)` (empty when the sheet has no drawing/charts or the name isn't in the file).
   Genuine per-sheet parse (only that sheet's chart XML is read), reusing `parse_discovered_chart`.
3. `src/chart/binding.rs`:
   - `specs_by_sheet(&self) -> Vec<(SheetId, Arc<[ChartSpec]>)>` (was `Vec<(SheetId,
     Vec<ChartSpec>)>`) — collect per sheet then `Arc::from(vec)`. Update the P11 NOTE (source now
     Arc-shared, so the per-snapshot clone is cheap; the render `Chart` is still cloned, as intended).
   - add `pub fn add_missing(&mut self, groups: Vec<(SheetId, SheetCharts)>) -> bool` — appends only
     charts whose `chart_part` isn't already bound (dedup by part, robust to a name-fallback
     collision), anchoring each to its group's sheet, parsing its binding; returns whether anything
     was added. Used by both the per-sheet lazy path and the save-time full sweep, so a chart is
     never bound twice (and a live-resolved chart's values are never clobbered by a re-parse).
   - keep `from_specs_by_sheet` (tests still build bindings directly).
4. `src/worker/charts.rs`: `ChartSnapshot.sheets: Vec<(SheetId, Arc<[ChartSpec]>)>`.
5. `src/worker/run.rs`:
   - Worker fields: `discovered_chart_sheets: HashSet<SheetId>` (sheets whose zip we've already
     walked — walk each at most once), `charts_fully_discovered: bool`.
   - **Remove** the eager `discover_and_parse_by_sheet` block from `load_and_run` (charts start
     empty; discovery is deferred).
   - `process_batch`: after the viewport publish + `ensure_active_cache_built`, when
     `viewport_changed`, call `self.ensure_active_sheet_charts_discovered()`.
   - `ensure_active_sheet_charts_discovered(&mut self)`: no-op if `charts_fully_discovered`, no file
     path, or the active sheet was already walked (`discovered_chart_sheets` insert returns false);
     else resolve the active sheet's current name, `discover_and_parse_for_sheet`, `add_missing`,
     and on any add: bump `chart_version`, `store_chart_snapshot`, emit `Published` (charts ride the
     same event P9 uses). Cells already published above → parse is off the first-paint path.
   - `save_workbook`: call `self.ensure_all_charts_discovered()` **first**, so a never-painted chart
     sheet is still saved.
   - `ensure_all_charts_discovered(&mut self)`: no-op if already full; else
     `discover_and_parse_by_sheet` → `groups_to_sheet_ids` → `add_missing` (dedup-by-part keeps the
     already-live charts and adds the rest); set `charts_fully_discovered`; publish if anything added.
   - `store_chart_snapshot` unchanged (reads `specs_by_sheet`).

### C. Off-screen free in the grid (`freecell-app`)

6. `src/grid/chart_layer.rs`: replace `RenderedChart { chart, anchor, fidelity }` with
   `ChartPlacement { anchor: Anchor, fidelity: Fidelity }` (both `Copy`, tiny) +
   `ChartPlacement::from_spec(&ChartSpec)` (derives fidelity once). The render `Chart` is **no
   longer copied** into the grid — it stays in the shared spec. Update the module's `RenderedChart`
   test to a `ChartPlacement` test (drop the `.chart` assertions).
7. `src/grid/view.rs`:
   - `charts: HashMap<SheetId, SheetCharts>` where
     `struct SheetCharts { specs: Arc<[ChartSpec]>, placements: Vec<ChartPlacement> }`.
   - `set_sheet_charts(&mut self, sheet, specs: Arc<[ChartSpec]>, cx)`: empty → remove; else derive
     `placements` (one per spec) + store `{ specs, placements }`.
   - Paint loop: iterate `placements` (the tiny per-chart scan); compute `anchor_rect`; cull
     off-screen (unchanged); on-screen → borrow `&specs[i].chart` + `placements[i].fidelity` for
     `in_grid_chart_element`. Off-screen charts never touch the heavy `Chart`.
   - add `pub(crate) fn on_screen_chart_indices(&self, scroll_x, scroll_y, vw, vh) -> Vec<usize>`
     used by the test to prove the cull frees off-screen charts + re-materializes on scroll-back.
   - `sheet_chart_fidelities` reads `placements`.
8. `src/shell/window.rs` `sync_charts`: hand the grid each sheet's **shared** `Arc<[ChartSpec]>`
   from the snapshot (`specs.clone()` = refcount bump, zero copy); dropped sheets clear with an
   empty `Arc`.

### D. Call-site + test updates

9. `src/shell/app.rs` tests (3), `src/grid/view.rs` tests (2), `render-tests/src/render.rs`: wrap
   the per-sheet spec `Vec` in `Arc::from(...)` for `set_sheet_charts` / `ChartSnapshot`.
10. `tests/worker_seam.rs`: `spawn_line_fixture` + the save test now **send a viewport** before
    expecting charts (lazy discovery triggers on first paint — the real app always sends one).
    `snapshot_series_values` indexes the `Arc<[ChartSpec]>` (derefs to slice, unchanged).

### E. Perf harness (`render-tests`)

11. `src/bin/chart_perf.rs` (+ `[[bin]]` in `Cargo.toml`): a **headless** (no GPU) foreground bench
    that measures + reports **p50/p99** for the three ops, environment-stamped, FORCE+ASSERTING each
    op actually happened, and writes `results/chart-perf.json`:
    - **first-paint**: R× `discover_and_parse_for_sheet(line fixture, "Data")` + `add_missing` +
      `specs_by_sheet`; assert one chart with the fixture's values.
    - **edit-rerender**: bind once; R× `dirty_indices` (edit inside the chart range) + `reresolve`
      (fake live reader) + `specs_by_sheet`; assert the resolved value changed.
    - **scroll-with-K**: synthesize **K = 1000** line-chart specs spread down a 1,048,576-row sheet;
      build the grid's placement store; over a sweep of scroll offsets run the rect+cull scan
      (`anchor_rect` + `ChartRect::is_offscreen`), materializing only the on-screen few; assert
      K = 1000 total and only a handful on-screen (most culled) and many distinct scrolls; report
      per-scan p50/p99 vs `FRAME_TARGET_NS` / `FRAME_WORST_NS`.
    - Run with `cargo run -p render-tests --release --bin chart_perf` under a `timeout`; record the
      numbers + environment in this plan and the summary. Adversarially review (a scan that measures
      ~nothing = the FORCE assert would have failed).

## Tests

Engine (headless):
- `binding.rs::add_missing_dedupes_by_chart_part` — adding a group whose part is already bound is a
  no-op; a new part is appended (the lazy + save sweeps never double-bind / clobber live values).
- `binding.rs::coalesced_multi_edit_recompute_is_one_pass` — a batch touching two charts' ranges →
  one `dirty_indices` selects both → one `reresolve` updates both → one `specs_by_sheet` snapshot
  (the dirty-set coalesces N edits into a single recompute).
- `load.rs::discover_and_parse_for_sheet_parses_only_the_named_sheet` — the two-sheet fixture: asking
  for "Data" returns only its column chart; "Summary" only its line chart; an unknown name → empty.
- `worker_seam.rs::charts_are_not_discovered_until_first_paint` — after open (no viewport) the chart
  snapshot version stays 0; after a `SetViewport` it becomes ≥ 1 with the chart (lazy, off the
  critical path).
- `worker_seam.rs::save_preserves_charts_when_their_sheet_was_never_painted` — open a chart file,
  `Save` **without** sending a viewport, reopen → the chart is present (save forces a full sweep).
- `worker_seam.rs::coalesced_edits_converge_the_chart` — two edits to two cells in the chart's range
  → the final snapshot reflects **both** (correctness under coalescing; version advanced).
- Existing P9/P10 chart seam tests still pass (with the viewport-send added to the helpers).

App (headless gpui):
- `chart_layer.rs::chart_placement_from_spec_derives_fidelity` — placement carries anchor + derived
  fidelity (Faithful / Degraded / Unsupported), no `Chart` copy.
- `view.rs::set_sheet_charts_stores_and_derives_fidelity` — unchanged behavior via the Arc signature.
- `view.rs::offscreen_charts_are_freed_and_rematerialize_on_scrollback` — install K charts where only
  a few are on-screen; `on_screen_chart_indices` returns just those (off-screen freed from the
  build); scroll a previously-off-screen chart into view → it now appears in the on-screen set
  (re-materialized), and a scrolled-away one drops out.

Render (subset, no new baseline):
- `render_tests.sh test grid_chart` + `test chart_` stay green (on-screen chart pixels unchanged by
  the perf refactor). No new committed baseline (off-screen-free is pixel-identical on-screen).

## Measured p50/p99

Environment (stamped into `results/chart-perf.json`): Intel Xeon @ 2.10 GHz, 4 cores, x86_64 linux;
**profile=release**; rustc 1.95.0. Headless — no GPU; the three ops are CPU engine/render work off
the pixel path. Bench: `render-tests/src/bin/chart_perf.rs` (FORCE+ASSERTED each op ran).

| Op | p50 | p99 | max | Reference budget | Notes |
|---|---|---|---|---|---|
| first-paint (discover+parse+bind+snapshot, 1 line chart) | 369 µs | 435 µs | 455 µs | (target set at checkpoint) | off the critical path — cells paint first; this parse runs a frame later. Includes the real zip open + XML parse each run. |
| edit-rerender (dirty-set → reresolve → snapshot) | 1.87 µs | 2.58 µs | 24.8 µs | (target set at checkpoint) | one intersecting chart; the `Arc`-shared source keeps the snapshot clone O(values). |
| scroll-with-K (K=1000, per-frame rect+cull scan) | 6.37 µs | 14.1 µs | 36.7 µs | 8.33 ms / 16.67 ms | K=1000 charts scanned per frame, ~2 on-screen (rest culled) → ~1300× under the frame budget. |

**Adversarial review.** None is a measured no-op: first-paint asserts the chart bound with the
file's cached values; edit-rerender asserts the dirty set selected the chart, the value changed, and
the new value republished; scroll asserts all K were scanned, only ~2 on-screen, and the on-screen
charts' data was materialized (checksummed). The scroll scan at 6 µs p50 for 1000 charts confirms
"scroll stays at frame budget with charts present." **Targets are ratified at the post-P11 human
checkpoint** (`functional_spec.md §8` / `architecture.md §5`); the numbers above are the baseline.

## Render baselines

No new committed baseline. The perf refactor (lazy parse, `Arc`-shared snapshot, placement-based
cull) must not move an on-screen chart's pixels — the existing committed `chart_line_*` +
`grid_chart_*` perceptual-diff baselines are the guard; the `grid_chart` + `chart_` subset is run to
confirm no drift.
