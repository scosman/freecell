---
status: complete
---

# Phase 3: Point-mode routing

## Overview

Ships the second half of the feature: **point-mode**. While a **formula** edit is open, a grid
click (or click-drag) on a reference-ready caret **inserts** the clicked cell/range into the
formula at the caret instead of committing + moving the selection; the pending-ref state lets a
follow-up click **re-aim** (replace) the just-pointed reference (Excel's pointing model). Builds
directly on Phase 2's plumbing: `reference_ready` / `pending_ref` are already pushed to the grid
via `EditState`; this phase makes the grid **consume** them, and adds the chrome-side splice
(`insert_reference`) + the grid-side `point_drag` state machine + merge resolution.

No new behaviour is added to the non-point path: a click on a **not**-reference-ready caret (or any
non-formula edit) still runs the existing commit-on-click (`SelectionChanged` →
`commit_then_adopt_selection`). The whole feature is FreeCell-only — no engine/fork change, no
gpui-component / vendored-widget change.

Anchors verified against source: `grid/mod.rs` `GridEvent`; `shell/window.rs` `make_grid_sink`
(~:1574) + the `EditState` forward (~:1926); `grid/view.rs` `mouse_down_cell` (:1494),
`handle_mouse_down` guard (:1305), `handle_mouse_move` (:1606) fill-drag block (:1632),
`handle_mouse_up` (:1662), `update_fill_drag`/`set_fill_target_from_cell` (:2355/:2396),
`maybe_start_autoscroll` (:2507) + `autoscroll_tick` (:2545), the overlay pass (fill-drag preview
:3228); `chrome/view.rs` `accept_autocomplete` (:1499), `recompute_formula_edit_state` (:1392),
`refresh_edit_grid_state` (:1303), `insert_reference` is new; `EditController::{pending_ref,
set_pending_ref, recompute_formula}` (`chrome/edit.rs`, already present from Phase 2);
`cache.merges() -> &[CellRange]` (`freecell-core/cache.rs:379`).

## Steps

### A. `GridEvent::InsertReference` + window route

1. **`grid/mod.rs` — new `GridEvent` variant.**
   `InsertReference { a1: String, replace_pending: bool }` with a doc comment (grid emits it when a
   reference-ready / pending-ref grid click points; the window routes it to
   `ChromeView::insert_reference`; `replace_pending` overwrites the pending span vs appends at the
   caret).

2. **`shell/window.rs` — route in `make_grid_sink`.** Add an arm (near the other chrome-routed
   variants):
   ```rust
   GridEvent::InsertReference { a1, replace_pending } => {
       if let Some(chrome) = chrome_slot.get().and_then(|w| w.upgrade()) {
           let a1 = a1.clone();
           let replace_pending = *replace_pending;
           chrome.update(cx, |c, cx| c.insert_reference(&a1, replace_pending, window, cx));
       }
   }
   ```

### B. Chrome — `insert_reference` + pending lifecycle

3. **`chrome/view.rs` — `insert_reference`** (the analog of `accept_autocomplete`,
   `architecture.md §5`):
   ```rust
   pub fn insert_reference(&mut self, a1: &str, replace_pending: bool,
                           window: &mut Window, cx: &mut Context<Self>);
   ```
   - Guard: only while a live edit (`self.data_row.mode() == FieldMode::Editing`) — point-mode is
     unreachable otherwise (the grid only emits when `reference_ready || pending_ref`).
   - Read the **driving** editor's `text` (`.value()`) + `caret` (`.cursor()`, byte offset).
   - Splice region `[start, end)`: if `replace_pending` **and** `self.edit.pending_ref()` is
     `Some(span)` (and `span` is within `text`) → `span`; else → `caret..caret` (append). Build
     `new_text = text[..start] + a1 + text[end..]`; `new_caret = start + a1.len()`.
   - Drive the shared reducer (`data_row.reduce(Edited{new_text})`), then
     `set_driving_text_and_caret(origin, &new_text, char_col_of(new_caret), …)`,
     `apply_data_effects`, `mirror_other_editor` — **exactly** the accept path's programmatic-text
     mechanics (`set_value` suppresses `Change`, so the insert does not re-fire the Change handler
     → does not clear its own pending span).
   - `self.edit.set_pending_ref(Some(new_caret - a1.len() .. new_caret))` — the just-inserted span
     becomes pending (`§5` Set).
   - `recompute_formula_edit_state_keep_pending(cx)` (the `keep_pending = true` recompute so the
     span it just set survives its own recompute), then `refresh_edit_grid_state`, `notify`.

4. **`chrome/view.rs` — split `recompute_formula_edit_state` on `keep_pending`.** Extract the body
   into `recompute_formula_edit_state_keep_pending(&mut self, keep_pending: bool, cx)` (cap-error →
   `clear_formula_state`; else read driving text/caret + active sheet name → `edit.recompute_formula(text,
   caret, &sheet, keep_pending)`); keep the existing `recompute_formula_edit_state(cx)` as the
   `keep_pending = false` caller so every current call site is unchanged. `insert_reference` calls
   it with `true`.

   Pending-ref lifecycle falls out for free: the user-driven Change handlers (`on_content_event`,
   `on_incell_event`) and the caret-move recompute already call `recompute_formula_edit_state(cx)`
   (i.e. `keep_pending = false`) → clears `pending_ref`; commit/escape clear via
   `clear_formula_state` (Phase 2). Only `insert_reference` keeps it (`§5` Cleared).

### C. Grid — point branch + `point_drag` + merges

5. **`grid/view.rs` — `PointDrag` struct + field.**
   ```rust
   struct PointDrag { origin: CellRef, last_range: CellRange }
   ```
   New field `point_drag: Option<PointDrag>` on `GridView` (mirror `fill_drag`); init `None` in
   `new`; clear in `set_active_sheet` (a sheet switch commits the edit; drop defensively so it can
   never leak).

6. **Merge helpers (Q6, one source of truth — `cache.merges()`).** Free fns over `&[CellRange]`
   (pure, unit-testable) + thin `&self` wrappers that read the active cache:
   - `resolve_merge_anchor_in(row, col, merges) -> CellRef`: first merge that `contains((row,col))`
     → its `start`; else `CellRef::new(row,col)`.
   - `expand_range_for_merges_in(range, merges) -> CellRange`: union `range` with every merge it
     `intersects`, iterating to a fixed point (an expansion can newly touch another merge).
   - `GridView::resolve_merge_anchor(&self, row, col)` / `expand_range_for_merges(&self, range)`:
     read `caches.get(active).merges()`, delegate; identity if no cache.

7. **`mouse_down_cell` point branch** (top of the method, before the selection/emit + drag-arm):
   ```rust
   let point_ready = self.reference_ready || self.pending_ref;
   if point_ready && !event.modifiers.shift {
       let cell = self.resolve_merge_anchor(row, col);
       let a1 = CellRange::single(cell).to_a1();
       self.events.emit(&GridEvent::InsertReference { a1, replace_pending: self.pending_ref }, window, cx);
       self.point_drag = Some(PointDrag { origin: cell, last_range: CellRange::single(cell) });
       window.prevent_default();  // keep editor focus (as the dbl-click path does)
       cx.notify();
       return;                    // NO set_selection_and_emit, NO DragMode::Cell
   }
   ```
   Shift-click is excluded (range-extend selection semantics stay). A single click on a merge-covered
   cell inserts the **anchor** ref (DPM.6); a drag expands (step 8).

8. **`point_drag` move — `update_point_drag` + `set_point_target_from_cell`.** In `handle_mouse_move`,
   **before** the `incell_open` early-return guard (so a point-drag driven by an in-cell formula edit
   still extends), add:
   ```rust
   if self.point_drag.is_some() {
       self.update_point_drag(local_x, local_y, window, cx);
       return;
   }
   ```
   - `update_point_drag`: map the pointer to a cell via `layout::cell_at_point` (as `update_fill_drag`
     does), call `set_point_target_from_cell(cell, window, cx)`, then `maybe_start_autoscroll`, `notify`.
   - `set_point_target_from_cell(cell, window, cx)`: `expanded = expand_range_for_merges(CellRange::new(origin, cell))`;
     if `expanded != last_range` → set `last_range = expanded` and emit
     `InsertReference { a1: expanded.to_a1(), replace_pending: true }` (**always replace during a drag** —
     the grid's own prior insert is the pending ref, `architecture.md §10` mid-drag correctness; never
     depends on the pushed `pending_ref` catching up). Dedupe (no emit when unchanged) means a release
     on the origin cell keeps `single(origin)` → a single-cell ref.

9. **`point_drag` up.** In `handle_mouse_up`, before the `self.drag.take()` block:
   ```rust
   if self.point_drag.take().is_some() {
       self.autoscroll_epoch = self.autoscroll_epoch.wrapping_add(1); // stop the loop
       cx.notify();
       return;
   }
   ```
   The last emitted `InsertReference` already left the correct text.

10. **Guards — extend the drag-active gates with `point_drag`.**
    - `handle_mouse_down` early-return (:1305): `|| self.point_drag.is_some()`.
    - `maybe_start_autoscroll` (:2509) + its loop's re-check (:2527) + `autoscroll_tick` (:2546):
      add `&& self.point_drag.is_none()` / `|| self.point_drag.is_some()` alongside `fill_drag`.
    - `autoscroll_tick`'s drag re-extend (:2615 `if let Some(drag)…else if fill_drag…`): add an
      `else if self.point_drag.is_some() { self.set_point_target_from_cell(cell, window, cx); changed = true; }`
      arm so a point-drag auto-scrolls + re-extends like a fill drag.

11. **Overlay — point-drag preview.** In the overlay pass, after the ref highlights and beside the
    fill-drag preview: when `point_drag` is set, draw its `last_range` (clipped to the frame like the
    selection overlay) as a **2px dashed** border in a distinct `POINT_PREVIEW_BORDER` constant
    (visually distinct from the solid-blue selection rectangle and the colored highlights — Excel's
    marching-ants marquee). No fill, no handles (DPM.7). Add the constant beside
    `REF_HIGHLIGHT_FILL_ALPHA`.

## Tests

**Chrome gpui view tests (`chrome/view.rs`):**
- `insert_reference_appends_at_caret`: `begin_typed("=")` → `insert_reference("C3", false)` → text
  `=C3`, caret after `C3`, `pending_ref == Some(1..3)`.
- `insert_reference_replaces_pending`: `=` → insert `A1` (pending) → insert `B2` (replace) → `=B2`
  (not `=A1B2`), pending now the `B2` span.
- `keystroke_after_point_appends_next`: `=` → insert `B2` (pending) → simulate a keystroke
  (`content_input` → `=B2+`, caret 4, `recompute_formula_edit_state`) → pending cleared → insert
  `C3` (append) → `=B2+C3`.
- `pending_cleared_by_caret_move`: after a point insert, a caret move + recompute clears `pending_ref`.
- `own_insert_keeps_pending`: `insert_reference` does not clear the span it just set (asserted via
  the append/replace tests + a direct pending-still-Some check).
- `self_reference_allowed`: editing A1 in the data row, `insert_reference("A1", false)` → `=A1`
  (not blocked; DPM.5).
- `autocomplete_then_point_happy_path`: `begin_typed("=sum")` → `autocomplete_accept` → `=SUM(`,
  `edit.reference_ready()` true → `insert_reference("C3", false)` → `=SUM(C3` (`functional_spec.md §4`).

**Grid gpui view tests (`grid/view.rs`):**
- `point_ready_click_inserts_not_selects`: `set_edit_state(reference_ready=true …)`, then
  `mouse_down_cell(2,2)` emits `GridEvent::InsertReference{ a1:"C3", replace_pending:false }`, emits
  **no** `SelectionChanged`, and the selection is unchanged; a `pending_ref=true` push makes the emit
  carry `replace_pending:true`.
- `not_ready_click_selects_as_today`: with `reference_ready=false, pending_ref=false`, `mouse_down_cell`
  emits `SelectionChanged` (commit path) and arms a cell drag as before (no `InsertReference`).
- `point_drag_emits_expanded_range_and_dedupes`: reference-ready, `mouse_down_cell(2,2)` (origin C3,
  emits `C3`), `set_point_target_from_cell((6,4))` → emits `InsertReference{ "C3:E7", replace_pending:true }`,
  re-calling with the same cell emits nothing (dedupe), returning to origin emits `C3` (release-on-origin
  → single).
- `point_click_on_merge_inserts_anchor`: a cache with merge `B2:C3`; a reference-ready click on the
  covered cell `C3` emits `InsertReference{ "B2", … }` (the anchor).
- `resolve_merge_anchor_in` / `expand_range_for_merges_in` pure unit tests: anchor of a covered cell;
  a swept rect touching a merge unions to the whole span (incl. a chained fixed-point expansion).
- `set_active_sheet_clears_point_drag`: an armed `point_drag` is dropped on a sheet switch.

## Checks (run cargo from `app/`)

- `cargo build -p freecell-app`
- `cargo test -p freecell-app --lib`
- `cargo fmt --all --check`
- Render **subset** while iterating: `app/render-tests/scripts/render_tests.sh test formula_ref` —
  no `formula_ref` baseline exists yet (baseline creation + the full suite are Phase 5), so this
  only confirms no unexpected diff on existing grid cases; the point-preview baseline is generated +
  eyeballed in Phase 5.

## Notes

- **Mid-drag correctness is grid-owned (`architecture.md §10`).** Every drag insert after the first
  is `replace_pending: true` locally, so the drag never waits on the deferred `EditState` round-trip
  to catch `pending_ref` up.
- **In-cell point-mode.** The point-drag move branch sits **before** `handle_mouse_move`'s
  `incell_open` early-return, so a point-drag armed during an in-cell formula edit extends normally
  (the in-cell overlay occludes only its own cell; clicks on other cells reach the grid).
