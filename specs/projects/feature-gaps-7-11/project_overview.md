---
status: complete
---

# Feature Gaps 7_11

A batch of feature gaps that aren't particularly large individually, but that I
want to **plan in a batch, then execute in a batch**. The reason to combine them
into one project is to get planning done synchronously (with me in the loop on
decisions), then let coding happen asynchronously across the batch.

These are the gaps:

## Bugs / warnings

- **SVG font warnings printing during running.** Do our SVGs have these fonts?

  ```
  2026-07-12T00:48:50.505045Z  WARN gpui::svg_renderer: Failed to load bundled font fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf: could not find asset at path "fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"
  2026-07-12T00:48:50.505119Z  WARN gpui::svg_renderer: Failed to load bundled font fonts/lilex/Lilex-Regular.ttf: could not find asset at path "fonts/lilex/Lilex-Regular.ttf"
  ```

## Features

- **Drag to re-order sheets.** Allow dragging sheet tabs to re-order sheets.

- **Right-click col/row headers to add/remove cols/rows.** Context menu on
  row/column headers to insert/delete rows and columns.

- **Text overflow / spill.** Cell content should flow over top of the next cell
  if it's too long and the next cell is empty. Should continue until it ends or
  hits a non-empty cell. This is **only when text wrap is off** (horizontal
  overlap, not vertical overlap). If it spans many cells, it should stop at the
  first non-empty cell (never starting up again later).

- **Auto-grow cells** with large text or wrapped content (unless they have a
  manual height set).

- **Find / replace.**
  - A new bar that appears under the formula bar.
  - Can be dismissed — an "X" on the right or similar.
  - Standard find/replace functionality.
  - Opens via Cmd-F or a button in the action bar (search icon?).

- **Quick edit mode (UX improvement).** When I enter cell text via typing while
  focused on a cell, I should be in a "quick edit" mode, and still be able to use
  keyboard nav. Arrow keys when in quick edit should navigate between cells:
  e.g. "[focus cell]abcd[RIGHT]" adds "abcd" to the current cell, then moves
  focus right to the next cell. This is **only** for "quick" mode triggered by
  typing when a cell is focused — **not** when I entered edit mode by
  double-clicking the cell directly (arrows control the cursor then), or set
  focus to the formula bar (arrows for cursor as well), or manually placed the
  cursor after I start quick edit. In those cases, arrow keys remain tied to
  cursor position. But if I just type and arrow, it should move cell focus.

- **Command-click (and Windows/Linux equivalent) to select non-adjacent
  cells.** Nice for formatting: select 8 non-adjacent cells and make bold or
  delete, etc.

- **Freeze panes.** Should support right-clicking a row/column header and saying
  "freeze" to freeze this and all left/above. If the selected row/col is already
  the target frozen row/col, make it say "unfreeze".

## Scope decisions (2026-07-12)

Made during planning after mapping the codebase:

- **Right-click headers → add/remove rows/cols is already built** (`header_menu_elements`,
  shipped in `mvp-gaps` Phase 7). This batch **verifies** it rather than rebuilding it —
  no new code beyond a smoke check.
- **Command-click non-adjacent selection is DEFERRED** to the backlog. It's a core
  `SelectionModel` refactor (single rect → multiple areas) that ripples across render,
  motion, clipboard, and formatting — too large for this "small gaps" batch.
  → [`projects/disjoint-selection.md`](../../../projects/disjoint-selection.md)
- **Freeze panes is DEFERRED** to the backlog. IronCalc already models it (no fork change),
  but it needs split-viewport rendering + scroll clamping in the custom grid — the single
  biggest UI task, better as its own project.
  → [`projects/freeze-panes.md`](../../../projects/freeze-panes.md)
- **Sheet reorder includes an IronCalc fork change**: add an undoable, xlsx-order-preserving
  `move_sheet` / `set_worksheet_index` to `scosman/ironcalc` (upstreamed per CLAUDE.md), then
  wire a `Command::MoveSheet` + tab drag.
- **Font warning** is benign gpui-core noise (Zed's hardcoded bundled-font paths, not our
  SVGs — our icons contain no `<text>`); fix by silencing the `gpui::svg_renderer` log target.
- **Find/replace** scope: standard find/replace (find next/prev, replace, replace-all,
  match-case + match-entire-cell toggles), scoped to the **current sheet**.

**In this batch:** font-warning fix · text spill/overflow · auto-grow cells · find/replace ·
quick-edit mode · sheet reorder (drag + fork API) · verify right-click add/remove rows/cols.
