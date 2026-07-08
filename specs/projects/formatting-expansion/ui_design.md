---
status: complete
---

# UI Design: Formatting Expansion

Covers the toolbar (chrome action row) additions for Part 1 and the redesigned
borders popover for Part 2. The grid-rendering behavior (strike line, wrapped text,
vertical text placement, patterned borders) is specified in the functional spec; this
doc is the **control surface**.

Context: the action row is one flex row of `gpui_component` `Button`s grouped by thin
vertical dividers. There is **no icon-font / SVG system** — existing "icons" are Unicode
glyphs passed as a button `.label(...)` (e.g. align L/C/R = `⇤ ≡ ⇥`). New controls follow
that convention; the only genuinely drawn icons are the border 2×2 diagrams (§2.2), which
are composed from `div` rectangles (the same primitive the grid uses to paint borders).

---

## 1. Toolbar (action row) additions

### 1.1 New group order

```
[ Font family ▾ ][ Size ▾ ] │ B  I  U  S  ⤶ │ A▾  Fill▾ │ ⊞▾(Borders) │ ⇤ ≡ ⇥ │ ⤒ ⯀ ⤓ │ Number… │ ……spinner
                              └── text style ──┘                         └ H-align ┘ └ V-align ┘
```

Two insertions, both following existing patterns:

