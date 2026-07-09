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
- Drawn at its anchor rect (`twoCellAnchor` from/to → pixels); the PoC renderer paints title,
  plot, axes, legend. No border/handles in v1 core (read-only). Off-screen charts aren't
  drawn; partially-scrolled charts are clipped.

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
  **menu of chart types**, each shown as a **glyph** of that type (line, column, bar, area,
  pie, doughnut, scatter, bubble).
- **Choosing a type** inserts a chart of that type onto the sheet and immediately opens its
  **edit panel** (§4). The chart comes up **nearly empty** — the user is expected to **edit it
  into good form** (set its range, title, etc.) via the panel. (No pre-selected range
  required.)

### 3.2 Manipulate a chart object
- **Select** a chart → selection outline + resize handles on the ChartLayer.
- **Move** (drag body) / **resize** (drag handle) → anchor updates; **delete** via
  `Delete`/`Backspace` or a context-menu entry. (Interaction detail with 6.A.)

## 4. Editing — the Edit panel (end-phase; **detailed speccing deferred**)

- The chart **Edit panel** is a **new floating window docked to the right side of the sheet**
  (a chrome overlay, not a popover on the chart). It is how a chart is shaped/edited.
- **Options it exposes** (indicative): **chart type, data range, title, axis titles**, … and
  the rest of the §6.B chrome (legend, series colors, data-label toggles).
- **Deferred:** the panel's exact layout, control set, and interaction are **specced when we
  start that phase**, per your call — this doc only fixes its *form* (right-side floating
  window) and *entry* (opens on insert / when a chart is selected).

## 5. UX principles

- **v1 core is near-zero-chrome:** charts just display; the only added pixel is the small grey
  bottom-right warning, and only when honest to show it. Low cognitive load — "a spreadsheet
  with charts in it."
- **Authoring is create-then-shape:** pick a type from the action-bar chart menu → a
  near-empty chart + its edit panel → build it up. Familiar (matches how you drop in an
  object then format it) and keeps the insert click trivial.
- **Convention reuse:** action-bar entry, selection handles for move/resize, a docked side
  panel for editing — no new mental models.
