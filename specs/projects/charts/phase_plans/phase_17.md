---
status: complete
---

# Phase 17: Insert flow

## Overview

P17 opens the authoring UI on top of P16's write path: an **action-bar chart menu** (type glyphs)
that inserts a **near-empty authored chart** of the chosen type, which then (a) renders in the grid
via the existing `ChartLayer` and (b) saves through the P16 authored write path
(`write_authored_charts`). Exit: insert a line chart via the UI → it renders in the grid → it saves
(and reopens with the chart).

The one genuinely new subsystem (write-from-model) already exists (P16). P17 is the *plumbing*:
a mutating action-row control (menu → `Command::InsertChart`), a worker handler that holds authored
charts **snapshot-but-not-live** (they carry no `c:f` binding yet — ranges come in P19), and a
combined save that runs BOTH the loaded re-inject (`reinject_live_charts`, modes 1/2) AND the
authored write (`write_authored_charts`, mode 3) over one workbook without conflating their reports.

**Design decisions:**
- **Near-empty = a minimal placeholder template, not truly empty.** The in-grid renderers show the
  grey *"Unsupported chart type"* placeholder box for a bar/area/pie whose only series has no data
  (only line/scatter frame-render an empty series). So a near-empty inserted chart carries **one
  placeholder series with a few points** (category/value, or xy for scatter) + a generic title, so
  **every** type renders as its real kind. It has no `c:f` refs (the write path emits literals), so
  it is a template the user reshapes via the P19 edit panel / re-range — matching `ui_design §3.1`
  ("comes up nearly empty… edit it into good form").
- **Authored charts are held separately from `ChartBindings`** (the loaded live-bind set). They ride
  the same published `ChartSnapshot` (so they render) but are **never** touched by the dirty-set
  re-resolve nor the loaded re-inject on save — the write-path doc's "composable modes" split.
- **Insert is a mutating action-row control** → it follows the sibling contract: commit the pending
  in-cell edit first (`commit_pending_edit`), disable/close in degraded mode, and the worker rejects
  it when degraded.

Per CLAUDE.md the **action row has no pixel-render baseline**, so the pixel suite is OUT OF SCOPE;
the new menu UI is validated by gpui view tests + an Xvfb smoke launch. A near-empty chart in the
grid should not move any existing `grid_chart_*` baseline (new authored charts, not the fixtures the
baselines capture).

## Steps

### Model (`freecell-chart-model`)
1. **`src/authoring.rs`** (new): `pub enum ChartInsertKind { Line, Column, Bar, Area, Pie, Doughnut,
   Scatter }` (the menu's authorable types — every one maps to a `ChartKind` and has a renderer +
   serializer; bubble is excluded, it has no model/renderer). Add
   `pub fn near_empty_chart(self) -> Chart`: a titled ("Chart") one-series template with a few
   placeholder points — `SeriesData::CategoryValue` for line/column/bar/area/pie/doughnut,
   `SeriesData::Xy` for scatter — default axes + right legend. `impl ChartInsertKind { fn chart_kind
   }` centralizes the `→ ChartKind` mapping (Column→`Bar{Col}`, Bar→`Bar{Bar}`, Doughnut→`Pie{hole}`).
   Re-export both from `lib.rs`.

### Protocol (`freecell-engine::worker::protocol`)
2. `Command::InsertChart { sheet: SheetId, kind: ChartInsertKind, anchor: Anchor }` — engine-free
   (both `ChartInsertKind` + `Anchor` are `freecell_chart_model` types, already crossing this seam
   via `ChartSnapshot`/`ChartSpec`). Update the module doc note. Re-export `ChartInsertKind` from
   `worker/mod.rs` + crate `lib.rs`.

### Worker (`freecell-engine::worker::run`)
3. Add `authored_charts: Vec<AuthoredEntry>` to `Worker` (`AuthoredEntry { anchor_sheet: SheetId,
   spec: ChartSpec }`), init empty. Route `Command::InsertChart` in `process_batch` into its own
   bucket (exhaustive match — no catch-all), processed after `font_ops` via `insert_authored_chart`.
