---
status: complete
---

# Phase 4: Merge/Unmerge control + data-loss dialog

## Overview

Make the merged-cell engine plumbing (P1) and merge-aware rendering/selection (P2–P3)
*controllable* from the app: an action-row toggle + Edit-menu item + ⌃⌘M shortcut that
merges or unmerges the current selection, and the data-loss confirm dialog that routes the
`MergeCells{confirmed}` / `MergeNeedsConfirm` round-trip. Also re-word the fill-into-merge
rejection copy. This is a **chrome** phase — out of scope for the pixel render suite (no
action-row / dialog baselines); validated with gpui view/chrome tests + an Xvfb smoke launch.

## Steps

1. **Vendor the icon** (`assets/icons/table-cells-merge.svg`). The gpui-component Lucide
   bundle does **not** ship `table-cells-merge` (99-icon subset, verified), so vendor the
   real Lucide glyph in the same tintable `stroke="currentColor"` form, and register it in
   `shell/assets.rs` `FREECELL_ICONS`. Add an assets test that it resolves.

2. **`ChromeClient::sheet_merges`** (`chrome/client.rs`). Add
   `fn sheet_merges(&self, sheet: SheetId) -> Vec<CellRange>;` so the chrome can read the
   active sheet's regions live (merge counts tiny → cloning the small Vec is cheap, and
   reading live keeps the toggle correct after a merge/unmerge without a selection change).
   Implement on `DocumentClient` (cache `merges().to_vec()`) and on `RecordingClient`
   (injected map + `set_merges`).

3. **Chrome logic** (`chrome/view.rs`): import `effective_range`, `region_at`,
   `regions_intersecting`. Add:
   - `active_sheet_merges(&self) -> Vec<CellRange>` — `self.client.sheet_merges(active)`.
   - `merge_active(&self) -> bool` — `!regions_intersecting(merges, effective_range(sel)).is_empty()`.
   - `merge_disabled(&self) -> bool` — `self.degraded || (sel.is_single() && region_at(active).is_none())`.
   - `toggle_merge(&mut self, window, cx)` — commit pending edit; `range = effective_range`;
     `hit = regions_intersecting`; if `!hit.is_empty()` → send `UnmergeCells{anchor}` per
     region; else if range not 1×1 → send `MergeCells{area:range, confirmed:false}`.
     Early-return on `self.degraded` (backstop).

4. **Action-row button** (`chrome/view.rs` `render_action_row`): after the wrap-text
   toggle, add `action_divider()` then a bespoke `Button` (not the shared `toggle` closure,
   which drives character styles): icon `icons/table-cells-merge.svg`,
   `.selected(merge_active())`, `.disabled(merge_disabled())`, tooltip "Unmerge cells" when
   active / "Merge cells" when inactive, `on_click → toggle_merge`.

5. **Action + menu + shortcut**:
   - `shell/mod.rs`: add `ToggleMerge` to the `actions!` list.
   - `shell/menus.rs`: import `ToggleMerge`; bind `key("ctrl-m")` next to `ToggleBold`
     (⌘+ctrl+M on macOS = ⌃⌘M; ctrl+M on Linux — duplicate ctrl is idempotent); add an
     Edit-menu `MenuItem::action("Merge Cells", ToggleMerge)` after Find.
   - `shell/window.rs`: register `on_action(&ToggleMerge)` → `chrome.toggle_merge`.

6. **Confirm dialog** (`shell/window.rs`):
   - New `ActiveModal::Confirm { sheet: SheetId, area: CellRange }`.
   - Replace the log-only `WorkerEvent::MergeNeedsConfirm` arm with opening the Confirm
     modal (gated on `self.modal.is_none()`).
   - `render_modal`: a two-button `dialog_card` — title "Merge cells?", body "Merging keeps
     only the upper-left value and discards the other values in the selection.", buttons
     Cancel (ghost) · **Merge** (primary, rightmost per the unsaved-changes pattern).
   - `confirm_merge(sheet, area, …)` — clear modal + re-send `MergeCells{confirmed:true}`.
   - Test seams `has_confirm_modal()` + `confirm_modal_target()`.

7. **Fill-rejection copy** (`shell/window.rs` `on_edit_rejected`): `MergedCells` arm → title
   "Can't fill merged cells", body "Fill (⌘D / ⌘R) can't write into a merged region."

## Tests

- **assets** (`shell/assets.rs`): `table-cells-merge` resolves through the combined source.
- **chrome** (`chrome/view.rs`):
  - `merge_toggle_inactive_on_mergeable_multicell` — multi-cell, no merge → `!merge_active()`,
    `!merge_disabled()`.
  - `merge_toggle_active_when_selection_contains_a_merge` — selection over a region →
    `merge_active()`, `!merge_disabled()`.
  - `merge_toggle_disabled_on_lone_single_cell` / `..._when_degraded`.
  - `toggle_merge_merges_a_plain_multicell_selection` → one `MergeCells{confirmed:false}`.
  - `toggle_merge_unmerges_when_selection_contains_regions` → `UnmergeCells` per region.
  - `toggle_merge_noop_on_lone_single_cell` → no command.
- **menus** (`shell/menus.rs`): Edit menu has "Merge Cells" directly after "Find…".
- **window/app** (`shell/app.rs`):
  - `merge_needs_confirm_opens_confirm_modal` — inject `MergeNeedsConfirm` → `has_confirm_modal`
    + `confirm_modal_target` carries (sheet, area).
  - `confirm_modal_merge_dismisses` / `confirm_modal_cancel_dismisses`.
  - `fill_into_merge_shows_reworded_error` — inject `EditRejected{MergedCells}` → error modal
    titled "Can't fill merged cells".
- **Xvfb smoke**: `timeout 120 xvfb-run -a cargo run -p freecell-app` boots without panic.
