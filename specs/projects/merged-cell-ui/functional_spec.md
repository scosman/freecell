---
status: complete
---

# Functional Spec: Merged Cell UI

This spec defines the **UI-layer behavior** of merged cells in FreeCell, built on the
merged-cell engine API now present on our IronCalc fork (`freecell-fixes`). The engine
owns the model semantics (create/remove, keep-anchor-value, undo/redo, structural
displacement, xlsx round-trip); this project makes those visible and controllable in
the app.

## Terminology

- **Merged region** (or **merge**) — a rectangle of cells that displays and behaves as
  one, addressed by its top-left cell.
- **Anchor** — the top-left cell of a region; the only cell that holds a value.
- **Covered cell** — a non-anchor cell inside a region; always empty of content, may
  keep its own style.

Semantics follow Excel: you cannot edit part of a merge, merging keeps only the
top-left value, and a region is addressed by its anchor.

## Engine API this builds on (context, not in scope to change)

```rust
UserModel::merge_cells(sheet, row, column, width, height) -> Result<(), String>
UserModel::unmerge_cells(sheet, row, column) -> Result<(), String>
UserModel::get_merge_cells(sheet) -> Result<Vec<MergeCell>, String>   // MergeCell { row, column, width, height }
UserModel::get_merge_cell(sheet, row, column) -> Result<Option<MergeCell>, String>
```

- `merge_cells` keeps the anchor value, **clears covered content** (recorded for undo),
  rejects out-of-bounds / degenerate (1×1) / overlap / array-spill collisions.
- `unmerge_cells` removes the region containing `(row,column)`; a cell not in any region
  is a no-op.
- Writing to a **covered** cell via `set_user_input` returns `Err` — the UI must
  redirect edits to the anchor.
- Structural edits (insert/delete rows & columns) **displace** merges (grow / shrink /
  drop on collapse; never split). A **move** that would split a region returns `Err`.
- Merges survive xlsx open→save (importer/exporter already handle `<mergeCells>`).

## Current FreeCell state (what this replaces)

Today FreeCell ships only an **interim guard** from `mvp-gaps`:

- File-loaded merges are parsed but **not rendered as a single box** — covered cells
  paint independently, interior gridlines show, no visual merge.
- Selection/editing is **merge-unaware** — a covered cell is a normal, separately
  selectable/editable cell.
- Insert/delete rows/cols and fill (⌘D/⌘R) that would touch a merge are **blocked**
  with an OK-only "Merged cells not supported" dialog
  (`EditRejectedReason::MergedCells`, `freecell_core::merge_guard`).

This project renders merges, makes selection/editing merge-aware, adds a
create/remove control, and retires the parts of the interim guard the engine now
handles.

## Features

### F1 — Render merged regions as one box

A merged region renders as a single rectangle spanning all its rows/columns:

- The **anchor's** content (text/number, formatted per its style) is drawn once,
  laid out across the **whole region rectangle**.
- **Covered cells are not painted** as separate cells: no separate content, and the
  **interior gridlines** between cells of the region are suppressed (the region reads as
  one cell). The region's outer gridlines/borders render normally.
- The region's **fill** is the anchor's fill; the anchor's borders apply to the region's
  outer edge (per-cell border styles as stored — unified-border polish is out of scope).
- **Alignment:** the anchor's own horizontal/vertical alignment governs how its content
  sits in the region box (we are not adding a "center on merge" behavior — see scope).
- Applies identically to file-loaded merges and merges created in-app; both come from the
  same live merge state.
- A region partly or wholly scrolled off-screen clips correctly (anchor content is
  clipped to the visible portion of the region, like any wide cell today).

### F2 — Merge / Unmerge toggle control

A single **"Merge cells" toggle** is the only merge control (no Merge & Center, Merge
Across, or no-center variants — see scope). It is reachable from:

- **Action-row toolbar button** — primary surface, mirroring existing formatting toggles
  (bold, borders, fill).