4. `insert_authored_chart(sheet, kind, anchor)`:
   - **degraded guard** → `EditRejected{Degraded}` (criterion #3), consistent with `apply_set_font`.
   - resolve sheet exists (backstop; UI only sends the active sheet).
   - build `ChartSpec::authored(kind.near_empty_chart(), anchor)`, push an `AuthoredEntry`.
   - bump `ops_seen` + `committed_ops` (dirty tracking so the unsaved chart is savable — it is NOT
     an IronCalc-undoable op, so nothing is pushed onto the undo/touch stacks; chart delete/undo is
     P18).
   - bump `chart_version`, `store_chart_snapshot`, emit `Published` (drives the window's
     `sync_charts` install).
5. `store_chart_snapshot`: merge `authored_charts` specs into their anchor-sheet groups. Keep the
   existing shared-`Arc` fast path when `authored_charts` is empty (off-screen-free unchanged).
6. `save_workbook`: run the **combined** save — mode 1/2 (`reinject_live_charts`, loaded) then mode 3
   (`write_authored_charts`, authored) over one `to_xlsx_bytes()`:
   - fast path unchanged: no loaded + no authored charts → `self.doc.save(path)` (byte-identical to
     before).
   - assign each authored chart a **free** `xl/charts/chartN.xml` part (scanning the post-reinject
     bytes so it never collides with a loaded part), resolve its host sheet name (drop a
     deleted-host authored chart), build `write::AuthoredChart` (empty `refs` → literals).
   - `write_authored_charts` **fails loudly** on a sheet that already carries a `<drawing>` — i.e.
     authoring onto a loaded chart's own sheet — surfaced as `SaveError` (criterion #4's fail-loud).
   - Only advance `chart_source_path` to the just-saved file when there are **no** authored charts
     (a resave must re-synthesize authored charts fresh; pointing `reinject`'s carry-source at a file
     that already holds them would double them). Documented.

### Chrome (`freecell-app::chrome::view`)
7. Add `chart_menu_open: bool`; add `Anchor::Chart` (idx 6, `ANCHOR_COUNT → 7`) for the popover x.
8. Action row: a **chart-icon trigger** button (its own group before the flex spacer), `disabled` in
   degraded mode, `selected` on `chart_menu_open`, toggling `toggle_chart_menu`.
9. `render_chart_menu`: a popover (same backdrop/occlude pattern as `render_num_fmt_popover`) listing
   the 7 `ChartInsertKind`s, each a `Button` with its type glyph + label, `on_click → insert_chart`.
10. `insert_chart(kind, window, cx)` — the **mutating-control contract**: close the menu, degraded
    backstop, then `if !self.commit_pending_edit(window, cx) { return; }` (criterion #1 — `window`
    threaded through), build an `Anchor` from the active cell (`from = active`, `to = active + 8×15`,
    clamped to Excel-max), send `Command::InsertChart { sheet: active_sheet, kind, anchor }`.
11. `set_degraded`: also clear `chart_menu_open` in the degraded branch (criterion #2). Add a
    `chart_menu_open()` test accessor.

### Assets (`freecell-app::shell::assets`)
12. Vendor tintable chart glyph SVGs (`icons/chart-{line,column,bar,area,pie,scatter,doughnut}.svg`,
    `stroke="currentColor"`) + add them to `FREECELL_ICONS`. Trigger uses `chart-column`.

## Tests

- **model** (`authoring.rs`): `near_empty_chart` returns the right `ChartKind` per `ChartInsertKind`
  (Column→col bar, Bar→horizontal bar, Doughnut→pie-with-hole, Scatter→Xy), one non-empty series,
  and (round-trip guard) serializes+re-parses through the write path.
- **chrome view** (criterion #1, #2):
  - `insert_chart_commits_pending_edit_first` — with a pending data-row edit, `insert_chart` commits
    it (a `SetCellInput` is recorded) **before** the `InsertChart` command; a cap-rejected edit
    blocks the insert (no `InsertChart`).
  - `insert_chart_sends_command_for_active_sheet` — records `InsertChart` with the active sheet +
    chosen kind + a sensible anchor.
  - `degrade_closes_open_chart_menu` — **open the menu**, THEN `set_degraded(true)`, assert the menu
    closed (and the trigger disables in render).
- **worker unit** (`run.rs`, criterion #3): `insert_chart_rejected_when_degraded` — a degraded worker
  rejects `InsertChart` with `EditRejected{Degraded}` and does not bump `chart_version` / publish an
  authored chart.
- **worker seam** (`worker_seam.rs`, criterion #4):
  - `insert_line_chart_publishes_authored_snapshot` — `InsertChart` → the snapshot carries an
    **authored** (`is_authored`) line chart on the active sheet; version bumped.
  - `combined_save_writes_loaded_and_authored_charts` — open a line-chart fixture, add a 2nd sheet,
    insert an authored chart on **that** sheet, Save → reopen has **both** charts.
  - `authored_chart_on_a_loaded_charts_sheet_fails_loudly` — inserting an authored chart on the
    loaded chart's OWN sheet → Save replies `SaveFailed` (the two reports not conflated; fail-loud).

## Render validation

Action row = no baseline → pixel suite out of scope. Validate with the gpui view tests above + an
Xvfb smoke launch (`xvfb-run -a cargo run -p freecell-app`, open the welcome window, insert a line
chart). A near-empty authored chart is not a `grid_chart_*` baseline fixture, so no baseline moves;
if in doubt, run only `render_tests.sh test grid_chart_` (do NOT run the full suite — deferred P21).
