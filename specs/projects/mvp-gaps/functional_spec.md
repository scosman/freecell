---
status: complete
---

# Functional Spec: MVP Gaps — Core Spreadsheet Feel

Extends the shipped MVP (`specs/projects/mvp/functional_spec.md`, still authoritative
for everything not restated here). Section numbers below are this project's own.

## 1. Editing feel

### 1.1 Type-to-replace

- With the grid focused and a **single cell** selected, typing any printable character
  starts an edit: focus moves to the data row, its content is **replaced** by the typed
  character, caret at end. Subsequent keys append normally.
- `=`, digits, letters, punctuation all qualify. Modifier-held keys (Cmd/Ctrl shortcuts)
  do not. Space qualifies. Delete/Backspace keep their existing clear-cells meaning.
- With a **multi-cell** selection, typing behaves as if only the anchor were selected
  (Excel behavior): edit targets the anchor cell; the range collapses to the anchor on
  commit-and-move.
- Escape during such an edit restores the cell's prior content in the data row and
  returns focus to the grid. Commit keys behave per MVP §3.3 (Enter down, Shift+Enter
  up, Tab right / Shift+Tab left — Tab-commit gap from DECISIONS_TO_REVIEW is fixed by
  this project, §1.4).

### 1.2 Live cell mirror

- While an edit is pending (data row **or** in-cell editor), the active cell renders
  the **raw text being typed** (left-aligned, default style, clipped) instead of its
  committed display value. This is presentation-only: no engine call, no evaluation,
  styles unchanged.
- On commit the mirror is replaced by the optimistic-pending display that already
  exists (MVP §4); on cancel it reverts to the committed value.
- The mirror renders in the grid's default font/size regardless of cell style (raw
  input, not a value preview).

### 1.3 In-cell editor

- **Double-click** a cell, or press **F2** with a single cell selected, opens an editor
  overlay exactly covering the cell rect (grows to at least 80 px wide for narrow
  columns; never smaller than the cell; clips at the grid edge).
- Shows the cell's **raw content** (formula text for formula cells), fully selected on
  double-click, caret-at-end on F2. Typing after type-to-replace stays in the data
  row — type-to-replace does **not** open the in-cell editor.
- The in-cell editor and data row are **the same edit**: text is mirrored live in both;
  editing either updates the other. There is exactly one pending edit at a time.
- Commit/cancel semantics identical to the data row (Enter/Shift+Enter/Tab/Shift+Tab
  commit + move, Escape cancels, click-elsewhere commits first, input caps + danger
  border + §4.2 popover apply). On commit/cancel the overlay closes.
- Scrolling while the in-cell editor is open: the overlay moves with the cell (it is
  positioned in content space). If the cell scrolls out of view the editor stays open
  (Excel behavior); commit still applies to the anchored cell.
- Out of scope: IME composition (existing project), rich intra-formula range
  highlighting, in-cell autocomplete.

### 1.4 Commit-key completeness

Tab / Shift+Tab commit a pending edit and move right/left (closing the MVP's known
"Tab doesn't commit" gap) — from both the data row and the in-cell editor.

## 2. Range clipboard

### 2.1 Copy / Cut (Cmd/Ctrl+C, Cmd/Ctrl+X — grid focused)

- Operates on the current selection (cell, range, full rows/cols, select-all —
  engine-clamped to the used area).
- Internal payload: the engine's clipboard (raw content/formulas + resolved styles per
  cell + source range). System clipboard simultaneously receives the engine's
  **tab-separated formatted text** (TSV; rows separated by `\n`) so other apps can
  paste values.
- Cut is copy + pending-move: visually identical to copy in MVP scope (no marching
  ants); the source clears when pasted (§2.2), not at cut time. A second copy/cut
  replaces the pending one.

### 2.2 Paste (Cmd/Ctrl+V — grid focused)

