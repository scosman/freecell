---
status: complete
---

# Phase 2: Header-menu Freeze/Unfreeze entry

## Overview

Add the single **Freeze / Unfreeze** item to the existing header context menu
(`functional_spec.md §1`, `architecture.md §4`). Right-clicking a row-number or
column-letter header already opens `HeaderMenu` (Insert / Delete / Hide / Unhide); this
phase appends one Freeze/Unfreeze slot that reads the axis's current frozen count from the
cache and emits the `GridEvent::SetFrozen` wired in Phase 1. FreeCell-only, no fork, no
pixel change (the menu overlay is not a render-suite baseline surface).

The boundary track is the run's last index `b = run.1`, implied count `b + 1`. If the
axis's current frozen count already equals `b + 1` the item is **Unfreeze** (sets that axis
to `0`); otherwise **Freeze** (sets that axis to `b + 1`, moving the boundary if a different
freeze existed). A row header drives `SetFrozen { rows: Some(_), cols: None }`; a column
header drives `{ rows: None, cols: Some(_) }`. Always enabled (freeze hides nothing).

## Steps

1. **`grid/view.rs` — `HeaderMenu` gains `frozen: u32`.** Add the field (doc: current
   frozen count on the menu's axis — `M` for a row header, `K` for a column header) after
   the existing hide/unhide fields.

2. **`grid/view.rs` `handle_right_mouse_down` — read the frozen counts under the one lock.**
   Extend the cache-read tuple to also carry `cache.frozen_rows()` and `cache.frozen_cols()`
   (both `u32`, alongside `hidden_rows`/`hidden_cols`/`dims`). At `HeaderMenu` construction
   (`:2203`) set `frozen` = the axis-appropriate count (`Row => frozen_rows`,
   `Col => frozen_cols`).

3. **`grid/view.rs` `header_menu_items` (pure) — append the Freeze/Unfreeze tuple.** After
   the Hide/Unhide items, with `b = menu.run.1` and `count = b + 1`:
   - unit word already computed (`"row"` / `"column"`); label uses the plural form
     (`"Freeze rows"` / `"Freeze columns"`, `"Unfreeze rows"` / `"Unfreeze columns"`).
   - if `menu.frozen == b + 1` → label `"Unfreeze {unit}s"`, event sets the axis to `0`;
     else → label `"Freeze {unit}s"`, event sets the axis to `b + 1`.
   - the axis-specific `SetFrozen` constructor: `Row => SetFrozen { rows: Some(target),
     cols: None }`, `Col => SetFrozen { rows: None, cols: Some(target) }`.
   - always enabled (`true`).

4. **Tests fixture — `header_menu_fixture` gains a `frozen` param.** Thread `frozen` through
   the four existing Hide/Unhide callers (pass `0`).

## Tests

- **Pure `header_menu_items_include_freeze_and_unfreeze`** — the new last item:
  - row header, run `(0,0)`, frozen `0` → `"Freeze rows"`, enabled, `SetFrozen { rows:
    Some(1), cols: None }`.
  - row header, run `(0,0)`, frozen `1` → `"Unfreeze rows"`, `SetFrozen { rows: Some(0), .. }`.
  - row header, run `(2,4)` (boundary 4), frozen `3` → `"Freeze rows"`, `SetFrozen { rows:
    Some(5), .. }` (moves the boundary to a different track).
  - row header, run `(2,4)`, frozen `5` → `"Unfreeze rows"`, `SetFrozen { rows: Some(0), .. }`.
  - column header symmetry: run `(0,0)` frozen `0` → `"Freeze columns"`, `SetFrozen { rows:
    None, cols: Some(1) }`; run `(1,3)` frozen `4` → `"Unfreeze columns"`, `cols: Some(0)`.
- **gpui `header_menu_carries_axis_frozen_count`** — with sources seeded `frozen_rows = 2`,
  `frozen_cols = 1`, a right-click on a row header populates `menu.frozen == 2`; on a column
  header `menu.frozen == 1`.
- **gpui `freeze_menu_item_emits_set_frozen`** — emitting the Freeze item on a fresh row
  header records `GridEvent::SetFrozen { rows: Some(_), cols: None }` (one `Some` axis).
