---
status: draft
---

# UI Design: Merged Cell UI

Visual + interaction design for the merged-cell UI. Behavior/semantics live in
`functional_spec.md`; this doc pins the *look and feel* and the small interaction
details (icon, button states, region rendering treatment, outlines, editor overlay,
dialog copy). No new screens or navigation — the surfaces are the existing grid,
action row, Edit menu, and modal layer.

## 1. The "Merge cells" toggle (action row)

A single toggle button, styled exactly like the existing character-format toggles
(bold/italic), added to `ChromeView::render_action_row`.

- **Icon:** a Lucide merge glyph — `table-cells-merge`. Per the repo icon convention,
  prefer it from the gpui-component Lucide bundle; if absent, vendor that single glyph
  (tintable `stroke="currentColor"`) under `freecell-app/assets/icons/` and register it in
  `shell/assets.rs`. One icon for both directions (do **not** swap to a split icon) — the
  pressed state + tooltip convey mode, matching the bold/italic toggle pattern.
- **Placement:** in the **alignment/wrap group** (merge is a cell-layout concern, as in
  Excel/Numbers), immediately after the wrap-text button, preceded by an `action_divider()`.
- **States:**
  - **Pressed/active** whenever the selection contains a merged region → reads as "merge
    is on"; a click unmerges. Driven by a `merge_active()` helper mirroring `bold_active()`.
  - **Inactive** for a mergeable multi-cell selection with no interior merge → a click merges.
  - **Disabled** when the selection is a lone 1×1 cell not in any merge (nothing to toggle),
    **or** when `self.degraded` (read-only), consistent with every other mutating button.
  - **Tooltip:** "Merge cells" when inactive, "Unmerge cells" when active.

**Discoverability note (design pushback):** a single toggle whose action depends on the
selection is a known small risk. It's acceptable here because (a) it is the exact
Excel/Numbers convention users already know, and (b) the pressed state + tooltip swap make
the current mode explicit. The alternative (separate Merge / Unmerge buttons) adds chrome
for no real gain at this scope.

## 2. Menu + keyboard

- **Edit menu:** a "Merge Cells" item (static label) after Find, dispatching the same toggle
  action. It follows the standard menu enable/disable (grayed when the action is disabled).
- **Shortcut:** **⌃⌘M** (Control+Command+M — Apple Numbers' merge shortcut), registered in
  `shell/menus.rs` next to `ToggleBold`.

## 3. Merged-region rendering treatment

Rendered by `build_grid_layers` using `span_rect(region_rows, region_cols, frame)` for the
region's pixel box:

- **Anchor content** is drawn once across the **whole region box**: its text/number,
  formatted per the anchor's style (font, size, bold/italic/underline/strike, number format).
  Horizontal + vertical alignment come from the **anchor's own** `h_align`/`v_align` (we do
  not add centering). With the full box width available, text that would spill/clip in a 1×1
  cell now uses the box: wrap-text wraps to the box width; non-wrap clips to the box.
- **Covered cells are not painted** as separate cells — skipped in the per-cell loop. The box
  shows the **anchor's fill**; **interior gridlines** between the region's cells are suppressed
  (a consequence of skipping covered cells' right/bottom gridline edges). The region's **outer**
  gridlines/borders render normally.
- **Spill overlay:** the anchor's text-spill logic treats the merged box as the cell bounds
  (spill is measured against the box, not the 1×1 anchor).
- Applies identically to file-loaded and in-app-created merges (same live merge state).

## 4. Selection & active-cell outline over a region

- **Active-cell outline:** when the active cell is the anchor of / inside a region, the 2px
  active-cell border is drawn around the **whole region box** (`span_rect`), not a 1×1 cell.
- **Range overlay:** the translucent range fill + 2px range border cover the selection
  rectangle *after* merge-snapping (which already includes whole regions), so the overlay
  spans merges with no partial-region slivers.
- **Header highlight:** selected row/column header shading reflects the snapped selection.
- **Ref/formula box:** shows the anchor's A1 address when a region is active.

## 5. In-cell editor over a region

- The in-cell editor overlay is **positioned and sized to the whole region box**
  (`span_rect`), not the 1×1 anchor — Excel-like editing across the merged area. It uses the
  anchor's font/style and commits to the **anchor** (never a covered-cell write).
- Editor growth (multi-line / long input) follows the existing `incell_geom` measurement,
  seeded from the region box as the base rect.

## 6. Dialogs (modal layer)

Reuse the existing `ActiveModal` system in `shell/window.rs`.

- **Data-loss confirm** (new two-button `Confirm` variant; the 3-button unsaved-changes modal
  is the pattern to follow):
  - **Title:** "Merge cells?"
  - **Body:** "Merging keeps only the upper-left value and discards the other values in the
    selection."
  - **Buttons:** **Merge** (primary) · **Cancel**. Merge proceeds (single undo step); Cancel
    aborts with no change.
  - Shown only when the merge would discard non-empty covered content (F3). All-empty or
    single-value selections merge with no dialog.
- **Fill-into-merge rejection** (existing OK-only `Error` modal, updated copy — replaces the
  old "Merged cells not supported" text):
  - **Title:** "Can't fill merged cells"
  - **Body:** "Fill (⌘D / ⌘R) can't write into a merged region."
- **Merge validation error** (array/spill collision): OK-only `Error` modal carrying the
  engine's reason; no change.
- The old insert/delete "Merged cells not supported" dialog is **removed** (those ops now
  succeed via engine displacement).

## 7. Motion / redraw feel

- Merge, unmerge, and structural displacement re-render from the republished merge state — the
  box appears/disappears/reshapes in the same frame the worker publishes, like any style edit.
- Undo/redo restores the prior box (and any discarded content) in one step.
