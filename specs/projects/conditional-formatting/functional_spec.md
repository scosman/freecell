---
status: complete
---

# Functional Spec: Conditional Formatting

Conditional formatting (CF) restyles cells based on their **computed values**, via rules the
user authors and manages in a new **Conditional Formatting sidebar**. The calculation engine
(IronCalc, our fork) already evaluates CF and exposes the full model + effective-style read; this
project is the FreeCell-side integration — engine wrapper, worker protocol, sidebar UI, and the
grid-render wiring that makes CF appear and update live.

Scope decisions were confirmed with the requester (2026-07-17):
- **Families:** *Highlight (classic) rules + color scales* ship as the **core first pass**. *Data
  bars, icon sets, and ratings* are **planned as later phases in this same project** and are
  **out of scope for the first pass** (see §9 and the GAPS log).
- **Rule management:** **Full** — list, add, edit, delete, and reorder priority — in the first
  pass.

---

## 1. Goals & non-goals

**Goals**
- Author CF rules over a selected range and see them applied to the grid immediately.
- CF is **value-dependent**: rules re-evaluate and the grid re-paints when underlying values change.
- **Full rule management** for a sheet: see every rule, add, edit, delete, reorder priority.
- CF round-trips through `.xlsx` (open a file with CF → it renders; save → it persists).
- Reuse FreeCell chrome conventions: an action-bar button + a right-docked sidebar built on a
  **reusable sidebar-container component** extracted from the chart edit panel.

**Non-goals (this project)**
- No new calculation behavior — the engine already evaluates CF; FreeCell reads results.
- No fork/upstream work — the CF engine API is already present on `freecell-fixes`.
- First pass renders only the two families whose overlay is a **cell style** (fill/font/color).
  Data bars / icon sets / ratings (in-cell decorations needing new grid primitives) are deferred
  to later phases (§9).

---

## 2. User flows

### 2.1 Open / close the sidebar
- An **action-bar button** (lucide **`split`** icon, tooltip "Conditional formatting") toggles the
  **Conditional Formatting sidebar** (right-docked, like the chart edit panel).
- The sidebar **stays open while the user selects cells/ranges** — unlike the chart panel it is
  **not** dismissed by a grid selection change. It closes on its **×** button, on the action-bar
  button toggle, or on degrade/read-only (§8).
- In degraded/read-only mode the button is disabled (consistent with every other mutating control).

### 2.2 Rules list (sidebar home)
- The sidebar's default view lists **every CF rule on the active sheet**, ordered by **priority**
  (highest first — the order the engine applies them). Each row shows:
  - a compact **rule summary** (e.g. "Cell value > 100", "Text contains 'foo'", "3-color scale",
    "Top 10 items"),
  - its **range** (e.g. `B2:B20`),
  - a small **preview swatch** of the format (fill/text color) for highlight rules, or a gradient
    swatch for color scales,
  - **reorder** controls (move up / move down = raise/lower priority),
  - **edit** and **delete** controls.
- An **"Add rule"** button opens the rule editor (§2.3) for a new rule.
- If the sheet has no rules, the list shows a short empty state with the Add-rule affordance.

### 2.3 Add / edit a rule (rule editor)
The editor is the same form for add and edit. Fields:
1. **Applies to (range)** — a text field defaulting to the **current selection** when adding;
   editable. Accepts an A1 range (`B2:B20`, multi-area `A1:A5,C1:C5`). Validated (§8).
2. **Rule type** — a picker grouped like Excel:
   - **Format cells based on their value** → *Cell value* (operators: greater than, less than,
     greater than or equal, less than or equal, equal to, not equal to, between, not between).
   - **Format cells that contain text** → *Text* (contains, does not contain, begins with, ends
     with, equal to).
   - **Format cells with dates** → *A date occurring* (Today, Yesterday, Tomorrow, Last 7 days,
     Last/This/Next week, Last/This/Next month, Last/This/Next year — the parameterless periods).
   - **Top / Bottom** → *Top N*, *Bottom N* (with a "% of range" toggle), *Above average*,
     *Below average*.
   - **Duplicate / Unique** → *Duplicate values*, *Unique values*.
   - **Blanks / Errors** → *Blank*, *No blanks*, *Error*, *No errors*.
   - **Custom formula** → *Formula* (one formula that returns TRUE/FALSE for the top-left cell,
     relative-referenced across the range).
   - **Color scale** → *2-color scale*, *3-color scale*.
