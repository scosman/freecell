---
status: complete
---

# Phase 11: Integration — real DocumentClient wired end-to-end

## Overview

Phases 1–10 built the seams; this phase connects them. Each `WorkbookWindow` gets a
real `GridView` + `ChromeView` composed into its body (replacing the Phase-10
placeholder), a live `DocumentClient`, and full event routing so open / edit / eval /
save / sheet-switch / error flows work end-to-end against the real IronCalc worker.

The load-bearing constraints preserved:

- **Grid render path stays zero-engine-call** — it reads only the caches + publication
  (the `DocumentClient`'s shared read-surfaces).
- **No cross-entity reentrant `entity.update`.** Grid ⇄ chrome coupling routes through
  boxed-closure sinks that capture the *sibling* entity handles (not the window
  entity), and cyclic flows (`MoveActive`, `SetActiveSheet`, cap-reject revert) are
  broken with `window.defer`. The window entity is only updated by the worker-event
  task, action handlers, and modals — never from within a sibling's `update`.

## Steps

### A. Engine: expose the grid's shared read-surfaces + source `has_content`

1. `worker/client.rs` — `Shared`: change `publication: ArcSwap<Publication>` →
   `Arc<ArcSwap<Publication>>` and `generation: AtomicU64` → `Arc<AtomicU64>` (the
   worker's `self.shared.publication.store(..)` / `generation.fetch_add(..)` still work
   through the `Arc`). Add `DocumentClient::publication_swap() -> Arc<ArcSwap<Publication>>`
   and `generation_counter() -> Arc<AtomicU64>` so the window can build
   `GridDataSources` (which need exactly those two `Arc` shapes + `caches()`).
2. `worker/protocol.rs` — add `has_content: bool` to `SheetMeta` (DECISIONS Phase-9
   follow-up: the delete-confirm rule needs it; a non-empty sheet must confirm before
   delete — data-safety). `document.rs` — add `pub(crate) fn sheet_has_content(idx) ->
   bool` (`!worksheet.sheet_data.is_empty()`). `run.rs sheet_metas()` populates it.
   Update engine + chrome tests that construct `SheetMeta { id, name }`.
3. `chrome/mod.rs` / `chrome/view.rs` — `merge_sheet_metas` now sources `has_content`
   from `meta.has_content` (drop the "preserve own guess" fallback).

### B. Grid: laid-out bounds (the Phase-6/8 "PHASE 11 window-vs-element bounds" marker)

4. `grid/view.rs` — add `bounds: Option<Bounds<Pixels>>`. In `render`, prepend a
   `gpui::canvas` probe (`.absolute().size_full()`) whose prepaint captures the grid
   element's real bounds into the entity (notify only on change). Replace every
   `window.viewport_size()`-derived viewport (render, scroll, mouse, page_rows,
   autoscroll, reveal) with a `viewport_wh(window)` helper = `bounds.size` (fallback to
   `window.viewport_size()` when not yet captured). Make `event_local` a `&self` method
   subtracting `bounds.origin`. Backward-compatible: when the grid is full-window
   (render-tests / demo) bounds ≈ full window, identical behavior.
5. `grid/view.rs` — factor the `handle_key_down` Motion arm into
   `pub fn move_active(&mut self, motion, window, cx)`; add `pub fn focus_self(&self,
   window, cx)`. In `render`, emit a debounced `ViewportChanged` when the resolved
   visible range differs from `last_viewport` (the single viewport-announce mechanism —
   covers initial paint, sheet switch, resize; scroll/keyboard still emit eagerly, all
   debounced through `last_viewport`).

### C. Chrome: host the grid + small Phase-11 hooks

6. `chrome/view.rs` — add `body: Option<gpui::AnyView>` + `set_grid_body(view, cx)`.
   `render` places the body with `flex_1().min_h_0()` between the data row and tab bar,
   and the chrome root gains `flex_1()` when it has a body (so the grid slot fills). Add
   `refresh_active_style(cx)` (re-read the active cell's `render_style` without touching
   the data row — for `StyleCacheUpdated`).

### D. Window: compose + route (the bulk)

7. `shell/window.rs` — `WorkbookWindow`:
   - `client: Rc<DocumentClient>` (shared with chrome as `Rc<dyn ChromeClient>`).
   - hold `grid: Entity<GridView>`, `chrome: Entity<ChromeView>`, and the shared
     `active_sheet: Rc<Cell<SheetId>>` + `last_sel: Rc<Cell<SelectionModel>>` the sinks
     read.
   - `new`: build `GridDataSources` from the client; create the two entities with sinks
     (via `Rc<OnceCell<WeakEntity<..>>>` slots resolved after both are built); set the
     grid as the chrome's body; set the grid loading state for an `OpenFile`.
   - `render_body` → the chrome entity (which hosts the grid + shows the loading
     overlay); drop the placeholder.
   - **GridEventSink** (captures chrome + grid weak, client Rc, `active_sheet`,
     `last_sel`): `SelectionChanged` → commit-first (`on_edit_commit_requested`) then
     `on_selection_changed` + update `last_sel`, else (cap-blocked) defer a grid
     `set_selection(last_sel)` revert; `ViewportChanged` → `client.send(SetViewport{
     active_sheet, 3× overscan })`; `ClearCells` → `client.send(ClearCells{active_sheet})`.
   - **ChromeGridSink** (captures grid + chrome weak, client Rc, `active_sheet`):
     `FocusGrid` → `grid.focus_self` (direct); `MoveActive` → `window.defer` →
     `grid.move_active`; `SetActiveSheet` → `window.defer` → set `active_sheet`,
     `grid.set_active_sheet`, `chrome.on_selection_changed(restored sel)`.
   - `on_worker_event` — extend past lifecycle: `Loaded`/`SheetsChanged` → reconcile the
     sheet list + auto-switch (new sheet on add / away from a deleted active) and, on
     first load, set active sheet + initial selection fetch; `Published` → `grid.notify`;
     `StyleCacheUpdated` → `grid.notify` + `chrome.refresh_active_style`; `CellContent`
     / `EvalStarted` / `EvalFinished` / `EditRejected{InputCap}` → `chrome.on_worker_event`;
     `EditRejected{EnginePanic|Engine|InvalidSheetName}` → transient error dialog;
     `EditRejected{Degraded}` → ignore (bar already shown via `WorkerDegraded`).
   - Wire `ToggleBold/Italic/Underline` actions → `chrome.toggle_style(..)` (Phase-10
     left them no-op placeholders).
   - The `3× overscan` on `ViewportChanged` = a small pure helper (`overscan_range`).

## Tests

Integration tests via `#[gpui::test]` + `TestAppContext`, driving the composed window
by **injecting** worker events (`inject_worker_event_for_test`) and invoking grid/chrome
methods directly — never `run_until_parked` on the live OS-thread worker (the documented
Phase-10 determinism boundary).

- `loaded_populates_tabs_and_switches_active_sheet` — inject `Loaded{sheets}` → tabs
  mirror the metas, active sheet adopts `sheets[0]`, an initial `GetCellContent` fetch
  is issued.
- `grid_selection_routes_to_chrome_ref_box` — a grid `SelectionChanged` updates the
  chrome ref box + issues a content fetch (commit-first path, field Idle).
- `grid_viewport_change_sends_overscanned_setviewport` — a grid `ViewportChanged`
  produces a `SetViewport` for the active sheet with ~3× overscan (assert via a
  recording seam / the real client's stored viewport is untestable, so assert the pure
  `overscan_range`).
- `overscan_range_expands_and_clamps` — pure unit test of the overscan helper (3×,
  clamped to `[0, Excel-max)`).
- `chrome_move_active_deferred_updates_grid_selection` — chrome `MoveActive(Down)` (via
  a commit) moves the grid's active cell after the deferred tick (`run_until_parked`).
- `toggle_bold_action_routes_to_chrome` — the `ToggleBold` action sends a
  `SetStyleAttr{Bold}` through the client.
- `published_repaints_grid` / `style_cache_updated_refreshes_toggles` — injected events
  notify the grid / refresh the chrome active style.
- `clear_cells_uses_active_sheet` — a grid `ClearCells` sends `Command::ClearCells` with
  the window's active sheet.
- `sheet_has_content_gates_delete_confirm` (chrome) — a `has_content` tab opens the
  confirm modal; an empty one deletes immediately.
- `edit_rejected_engine_panic_shows_transient_dialog` — injected `EditRejected{EnginePanic}`
  shows the "couldn't be applied" error modal (non-closing).
- Engine: `sheet_meta_carries_has_content` — a sheet with a value reports
  `has_content = true`, an empty one `false`.

## Untestable boundary (documented, not silently skipped)

- **End-to-end flows that wait for the live worker to *emit*** (real `Loaded` closing
  Welcome, a real `Saved` round-trip, real `Published` values landing in the grid) are
  not deterministic under gpui's `TestScheduler` + a live OS thread (Phase-10 boundary).
  Covered instead by (a) the `freecell-engine` seam/round-trip tests, (b) event
  **injection** here, and (c) the manual smoke + the Xvfb launch below.
- **Pixel composition** of the wired window (grid inside chrome, real click offsets) is
  verified by an Xvfb+lavapipe smoke launch (open a fixture `.xlsx`, edit, save), not by
  the render-test baselines (which render the grid standalone/full-window).
</content>
</invoke>
