---
status: complete
---

# UI Design: Charts (production)

Scope of the UI: charts are objects that float **over the grid** in sheet coordinates.
This doc covers the **v1-core display UI** (concrete) and sketches the **end-phase
authoring/editing UI** (structural — detail deferred to those phases). It reuses FreeCell's
existing chrome: `chrome/view.rs` (action bar + `.absolute()` floating overlays via
gpui-component `Popover`/`ContextMenu`/`Modal`) and `shell/menus.rs`.

## 1. Where charts live

- Charts render as a **ChartLayer over the grid cells**, in sheet coordinates, so they scroll
  and zoom with the sheet (like the existing "Opening…" overlay, but sheet-anchored).
- **Z-order:** above cells + gridlines, **below** app chrome overlays (popovers, menus,
  modals, the edit panel). Clipped to the grid viewport.
- No separate "charts screen" — there is only the sheet; charts are in it.

## 2. v1-core display UI (read-only)

### 2.1 A rendered chart
- Drawn at its anchor rect (`twoCellAnchor` from/to → pixels); the renderer paints title,
  plot, axes (a solid category **and** value axis line at the plot boundaries, with gridlines
  clipped to the plot rect — not run through the tick-label gutters), legend, framed by a
  subtle ~1px light-grey outline around the chart's outer edge (every chart type, pie/doughnut
  included). No **selection** border / resize handles in v1 core (read-only). Off-screen charts
  aren't drawn; partially-scrolled charts are clipped.

### 2.2 Compatibility warning (functional_spec §5)
- When a chart's `compatibility_warning` flag is set, show a small **inline** label in the
  **bottom-right** corner of the chart reading **"⚠ May not display as intended"** — **light
  grey, small**. That's the whole signal — **no detail list, no hover popover.**

### 2.3 Placeholder (unsupported / parse-failed — category 3)
- A quiet bordered rectangle at the anchor with the chart **title** (if any) and a centered
  muted line **"Unsupported chart type"**. Occupies the chart's space so layout is faithful;
  never blocks opening the workbook.

## 3. Authoring — Stage 6.A (end-phase; structural)

> Detail finalized when Phase 6.A is planned. Structure below is the agreed shape.

### 3.1 Insert a chart
- **Entry point:** a **chart icon on the action bar** (`chrome/view.rs`). Clicking it opens a
  **menu of chart types**, each a **glyph + label**, ordered **Line → Area → Column → Bar → Pie
  → Doughnut → Scatter → Bubble** (Excel grouping — the single canonical `CHART_MENU` list, shared
  with the edit panel's Type row). Menu items are **left-aligned** (icon + label pack at the left,
  not centered — post-v1 Batch 3, item 14). The dropdown **paints above** the right-docked edit
  panel when both are open (item 10).
- **Choosing a type** inserts a chart of that type onto the sheet and immediately opens its
  **edit panel** (§4).
  - **Default data range (post-v1 Batch 3, item 8):** if a **real range** (more than one cell) is
    selected at insert time, the new chart is **bound to that selection immediately** — it comes up
    as a **live chart** of the chosen type (real `c:f` refs + resolved values), no follow-up "Use
    selection" click. The worker binds it at creation (same block→series binding as `SetChartRange`,
    on the freshly-inserted id).
  - With **no usable selection** (a single cell / empty), the chart comes up **nearly empty** — the
    user shapes it (set its range, title, etc.) via the panel.

### 3.2 Manipulate a chart object
- **Select** a chart → selection outline + resize handles on the ChartLayer.
- **Move** (drag body) / **resize** (drag handle) → anchor updates; **delete** via
  `Delete`/`Backspace` or a context-menu entry. (Interaction detail with 6.A.)