3. **Operands** — shown per type: one/two value-or-formula inputs (Cell value / Between), a text
   input (Text), a rank + "%" (Top/Bottom N), a formula input (Formula / Cell-value-as-formula),
   nothing (Duplicate, Blanks, Above average, date periods).
4. **Format** (all types **except** color scales) — a compact format editor exposing **fill
   color**, **text color**, **bold**, and **italic**, plus a set of **Excel-style presets**
   (e.g. "Light red fill with dark red text", "Yellow fill with dark yellow text", "Green fill",
   "Red text", "Bold"). Live **preview** of the chosen format. (Underline, strikethrough, borders,
   number format, and alignment in the differential format are **deferred** — §9 / GAPS.)
5. **Color-scale editor** (color-scale types only) — 2 or 3 stops, each with a **color** and a
   **threshold type** (Min/Max for the endpoints; Number/Percent/Percentile for any stop; the
   midpoint defaults to 50th percentile). Excel default color presets are offered; colors and
   thresholds are editable. A live gradient preview. (Formula thresholds deferred — §9.)
6. **Stop if true** — a checkbox (default off) controlling whether lower-priority rules are
   skipped for a cell this rule matches.
7. **Save / Cancel.** Save adds the new rule (or replaces the edited one) and returns to the list;
   the grid re-paints immediately.

### 2.4 Delete / reorder
- **Delete** removes a rule (from its list row); the grid re-paints.
- **Reorder** (move up/down) raises/lowers a rule's priority; the list re-orders and the grid
  re-paints (priority changes which rule wins on overlapping cells).

All add/edit/delete/reorder operations are **undoable/redoable** on the unified Ctrl+Z/Ctrl+Y
timeline (the engine's CF mutators already record diffs).

---

## 3. Rule types in scope (first pass)

Mapped to the engine's `CfRuleInput` variants. Every dxf-carrying variant takes a **Format** (§2.3
step 4); color scales take a color-scale definition (step 5).

| UI group | Rule | `CfRuleInput` variant | Operands |
|---|---|---|---|
| Cell value | Greater/Less/≥/≤/=/≠ | `CellIs` | 1 value/formula |
| Cell value | Between / Not between | `CellIs` | 2 values/formulas |
| Text | Contains / Not / Begins / Ends / Equals | `Text` | text |
| Dates | A date occurring (parameterless periods) | `TimePeriod` | period enum |
| Top/Bottom | Top N / Bottom N (+ % toggle) | `Top10` / `Bottom10` | rank, percent |
| Top/Bottom | Above / Below average | `AboveAverage` / `BelowAverage` | — |
| Dup/Unique | Duplicate / Unique values | `DuplicateValues` / `UniqueValues` | — |
| Blanks/Errors | Blank / No blanks / Error / No errors | `Blanks` / `NotBlanks` / `Errors` / `NoErrors` | — |
| Formula | Custom formula | `Formula` | formula |
| Color scale | 2-color / 3-color | `ColorScale` | 2–3 stops |

**Deferred variants** (recorded in GAPS): `TimePeriod` Between/NotBetween (explicit date range),
`Cfvo::Formula` thresholds in color scales, and the whole `DataBar` / `IconSet` / `IconRating`
families.

---

## 4. Format model (the differential format, "Dxf")

- The engine's differential format is `Dxf { font, fill, border, num_fmt, alignment }`.
- The first-pass editor writes only: `fill.fg_color` (fill color), `font.color` (text color),
  `font.b` (bold), `font.i` (italic). All other `Dxf` fields are left `None` (unset → inherit the
  cell's base style), which is the correct Excel semantics for a differential format.
- **Reading back for edit:** an existing rule's format is fetched via
  `get_dxf_for_conditional_formatting(sheet, index)`; the editor shows the four supported
  attributes and preserves any it does not edit (round-trips unmodeled `Dxf` fields untouched).

---

## 5. Rendering behavior

- CF applies on top of a cell's base style. The **winning** rule (highest priority that matches,
  respecting *stop if true*) determines the overlay.
- For the two in-scope families the overlay is **entirely a cell style**: `CellIs`/`Text`/…/dxf
  rules contribute fill/font/color; color scales contribute an **interpolated fill**. The engine
  folds all of this into a single effective `Style` (`get_extended_cell_style(...).style`), so the
  grid renders CF through its **existing** fill/font/color paint path — **no new draw primitive**.
- **Value-dependent & live:** because rules like Top-N, Above-average, Duplicate, and color scales
  depend on other cells' values, the effective style must re-evaluate when values change. Editing a
  source cell (or any recompute) **re-paints** the CF-affected cells. This is the core rendering
  change vs. today's purely-static style path (§ architecture).
