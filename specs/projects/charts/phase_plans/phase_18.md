---
status: complete
---

# Phase 18: Manipulate (select / move / resize / delete)

## Overview

P18 makes in-grid charts **interactive**: a chart can be **selected** (outline + resize handles on the
ChartLayer), **moved** (drag its body), **resized** (drag a handle), and **deleted** (Delete/Backspace
or a chart context-menu entry). Every manipulation persists to the chart's **anchor** and **round-trips**
(open → move/resize/delete → save → reopen reflects the change), for BOTH:

- **Authored** charts (P17) — the anchor lives in the model (`AuthoredEntry.spec.anchor`), so move/resize
  just rewrites it and delete drops the entry; the write-from-model path re-synthesizes the drawing.
- **Loaded** charts — the anchor lives in the **retained drawing part** (carried byte-for-byte by the
  source-first save). Move/resize must **patch that drawing part's `twoCellAnchor`**; delete must **drop
  the chart from the saved package** (its anchor + part chain) without corrupting the rest.

### Identity across the app↔engine seam
Charts get a stable **`ChartId`** (new `chart-model` newtype). The worker assigns one per loaded chart
(at bind) and per authored chart (at insert) from a single monotonic counter, and **stamps it onto each
published `ChartSpec`**. The grid selects/manipulates by `ChartId`; the manipulation commands carry it;
the worker resolves it to a loaded `BoundChart` (→ its `chart_part`) or an `AuthoredEntry`.

### Move == Resize == "set anchor"
Move and resize both produce a **new `Anchor`**, so they share one command (`SetChartAnchor`). The grid
computes the new anchor by the **inverse** of the P8 `anchor_rect` (`rect_to_anchor`): a content-pixel
rect → cell + EMU-offset corners against the grid geometry, so it tracks scroll/zoom for free.