- **Menu-bar item + keyboard shortcut** — a **"Merge Cells" item in the Edit menu** (there
  is no Format menu today; formatting toggles live only as keybindings + toolbar buttons)
  and a keybinding **⌃⌘M** (Control+Command+M, matching Apple Numbers' merge shortcut;
  ⌘M alone is the system minimize), mirroring the `ToggleBold` action wiring in
  `shell/menus.rs`.

There is **no right-click context menu** entry (deferred — FreeCell has no context-menu
component today).

**Toggle semantics (matches Excel's Merge & Center toggle):**

| Current selection | Button state | Click action |
|---|---|---|
| Contains **any** merged region (fully or partially, incl. a single covered cell) | **Active/pressed** | **Unmerge** every region intersecting the selection |
| A multi-cell rectangle with **no** merged region inside it | Inactive | **Merge** the selection rectangle (data-loss warning per F3) |
| A single 1×1 cell not in any merge | **Disabled** | — (nothing to toggle) |

- The button shows a **pressed/active** style whenever the selection contains a merge (so
  it reads as "merge is on for this selection"), mirroring how bold/italic reflect state.
- After a merge, the selection becomes the new region (active cell = anchor, region
  highlighted as one box). After an unmerge, the selection is the same rectangle of
  now-independent cells.
- **Merge** creates one region over the exact selection rectangle. **Unmerge** removes
  all regions that intersect the selection (so unmerging a multi-merge selection clears
  all of them in one gesture); each removed region is one engine call.

### F3 — Data-loss warning on merge

When a **merge** would discard content (the selection rectangle contains **more than one
non-empty cell**, i.e. at least one covered cell has content that isn't the anchor's):

- Show an **OK / Cancel confirm dialog**: *"Merging cells keeps only the upper-left value
  and discards the rest. Merge anyway?"* (reusing the existing modal system).
- **OK** performs the merge (single undo step; discarded content restorable via ⌘Z).
- **Cancel** aborts; nothing changes.

No dialog when the selection has at most one non-empty cell (nothing is lost). **Unmerge
never warns** (it discards nothing).

### F4 — Selection is merge-aware

Selection treats a region as one unit (Excel semantics):

- **Click** on any cell of a region (anchor or covered) selects the **whole region**; the
  active cell is the anchor. The active-cell border spans the whole region.
- **Keyboard navigation** (arrows) moves by whole regions: an arrow that lands inside a
  region selects the region as a unit; an arrow leaving a region moves to the first cell
  **past** the region's far edge in that direction (never into a covered interior cell).
- **Range selection** (shift+click, shift+arrow, drag) **snaps to whole regions**: if a
  range's edge cuts through a region, the range expands to include the entire region.
  Expansion is a **fixpoint** — pulling in a region can extend an edge so it now cuts a
  *different* region, which is then also pulled in, until stable.
- The **data/formula row** and any "active cell address" UI show the **anchor's** address
  when a region is active.

### F5 — Editing routes to the anchor

- Starting an edit (double-click, F2, or typing) on any cell of a region edits the
  **anchor**. The in-cell editor is positioned over the region (anchor cell position;
  sizing to the region box is acceptable but not required for v1 — see UI design).
- A committed edit writes to the **anchor** via the normal edit path; the UI never issues
  a covered-cell write (which the engine would reject).
- Clearing content (Delete/Backspace) over a region clears the **anchor** (covered cells
  are already empty). Clearing a range that includes regions clears each region's anchor.

### F6 — Structural edits displace merges (retire the interim guard)

Because the engine now displaces merges across insert/delete rows & columns:

- **Insert/delete rows or columns near/through a region is no longer blocked** — the op
  proceeds and the engine grows / shrinks / drops the region; the grid re-renders the
  updated merge state. The insert/delete arm of the interim `merge_guard` and its
  "Merged cells not supported" dialog are **removed**.
- **Move rows/columns:** FreeCell has **no row/column move gesture** (only sheet-tab
  reorder), so the engine's region-splitting-move rejection is **unreachable** — N/A for
  this project. (If a row/col move is ever added, it must surface the engine `Err`.)
- **Fill (⌘D / ⌘R) into a region** stays **rejected** with a clear message (fill across
  merges is a documented limitation, consistent with the engine's covered-cell write
  rejection — see scope). The generic "not supported" wording is updated to reflect that
  merges *are* supported but not as a fill target.

### F7 — Undo / redo and re-render

- Merge and unmerge are each a **single undo step** (engine-guaranteed). ⌘Z / ⌘⇧Z
  restore the prior merge state **and** any content the merge discarded.
- Every path that changes merges (create, remove, structural displacement, undo/redo,
  sheet switch, file open) leaves the grid rendering the **current** engine merge state —
  the resident merge state the UI reads is kept in sync.

### F8 — xlsx round-trip coverage

Merges created in-app and merges loaded from a file both survive open→save→reopen
(engine/importer/exporter already do this). This project adds **test coverage**, not new
round-trip code.

## Edge cases

- **Merge over an existing merge / partial overlap:** the engine rejects `merge_cells`
  that overlaps an existing region. The toggle avoids this: if the selection contains any
  merge, the click **unmerges** (never merges). So a merge is only ever issued on a
  selection with no interior merges. (No auto-unmerge-then-remerge in v1.)
- **Merge fails validation** (out-of-bounds is impossible from a valid selection;
  array/spill collision is possible): surface the engine `Err` as a clear dialog; no
  partial change.
- **Selection is exactly one existing region:** button is active → click unmerges.
- **Selection is a single covered cell** (e.g. via address box): resolves to its region →
  active/unmerge.
- **Empty merge:** merging all-empty cells shows no warning and just merges.
- **Region at the grid edge / scrolled:** renders clipped; selection/nav clamp at grid
  bounds as usual.
- **Multiple regions in one selection:** unmerge removes all; the data-loss warning (merge
  path only) is not reached because such a selection unmerges.
- **Sheet with hundreds of merges:** rendering and selection stay responsive (see
  Constraints).

## Error handling / messaging

- **Data-loss warning:** OK/Cancel confirm (F3).
- **Engine rejection** (array/spill collision on merge, split-move): OK-only error dialog
  with the engine's reason, no change.
- **Fill into a merge:** OK-only "can't fill into merged cells" dialog (updated wording).
- The old **"Merged cells not supported"** insert/delete dialog is **removed** (that path
  now succeeds via displacement).

## Constraints

- **Compatibility:** no bitcode/xlsx format change; `Worksheet.merge_cells` stays A1
  strings in the engine. FreeCell reads merges through the engine API.
- **Performance:** merge counts per sheet are small (typically < a few hundred). Coverage
  lookups (covered→anchor, region-at-cell) used in render and per-keystroke selection must
  be **synchronous and cheap** on the UI side — the resident merge state supports O(1)–
  O(log n) or small-linear lookups, no per-keystroke worker round-trip. Rendering a
  screenful must not regress the frame budget.
- **Consistency:** the resident merge state the UI renders/selects from always reflects the
  authoritative engine state after any mutation, undo/redo, sheet switch, or file load.

## Out of scope / documented limitations

- **Merge variants:** no Merge & Center, Merge Across, or no-center Merge Cells — a single
  toggle only. (Centering / variants can be a follow-up.)
- **Right-click context menu** for merge/unmerge — deferred (no context-menu infra yet).
- **Clipboard merge fidelity:** copy/paste does **not** carry merge structure (engine
  Phase-2 limitation) — pasting a region reproduces values/styles only, not the merge.
- **Fill (⌘D/⌘R) across merges:** unsupported (rejected), consistent with the engine's
  covered-cell write rejection.
- **Unified merge borders / merge-aware autofill:** the region uses per-cell stored styles;
  no special unified-border rendering beyond suppressing interior gridlines.
- **No engine changes:** this project consumes the merged-cell API; any engine gap found is
  fixed in the fork per the standing IronCalc-fork workflow, not worked around in FreeCell.
```
