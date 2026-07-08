---
status: complete
---

# Functional Spec: Formatting Expansion

Adds a batch of cell-formatting options to FreeCell, following the existing
formatting patterns (bold/italic/underline, align L/C/R, the borders popover).

Two parts, buildable independently:

- **Part 1 — Text formatting:** strikethrough, wrap text, vertical alignment.
- **Part 2 — Border formatting:** line style + color, applied through a redesigned
  borders menu with a per-target "pen" modality.

## Decisions locked at spec time

- **Border weight × style → combined "line style" gallery (option 1A).** The engine
  (IronCalc/OOXML) does **not** model weight and pattern as an independent
  cross-product — it has a fixed set of ~9 `BorderStyle` variants (thin/medium/thick
  solid, dotted, medium-dashed, double, dash-dot…). There is no "thin dashed" or
  "thick dotted". So instead of separate weight + style controls, the menu offers a
  single **line-style gallery** (each entry a small line preview that bakes in weight
  + pattern), plus a separate **color** control. This is exactly how Excel's Border
  tab works: fully `.xlsx` round-trip-safe, no engine fork changes required.
- **Wrap text → wrap within the current row height (option 2A).** The toggle sets the
  wrap attribute; text wraps onto multiple lines but only the lines that fit the row's
  current height are visible. Auto-growing row height to fit wrapped content is
  **deferred** and recorded in [`GAPS.md`](../../../GAPS.md).

---

# Part 1 — Text formatting

All three additions reuse the established toggle/alignment machinery end-to-end. The
"active/pressed" state reuses the **exact bold pattern**: a field on `RenderStyle`
populated by the engine's cache resolver (`render_style_from`), which the toolbar
button reads to light up — identical to how `bold`/`italic`/`underline` already work.
The underlying IronCalc `Style` already supports all three fields and they round-trip
through `.xlsx` (verified: `font.strike`, `alignment.vertical`, `alignment.wrap_text`).

## 1.1 Strikethrough

- **Control:** a toggle button placed **immediately after Underline** in the B / I / U
  group.
- **Behavior:** toggles strikethrough on the current selection, using the same
  "any cell in range lacks it → set the whole range, else clear" semantics as bold.
- **Active state:** the button is pressed when the active cell has strikethrough set
  (`RenderStyle.strikethrough`).
- **Rendering:** a horizontal line through the vertical middle of the cell text,
  drawn wherever text is drawn (parallels the existing underline rendering). Combines
  with underline (a cell can be both).
- **Persistence:** stored as IronCalc `font.strike`; round-trips on save/reopen.
- **Shortcut:** none required for MVP (bold-style `⌘…` shortcut optional, not in scope).

## 1.2 Wrap text

- **Control:** a **single toggle button** placed **after strikethrough**. Toggles like
  bold/italic (on/off), not a multi-option control.
- **Behavior (2A):** toggles the wrap attribute on the selection. When **on**, the
  cell's text wraps onto multiple lines constrained to the cell's column width; when
  **off**, the current single-line behavior is unchanged.
- **Active state:** pressed when the active cell has wrap set (`RenderStyle.wrap`).
- **Rendering:**
  - Text wraps to the cell's column width and renders as multiple lines.
  - Only the lines that fit within the row's **current** height are shown; content
    beyond the bottom of the cell is clipped. The user grows the row manually to
    reveal more (row-resize already exists).
  - With wrap **on**, text no longer clips-at-boundary / overflows into neighbors the
    way unwrapped text does — it wraps inside its own cell.
- **Deferred:** auto-growing the row height to fit all wrapped lines (true Excel wrap)
  is out of scope for this project and logged in `GAPS.md` as a follow-up.
- **Persistence:** stored as IronCalc `alignment.wrap_text`; round-trips.

## 1.3 Vertical alignment

- **Control:** three icon buttons — **Top / Center / Bottom** — in a **new section**
  (its own divider group) placed **after** the horizontal align L / C / R group.
- **Behavior:** radio-style, mirroring horizontal alignment. Clicking a button sets
  that vertical alignment on the selection (`alignment.vertical` = `top`/`center`/`bottom`).
- **Active state:** a button is pressed only when the active cell has that vertical
  alignment set **explicitly** (mirrors horizontal align, where a value present only
  when explicit lights the button). A cell with no explicit vertical alignment shows
  **no** button pressed.
- **Rendering:**
  - When set explicitly, the cell's text block is positioned at the top / vertical
    center / bottom of the row.
  - When **unset**, the cell keeps FreeCell's **current** default vertical rendering
    (unchanged from today — see edge cases). This avoids moving every existing render
    baseline; only cells that opt in change position.
  - Interacts with wrap: a wrapped multi-line block is positioned as a unit.
