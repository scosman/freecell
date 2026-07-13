---
status: complete
---

# Phase 6b: Sheet reorder wiring + tab drag (§6.2–6.3)

## Overview

Phase 6a landed the fork API `UserModel::set_worksheet_index(sheet_index, new_index)` and
re-pinned FreeCell against it. This phase wires that API through FreeCell so a user can
**drag a sheet tab to reorder it**:

- **Engine wiring** — a new `Command::MoveSheet { sheet, to_index }` routes through the
  single-writer worker to a `document.rs::move_sheet` wrapper. `move_sheet` maps the stable
  `SheetId` → its current worksheet index and calls the fork's index-based
  `set_worksheet_index(current_index, to_index)`. The reorder rides the fork's undo and is
  xlsx-order-preserving. Because tab order is derived from engine order, the reorder **must**
  round-trip the worker: republishing `WorkerEvent::SheetsChanged` (already driven by the
  worker's before/after `sheet_metas()` comparison) rebuilds the tabs in the new order. The
  UI never locally reorders `self.sheets`.
- **Tab drag (chrome)** — press-and-drag a tab past a ~4 px threshold enters a drag; a 2 px
  accent drop indicator shows the insertion gap; the dragged tab lifts (stronger bg / 1 px
  accent outline / 90 % opacity); release sends `MoveSheet` (or nothing on a drop-to-origin).
  Click-select and double-click-rename and right-click-menu are preserved.
- The **insertion-index computation** (cursor x + tab centers → target slot) is extracted as
  pure free functions so it is unit-testable without a `Window`.

`functional_spec.md §6.1–6.2 & §6.4`, `architecture.md §6.2–6.3`, `ui_design.md §3` (+ §6
cursor). Tab bar is **not** baselined by the pixel suite → validate with gpui view/unit tests
+ one Xvfb smoke launch; **no pixel render run**.

## Steps

### Engine wiring

1. **`worker/protocol.rs`** — add, right after `DeleteSheet` (~`:302`):
   ```rust
   /// Move the sheet with stable id `sheet` so it lands at 0-based worksheet index `to_index`,
   /// shifting the intervening sheets (`functional_spec.md §6`). Undoable (rides the fork's
   /// history); the new order is preserved on xlsx save. The worker maps `sheet` → its current
   /// worksheet index before calling the fork's index-based reorder API.
   MoveSheet { sheet: SheetId, to_index: u32 },
   ```

2. **`document.rs`** — add a `move_sheet` wrapper right after `delete_sheet` (~`:1003`):
   ```rust
   /// Moves the sheet at `sheet_idx` to `to_index` (`MoveSheet`), shifting the intervening
   /// sheets. Undoable (rides the fork's history); the new order is preserved on xlsx save;
   /// cross-sheet references stay valid (order is a vector position, not an identity). Wraps
   /// the fork's `UserModel::set_worksheet_index`.
   pub(crate) fn move_sheet(&mut self, sheet_idx: u32, to_index: u32) -> Result<(), String> {
       crate::instrument::record_engine_call();
       self.model.set_worksheet_index(sheet_idx, to_index)
   }
   ```

3. **`worker/run.rs`** — three edits:
   - **Classify** (exhaustive routing ~`:426`): add `| Command::MoveSheet { .. }` to the
     `edits.push(edit)` sheet-op group (alongside Add/Rename/Delete). (Required to compile —
     the routing match has no catch-all.)
   - **Dispatch** in `apply_one` right after the `DeleteSheet` arm (~`:2506`):
     ```rust
     Command::MoveSheet { sheet, to_index } => {
         let idx = resolve_idx(doc, *sheet)?;
         doc.move_sheet(idx, *to_index)?;
         Ok(AppliedKind::SheetOp)
     }
     ```
   - **`op_of`** (~`:2657`): add `Command::MoveSheet { .. }` to the `AppliedOp::Sheets` arm
     (required — the `_` arm is `unreachable!`, and a MoveSheet reaches `op_of`).
   - No `pre_validate` arm needed: MoveSheet falls to `_ => Ok(())`; the fork validates bounds
     and no-ops a same-index move. (The UI never sends a no-op move — see step 8 — so the
     fork-history / FreeCell `undo_touches` stacks stay 1:1; a same-index move that the fork
     skips must not push a FreeCell touch, and it won't be sent.)

   `SheetsChanged` republish is automatic: `process_batch` compares `sheet_metas()` before/
   after the batch and emits `SheetsChanged` on any order change (including undo/redo of a
   move). No new emit code.

### Tab drag (chrome/view.rs)

4. **Pure helpers** (gpui-free free functions near the top of `chrome/view.rs`):
   ```rust
   /// The insertion gap a tab drop lands in: the count of tab centers at/left of `cursor_x`
   /// (`tab_centers` ordered left→right, same coordinate space as `cursor_x`). Result in
   /// `0..=n` — the drop indicator gap, clamped so a drop can't pass the trailing `+` button.
   fn tab_insertion_index(cursor_x: f32, tab_centers: &[f32]) -> usize
   /// Convert an insertion gap to the fork's final `to_index` for a sheet currently at
   /// `from_slot`, or `None` when the drop is a no-op (lands back on the origin slot). Removing
   /// the dragged tab shifts later gaps left by one: `to = if gap <= from { gap } else { gap-1 }`;
   /// `to == from` ⇒ no-op.
   fn move_target_for_gap(gap: usize, from_slot: usize) -> Option<usize>
   ```

5. **State** — add a `TabDrag` struct (modeled off `ResizeDrag`) + fields on `ChromeView`
   (near the other tab state ~`:376`):
   ```rust
   struct TabDrag { sheet: SheetId, start_x: f32, cur_x: f32, dragging: bool }
   tab_drag: Option<TabDrag>,
   /// Each tab's window-space horizontal span, captured by a per-tab `canvas` probe during
   /// paint — the Window-free geometry the pure insertion computation reads. Keyed by SheetId
   /// (read back in `self.sheets` order), so a partial/stale capture is simply ignored.
   tab_spans: Vec<TabSpan>,   // TabSpan { sheet, left, right }
   ```
   Initialize both in `new`; prune stale `tab_spans` in `merge_sheet_metas` / `set_sheets`.

6. **Drag methods** (plain methods, unit-testable by driving them + setting `tab_spans`):
   - `tab_press(&mut self, sheet, x)` — record a *potential* drag (`dragging = false`).
   - `tab_drag_move(&mut self, x, cx)` — update `cur_x`; flip `dragging` once
     `|x - start_x| > TAB_DRAG_THRESHOLD_PX` (4.0); `cx.notify()` while dragging.
   - `tab_drag_end(&mut self, x, cx)` — take the drag; if it was a real drag, send
     `MoveSheet` when `tab_move_target(sheet, x)` is `Some(to)`; else nothing.
   - `tab_move_target(&self, sheet, cursor_x) -> Option<u32>` — returns `None` unless every tab
     has a captured span (`tab_spans.len() == sheets.len()`); builds ordered centers, finds the
     dragged sheet's `from_slot`, then `move_target_for_gap`.
   - `tab_drop_indicator_x(&self) -> Option<f32>` — window-x of the 2 px bar for the live gap
     (midpoint of the neighboring tab edges; outer edges ± half the tab gap).

7. **`render_tab`** (~`:3444`): add `on_mouse_down(Left) → tab_press(id, event.position.x)`
   (no `stop_propagation`, so the existing `on_click` select/double-click-rename still forms).
   When `tab_drag` is a live drag on this tab, **lift** it: stronger bg (`ACTIVE_TAB_BG`), 1 px
   accent border, `.opacity(0.9)`. Preserve the existing `on_click` (select / rename) and
   `on_mouse_down(Right)` (menu) untouched — a left-drag never reaches them (rename needs
   `click_count ≥ 2`; a real drag releases over a *different* tab so gpui's hover-gated click
   never fires on the origin tab).

8. **`render_tab_bar`** (~`:3415`): make the container `.relative()`; attach the drag's
   move/up handlers here (the container spans the full strip, so moves/ups fire even as the
   pointer crosses tabs — a per-tab `on_mouse_move` only fires while *that* tab is hovered):
   - `on_mouse_move → tab_drag_move(event.position.x, cx)`
   - `on_mouse_up(Left) → tab_drag_end(event.position.x, cx)`
   - `.cursor(CursorStyle::ClosedHand)` (CSS `grabbing`) while a drag is live.
   - Append the **drop indicator** child (2 px accent bar, `absolute`, full height) at
     `tab_drop_indicator_x()` when dragging.
   - Each tab embeds a zero-cost `canvas` probe (like `anchored_trigger`) that upserts its
     window-space span into `tab_spans` (no `notify` — read on the next mouse event).

9. **Constants** near the tab constants: `TAB_DRAG_THRESHOLD_PX: f32 = 4.0`, a
   `TAB_DROP_ACCENT: u32` (Office Accent 1, `0x4472C4`, matching the existing selected-swatch
   ring), and `TAB_GAP_HALF` for the indicator outer offset.

Active-sheet follows the `SheetId` for free: on `SheetsChanged` from a reorder,
`reconcile_sheets` (window.rs) sees the same id set (no add/delete), so the active sheet is
unchanged — only the tab order changes.

## Tests

- **Worker** (`worker/run.rs` tests): `move_sheet_reorders_metas_and_undo_restores` — add two
  sheets (3 total), `MoveSheet` the first to index 2, assert `sheet_metas()` order became
  `[s1, s2, s0]` and a `SheetsChanged` was emitted; `Undo` restores `[s0, s1, s2]` and
  re-emits `SheetsChanged`.
- **Pure fns** (unit, no Window): `tab_insertion_index` across cursor positions
  (before-all → 0, between i-1/i → i, after-all → n); `move_target_for_gap`
  (gap==from & gap==from+1 → None; leftward and rightward moves map to the expected final
  index).
- **gpui view** (drive the drag state, set `tab_spans` directly since the unit harness does
  not paint): `tab_drag_below_threshold_is_no_command` (small move → no `MoveSheet`);
  `tab_drag_reorders_sends_move` (press slot 0, move past threshold to slot 2's region, end →
  `MoveSheet { sheet: s0, to_index: 1 }`); `tab_drag_to_origin_sends_nothing` (drop back on
  origin → no command); `tab_drag_sets_indicator` (dragging flips + `tab_drop_indicator_x` is
  `Some`).

## Validation

- Project checks from `app/`: `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` (ignore
  the 2 known `charts_roundtrip_libreoffice` LibreOffice failures).
- One Xvfb smoke launch (`xvfb-run -a cargo run -p freecell-app`) — confirms build/launch, no
  panic. Drag is not drivable headlessly.
- **No pixel render suite** (tab bar is not baselined).
