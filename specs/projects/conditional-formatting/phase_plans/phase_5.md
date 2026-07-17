---
status: draft
---

# Phase 5: Rules list (List mode)

## Overview

P4 shipped the CF sidebar shell: the action-bar `split` button, `ChromeView.cond_fmt`
open/close/toggle/refresh/rescope, `CondFmtPanel { sheet, rows: Vec<CfRuleView> }` already
populated from `DocumentClient::cond_fmt_rules` (refreshed on `CondFmtUpdated` + sheet switch),
and a minimal List-mode body (intro line + a no-op "+ Add rule" button, **no rows**).

Phase 5 renders those rows and wires their management controls. Each `CfRuleView` becomes one
row — preview swatch + summary/range + reorder/edit/delete controls — and the reorder/delete
controls send the P2 `Command` CF variants through `self.client`. The list refreshes itself via
the already-wired `CondFmtUpdated` → `refresh_cond_fmt` path (verified: `shell/window.rs:510`).
The rule **editor** (opened by "+ Add rule" and the row ✎) is Phase 6; P5 keeps its click a
`open_cf_editor` stub. No engine, protocol, or grid changes — this is app-side render + wiring
only. The CF sidebar has **no pixel baseline**, so it is validated with gpui view tests only (no
pixel suite, per CLAUDE.md render scope).

## Steps

1. **Vendor two icons** (`components/cf_sidebar.md §5`, CLAUDE.md icon convention). The
   gpui-component bundle already ships `chevron-up`/`chevron-down` (used by the find bar) but
   **not** `pencil` or `trash-2` (bundle has only `delete.svg`, a backspace glyph). Add, in the
   tintable `stroke="currentColor"` Lucide form used by the other vendored icons:
   - `app/crates/freecell-app/assets/icons/pencil.svg`
   - `app/crates/freecell-app/assets/icons/trash-2.svg`

   Register both in `shell/assets.rs::FREECELL_ICONS`. The existing
   `vendored_icons_load_and_are_tintable` test then enforces tintability; add a
   `cf_sidebar_icons_all_resolve` test asserting `pencil`/`trash-2`/`chevron-up`/`chevron-down`
   all resolve through `AppAssets`.

