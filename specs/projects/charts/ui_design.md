---
status: draft
---

# UI Design: Charts (production)

Scope of the UI: charts are objects that float **over the grid** in sheet coordinates.
This doc covers the **v1-core display UI** (concrete) and sketches the **end-phase
authoring/editing UI** (structural — finalized when those phases are planned). It reuses
FreeCell's existing chrome patterns rather than inventing new ones:
`chrome/view.rs` (action bar + `.absolute()` floating overlays via gpui-component
`Popover`/`ContextMenu`/`Modal`), `shell/menus.rs` (menu bar), and the header/tab
context-menu precedent.

## 1. Where charts live

- Charts render as a **ChartLayer over the grid cells**, in sheet coordinates, so they
  scroll and zoom with the sheet (like the existing "Opening…" overlay, but sheet-anchored).
- **Z-order:** above cells + gridlines, **below** app chrome overlays (popovers, context
  menus, modals) and the selection/edit cursor. Clipped to the grid viewport.
- No separate "charts screen" — there is only the sheet; charts are in it.

## 2. v1-core display UI (read-only)

### 2.1 A rendered chart
- Drawn at its anchor rect (`twoCellAnchor` from/to → pixels); the PoC renderer paints the
  chart (title, plot, axes, legend). No border/handles in v1 core (read-only).
- Off-screen charts aren't drawn; charts partially scrolled off are clipped.

### 2.2 Compatibility warning badge (functional_spec §5)
- When a chart's `compatibility_warning` flag is set, show a **small, unobtrusive marker in a
  corner** of the chart rect — a subtle ⚠ glyph in a muted pill, not competing with the data.
- **Progressive disclosure:** hover (or click/tap) reveals a short popover — **"May not
  display as intended"** + a one-line list of *what* degraded (e.g. "3D shown as 2D",
  "gradient fill simplified", "data labels not shown"). The detail comes from the parse
  outcome the engine attached, so the message is specific, not generic.
- Placement default: **top-right** corner, inset a few px; never overlaps the title.

### 2.3 Placeholder (unsupported / parse-failed — category 3)
- A quiet bordered rectangle at the anchor, with the chart **title** (if any) and a centered
  muted line: **"Unsupported chart type"** (or "Couldn't display this chart"). It occupies
  the chart's space so layout is faithful; it never blocks opening the workbook.

## 3. Authoring UI — Stage 6.A (end-phase; structural)

> Detail finalized when Phase 6.A is planned. Conventions below follow existing chrome.

### 3.1 Insert a chart
- **Entry points** (discoverable, convention-matching): a **menu-bar** `Insert ▸ Chart ▸ <type>`
  (via `shell/menus.rs`) **and** a **right-click "Insert chart"** on a selected cell range
  (matching the header/tab context-menu precedent). Type choice via a small `Popover`/menu
  listing the in-scope types with icons.
- On choose: a chart appears anchored near the selection with FreeCell-native defaults
  (functional_spec §6.A), immediately selected.

### 3.2 Manipulate a chart object
- **Select:** click a chart → selection outline + **resize handles** (8 handles) drawn on the
  ChartLayer; click-away deselects. (New interaction on the chart layer.)
- **Move:** drag the body; snaps to nothing special (free float), anchor updates.
- **Resize:** drag a handle; anchor to/from cells update.
- **Delete:** `Delete`/`Backspace` when selected, or a context-menu "Delete chart".
- **Change type / re-range:** the selected chart's **context menu** (`Change chart type ▸`,
  `Select data…`) — re-range enters a range-pick mode (reuse the selection machinery).

## 4. Editing UI — Stage 6.B (end-phase; structural)

- When a chart is selected, a **contextual edit surface** exposes: title text, legend on/off +
  position, axis titles, series colors, data-label toggles.
- **Form factor** (open choice, §6): most likely a **selected-chart popover/panel** reusing
  the `chrome/view.rs` fill-popover + color-picker patterns; alternatively a contextual
  **action-bar** row that appears while a chart is selected (mirrors how the action bar hosts
  cell-formatting controls today). Chrome edits follow the §6 edit contract (a loaded chart
  becomes written-from-model).

## 5. UX principles

- **v1 core is near-zero-chrome:** charts just display; the only added pixel is the tiny
  degrade badge, and only when honest to show it. Low cognitive load, matches "it's a
  spreadsheet with charts in it."
- **Progressive disclosure:** the badge is a glyph → detail on demand; authoring affordances
  appear only on selection; complexity (editing) is behind selecting a chart.
- **Convention reuse:** insert via menu + right-click, handles/drag for move/resize, popovers
  for editing — all patterns FreeCell (or any spreadsheet) already uses. No new mental models.

## 6. Open UI choices (for review)
1. **Badge:** top-right corner + hover-popover detail — good? Or a different corner / a
   status-bar note instead of an on-chart glyph?
2. **Insert entry points:** menu-bar **and** right-click (recommended), or just one?
3. **Editing form factor (6.B):** selected-chart **popover panel** (recommended) vs a
   **contextual action-bar row**.
4. **Selected-chart visual:** standard 8-handle box (recommended) — any house style to match?
