---
status: complete
---

# UI Design: FreeCell MVP

Design philosophy (from the overview): **functional proof of concept**. Stock
gpui-component controls for all chrome, sensible spacing, no opinionated styling — with
one exception: **the grid is ours and gets real design attention. It should look like a
good spreadsheet.**

Base theme: gpui-component's default **light** theme throughout — **light-only for
MVP** (decided, UI round). No dark mode (chrome would follow the theme, but the grid
ships light-only; P2).

## 1. Window inventory

| Window | Size | Resizable | Count |
|---|---|---|---|
| Welcome | ~460×320, centered | No | 0 or 1 |
| Spreadsheet | 1200×800 default, min 640×480 | Yes | 0..n (one per workbook) |

Standard macOS title bars and traffic lights on both. Menu bar per
`functional_spec.md §2.4`.

## 2. Welcome window

Simple vertical stack, centered content, comfortable padding (~32 px):

```
┌──────────────────────────────┐
│                              │
│          FreeCell            │  ← app name, large text
│   Fast, GPU-rendered         │  ← one-line tagline, muted
│      spreadsheets            │
│                              │
│   [  New Spreadsheet  ]      │  ← primary button
│   [      Open…        ]      │  ← secondary button
│                              │
└──────────────────────────────┘
```

- Stock gpui-component `Button`s (one primary, one outline/secondary), stacked, equal
  width (~200 px), 12 px gap.
- No imagery/logo asset in MVP; styled text is the logo.
- Enter triggers New; Cmd+O triggers Open.

## 3. Spreadsheet window layout

Vertical flex, full-window:

```
┌────────────────────────────────────────────────────────────┐
│  [B][I][U] │ [Fill ▾]                    (action row) (⟳)  │  ~36 px, grey bg
├────────────────────────────────────────────────────────────┤
│  [ B7 ]  │ [ =SUM(A1:A5)                             ]     │  ~32 px, grey bg
├────────────────────────────────────────────────────────────┤
│    │  A   │  B   │  C   │  D   │  E   │  F   │  G   │ ...  │ ┐
│  1 │      │      │      │      │      │      │      │      │ │
│  2 │      │ 42.5 │      │      │      │      │      │      │ │ grid: full bleed,
│  3 │      │      │      │      │      │      │      │      │ │ fills remaining
│ ...│      │      │      │      │      │      │      │    ▓ │ │ space
│    ├──────┴──────┴──────┴──────┴──────┴──────┴─────▓▓▓─────┤ │
├────┴───────────────────────────────────────────────────────┤ ┘
│ ⟨Sheet1⟩ ⟨Sheet2⟩ ⟨Sales⟩  [+]              (sheet tabs)   │  ~30 px, grey bg
└────────────────────────────────────────────────────────────┘
```