### Undo (reconciling P17's deferral)
Chart insert/move/resize/delete are **worker-side state**, not IronCalc model ops — there is no IronCalc
undo hook for them, and interleaving a parallel chart-undo history with IronCalc's undo/touch stacks
(which P17 explicitly kept them off of) is a large, desync-prone subsystem the interaction spec does
**not** call for (`functional_spec §6.A`, `ui_design §3.2` list only select/move/resize/delete). **Decision
(spec-aligned, per the phase brief's "or if the spec scopes undo differently, follow the spec and note
it"):** chart manipulation is **immediate and not on the Ctrl+Z stack**, matching P17's insert. Cell
undo/redo stays fully correct with charts present (charts are independent of IronCalc's stacks). This is
documented in code + the summary. (A future phase can add a dedicated chart-history if the product wants
undoable chart edits.)

## Steps

### Model (`freecell-chart-model`)
1. **`spec.rs`**: add `pub struct ChartId(pub u64)` (Copy/Eq/Hash) with `ChartId::NONE = ChartId(0)`;
   add `pub id: ChartId` to `ChartSpec` (constructors default it to `NONE`) + a builder
   `with_id(mut self, id) -> Self`. `id` is render-irrelevant (not drawn) so no baseline moves. Re-export
   `ChartId` from `lib.rs`.

### ChartLayer geometry (`freecell-app::grid::chart_layer`)
2. Extend `GridGeometry` with `col_at(&self, x) -> u32` / `row_at(&self, y) -> u32` (index-at-offset). Add
   `rect_to_anchor(rect, geom, scroll_x, scroll_y) -> Anchor` — the inverse of `anchor_rect` (content
   rect → from/to cells + EMU offsets). Round-trips with `anchor_rect` on the uniform mock.
3. Resize-handle model: `enum Handle` (8 = 4 corners + 4 edge midpoints); `HANDLE_PX`/`MIN_CHART_PX`
   consts; `handle_rects(rect) -> [(Handle, ChartRect); 8]` (the small squares); `handle_at(rect, x, y)`
   (hit-test, with a small slop); `resize_rect(start, handle, dx, dy) -> ChartRect` (apply a drag delta to
   the dragged edges, clamped ≥ `MIN_CHART_PX`). Pure + unit-tested.

### Grid interaction + render (`freecell-app::grid::view`)
4. New state: `selected_chart: Option<ChartId>`, `chart_drag: Option<ChartDrag>`
   (`ChartDrag { id, mode: Move | Resize(Handle), grab: (f32,f32) content-px, start_rect, current_rect }`).
5. `chart_hit_test(...)` helper: given the active layer + an axis geometry + scroll + content dims +
   currently-selected id, hit-tests topmost-first — a handle of the selected chart → `Resize`, else a
   chart body → `Body`, else `Miss`. Built from `visible_charts` over a lightweight `AxisGeometry`.
6. `handle_mouse_down`: **before** cell hit-testing, run `chart_hit_test`. A handle hit → begin a resize
   `chart_drag`; a body hit → select + begin a move `chart_drag` (focus grid; **do not** change the cell
   selection); a miss → clear `selected_chart` and fall through to the existing cell path.
7. `handle_mouse_move`: if `chart_drag` is `Some`, update `current_rect` (translate for Move, `resize_rect`
   for Resize) and `notify` (before the resize/cell-drag arms).
8. `handle_mouse_up`: if a `chart_drag` is taken and the rect actually changed, `rect_to_anchor` →
   `GridEvent::ChartAnchorChanged { id, anchor }`; clear the drag. A zero-movement press stays a pure
   select.
9. `handle_key_down`: with a `selected_chart` and no in-cell editor, **Delete/Backspace** →
   `GridEvent::ChartDeleted { id }` + clear selection (intercept before the `ClearCells` mapping);
   **Escape** cancels a live `chart_drag` / clears the chart selection.
10. Render (ChartLayer build): paint the dragged chart at `current_rect`; when `selected_chart` matches an
    on-screen chart, add the **selection outline** (accent border) + **8 handle squares** at its rect —
    NEW grid chrome (in-scope for the pixel suite → one new baseline). `set_selected_chart(Option<ChartId>,
    cx)` (pub, for the window + render harness); on `set_sheet_charts`, drop a selection whose id is gone.
    A right-click on a selected chart body opens a tiny **"Delete chart"** context menu (reuses the
    header-menu overlay pattern) — the spec's alternate delete affordance.

### GridEvent + window wiring
11. **`grid/mod.rs`**: add `GridEvent::ChartAnchorChanged { id: ChartId, anchor: Anchor }` and
    `GridEvent::ChartDeleted { id: ChartId }`.
12. **`shell/window.rs`** (`make_grid_sink`): route both straight to the worker (like the other
    grid-initiated structure ops) — `Command::SetChartAnchor { sheet: active, id, anchor }` /
    `Command::DeleteChart { sheet: active, id }`.

### Protocol (`freecell-engine::worker::protocol`)
13. Add `Command::SetChartAnchor { sheet, id: ChartId, anchor: Anchor }` and
    `Command::DeleteChart { sheet, id: ChartId }` (both engine-free seam types). Re-export `ChartId` from
    `worker/mod.rs` + crate `lib.rs`.

### Worker (`freecell-engine::worker::run`)
14. Worker fields: `next_chart_id: u64` (from 1); `loaded_anchor_edits: HashMap<String, Anchor>`
    (chart_part → new anchor, moved/resized loaded charts vs. the current `chart_source_path`);
    `loaded_deletes: HashSet<String>` (deleted loaded chart parts). `AuthoredEntry.id: ChartId`.
15. `ChartBindings` (`binding.rs`): `BoundChart.id: ChartId`; `assign_missing_ids(&mut u64)` (stamp NONE
    ids from the counter); `set_anchor_by_id(id, anchor) -> Option<String>` (updates the render spec +
    returns the chart_part); `remove_by_id(id) -> Option<String>`; id-stamping in `specs_by_sheet`. The
    worker calls `assign_missing_ids` after every `add_missing`, and filters discovered specs by
    `loaded_deletes` so a save-time full sweep never resurrects a deleted chart.
16. Route `SetChartAnchor`/`DeleteChart` into their own post-batch bucket (exhaustive match). Handlers
    (degraded-guarded like insert): resolve the id (authored first, then loaded); authored → mutate/remove
    the `AuthoredEntry`; loaded → `set_anchor_by_id` + record `loaded_anchor_edits`, or `remove_by_id` +
    record `loaded_deletes` (and clear any pending anchor edit for it). Bump `ops_seen`/`committed_ops`
    (savable + dirty), `chart_version`, `store_chart_snapshot`, emit `Published`. Stamp ids in
    `store_chart_snapshot` (fast path + authored path).
17. `save_workbook`: thread `loaded_anchor_edits` + `loaded_deletes` into `reinject_live_charts`; on a save
    that advances `chart_source_path` (no authored charts), **clear** both sets (baked into the new source).

### Chart save path (`freecell-engine::chart::save`)
18. `reinject_live_charts(original, model_bytes, live, anchor_edits, deletes)` + `reinject(...)` gain the
    two maps. New `patch_drawing_xml(drawing_xml, drawing_rels_xml, edits, deletes) -> (xml, rels, remaining)`:
    map `chart_part → rel_id` (drawing rels), walk each anchor's enclosing `<c:chart r:id>`; a **deleted**
    chart's whole anchor element is spliced out (+ its rel dropped from the drawing `_rels`), a **moved**
    chart's `<from>`/`<to>` are rewritten (prefix-preserving byte splices, like `patch_chart_source`). In
    `reinject`: force a drawing whose charts are **all** deleted to a dropped target (whole chain excluded);
    for a surviving drawing with edits/partial-deletes, substitute the patched drawing XML + `_rels` in the
    carry loop and exclude the deleted charts' chart-part chains from carry + content-types.

### Render baseline (`app/render-tests`)
19. Add ONE new case `grid_chart_selected` — the existing in-grid line chart with `selected_chart` set
    (chart built `.with_id(ChartId(1))`). `RenderCase.selected_chart: Option<ChartId>`; the harness calls
    `GridView::set_selected_chart` after installing charts. Generate + **eyeball** + commit that ONE
    baseline; existing `grid_chart_*` baselines must NOT move (unselected charts render exactly as before).

## Tests

- **chart_layer** (unit): `rect_to_anchor` round-trips `anchor_rect`; `handle_at` hits each of the 8
  handles + misses the interior; `resize_rect` moves the right edges + clamps to `MIN_CHART_PX`.
- **grid view** (gpui): mouse-down on a chart body selects it (`selected_chart` set) without moving the
  cell selection; a body drag → `ChartAnchorChanged` with the translated anchor; a handle drag →
  `ChartAnchorChanged` with a resized anchor; Delete with a selected chart → `ChartDeleted`; a miss-click
  clears the selection; Escape cancels a drag.
- **save/drawing** (engine unit): `patch_drawing_xml` rewrites a moved chart's `<from>`/`<to>` (prefix
  preserved, other bytes intact) and removes a deleted chart's anchor (+ its rel), reporting the remaining
  count.
- **worker seam** (integration, via the IronCalc reopen path — LibreOffice is env-broken, do not rely on
  it):
  - `move_authored_chart_roundtrips` — insert → `SetChartAnchor` → Save → `discover_and_parse` shows the
    new anchor.
  - `delete_authored_chart_roundtrips` — insert two → delete one → Save → reopen has one chart.
  - `move_loaded_chart_patches_drawing_and_roundtrips` — open line fixture → `SetChartAnchor` → Save →
    reopen's `spec.anchor` is the new anchor; an untouched resave stays stable.
  - `delete_loaded_chart_drops_it_from_the_package` — open line fixture → `DeleteChart` → Save → reopen has
    zero charts (workbook still opens).
  - `set_chart_anchor_rejected_when_degraded` / `delete_chart_rejected_when_degraded`.
  - `cell_undo_redo_is_correct_with_charts_present` — documents the undo decision (a cell edit undoes/redoes
    cleanly while an authored chart rides the snapshot).

## Render validation

This phase adds NEW ChartLayer chrome (selection outline + handles) → in-scope for the pixel suite, but
full-suite runs are the P21 gate. While coding: an **unselected** chart must not move any `grid_chart_*`
baseline (verify with `render_tests.sh test grid_chart_` only); the ONE new `grid_chart_selected` baseline
is generated + eyeballed + committed here. Do NOT run the full suite or blanket-regenerate.