- **Persistence:** stored as IronCalc `alignment.vertical` (Top/Center/Bottom;
  Justify/Distributed out of scope); round-trips.
- **New model type:** a `VAlign { Top, Center, Bottom }` enum in `freecell-core::style`
  (parallel to the existing horizontal `Align`), surfaced on `RenderStyle` as
  `v_align: Option<VAlign>`.

## Part 1 edge cases

- **Mixed selection (some cells have the attribute, some don't):** toggles follow the
  bold rule (any cell lacking it → set all; else clear). Vertical align is a set (not
  toggle): applying sets all selected cells to the chosen value.
- **Current default vertical position:** implementation must confirm FreeCell's present
  vertical text placement and make "unset" render identically to today (likely center).
  If explicit `Bottom` happens to match Excel's default, that's fine, but unset must not
  shift baselines.
- **Wrap + very narrow column:** wrapping still applies at the column width; extremely
  narrow columns may show one character per line (acceptable, matches Excel).
- **Undo/redo:** each of these is one IronCalc-native undoable step, like existing
  formatting.

---

# Part 2 — Border formatting

Redesigns the existing borders popover from eight text preset buttons into a
two-region control: a row of **"which lines" target icons** and, below it, a row of
**line-style + color** controls. The defining requirement is a **simple UX over a
complex modality**: a selected range can contain a mix of borders, and the user must be
able to restyle one part (e.g. just the outer box, or just the top) without disturbing
the rest.

## 2.1 The "pen" model (how the modality works)

The popover holds two pieces of **transient state**, both reset every time the popover
opens:

1. **Selected target** — which set of edges the controls act on right now
   (`None` on open; exactly one target at a time once chosen).
2. **Current pen** — the line settings the controls hold: `{ style, color }`,
   initialized to the default **thin solid black** on open.

Core rules:

- **Clicking a "which lines" target** (e.g. Outer): it becomes the selected target
  (its icon shows selected), and the **current pen is painted onto that target's edges
  only** — leaving every other edge (including interior borders) untouched. The popover
  **stays open** (unlike today, where selecting a preset applies and closes).
- **Adjusting the line style or color** while a target is selected **re-paints that
  target's edges** with the new pen — and nothing else.
- **Switching to a different target** (e.g. Top): the previous target is deselected,
  the new one selected, and the current pen is painted onto the new target's edges.
  The pen carries over across target switches (it is "the pen you're drawing with").
- **Closing** the popover (click-away / Esc) discards the transient state.
- **Reopening** the popover shows **no target selected** and the pen reset to default —
  even though many cells in the selection may already have borders. Border selection
  state is never derived from existing cell borders.

Worked example (matches the requested flow):

1. Select a large block containing assorted interior + edge borders.
2. Click **Outer** → default thin solid black appears on the outer perimeter; interior
   borders unchanged; Outer icon selected; popover stays open.
3. Change style to **dashed**, color to **red** → the outer perimeter becomes dashed
   red; interior untouched.
4. Click **Top** → Top selected; the current pen (dashed red) paints the selection's
   top edge; change style to **dotted** → only the top edge becomes dotted.
5. Click away → popover closes.
6. Reopen → nothing selected; pen back to thin solid black.

### Why this maps cleanly to the engine

Each target corresponds to an IronCalc border **type** (All / Inner / Outer / Top /
Bottom / Left / Right / None). Applying a border of a given type writes the border item
**only to the edges that type implies** and does not clear the others — so "paint Outer"
inherently leaves interior borders intact. The current border pipeline already does
exactly this; the only change is that the written border item carries the pen's
`{style, color}` instead of a hardcoded thin-black item.

## 2.2 "Which lines" targets (icon buttons)

Replace the eight text buttons with **icon buttons**. Each icon depicts a **2×2 block of
cells**; the edges the target affects are drawn as **solid dark lines** and the other
gridlines as **light grey**, so the icon reads as "these lines".

The target set keeps today's semantics:

| Target | Affects | Icon (dark lines) |
|--------|---------|-------------------|
| **All** | every interior + perimeter edge | all lines dark |
| **Inner** | interior edges only | inner cross dark, outer grey |
| **Outer** | perimeter box only | outer box dark, inner cross grey |
| **Top** | selection's top edge | top line dark |
| **Bottom** | selection's bottom edge | bottom line dark |
| **Left** | selection's left edge | left line dark |
| **Right** | selection's right edge | right line dark |
| **None** | clears all borders in the selection | all grey / explicit "no border" |

- **None** is an action, not a paintable target: clicking it clears all borders in the
  selection and leaves **no** target selected (there is nothing to keep styling). The
  pen is unchanged.
- Excel's inner-horizontal / inner-vertical targets are **not** in scope but the icon
  layout makes them a cheap future addition.

## 2.3 Line-style + color controls

A second row under the target icons:

- **Line style — a gallery/dropdown (option 1A).** Each entry is a small preview of the
  actual line, combining weight and pattern into one choice. **MVP set** (kept lean —
  solid weights render for free; only dashed + double are new paint work):
  - Thin solid (1px)
  - Medium solid (2px)
  - Thick solid (3px)
  - Dashed (2px)
  - Double (3px)
  - **Deferred → `GAPS.md`:** Dotted (round-trips lossily at 0.7.1) and Dash-dot
    (niche). Adding them later is a gallery entry + one more render pattern each.
- **Color — reuse the existing color picker** (the same `ColorPicker` + 10-swatch
  `FILL_PALETTE` + "Custom…" affordance used for text color and fill). Default black.
- The controls always display the **current pen**. Changing a control updates the pen
  and (if a target is selected) repaints that target. See §2.5 for the no-target case.

## 2.4 Rendering (dashed / dotted / double are new)

The grid currently draws every border as a **solid filled strip** and already honors
arbitrary **weight (px)** and **color**. To support line style faithfully:

- The render-model `Edge` gains a **style/pattern** discriminant (solid / dashed /
  dotted / double) alongside its existing `weight` + `color`.
- The edge-drawing routines render the pattern: dashed → evenly spaced dashes, dotted →
  dots, double → two thin parallel lines, solid → today's strip.
- The heavier-wins resolution between adjacent cells' shared edge is unchanged; where
  two differing styles meet on a shared edge, weight wins as today (tie keeps own).

## 2.5 P2 — restyle-all with no target selected (deferred, designed-for)

The requested P2 behavior: with **no target selected**, adjusting style/color changes
**all existing borders in the selection** (restyle-in-place, preserving which edges
exist). This requires read-modify-write of each cell's current edges and is **deferred**.

- **MVP behavior with no target selected:** the controls are live and editable, but a
  change only updates the **pen** (what the next target click will paint); it does not
  modify any existing borders. Nothing on the sheet changes until a target is clicked.
- **P2 upgrade:** a change with no target selected instead re-emits every existing
  border in the selection with the new style/color, keeping each edge's presence.

This is recorded so the UI (controls usable without a target) is forward-compatible;
P2 is not built in the initial phases.

## Part 2 edge cases & constraints

- **Single-cell selection:** Outer = that cell's four edges; Inner = no-op; Top/Bottom/
  Left/Right = the corresponding single edge.
- **Undo/redo (intended design):** each border paint is one undoable step — selecting
  a target, and each subsequent style/color change, is its own undo entry. This is the
  correct model (one discrete user action → one undo step), **not** a limitation:
  consecutive pen tweaks are deliberately **not** coalesced.
- **`.xlsx` round-trip fidelity:** the MVP gallery (thin/medium/thick solid, dashed,
  double) is fully representable and round-trips. **Dotted is dropped** precisely
  because it degraded to `Thin` on import at 0.7.1 — deferred to `GAPS.md` rather than
  shipping silent data loss.
- **Degraded / read-only worker:** the borders control is disabled and any open popover
  force-closes, exactly as today.
- **Diagonal borders:** out of scope.
- **Border color on `indexed=` palettes:** unaffected — we write explicit `#RRGGBB`.

---

# Out of scope (whole project)

- Auto-grow row height for wrap (→ `GAPS.md` F1).
- P2 restyle-all-with-no-target (designed-for, deferred → `GAPS.md` F2).
- Dotted + dash-dot line styles (→ `GAPS.md` F3).
- Diagonal borders; inner-horizontal / inner-vertical border targets.
- Justify / Distributed vertical alignment.
- Merged cells, overflow-into-neighbors changes, and any non-formatting behavior.
- New keyboard shortcuts (formatting is mouse/menu-driven for this project).

# Constraints

- **Follow existing patterns.** Every addition mirrors an existing analog: strikethrough/
  wrap ≈ bold toggle; vertical align ≈ horizontal align; border controls extend the
  current borders popover + reuse the color picker.
- **Render coverage.** All of these move pixels (new toolbar buttons, strike line,
  wrapped/vertically-positioned text, patterned borders). Per the project's render-test
  policy, intentional baseline changes must be regenerated + eyeballed, and the CI
  `render` gate dispatched and green before merge. Baked into the implementation plan.
- **Engine-boundary discipline.** No IronCalc type crosses the `freecell-engine`
  boundary; new formatting flows through the existing command/protocol/cache seam.
- **`.xlsx` fidelity.** Ship only fully-representable styles (the MVP gallery
  round-trips cleanly); the one lossy style (dotted) is deferred rather than shipped
  with silent degradation.
