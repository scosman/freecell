---
status: complete
---

# UI Design: Recent Files + Welcome Screen

Redesign of the welcome window (`crates/freecell-app/src/shell/welcome.rs`) and the macOS
File menu (`menus.rs`). The mockups define **layout and hierarchy**; **colors come from the
existing app palette**, not the mockups (§0).

## Design fidelity (read first)

The mockups are **directional, not pixel-perfect specs**. They are decent layouts drawn
without our design system — do **not** reproduce them exactly. Instead:

- Use our **standard palette tokens** (§0), never the mockups' ad-hoc hexes.
- Prefer **gpui-component controls** and the app's existing GPUI idioms (`Button`, the same
  `div()` styling patterns used in `welcome.rs` / `chrome/view.rs`) over bespoke widgets.
- All px sizes/paddings/gaps below are **approximate guidance** — round to what reads clean
  and matches neighboring chrome (the action row / titlebar spacing), not to the PNG.
- Keep the code clean and idiomatic. Favor native-feel and consistency with the rest of the
  app over faithfully matching the mockup's spacing, card radius, or shadows.

When in doubt, match the existing app, not the mockup.

## 0. Palette (reuse the app's chrome tokens)

The mockups were drawn without our design system. Map their neutrals onto the constants the
chrome/titlebar already use (`chrome/view.rs`, `titlebar.rs`) — do **not** introduce new
hexes:

| Role | Token (value) | Used for |
|---|---|---|
| Panel / secondary bg | `CHROME_BG` = `0xF3F3F3` | left pane bg, row hover bg |
| Card / surface bg | `ACTIVE_TAB_BG` = `0xFFFFFF` | right pane bg, recent card |
| Hairline | `HAIRLINE` = `0xD9D9D9` | pane divider, row separators, card border |
| Stronger divider | `DIVIDER` = `0xC8C8C8` | (if a heavier line is needed) |
| Primary text | `TEXT` = `0x1F1F1F` | wordmark, file names |
| Muted text | `MUTED_TEXT` = `0x555555` | tagline, RECENT header, subtitle, timestamp, empty state |

Buttons use gpui-component's `Button` — `.primary()` (dark) for **New Spreadsheet** and the
default/outline variant for **Open…**, exactly as today (so they already track the
gpui-component theme). The welcome's existing `BG`/`CARD_BG` constants are realigned to the
values above (`BG` → `CHROME_BG` `0xF3F3F3`).

> These constants are currently duplicated between `titlebar.rs` and `chrome/view.rs`;
> `welcome.rs` mirroring the same values matches that established pattern. Extracting a
> shared `shell` color module is a reasonable future cleanup but is **out of scope** here
> (it would touch the render-tested titlebar/chrome) — capture it in `PROJECTS.md` if
> desired.

## 1. Window

- The welcome window grows from the current fixed **420×300** single column to a fixed
  **720×480** two-pane layout (`app.rs welcome_window_options`). Stays **non-resizable,
  non-minimizable, centered**; macOS custom titlebar (§7.1 of the MVP) or Linux server
  decorations, unchanged.
- Root is a horizontal flex filling the window (below the optional titlebar row): left pane
  fixed width **~264 px**, right pane `flex_1`, separated by a 1 px `HAIRLINE` vertical rule.

## 2. Left pane

`CHROME_BG` background, ~32 px padding, vertical flex:

```
┌───────────────────────┐
│  FreeCell             │  ← wordmark, bold ~28 px, TEXT
│  The open spreadsheet │  ← tagline, ~13 px, MUTED_TEXT
│                       │
│  [ + New Spreadsheet ]│  ← primary Button, full pane width
│  [     Open…        ] │  ← outline Button, full pane width
└───────────────────────┘
```

- Wordmark + tagline top-aligned; a gap, then the two buttons stacked with a ~12 px gap,
  each `w_full` within the pane. (Mockup shows a leading `+` glyph on New Spreadsheet — use
  the gpui-component button icon if trivial, otherwise the label alone; not load-bearing.)
- Enter still triggers New; Cmd/Ctrl+O still triggers Open (existing key bindings).

## 3. Right pane

`ACTIVE_TAB_BG` (white) background, ~24–28 px padding, vertical flex:

- **Header:** `RECENT` — small (~11 px), `MUTED_TEXT`, uppercase, slight letter-spacing,
  bottom margin.
- **Body:** either the recent list or the empty state.

### 3.1 Recent list

