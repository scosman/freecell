---
status: complete
---

# UI Design: Feature Gaps 7_11

Extends `specs/projects/mvp/ui_design.md` + `specs/projects/mvp-gaps/ui_design.md` (tokens,
spacing, existing controls unchanged unless restated). Reuses existing tokens: `CHROME_BG
0xF3F3F3`, `HAIRLINE 0xD9D9D9`, `DIVIDER 0xC8C8C8`, accent per MVP. Only surfaces this
project **changes or adds** are covered here; §1 font-warning, §5 quick-edit, and §7
verify-only have no new visual chrome.

## 1. Find/replace bar (§4)

A new horizontal bar in the chrome's vertical stack, **directly below the data/formula
row** and above the grid body (pushes the grid down; not an overlay). Height ≈ the data
row (`DATA_ROW_H 32`), `CHROME_BG`, bottom `HAIRLINE`.

```
├──────────────────────────────────────────────────────────────────────────┤ HAIRLINE
│ 🔍 [ Find……………… ] [ Replace with…… ]  [Aa] [▢] │ ↑ ↓  3 of 12 │ Replace  Replace All   ✕ │ 32px
├──────────────────────────────────────────────────────────────────────────┤ HAIRLINE
```

Left→right:
- **Find field** — gpui-component `Input`, ~220 px, placeholder "Find". Focused on open,
  existing text selected.
- **Replace field** — `Input`, ~220 px, placeholder "Replace with".
- **Match-case toggle** `Aa` — small ghost toggle button, `selected` when on (accent tint,
  same style as the action-row B/I/U toggles). Tooltip "Match case".
- **Match-entire-cell toggle** `▢` — small ghost toggle, tooltip "Match entire cell".
- Thin `DIVIDER`.
- **Prev / Next** — two small ghost icon buttons (chevron-up / chevron-down, reuse the
  bundled `chevron-*` icons), tooltips "Previous match (⇧⏎)" / "Next match (⏎)".
- **Match counter** — 13 px, `#3C3C3C`, min-width so it doesn't jitter: `"3 of 12"`,
  `"No results"` (muted), or empty when the find field is empty.
- Spacer (`flex_1`) pushes the trailing group right.
- **Replace** / **Replace All** — small ghost text buttons; disabled (40% opacity) when no
  current match / no matches respectively.
- **Dismiss ✕** — small ghost icon button on the far right (reuse the bundled `square-x` /
  an `x` glyph), tooltip "Close (Esc)".

States:
- **Disabled buttons:** prev/next/replace/replace-all are non-interactive at 40% opacity
  when there is nothing to act on (empty find, or no matches).
- **No results:** counter shows "No results" in muted color; nothing selected in the grid.
- **Narrow window:** the bar does not wrap; the window min-width already accommodates the
  action row, which is wider — so the find bar fits within existing min-width. If it
  somehow exceeds, the two `Input`s shrink first (flex), fields never below ~120 px.

Match reveal in the grid: the current match cell uses the **normal selection** visuals
(active-cell outline + scroll-into-view). No separate highlight overlay in this batch —
the selected cell *is* the current match.

## 2. Action-row search button (§4)

Add a **search** trigger to the action row (`render_action_row`), at the **trailing** end
just before the spacer/spinner (grouped after the insert-chart control, behind a
`action_divider()`): a small ghost icon button using a bundled `search.svg` (magnifier).
Tooltip "Find & Replace (⌘F)". `selected` (accent tint) while the find bar is open, so it
reads as a toggle. Clicking it toggles the bar open/closed (same as ⌘F / Esc).

`search.svg` is a new vendored Lucide stroke icon (`stroke="currentColor"`, tintable) added
to `assets/icons/` + `FREECELL_ICONS` per the existing icon convention.

## 3. Sheet tab drag-to-reorder (§6)

On the existing tab bar (`render_tab_bar`, `TAB_BAR_H 30`):

- **Drag affordance:** press-and-drag a tab horizontally. The pressed tab lifts subtly
  (raise elevation: slightly stronger bg / 1 px accent outline, optional ~90% opacity) and
  tracks the cursor horizontally within the strip.
- **Drop indicator:** a **2 px accent vertical bar** rendered at the insertion gap between
  tabs (the boundary nearest the cursor). It snaps between tab slots as the cursor moves.
- **Cursor:** `grabbing` (or default if unavailable) during drag.
- **Threshold:** a click that moves less than ~4 px is treated as a select (existing
  behavior), not a drag; double-click still opens rename. Only past the threshold does the
  lift/indicator appear.
- **Drop:** release commits the move to the indicated index; on drop back to the origin
  slot, no-op. Tabs re-render in engine order after the worker confirms `SheetsChanged`.
- The `+` add-sheet button and right-click menu are unaffected; you cannot drop past the
  `+`.

## 4. Spill rendering (§2)

No new chrome — a change to how existing cell text paints:

- Overflowing **text** in a wrap-off cell renders beyond its column into adjacent empty
  cells, in the alignment-determined direction (§2.2). The text uses the origin cell's
  font/size/color/vertical-alignment.
- The neighbor cells' gridlines and fills still render; the spilled text paints **on top**
  of the (empty) neighbors, clipped only at the first non-empty cell and at the grid
  content viewport edge.
- The origin cell's own borders/fill/selection outline are unchanged — the active-cell
  outline never extends over the spill.
- Visually indistinguishable from Excel: one continuous run of text crossing thin
  gridlines.

## 5. Auto-grow (§3)

No new chrome — rows simply render taller. The row-divider resize hotspot/guide/tooltip
(mvp-gaps ui §3) is unchanged; dragging a divider marks the row **manual** (auto-grow no
longer touches it). No indicator distinguishes auto vs manual rows in this batch.

## 6. Cursors

Adds `grabbing` on a tab during an active reorder drag (§3). Everything else unchanged.
