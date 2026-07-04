---
status: draft
---

# UI Design: MVP Gaps — Core Spreadsheet Feel

Extends `specs/projects/mvp/ui_design.md` (colors, spacing, and existing controls
unchanged unless restated). All new chrome uses existing tokens: `CHROME_BG 0xF3F3F3`,
`HAIRLINE 0xD9D9D9`, `DIVIDER 0xC8C8C8`, accent per MVP.

## 1. Titlebar (macOS)

```
┌────────────────────────────────────────────────────────────┐
│ ●●●            Budget.xlsx — Edited                        │  36px, CHROME_BG
├────────────────────────────────────────────────────────────┤  HAIRLINE
│ [Inter ▾][12 ▾] │ B I U │ [A▾][▨▾] │ [⊞▾] │ [⟸⟺⟹] │ [123▾][.00±]  ⟳ │  action row
```

- 36 px tall, `CHROME_BG`, bottom `HAIRLINE`. Traffic lights repositioned to center
  vertically (x-inset per macOS HIG ≈ 12 px). Title text centered, 13 px, medium,
  `#3C3C3C`; shows `Name` or `Name — Edited` (replaces the window-frame title; the
  close-button dot still works where the OS provides it).
- Entire row = window drag region. No custom buttons in it.
- Welcome window: same treatment, title "FreeCell".
- Linux: this row is not rendered; server decorations as today.

## 2. Action row (final layout)

Order: **Font family** (dropdown, 140 px, shows active cell's family, top entry
"System Default") · **Size** (dropdown, 56 px, fixed list) · divider · **B I U**
(existing) · divider · **Text color** (glyph "A" with color underline swatch, opens
the same palette popover as Fill, with **Automatic** instead of *No fill*) · **Fill**
(existing) · divider · **Borders** (grid glyph, popover with 8 preset icons in a 4×2
grid: All, Inner, Outer, None / Top, Bottom, Left, Right) · divider · **Alignment**
(3-button toggle group; active cell's effective explicit alignment shown pressed;
pressing the pressed one clears) · divider · **Number format** (dropdown, 92 px,
shows current category name; entries: General, Number, Currency, Percent, Date,
Time, Text) · **Decimals** (two small buttons `.00→` / `→.00`) · spacer · existing
eval spinner.

If the row overflows at narrow widths, trailing groups (decimals, alignment) wrap is
NOT supported — the window's min width raises to fit the row (simple, no overflow
menu in this project).

All dropdowns/popovers are gpui-component menus consistent with the existing Fill
popover. Disabled state (degraded mode / Welcome): 40% opacity, non-interactive.

## 3. Grid interactions

- **In-cell editor**: overlay exactly on the cell rect (min-width 80 px, expands
  rightward over neighbors, 2 px accent border, white bg, cell-default font 13 px,
  1 px inner padding matching cell padding so text doesn't shift on open). Danger
  border variant on cap reject + popover below (§4).
- **Live mirror**: pending raw text renders in the active cell, left-aligned, default
  font, no style — visually distinct from committed values only in that it updates
  per keystroke.
- **Resize**: 6 px-wide hotspot centered on each header divider; cursor
  `col-resize`/`row-resize`; while dragging, a 1 px accent guide line spans the
  viewport at the drag edge and the header shows a live size tooltip (`Width: 96`).
  Layout reflows live during drag.
- **Header selection**: selected full rows/cols tint their headers `HEADER_SELECTED_BG`
  and the range overlay spans the viewport. Select-all corner: existing corner cell
  becomes clickable (subtle hover tint).
- **Header context menu** (right-click a row/col header): `Insert 1 row above`,
  `Insert 1 row below`, `Delete 1 row` (counts/pluralization per selection; column
  variant uses left/right). Menu items disable with a tooltip when the merge guard
  applies.

## 4. Popovers & messages

- **Cap-error popover**: small dark tooltip (matching existing tooltip style) anchored
  below the active editor's left edge; auto-dismiss on keystroke/focus change.
- **Paste overflow / structural-edit failure / backup failure**: existing modal dialog
  style, single OK (backup failure: "Couldn't create backup — file not saved.").
- **Merge guard dialog**: OK-only, text per functional spec §5.3.

## 5. Cursors

`col-resize` / `row-resize` on divider hotspots only; default elsewhere (headers keep
pointer-default, not a selection cursor, in this project).