- **Undo/redo (charts feedback item 4):** chart **insert, delete, move/resize, and set-range**
  ops now ride the **same unified Ctrl+Z/Ctrl+Y timeline** as cell edits. Ctrl+Z reverses the
  single most-recent action regardless of kind (so deleting a chart then Ctrl+Z **brings it
  back**, and a following Ctrl+Y re-deletes it), and an interleave of cell edits + chart ops
  undoes/redoes in exact most-recent-first order. This **reverses** the earlier P18 decision that
  kept chart ops off the undo stack. Worker-side, an IronCalc cell edit and a chart op are two
  entry kinds on one ordered stack; a chart entry inverts from a stashed worker snapshot and never
  calls IronCalc's own undo/redo, so the cell entries stay 1:1 with IronCalc's stack (no desync).
  A loaded (imported) chart's delete/move restores its save-set bookkeeping (`loaded_deletes` /
  `loaded_anchor_edits`) on undo, so a later save writes the correct package. (Chart **type** and
  **chrome** edits stay immediate — not individually undoable — but still invalidate a pending
  redo.)

## 4. Editing — the Edit panel (end-phase; **detailed speccing deferred**)

- The chart **Edit panel** is a **new floating window docked to the right side of the sheet**
  (a chrome overlay, not a popover on the chart). It is how a chart is shaped/edited.
- **Options it exposes** (indicative): **chart type, data range, title, axis titles**, … and
  the rest of the §6.B chrome (legend, series colors, data-label toggles). The chart-**type**
  list orders **Line → Area → Column → Bar → …** (Excel grouping); the **legend** control is a
  row of lucide icons — `panel-top`/`panel-right`/`panel-left`/`panel-bottom` for the four
  positions, `square-x` for Off.
- **Series color overrides the imported original (charts feedback item 9):** a color set in the
  panel **takes precedence over — and replaces — the color a chart shipped with**, for **imported
  (loaded) charts** exactly as for FreeCell-authored ones. This is not just the shape fill: a
  **line/scatter** series carries its **visible** color on its `a:ln` **stroke** (and the renderer +
  loader prefer the stroke color over the fill), so for those two kinds a series-color edit recolors
  **both the fill and the stroke** — otherwise the imported line kept its old color on screen and on
  reopen. **Filled kinds** (column/bar/area/pie/bubble) render from the **fill**, and treat `a:ln`
  as a decorative border, so their edit recolors the fill **only** and leaves any imported `a:ln`
  **byte-identical** (recoloring a filled type's stroke would inject a border the user never asked to
  change — e.g. flip a borderless bar's `<a:noFill/>` to a colored `solidFill`). The new color holds
  through three layers: it renders **live**, **survives a data recompute** (re-resolve keeps the
  user's color, doesn't restore the file's), and **persists across save + reopen** (the source patch
  rewrites the series' `spPr/solidFill`, plus — **for line/scatter only** — its `a:ln/solidFill`,
  preserving the stroke's width/dash; clearing the color reverts both to the palette). Markers follow
  the resolved series color, so they recolor in FreeCell too. (Engine fix, not a render-widget
  change: the render code already reads the model's stroke-first precedence correctly — the fix makes
  the *edit* update the stroke as well as the fill, gated to the kinds that paint on the stroke.)
- **Live editing:** the **title** and **axis-title** fields update the chart **per keystroke**
  (not on Enter/blur), so shaping is immediate. A live worker republish never clobbers an
  in-progress edit (fields re-seed only when the *selected chart changes*, not on same-chart
  republish).
- **Dismissal:** the panel closes on its × button, on **click-away** (a click on a cell / empty
  grid), on the chart's deletion, or on degrade. Clicking **another chart** re-points the panel
  to it (a switch, not a close). Its body **scrolls vertically** and is **clipped to its own
  bounds**, so every control stays reachable and it never paints over other chrome at any window
  height.
- **Deferred:** the panel's exact layout beyond the above is refined as the editing phases land;
  this doc fixes its *form* (right-side floating window), *entry* (opens on insert / when a chart
  is selected), and the interaction rules above.

## 5. UX principles

- **v1 core is near-zero-chrome:** charts just display; the only added pixel is the small grey
  bottom-right warning, and only when honest to show it. Low cognitive load — "a spreadsheet
  with charts in it."
- **Authoring is create-then-shape:** pick a type from the action-bar chart menu → a
  near-empty chart + its edit panel → build it up. Familiar (matches how you drop in an
  object then format it) and keeps the insert click trivial.
- **Convention reuse:** action-bar entry, selection handles for move/resize, a docked side
  panel for editing — no new mental models.
