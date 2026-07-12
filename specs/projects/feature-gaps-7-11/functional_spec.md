---
status: draft
---

# Functional Spec: Feature Gaps 7_11

A batch of independent feature gaps for the FreeCell spreadsheet. Each section below is
self-contained; the features do not depend on one another (which is what lets them be
coded async in parallel after this sync planning pass). Section numbers are this project's
own. The shipped MVP + `mvp-gaps` specs remain authoritative for everything not restated
here.

**In-scope features:** §1 font-warning fix · §2 text spill/overflow · §3 auto-grow rows ·
§4 find/replace · §5 quick-edit mode · §6 sheet reorder (drag + IronCalc fork API) ·
§7 verify right-click insert/delete (already built).

**Deferred to backlog** (see `PROJECTS.md`): Cmd/Ctrl-click disjoint selection
(`projects/disjoint-selection.md`), freeze panes (`projects/freeze-panes.md`).

---

## 1. Font-warning fix

### 1.1 Problem

At runtime the app logs, on first SVG render:

```
WARN gpui::svg_renderer: Failed to load bundled font fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf: could not find asset at path "..."
WARN gpui::svg_renderer: Failed to load bundled font fonts/lilex/Lilex-Regular.ttf: could not find asset at path "..."
```

### 1.2 Root cause (investigated)

The warning does **not** come from FreeCell's SVGs. gpui *core* (`svg_renderer.rs` at the
pinned Zed rev) hard-codes two of **Zed's own** bundled font asset paths
(`fonts/ibm-plex-sans/…`, `fonts/lilex/…`) and tries to load them through the app's
`AssetSource` to build a font DB for rendering `<text>` inside SVGs. FreeCell's
`AssetSource` doesn't serve `fonts/*`, so each load returns an error and gpui logs a
benign `WARN`. FreeCell's own icon SVGs contain **no `<text>`/`font-family`**, so this
font DB is never actually used — nothing renders wrong.

### 1.3 Behavior after fix

- The two warnings no longer appear in normal runs.
- No visual change (the fonts were never needed).
- The fix is **log suppression of exactly the `gpui::svg_renderer` target**, added to the
  default `tracing` `EnvFilter` (so `RUST_LOG` overrides still work and can re-enable it).
  We do **not** ship Zed's fonts and do **not** alias our fonts under those paths.

### 1.4 Out of scope

Rendering `<text>` inside SVGs (we have none); changing the pinned gpui rev.

---

## 2. Text spill / overflow (horizontal)

Long cell text visually overflows into adjacent **empty** cells, Excel-style, instead of
being clipped at the cell boundary.

### 2.1 When a cell spills

A cell's text spills iff **all** of:

- **Wrap is off** for the cell (wrap-on cells grow vertically instead — see §3; never both).
- The cell's content is **text** (`CellKind::Text` / string). Numbers, dates, booleans,
  and errors do **not** spill — they clip as today. (Excel shows `#####` for
  too-narrow numbers; that indicator is **out of scope** for this batch — numbers simply
  clip. Flagged for review.)
- The rendered text is **wider** than the cell's own column width.
- There is at least one **empty** neighbor cell in the spill direction.

### 2.2 Spill direction (follows alignment, Excel-accurate)

- **Left / General-aligned** text (the default for text): spills to the **right**.
- **Right-aligned** text: spills to the **left**.
- **Center-aligned** text: spills **both** directions (text centered over the run of empty
  cells on both sides).

### 2.3 How far it spills

- Spill extends across **consecutive empty** neighbor cells in the spill direction and
  **stops at the first cell that contains content** (a value/formula-result). It clips
  there.
- "Empty" = no committed content. A cell that is empty but has styling (fill, borders)
  does **not** stop the spill (matches Excel). Only content stops it.
- Once stopped, the text is clipped at that boundary and **never resumes** further along
  the row (no re-start after a gap).
- For center spill, each side is bounded independently by the nearest non-empty cell on
  that side.

### 2.4 Rendering semantics

- The spilling text is painted **over** the empty neighbor cells (they have no content
  beneath). Neighbor gridlines and any neighbor fill still render; the text sits on top.
- The **origin cell's** fill/borders/gridlines are unchanged. Selection/active-cell
  outline stays on the **origin cell only** — the spill is presentation, not selection.
- Spill is clipped to the grid's content viewport as usual (never escapes into headers or
  outside the grid).