- **Strikethrough `S` + Wrap `⤶`** appended to the existing **B / I / U** toggle group
  (no new divider — they sit right after Underline, per "after underline / after
  strikethrough"). Built with the same `toggle(...)` closure as B/I/U → they get the
  identical ghost/small/selected pressed-state look.
- **Vertical align `⤒ ⯀ ⤓`** (Top / Center / Bottom) as a **new divider-bracketed group
  immediately after the horizontal align group** (`⇤ ≡ ⇥`). Built with the same
  `align_btn(...)` closure shape as horizontal align (radio-style, `.selected(...)`
  reflects the explicit value).

### 1.2 Glyphs (Unicode, to eyeball during build)

| Control | Candidate glyph | Tooltip | Notes |
|---------|-----------------|---------|-------|
| Strikethrough | `S̶` (S + U+0336) or plain `S` | "Strikethrough" | Matches the B/I/U letter convention; the overlay glyph reads clearer than a bare `S`. |
| Wrap text | `⤶` (U+2936) or `↵` (U+21B5) | "Wrap text" | Reads as "text turns onto next line". |
| V-align top | `⤒` (U+2912) | "Align top" | Mirrors the H-align arrow language (`⇤ ≡ ⇥`). |
| V-align center | `⯀` / `≡`-style center mark | "Align middle" | A centered bar; pick whichever reads as "middle" next to top/bottom. |
| V-align bottom | `⤓` (U+2913) | "Align bottom" | |

**Fallback if a glyph is illegible at 12px:** compose a tiny icon from `div`s (a cell
box with a short bar at top / middle / bottom) using the same technique as the border
icons (§2.2). Decide per-glyph when eyeballing render baselines — do not ship an
ambiguous glyph.

### 1.3 States

- All five text-style toggles (`B I U S ⤶`) show the pressed look when the active cell
  has that attribute (`RenderStyle.bold/italic/underline/strikethrough/wrap`).
- V-align buttons: pressed only when the active cell has that vertical alignment set
  **explicitly** (`RenderStyle.v_align == Some(x)`); none pressed when unset — exactly
  like horizontal align.
- All disabled (greyed) when the worker is degraded/read-only, like the existing group.

---

## 2. Borders popover (redesign)

Anchored under the `⊞▾` Borders button, same as today, but restructured from an 8-button
text grid into three stacked regions. **The popover no longer closes on a target click**
— only click-away / Esc closes it (the card is `.occlude()`d so inner clicks don't
dismiss).

### 2.1 Layout

```
┌──────────────────────────────────────────────┐
│  Which lines                                   │   ← muted section label
│   ┌───┐ ┌───┐ ┌───┐ ┌───┐                      │
│   │▦  │ │ ┼ │ │ ▢ │ │   │      All Inner Outer None
│   └───┘ └───┘ └───┘ └───┘                      │
│   ┌───┐ ┌───┐ ┌───┐ ┌───┐                      │
│   │ ▔ │ │ ▁ │ │▏  │ │  ▕│      Top Bottom Left Right
│   └───┘ └───┘ └───┘ └───┘                      │
│  ────────────────────────────────────────     │   ← divider
│  Line                                          │
│   ┌────┐┌────┐┌────┐┌────┐┌────┐               │
│   │──  ││━━  ││▬▬  ││┅┅  ││══  │   thin med thick dashed double
│   └────┘└────┘└────┘└────┘└────┘               │
│  Color                                         │
│   ■ ■ ■ ■ ■  ■ ■ ■ ■ ■   [ Custom ▾ ]          │   ← FILL_PALETTE + ColorPicker (reused)
└──────────────────────────────────────────────┘
```

- **Region A — "Which lines":** the 8 target icons in the same 4×2 arrangement as today
  (row 1: All / Inner / Outer / None; row 2: Top / Bottom / Left / Right). Icon-only, so
  **each carries a tooltip** with its name for discoverability.
- **Region B — "Line":** the line-style **gallery** — 5 small buttons, each a live preview
  of the actual line (thin / medium / thick solid, dashed, double). The current pen's
  style is shown `.selected`.
- **Region C — "Color":** the existing color control, reused verbatim from the Fill
  popover — the 10-swatch `FILL_PALETTE` grid + a `ColorPicker` "Custom…" affordance
  (inline, no nested popover). The current pen color's swatch is marked selected.

### 2.2 Border target icons (the 2×2 diagrams)

One parameterized component: a ~22px square drawing a **2×2 grid of mini-cells**. Every
gridline is drawn thin **light-grey** as context; the edges the target affects are drawn
**solid dark** (heavier). Implemented with `div` rectangles — the same primitive the grid
already uses for border quads.

| Target | Dark (affected) segments | Reads as |
|--------|--------------------------|----------|
| **All** | all outer + the inner cross | every line |
| **Inner** | inner cross only (mid-H + mid-V) | interior only |
| **Outer** | outer perimeter only | box |
| **None** | *(none dark — all grey)* | no borders |
| **Top** | top outer edge | top |
| **Bottom** | bottom outer edge | bottom |
| **Left** | left outer edge | left |
| **Right** | right outer edge | right |

Selected target: the button shows the standard `.selected` pressed background/ring.

### 2.3 Line-style gallery previews

Each gallery button renders a short horizontal sample of the real line at ~2px scale:

| Entry | Preview | Maps to |
|-------|---------|---------|
| Thin | 1px solid | `BorderStyle::Thin` |
| Medium | 2px solid | `BorderStyle::Medium` |
| Thick | 3px solid | `BorderStyle::Thick` |
| Dashed | 2px dashed | `BorderStyle::MediumDashed` |
| Double | two 1px lines | `BorderStyle::Double` |

(Dotted / dash-dot deliberately absent — see functional spec / GAPS F3.)

### 2.4 Interaction (the pen model, restated as UI states)

- **On open:** no target `.selected`; Line shows Thin selected; Color shows black
  selected (the default pen). Nothing on the sheet is touched yet.
- **Click a target icon:** it becomes `.selected`; the current pen is painted onto just
  those edges; popover stays open.
- **Click a Line preview / a Color swatch (with a target selected):** updates the pen and
  repaints the selected target's edges only.
- **Click a different target:** selection moves; the (carried-over) pen paints the new
  target.
- **Click None:** clears all borders in the selection; leaves **no** target selected.
- **Click-away / Esc:** closes; transient target + pen discarded.
- **No-target + change Line/Color (MVP):** updates the pen only; no sheet change until a
  target is clicked. *(P2 upgrades this to restyle all existing borders — GAPS F2.)*

**Optional discoverability aid (for review):** when no target is selected, show a faint
one-line hint under Region A — e.g. *"Pick which lines, then style them."* — since in MVP
changing Line/Color before picking a target has no visible effect. Low clutter; drop if
you'd rather keep the card minimal.

---

## 3. UX rationale

- **Progressive disclosure / low cognitive load:** the target icons answer *where*, the
  Line/Color row answers *how* — two small decisions, read top-to-bottom. The pen model
  means you never edit a modal "border editor"; you point at edges and paint.
- **Discoverability:** icon-only targets are backed by tooltips; the Line previews and
  Color swatches are self-describing.
- **Platform convention:** mirrors Excel's Border tab (a where-grid + a line style/color
  section) and FreeCell's own existing popover/toggle idioms — nothing new to learn.
- **Non-destructive:** painting a target never disturbs other edges, so a mixed-border
  range can be refined piece by piece — the core requirement.

## 4. Render-test impact

Every item here moves pixels: 5 new toolbar buttons, the strike line, wrapped and
vertically-placed text, the redesigned popover, the 2×2 icons, the gallery previews, and
dashed/double border rendering. New render-test cases + refreshed, eyeballed baselines are
required, and the CI `render` gate must be dispatched green before merge (tracked in the
implementation plan).