Up to 5 rows in a single card (1 px `HAIRLINE` border, rounded ~8 px), rows separated by 1 px
`HAIRLINE` hairlines (no separator under the last row). **Pure text — no icons/glyphs.** Each
row is a clickable horizontal flex, ~56 px tall, ~14 px inner padding:

```
┌───────────────────────────────────────────────────────────┐
│ Q3 Revenue Forecast.xlsx                          2h ago   │
│ 1.2 MB · Downloads                                         │
└───────────────────────────────────────────────────────────┘
```

- **Left / middle** (`flex_1`, min-width 0 so it truncates): name on top (~14–15 px, `TEXT`,
  semibold, single-line truncate), subtitle below (~12–13 px, `MUTED_TEXT`, single-line
  truncate) = `"{size} · {folder}"`. No leading icon.
- **Right:** relative timestamp (~12–13 px, `MUTED_TEXT`), right-aligned, does not shrink.
- **Hover:** row background → `CHROME_BG`; cursor pointer. Whole row is the click target.
- **Click:** `FreeCellApp::open_path(path)`.

### 3.2 Empty state

Centered in the body under the `RECENT` header — **pure text, no glyph/icon**:

```
      No recent spreadsheets
  Create a new spreadsheet or open
        a file to get started.
```

- **"No recent spreadsheets"** — ~15 px, semibold, `TEXT`.
- **"Create a new spreadsheet or open a file to get started."** — ~13 px, `MUTED_TEXT`,
  centered, wrapped.

## 4. File → Open Recent submenu (macOS)

```
File
  New
  Open…
  Open Recent  ▸  Q3 Revenue Forecast.xlsx
  ───────────    Team Headcount 2026.xlsx
  Save           …(up to 10, most-recent first)
  Save As…       ───────────
  ───────────    Clear Recent Files
  Close Window
```

- Submenu inserted directly after **Open…**.
- Item label = file name only. Selecting opens via `FreeCellApp::open_path`.
- When empty: a single **disabled** `No Recent Files` item, no separator/Clear.
- Linux: no menu bar (unchanged).

## 5. UX notes

- The list is read-only in this iteration — no per-row delete/pin/reveal (progressive
  disclosure; those are backlog items). Whole-row click is the single obvious affordance.
- Missing files never appear (silent prune), so a click always targets an existing file —
  no "dead" rows to confuse the user.
- Copy is fixed strings; the only dynamic text is names/sizes/folders/timestamps.

## 6. About window

Standalone window (Phase 4), replacing the About modal. **Directional mockup — build on our
design system** (§ "Design fidelity"). Reuse the palette tokens (§0) plus one link token.

- **Window:** ~460×340, fixed, non-resizable, non-minimizable, centered; same titlebar
  treatment as the welcome window (macOS custom titlebar / Linux decorations). Background
  `CHROME_BG` (`0xF3F3F3`), matching the mockup's light card.
- **Layout:** a single vertical flex, ~28–32 px padding.

```
┌────────────────────────────────┐
│            FreeCell            │  ← wordmark, bold ~28 px, TEXT
│      The open spreadsheet      │  ← tagline, ~13 px, MUTED_TEXT
│           Version 0.1.0        │  ← ~12 px, lighter MUTED_TEXT, centered
│  ───────────────────────────   │  ← 1 px HAIRLINE, horizontal margin
│  Homepage   github.com/scos…   │  ← label (MUTED_TEXT) · value (LINK)
│  Built with     IronCalc · GPUI│  ← label (MUTED_TEXT) · two LINKs + "·" (MUTED_TEXT)
└────────────────────────────────┘
```

- **Top block** (centered): wordmark → tagline → version, tight vertical rhythm.
- **Hairline:** `HAIRLINE` (`0xD9D9D9`), full content width (minus padding), generous space
  above/below.
- **Rows:** each a horizontal flex — label on the left (`MUTED_TEXT`), value right-aligned.
  The "Built with" value is `IronCalc` · `GPUI`, each a link with a `MUTED_TEXT` "·"
  separator between.
- **Link token:** our design system has no link color, so introduce **one** `LINK` constant
  (a blue in the app's spirit, e.g. `0x2563EB`) — do not copy the mockup's exact hex, and do
  not sprinkle per-link colors. (If gpui-component's theme exposes an accent/link color at
  the pinned rev, prefer that over a constant.) Links get a pointer cursor on hover; an
  underline-on-hover is optional and fine to add.
- Clicking a link opens its URL in the default browser.