- Vertical alignment of spilled text matches the origin cell.

### 2.5 Interactions & edge cases

- **Editing:** spill applies to the **committed** display value. While a cell is being
  edited, the existing live-mirror / in-cell-editor behavior applies (unchanged); the
  spill render is for non-editing cells.
- **A neighbor becoming non-empty** (edit, paste) re-clips the spill on the next frame.
- **Scrolling / frame boundary:** spill may extend to a neighbor just off the visible
  range; the text is clipped by the content viewport regardless. The empty/non-empty
  decision must consult the engine read-model, not only the on-screen index, so a spill
  that would be *stopped* by an off-screen non-empty cell is stopped correctly (no
  false-empty past the viewport edge — never treat "beyond the loaded/covered region" as
  reliably empty; when coverage is unknown, do not spill past it).
- **Both neighbors empty, right-aligned, etc.:** direction is by alignment only, not by
  which side has more room.
- **Wrapped origin cell:** never spills (mutually exclusive with §3).

### 2.6 Out of scope

`#####` overflow indicator for numbers; spill of non-text types; spill during active edit.

---

## 3. Auto-grow rows (vertical)

A row grows its height to fit its tallest cell, so large fonts and wrapped/multiline text
are fully visible — **unless the user has manually set that row's height**.

### 3.1 What already exists (baseline)

- **Large font:** changing font size already auto-grows the row (`SetFont` path). Retained.
- **Explicit newlines:** editing a cell to multiline text (literal `\n`) already auto-fits
  the row via IronCalc's auto-fit. Retained.

### 3.2 New behavior — wrap-driven growth

- A cell with **wrap on** whose content wraps to multiple visual lines at the current
  column width causes its row to grow to fit all wrapped lines.
- Growth recomputes when any input to wrapped height changes:
  - the cell's **content** changes (edit/paste/clear),
  - **wrap is toggled** on/off for the cell,
  - the cell's **font/size** changes,
  - the **column is resized** (narrower → more wrapped lines → taller row; wider → fewer).
- A row's height is the **max needed height** over all its cells (a row with one tall
  wrapped cell and several short cells takes the tall height).

### 3.3 The "manual height wins" rule

- A row is **manual** once the user drags its row-divider to resize it. While manual,
  auto-grow **never** changes its height (neither grows nor shrinks) — Excel behavior.
- A row is **auto** otherwise. Auto rows grow to fit and may **shrink back** toward the
  default when the tall content is removed/unwrapped/narrowed away (down to the default
  row height, never below).
- The manual flag is tracked per row, per sheet. (Scope note: it is **session-scoped** —
  it does not need to persist across save/reload in this batch; a reloaded file's rows
  start as auto. Flagged as an accepted limitation; revisit if it bites.)

### 3.4 Edge cases

- **Empty row / all default:** stays at default height.
- **Very tall content:** capped at a sane maximum (define in architecture, e.g. the same
  cap Excel-ish behavior or N lines) so a pathological cell can't make a row fill the
  screen; content beyond the cap clips within the wrapped cell.
- **Interaction with spill (§2):** wrap-on ⇒ auto-grow, wrap-off ⇒ spill. A cell is never
  both.
- **Undo/redo:** an explicit row resize is undoable as today; auto-grow height changes
  ride with the content/style/width edit that caused them (should not add extra undo
  steps the user must step through — architecture decides the exact coupling).

### 3.5 Out of scope

Auto-fit-on-double-click-divider gesture; persisting the manual/auto flag to xlsx;
per-cell (vs per-row) height.

---

## 4. Find / replace

A dismissible find/replace bar, scoped to the **current sheet**, opened via Cmd/Ctrl+F or
an action-bar button.

### 4.1 The bar

- A new horizontal bar that appears **directly under the formula/data row**, pushing the
  grid down (not an overlay).
- Contents (left→right): a **Find** text field, a **Replace** text field, a **match-case**
  toggle (`Aa`), a **match-entire-cell** toggle, **previous-match** (↑) and
  **next-match** (↓) buttons, a **match counter** ("3 of 12" / "No results"), a
  **Replace** button, a **Replace All** button, and a **dismiss (X)** on the right.
- Exact layout/visuals in `ui_design.md`.

### 4.2 Opening / closing / focus

- **Open:** Cmd/Ctrl+F (a new `OpenFind` action + keybinding) **or** a search-icon button
  in the action bar. Opening focuses the Find field and selects any existing text in it.