2. **Row-control enablement helper** (`chrome/view.rs`, module-private, near `render_cf_row`):
   ```rust
   struct CfRowControls { move_up: bool, move_down: bool, edit: bool, delete: bool }
   fn cf_row_controls(row: &CfRuleView, is_first: bool, is_last: bool) -> CfRowControls
   ```
   `move_up = !is_first`, `move_down = !is_last`, `edit = row.editable` (deferred-family/Badge
   rows are non-editable), `delete = true`. `render_cf_row` derives its `.disabled(...)` flags
   from this, so the row test asserts the enablement logic directly (the module's "every action
   is a plain method, unit-testable without pixel clicks" convention).

3. **Preview swatch** (`chrome/view.rs`, module-private free fn):
   `fn render_cf_preview(preview: &CfPreview) -> AnyElement` + a `fn cf_color(Rgb) -> Rgba`
   (`rgb(c.to_hex())`) helper. Small consts `CF_SWATCH_W`/`CF_SWATCH_H`/`CF_BADGE_BG`.
   - `Highlight { fill, text_color }` → a `CF_SWATCH_W×CF_SWATCH_H` hairline-bordered chip filled
     with `fill` (white when `None`) carrying an "A" glyph in `text_color` (`TEXT` when `None`) —
     so fill **and** text colour read.
   - `ColorScale { colors }` → the same-size hairline chip holding one equal-width `flex_1` colour
     band per stop colour (a stepped horizontal gradient).
   - `Badge(label)` → a muted grey tag (`CF_BADGE_BG`, hairline border, `MUTED_TEXT`) with the label.

4. **`render_cf_row`** (`chrome/view.rs`, `&self`, `AnyElement`):
   `fn render_cf_row(&self, row: &CfRuleView, is_first: bool, is_last: bool, cx: &mut Context<Self>)`.
   A `flex items_center gap_2` row (debug-selector `cf-row-{index}`) with:
   - `render_cf_preview(&row.preview)` (left, `flex_shrink_0`);
   - a two-line `flex_1 min_w_0` summary block: `row.summary` (`TEXT`) over `row.range` (`MUTED_TEXT`);
   - a `flex_shrink_0` controls cluster of four ghost/small icon `Button`s (debug-selectors
     `cf-row-{index}-{up,down,edit,delete}`):
     - **up** `icons/chevron-up.svg`, `disabled(!controls.move_up)` → `raise_cf_rule(index)`
     - **down** `icons/chevron-down.svg`, `disabled(!controls.move_down)` → `lower_cf_rule(index)`
     - **edit** `icons/pencil.svg`, `disabled(!controls.edit)` → `open_cf_editor(Some(index), cx)`
     - **delete** `icons/trash-2.svg` (always enabled) → `delete_cf_rule(index)`

   `index = row.index` is the **stable storage index** the index-based mutators take (not the
   display position); `is_first`/`is_last` are the display-position ends.

5. **List body in `render_cond_fmt_sidebar`** (`chrome/view.rs`): keep the intro line + the
   "+ Add rule" button (rewire its click to the `open_cf_editor(None, cx)` stub). Between them,
   render:
   - **empty** (`panel.rows.is_empty()`): a muted "No rules on this sheet yet." (debug-selector
     `cf-empty`), above the Add-rule button.
   - **non-empty**: a `flex_col gap_1` list, one `render_cf_row` per rule in the (already
     priority-descending) order, `i == 0` ⇒ `is_first`, `i + 1 == len` ⇒ `is_last`.

6. **Wiring methods on `ChromeView`** (`chrome/view.rs`, near the other CF methods):
   ```rust
   pub fn raise_cf_rule(&mut self, index: u32)   // → Command::RaiseCondFmtPriority { sheet, index }
   pub fn lower_cf_rule(&mut self, index: u32)   // → Command::LowerCondFmtPriority { sheet, index }
   pub fn delete_cf_rule(&mut self, index: u32)  // → Command::DeleteCondFmt { sheet, index }
   ```
   Each reads the panel's `sheet` via `cond_fmt_sheet()` and `self.client.send(...)`; a **no-op
   when the sidebar is closed** (no panel ⇒ no sheet). Fire-and-forget: the worker republishes and
   the list refreshes via `CondFmtUpdated` → `refresh_cond_fmt` (no optimistic mutation here). Plus
   a P6 stub:
   ```rust
   fn open_cf_editor(&mut self, _edit_index: Option<u32>, _cx: &mut Context<Self>) { /* TODO(P6) */ }
   ```

## Tests

`chrome/view.rs` (gpui view tests, `RecordingClient` double) unless noted:

- `cf_row_controls_reflect_position_and_editability` (pure): first row → `!move_up`; last row →
  `!move_down`; middle → both; a non-editable Badge row → `!edit` but `delete`.
- `cf_list_renders_one_row_per_rule` (VisualTestContext): three rules (distinct indices + a
  Highlight, a ColorScale, a Badge) → `cf-row-{index}` painted for each; `cf-empty` absent.
- `cf_empty_state_shown_when_no_rules` (VisualTestContext): open on a sheet with no rules →
  `cf-empty` painted; no `cf-row-*`.
- `cf_delete_sends_delete_command`: `delete_cf_rule(i)` → one `DeleteCondFmt { sheet, index: i }`.
- `cf_move_up_sends_raise` / `cf_move_down_sends_lower`: `raise_cf_rule`/`lower_cf_rule` → the
  matching `Raise`/`LowerCondFmtPriority { sheet, index }`.
- `cf_commands_noop_when_sidebar_closed`: with the sidebar closed the three mutators send nothing.
- `cf_delete_button_click_sends_delete` (VisualTestContext + `simulate_click`): clicking a row's
  `cf-row-{index}-delete` sends `DeleteCondFmt` (binds the button → method).
- `cf_first_row_move_up_disabled` (VisualTestContext + `simulate_click`): clicking the first row's
  up sends nothing (disabled), clicking a lower row's up sends `RaiseCondFmtPriority` for its index.
- `cf_last_row_move_down_disabled` (VisualTestContext + `simulate_click`): mirror for down.
- `cf_list_reorders_after_cond_fmt_updated`: open with rules A,B; republish reordered B,A +
  `refresh_cond_fmt` → the rows' summary order flips.
- `shell/assets.rs::cf_sidebar_icons_all_resolve`: `pencil`/`trash-2`/`chevron-up`/`chevron-down`
  all resolve through `AppAssets`.