- **Internal paste** (our copy is still the newest thing on the system clipboard):
  pastes the full-fidelity range at the selection anchor — values, formulas with
  Excel-style relative-reference adjustment, styles. Cut-paste moves (refs into the
  moved area follow; source contents cleared) and clears the pending cut.
- **External paste** (system clipboard text isn't ours): parse as TSV (tabs → columns,
  newlines → rows) and paste as **user input** starting at the anchor (each token as
  if typed: numbers, booleans, `=formulas`, text). Single token → single cell.
- Paste target = selection **anchor** (top-left of pasted range); the pasted area
  becomes the new selection. No tiling over larger targets in this project.
- Paste that would overflow the sheet edge is rejected with a brief status message
  (no partial paste).
- One undo step per paste (engine-provided). Errors surface as a dialog only for
  structural failure (e.g. overflow); per-cell junk becomes text/errors as usual.

### 2.3 Non-goals

Styles/HTML flavors to other apps, paste-special, marching-ants overlay, tiling —
`projects/excel-clipboard.md`.

## 3. Formatting

All actions apply to the full selection, are one undo step, and follow MVP §3.5
semantics (button state reflects the active cell). Whole-row/col selections use the
engine's band styles — **exception:** font family/size (§3.2) clamps to the used
range on full-row/col selections (engine limitation; documented deviation).

### 3.1 Action-bar additions

Final action-bar order (left → right):
`[Font family ▾][Size ▾] | B I U | [Text color ▾][Fill ▾] | [Borders ▾] | [⟸ ⟺ ⟹ alignment] | [Number format ▾][.00→ →.00] | (spinner, right-aligned)`

### 3.2 Font family & size

- **Family dropdown**: installed fonts (system enumeration), current cell's family
  shown; "System Default" entry at top clears the override.
- **Size dropdown**: fixed list 8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 36; shows
  the active cell's size.
- Grid renders per-cell family/size (weight/style combine with B/I/U). Missing fonts
  (file names a family not installed) fall back to the default font — display-only,
  the style is preserved and saved.
- **Row auto-grow**: after a size/family change, any affected row whose height is too
  small for the new font grows to fit (never shrinks, never touches rows with
  file-set/user-set larger heights). No auto-grow on file open — files carry their
  authored heights, and mutating on open would dirty a just-opened document.

### 3.3 Text color & alignment

- **Text color**: palette popover identical to Fill (10 theme colors + **Automatic**
  (clear) + Custom…).
- **Alignment**: three toggle buttons (left/center/right); pressing the active one
  clears the explicit alignment (back to type-default).

### 3.4 Number formats

Dropdown entries (exact codes in architecture): General, Number (`#,##0.00`),
Currency (`$#,##0.00`), Percent (`0.00%`), Date (`m/d/yyyy`), Time (`h:mm AM/PM`),
Text (`@`). Plus two buttons: increase / decrease decimals (adjusts the decimal places
of the current numeric format; no-op on General/Text/Date). Display remains fully
engine-owned.

### 3.5 Type-aware defaults + `[Red]` color (GAPS #1/#2)

- Cells without explicit alignment align by evaluated type: numbers & dates right,
  booleans & errors center, text left.
- Number-format color (e.g. `[Red]` negatives) renders as the text color when the
  format produces one. Explicit font color wins over format color.

### 3.6 Borders

- **Render**: cell borders loaded from files draw in the grid (thin/medium/thick/
  double/dotted/dashed families approximated per architecture; border draws **over**
  the default gridline). Shared edges: the heavier border wins.
- **Borders menu** (action bar): fixed presets — All, Inner, Outer, Top, Bottom,
  Left, Right, **None** — thin black only. Applies to the selection via the engine's
  border API (band-aware for full rows/cols, undoable).

## 4. Chrome & data safety

### 4.1 Uniform titlebar (macOS)