- If a single cell or range is selected when opening, the bar opens with whatever find
  text is already there (no auto-populate from the cell in this batch).
- **Close:** the X button, or **Escape** while the bar is focused. Closing returns focus
  to the grid and clears any transient match highlight. The find/replace text is retained
  for the next open within the session.

### 4.3 Find behavior

- **Scope:** the current sheet's **used range** (the populated area). Empty cells are not
  matched.
- **Match target:** the cell's **raw content** — the literal value for value cells, the
  **formula text** for formula cells (Excel's default "Look in: Formulas"). This is
  consistent with replace (§4.4). (Matching formatted display values is a possible future
  toggle; out of scope now.)
- **Toggles:**
  - *Match case* off (default) = case-insensitive; on = exact case.
  - *Match entire cell* off (default) = substring match; on = the whole cell content must
    equal the find string.
- **Next / Previous:** move to the next/previous matching cell in row-major order,
  **wrapping around** the used range. The matched cell is **selected and scrolled into
  view**. The counter shows position/total ("3 of 12").
- **No matches:** counter shows "No results"; next/prev/replace disabled; nothing selected.
- **Empty find field:** no matches; action buttons disabled.
- Live re-evaluation: editing the find field or toggling a switch recomputes matches.

### 4.4 Replace behavior

- **Replace** operates on the **raw content**: for the current match, replaces the matched
  substring (or whole content, if match-entire-cell) with the replace string, commits the
  cell, then advances to the next match. Replacing inside a formula edits the formula text
  (user's responsibility if it breaks the formula — matches Excel).
- **Replace All** replaces every match in the used range in **one undoable batch**, then
  reports the count ("Replaced 7").
- Replacements go through the normal edit/commit path (engine recompute, undo/redo).
- A replace that produces an invalid/over-cap value is handled by the existing edit
  validation (the same cap-error handling as normal edits); Replace All skips/█reports
  cells it can't write rather than aborting the whole batch (architecture defines exact
  handling).

### 4.5 Constraints & edge cases

- **Big sheets:** find/replace runs in the **worker** (which owns the model), scanning the
  used range — not over the viewport-bounded render publication. The used range is
  typically small; the scan must not block the UI (async command/response like other
  worker ops).
- **Sheet switch while open:** the bar stays open but its matches re-scope to the newly
  active sheet (counter/selection reset).
- **Concurrent edits:** if the sheet changes under an open find, the match set is
  recomputed on the next find/replace action.

### 4.6 Out of scope

Whole-workbook scope; regex; "Look in: Values"; find within a selection only; find-format;
find in charts.

---

## 5. Quick-edit mode

A UX improvement: when you start typing on a focused cell, arrow keys keep navigating
between cells (commit + move), so rapid data entry doesn't require reaching for Tab/Enter.

### 5.1 Entering quick-edit

- Quick-edit is entered **only** by **type-to-replace**: the grid is focused, a single
  cell is selected, and you type a printable character (the existing type-to-replace path
  that begins an edit in the data row). That edit is now in **quick-edit mode**.
- Quick-edit is **not** entered by: double-click / F2 (in-cell editor), clicking into the
  formula/data row, or any other path. Those are normal edits (arrows = caret).

### 5.2 Quick-edit arrow behavior

- While in quick-edit, an **unmodified arrow key** (←→↑↓) **commits** the current edit and
  **moves the active cell** one step in that direction (Left/Right = column, Up/Down =
  row), collapsing selection to the destination single cell. Example: focus a cell, type
  `abcd`, press → ⇒ `abcd` is committed to the cell and focus moves right.
- This mirrors the existing Tab/Enter commit-and-move, just directional.
- After the move, you are back in normal grid navigation (no active edit). Typing again
  starts a fresh quick-edit on the new cell.

### 5.3 Leaving quick-edit (arrows revert to caret control)

Quick-edit ends — and for the remainder of that edit arrows control the **text caret**, not
cell movement — as soon as the user signals caret intent by **any** of:

- **Clicking with the mouse to place the caret** inside the data-row field (or the in-cell
  editor if it were open).
- Pressing **Home** or **End** (explicit caret positioning).
- Pressing a **modified arrow** (Shift/Cmd/Ctrl+arrow) — treated as a caret/selection
  operation; it ends quick-edit and does not move the active cell.

After any of these, the edit continues as a normal edit until committed/cancelled;
arrows move the caret.

### 5.4 Preserved behavior

- **Commit/cancel keys unchanged:** Enter/Shift+Enter/Tab/Shift+Tab commit-and-move,
  Escape cancels — from both quick and normal edits.
- **Backspace/Delete** and printable typing edit the text and keep quick-edit active.
- The live cell mirror (raw text shown in the cell) is unchanged.
- Multi-cell selection + type-to-replace still targets the anchor (existing behavior); the
  quick-edit move collapses to the destination single cell.

### 5.5 Out of scope

Quick-edit for double-click/F2/formula-bar edits (explicitly excluded by design);
autocomplete; range-extend semantics while typing.

---

## 6. Sheet reorder (drag to re-order tabs)

Drag a sheet tab to change sheet order.

### 6.1 Interaction

- Press-and-drag a sheet tab horizontally along the tab bar. A **drop indicator** shows
  where the sheet will land (insertion point between tabs). On release, the sheet moves to
  that index; tabs re-render in the new order.
- A click without meaningful drag movement still **selects** the sheet (existing behavior);
  double-click still starts rename. Drag is distinguished by a movement threshold.
- The dragged sheet stays the **active** sheet after the drop (active follows the sheet,
  not the slot). Right-click menu (rename/delete) is unaffected.

### 6.2 Engine + persistence

- Reorder goes **through the engine** (the tab order is derived from engine/workbook order;
  a UI-only reorder would be overwritten on the next `SheetsChanged`). A new
  `Command::MoveSheet { sheet, to_index }` → worker → IronCalc.
- The move is **undoable** (Undo/Redo restore the prior order) and the new order is
  **preserved on xlsx save**.

### 6.3 IronCalc fork change (required)

IronCalc exposes no sheet-reorder API (confirmed: round-3 API audit — no
`move_sheet`/`set_worksheet_index`/`swap_worksheets`). Per CLAUDE.md ("fix upstream, don't
hack FreeCell"), this batch adds an **undoable, xlsx-order-preserving**
`UserModel::set_worksheet_index` (or `move_sheet`) to `scosman/ironcalc`, with
upstream-style tests, as a clean single-fix branch/PR, integrated into `freecell-fixes`
that FreeCell builds against. The FreeCell engine layer then wraps it.

### 6.4 Edge cases

- Dragging a tab to the far left/right edge with many tabs: auto-scroll the tab strip if it
  scrolls (only if the tab bar can overflow — otherwise N/A).
- Dropping a tab back on its original position is a no-op (no engine command, no undo step).
- Single sheet: nothing to reorder.
- Reorder during an active edit commits/cancels per existing focus rules before the tab
  interaction (drag starts on the tab bar, not the grid).

### 6.5 Out of scope

Reordering by keyboard; moving sheets between windows/workbooks; color tabs.

---

## 7. Verify: right-click header insert/delete (already built)

Right-click a row or column header already shows **Insert above/below** (rows) /
**Insert left/right** (columns) / **Delete**, wired to the engine with a merged-cell guard
(shipped in `mvp-gaps` Phase 7, `header_menu_elements`). This batch does **not** rebuild
it. Scope here is a **verification pass**:

- Smoke-check that right-clicking single and multi-row/col header selections shows the menu
  with correct counts ("Insert 3 rows above", etc.) and that insert/delete apply correctly.
- Confirm the merged-cell guard still blocks displacing edits.
- If a real deficiency is found, file it (or fix if trivial); otherwise this is a no-code
  confirmation. No new UI.

---

## Cross-cutting constraints

- **Excel compatibility** is the north star for spill (§2), auto-grow (§3), and
  find/replace target semantics (§4) — behaviors above are chosen to match Excel where a
  choice exists.
- **Performance:** spill and auto-grow touch the **render hot path** — they must stay
  allocation-light and O(visible cells), consistent with the "stupid-fast on huge sheets"
  goal. Find/replace runs off-thread in the worker.
- **Render baselines:** §2 (spill) and §3 (auto-grow) **change grid pixels** and are
  in-scope for the pixel render suite (per CLAUDE.md) — they get a dedicated late render
  phase (subset while iterating, full suite + CI `render` gate at the end, baselines
  regenerated and eyeballed). §1/§4/§5/§6/§7 touch chrome/tabs/logging/behavior that the
  pixel suite does **not** baseline — validated with gpui view tests + a smoke launch, no
  pixel run.