The action row, data row, and tab bar share one flat grey background
(gpui-component's standard secondary/panel background token) with a 1 px hairline
separating each from the grid. The grid is the only white, full-bleed surface.

### 3.1 Action row

- Left-aligned group of three **toggle buttons**: **B** (bold glyph), *I* (italic),
  U̲ (underline) — stock gpui-component toggle/ghost buttons, small size (~28 px
  square), 4 px gaps. Pressed state = the active cell has that attribute.
- Thin vertical divider, then the **Fill** button: paint-bucket icon (or "Fill" label
  if no suitable icon ships with gpui-component) with a dropdown chevron. Click opens a
  small popover palette (decided, UI round):
  - **Swatch grid — the Office default theme palette**, for consistency with existing
    spreadsheets (constants live in `freecell-core::palette`):

    | Name | Hex | | Name | Hex |
    |---|---|---|---|---|
    | Background 1 | `#FFFFFF` | | Accent 2 | `#ED7D31` |
    | Text 1 | `#000000` | | Accent 3 | `#A5A5A5` |
    | Background 2 | `#E7E6E6` | | Accent 4 | `#FFC000` |
    | Text 2 | `#44546A` | | Accent 5 | `#5B9BD5` |
    | Accent 1 | `#4472C4` | | Accent 6 | `#70AD47` |

  - A **No fill** entry (clears the background).
  - A **Custom…** entry opening gpui-component's **ColorPicker**; the picked RGB
    applies like any swatch (engine stores an arbitrary `#RRGGBB`).
  - Click a swatch / pick a color → applies to selection, popover closes.
- Buttons disabled when a data-row edit is mid-flight? No — formatting commits the
  pending edit first (same rule as clicking another cell), keeping behavior uniform.
- Tooltips on all four ("Bold ⌘B" etc. — see §6 shortcuts).
- **Evaluating spinner**: right-aligned at the far end of the action row (top-right of
  the chrome). Hidden by default; appears when an evaluation has been in flight
  > 250 ms and stays until completion (`functional_spec.md §4`). Stock gpui-component
  spinner, small, no label.

### 3.2 Data row (formula bar)

- **Cell reference box**: fixed-width (~72 px) read-only field showing `B7` or the
  range `B2:D9`. Stock text-input styling, non-editable in MVP.
- Thin divider, then the **content field**: single-line stock gpui-component text
  input, stretching to fill the row. Monospace-leaning is unnecessary; default UI font.
- Multi-cell selection: content field disabled + empty (greyed background).
- Inline error state (input-cap rejection): field border switches to the theme's
  danger color, error text appears in a tooltip-style popover below; clears on edit.

### 3.3 Grid — the component we design properly

The one custom-built component (raw GPUI, per the adopted architecture — matching
`experiments/04-ui-poc/raw-gpui` mechanics). Visual spec:

**Chrome**
- **Column headers** (top strip, ~24 px tall) and **row headers** (left gutter,
  ~48 px wide, grows if needed for 7-digit row numbers): light grey fill
  (`#F5F5F5`-class token), 1 px hairline borders (`#D9D9D9`-class), centered labels in
  a small (11–12 px) medium-weight UI font, muted-dark text. Top-left corner cap where
  they meet.
- Headers of the active selection's rows/columns get a slightly darker tint + accent
  underline/side-bar (2 px accent line on the header's grid-facing edge) — the standard
  "you are here" affordance in every good spreadsheet.
- Headers are **fixed** (don't scroll out); content scrolls under them.

**Font (decided, UI round): bundled Inter.** The grid's cell + header text uses
**Inter** (SIL OFL), shipped in the app bundle and registered at startup via GPUI's
`add_fonts` — not the system font. Rationale: pixel-stable render-test baselines
across machines/OS updates (font-version drift was round-3 C's top flakiness risk)
and a clean, tabular-figures-friendly face. Chrome outside the grid keeps the
gpui-component theme font.

**Cells**
- White background; **gridlines** as 1 px light grey (`#E2E2E2`-class) lines. Gridlines
  render *under* cell fills (a filled cell paints over its gridlines, like Excel).
- Default cell: 13 px Inter, dark-grey/near-black text, vertically centered,
  4 px horizontal padding, text clipped at the cell edge (`functional_spec.md §3.6`).
- Alignment, bold/italic/underline, fill color, number-format text/color per the
  functional spec.
- Default geometry: column width 100 px, row height 24 px (file overrides honored;
  IronCalc's unit conversions handled in the engine layer).

**Selection** (accent = gpui-component **primary blue** token everywhere — decided,
UI round)
- **Active cell**: 2 px solid accent-blue border (gpui-component primary token) drawn
  on top of gridlines, square corners, no glow/shadow.
- **Range**: accent-blue at ~10% opacity overlaying the range's cells, 1.5–2 px accent
  border around the range's outer rectangle; the active/anchor cell inside the range
  keeps its normal background (the Excel "white anchor" look).
- Selection renders above fills, below nothing (it's the topmost cell-layer element).

**Scrollbars**
- Slim (~10 px) overlay scrollbars on right + bottom edges, rounded thumb,
  semi-transparent; proportional to viewport/total-extent; draggable; fade when idle if
  cheap to do, otherwise always-visible is acceptable for MVP. Use gpui-component's
  scrollbar if it can be driven by our custom virtual scroll model; otherwise draw our
  own (two rects + drag handling) — decided at implementation.

**Empty expanse**
- The sheet is Excel-max sized; beyond the used range it's just white cells +
  gridlines + headers, scrollable to the true edge, rendered from the same
  virtualization (no special casing visible to the user).

**Loading state (file open)**
- Grid area shows a centered spinner + "Opening *name*…" over a blank grid; chrome
  (headers/toolbar) may render disabled. Window is closable to cancel.

### 3.4 Sheet tab bar

- Stock gpui-component tab strip if its API allows bottom placement + trailing button;
  otherwise a simple custom row of tab-shaped buttons (this is chrome — don't
  over-invest).
- Active tab: white background (connects visually to the grid above), medium-weight
  label. Inactive: grey background, regular weight. Height ~30 px, label padding
  12 px, 13 px font.
- `+` button at the end of the tabs (ghost icon button).
- Double-click a tab → the label swaps to an inline text input (same footprint);
  Enter/blur commits, Esc cancels, invalid names shake/danger-border and revert
  (validation per `functional_spec.md §3.7`).
- Right-click → stock context menu: **Rename**, **Delete** (disabled when it's the
  last sheet).
- Overflow (many sheets): tabs scroll horizontally within the bar (wheel/trackpad);
  no overflow menu in MVP.

## 4. Dialogs & alerts

All stock gpui-component modal/dialog components or native macOS panels:

| Dialog | Type | Notes |
|---|---|---|
| Open file | **Native** NSOpenPanel (macOS) / GPUI paths-prompt (Linux) | `.xlsx` filter |
| Save As | **Native** NSSavePanel (macOS) / GPUI paths-prompt (Linux) | enforces `.xlsx` |
| Unsaved changes on close | gpui-component modal | Save / Don't Save / Cancel; Save routes through Save/Save As |
| Open/Save failure | gpui-component modal | file name + reason, single OK |
| Delete sheet confirmation | gpui-component modal | only when sheet has content |
| About FreeCell | gpui-component modal | name, version, one-liner |

If GPUI's native-panel bindings at the pinned rev are awkward, gpui-component file
dialogs are an acceptable fallback — native preferred.

## 5. Navigation model

- **Welcome → documents**: Welcome opens at launch only; any document window opening
  closes it. Closing the last window (Welcome or document) quits the app. No other
  global navigation.
- **Within a document**: single screen; sheet tabs are the only intra-document
  navigation. No panels, sidebars, or settings surfaces in MVP.
- **Focus model**: exactly one of {grid, data-row field, tab-rename field} holds
  keyboard focus. Click grid → grid focus. Click/type into data row → field focus
  (editing state). Tab-rename focus is transient. Grid focus is the default after
  every commit/cancel.

## 6. Keyboard map (MVP-complete list)

| Keys | Context | Action |
|---|---|---|
| Arrows | grid | move active cell |
| Shift+Arrows | grid | extend range |
| Cmd+Arrow | grid | jump to sheet edge |
| Tab / Shift+Tab | grid, data row | commit (if editing) + move right/left |
| Enter / Shift+Enter | grid, data row | commit (if editing) + move down/up |
| Escape | data row | cancel edit |
| Delete/Backspace | grid | clear selected cells |
| Cmd+B / Cmd+I / Cmd+U | grid | toggle bold/italic/underline on selection |
| Cmd+Z / Cmd+Shift+Z | window | undo / redo |
| Cmd+N, Cmd+O, Cmd+S, Cmd+Shift+S, Cmd+W, Cmd+Q | app | menu equivalents |
| Page Up/Down | grid | move one viewport-height up/down |
| Home / Cmd+Home | grid | column A in current row / cell A1 |

**Linux**: identical map with **Ctrl replacing Cmd** (defined once via per-platform
keymaps, same actions). Since Linux has no menu bar in MVP, these shortcuts are the
only path to menu actions there.

Typing a printable character with grid focus does **not** start an edit in MVP (no
in-cell editing); users click/focus the data row. (Deliberate scope cut; revisit with
the in-cell editor.)

## 7. Component inventory (build vs stock)

| Component | Source |
|---|---|
| Grid (headers, cells, selection, scroll, scrollbars) | **Custom** (raw GPUI; port of the raw-gpui POC rendering approach) |
| Toggle buttons, fill button + popover palette | Stock gpui-component (`Button`, popover/menu, `ColorPicker` for Custom…) |
| Grid/cell font | Bundled **Inter** (SIL OFL), registered via `add_fonts` at startup |
| Text inputs (data row, tab rename) | Stock gpui-component `TextInput` |
| Sheet tab bar | Stock if fits, else thin custom row of buttons |
| Modals/alerts | Stock gpui-component |
| File pickers | Native macOS panels; GPUI's platform paths-prompt on Linux |
| Menu bar | GPUI native menu API (macOS); none on Linux in MVP — shortcuts only |
| Spinner/indicator | Stock gpui-component |

Spacing rhythm: 8 px base unit, 4 px within tight button groups, 12+ px around window
padding — applied via gpui-component defaults wherever they exist; do not build a
styling system.