- CF **fills cover interior gridlines** the same way explicit fills do (reuses the existing
  same-fill-block gridline suppression).
- Only **populated** cells within a rule's range receive a computed overlay (matches the engine's
  evaluation domain); blank cells are styled only by rules that target blanks.

---

## 6. Priority, overlap & "stop if true"

- Multiple rules can cover the same cell. The engine applies them by **priority** (the list order);
  a higher-priority rule's format wins per-attribute, and **stop if true** halts evaluation of
  lower-priority rules for a matched cell. FreeCell surfaces priority via the list order + reorder
  controls and exposes the stop-if-true checkbox; the *resolution* is the engine's.
- Reordering changes priority via `raise_/lower_conditional_formatting_priority`.

---

## 7. Persistence

- CF is part of the worksheet model, so it **saves and loads through IronCalc's native `.xlsx`
  writer/reader** — no special-case save path (unlike charts). Opening a file authored elsewhere
  shows its CF; saving preserves rules FreeCell modeled.
- **Fidelity limits** (deferred families, unmodeled `Dxf` attributes, deferred variants) are
  recorded in GAPS; a loaded rule of a deferred family still round-trips through the engine model
  but is not **authored/edited** in this first pass (see §9 for how the list handles them).

---

## 8. Edge cases & error handling

- **Invalid range** (unparseable / out of bounds): the range field shows an inline error; Save is
  blocked until valid. Empty range field ⇒ defaults back to the current selection on focus-out.
- **Invalid formula / operand**: the engine's `add/update_conditional_formatting` returns
  `Err(String)`; the sidebar surfaces the message inline and keeps the editor open (no partial
  apply).
- **Empty operands** where required (e.g. Cell value with no number): Save blocked with an inline
  hint.
- **No active document / degraded / read-only:** the action-bar button and all sidebar mutators are
  disabled; an already-open sidebar closes on degrade (mirrors the chart panel).
- **Sheet switch:** the list reflects the **active sheet's** rules; switching sheets refreshes it.
  An open editor for the previous sheet is cancelled on sheet switch.
- **Rule referencing a deleted/edited range:** handled by the engine (ranges shift with
  structural edits via its diff model); FreeCell re-reads the list after such edits.
- **Undo/redo:** every CF mutation is one undoable step; undo restores the exact prior rule set and
  re-paints.
- **Large ranges / many rules:** evaluation is the engine's (cached); FreeCell reads effective
  styles only for **populated** cells and repaints the **viewport**. Perf constraint in §10.

---

## 9. Out of scope for the first pass (planned later phases) — and how the list handles them

Deferred **families** and **variants** (each logged in GAPS with a follow-up):
- **Data bars** — in-cell horizontal bar (`ExtendedStyle.data_bar`), new grid primitive.
- **Icon sets & ratings** — in-cell glyphs (`ExtendedStyle.icon` / `.rating`), new grid primitive.
- **`TimePeriod` Between/NotBetween** (explicit date-range operands).
- **Color-scale `Formula` thresholds.**
- **`Dxf` attributes** beyond fill/text-color/bold/italic: underline, strikethrough, border,
  number-format override, alignment.

**Handling deferred rules already present on a loaded sheet:** the list still **shows** every rule
(including data-bar/icon/rating/deferred-variant rules) with a read-only summary badge and a
**delete** affordance, but its **Edit** action is disabled for a type the first pass can't author
(so a user can remove an unsupported rule but not misedit it). Deferred-family rules that are
present are **not rendered** as their decoration in the grid this pass — a data-bar/icon rule's
`ExtendedStyle.style` (any dxf/fill part) still applies, but the bar/icon glyph itself waits for
its render phase. This limitation is logged in GAPS.

---

## 10. Constraints

- **Performance (FreeCell's whole reason for being):** CF must not regress scroll/edit on huge
  sheets. Effective-style reads are bounded to **populated** cells in CF ranges and folded into the
  resident style cache off the paint path; the grid paint stays a cache read. Value-change
  invalidation recomputes only sheets that **have** CF rules. Benchmark the edit→repaint and
  scroll paths on a CF-heavy sheet before shipping (per repo benchmark conventions).
- **Compatibility:** authored rules must reopen correctly in Excel/LibreOffice for the modeled
  subset; verify a round-trip.
- **Rendering is in-scope for the pixel render suite** (CF fills/fonts change grid pixels) — a
  dedicated late render-validation phase (per CLAUDE.md) refreshes + eyeballs baselines and
  dispatches the CI `render` gate.