- The window draws its own titlebar row: action-bar grey (`0xF3F3F3`), centered
  document title (name + edited state per MVP §2.3), traffic lights repositioned to
  vertically center; the row is a window-drag area; double-click zooms (system
  behavior via drag-area). Welcome window gets the same treatment.
- Linux: unchanged (server decorations).
- If real-device verification shows traffic-light/fullscreen glitches at the pinned
  rev, this feature reverts cleanly (flag + row removal) rather than forcing a gpui
  bump.

### 4.2 Cap-error popover (GAPS #3)

Over-cap input (length/depth) keeps today's reject behavior and additionally shows a
small popover anchored under the active editor (data row or in-cell): "Formula too
long (max 8,192 characters)" / "Formula nested too deeply (max 64 levels)".
Dismisses on next keystroke/focus change.

### 4.3 `.back` backup before first save

Before the **first** successful save-in-place of a document opened from disk, copy
the original bytes to `<name>.xlsx.back` next to it (write-once; never overwritten on
later saves; not created by Save-As-to-new-path; creation failure aborts the save
with a dialog — data safety wins over convenience).

## 5. Structure & navigation

### 5.1 Row/col resize

- Hovering within 3 px of a column-header divider shows the col-resize cursor; row
  headers likewise (row-resize). Drag resizes live (guide line + live reflow; final
  engine write on release, one undo step).
- Minimum sizes: column 8 px, row 12 px; drag clamps. Double-click on a divider does
  nothing in this project (autofit is in GAPS.md).
- If the dragged header is inside the current multi-header selection, all selected
  rows/cols get the released size (one undo step).

### 5.2 Header selection & select-all

- Click a column header → that full column selected (anchor at its row 1... visible
  behavior: whole column highlighted, headers tinted). Drag across headers extends;
  Shift+click extends; row headers likewise. Corner button selects the whole sheet.
- Data row behavior follows MVP multi-select rules (disabled); the reference box
  shows `C:C`, `3:7`, or `A1:XFD1048576` style ranges.
- Formatting on header selections uses band styles (fast); Delete clears contents
  clamped to the used range; copy clamps to the used range (engine behavior).
- Keyboard: Cmd/Ctrl+A = select all (first press; no expand-to-region subtlety).

### 5.3 Insert/delete rows & columns

- **Right-click on a row/col header**: context menu — "Insert N row(s) above/below",
  "Delete N row(s)" (N = size of the header selection if the click is inside it,
  else 1; wording pluralizes). Columns: left/right.
- Engine-native, undoable, formulas adjust. Errors (e.g. shift would push used cells
  past the sheet edge) surface as a dialog.
- **Merge guard**: if the sheet's file-loaded merged ranges would be displaced by the
  operation (any merge at/after the affected index), the action shows a dialog —
  "This sheet contains merged cells (not yet supported); inserting/deleting here
  would corrupt them." — and does nothing. Merges strictly above/left of the edit
  don't block.

## 6. Edge cases & errors (cross-cutting)

- All new engine mutations run through the existing worker/coalescing/undo pipeline;
  the UI thread never blocks on them. Failures returned by the engine surface as a
  dialog only when the user must know (paste overflow, structural-edit failure, save
  backup failure); style no-ops fail silently to the log.
- New surfaces respect degraded-worker mode (read-only bar): all new mutating controls
  disable, resize/selection/copy still work.
- Scroll-perf gates (MVP §7) still bind: borders/fonts render from the resident style
  cache; zero engine calls on the scroll path; the in-cell editor and mirror add no
  per-frame engine reads.

## 7. Out of scope (this project)

Zoom; merged cells (render/selection/UI — `projects/merged-cells.md`); grid cell
context menu; fill handle / fill down-right; find/replace; autofit; recent files;
freeze panes; sort/filter; overflow/wrap; paste-special / rich clipboard; IME;
Cmd+arrow edge-of-data; font size/family band styles on full rows/cols (clamped
instead); marching-ants cut overlay.
