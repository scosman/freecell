---
status: complete
---

# Functional Spec: gaps_closing_7_12 (v0.5 low-hanging-fruit batch)

Eight independent, independently-shippable v0.5 gap closures. Each section is a phase.
Behavior is specified to Excel/Google-Sheets parity except where a deliberate FreeCell
deviation is called out. Mechanism (worker vs. render thread, engine APIs) is deferred to
`architecture.md`; this document fixes **what the user sees**.

Cross-cutting conventions (apply to every feature):

- **Shortcuts** are written with ⌘ (macOS, the primary target); the keymap layer already
  maps ⌘→Ctrl on other platforms. No per-feature restatement.
- **Undo:** every data-mutating action below is **one** undo step (rides existing single
  engine ops). Called out per feature only where subtle.
- **Selection semantics** use the existing `SelectionModel` (single rectangular range +
  full-row/full-column selections). Multi-area (⌘-click) is v1.0 and out of scope.
- **Correctness over the whole sheet, not just the viewport.** Several features
  (selection stats, edge-of-data) must be correct for selections that extend past the
  visible viewport (e.g. a whole-column selection). Where the visible published cells are
  insufficient, the correct full-sheet result is required — see each feature.

---

## 1. Status bar with selection stats

A persistent, thin **status bar** along the bottom of the window shows live aggregate
statistics for the current selection — the hallmark "totals a selection at a glance" win.

### Behavior

- **Placement (owner-decided):** on the **right side of the existing sheet-tab bar**
  (Google-Sheets-style) — **no** separate bottom row. The tab bar is refactored so the
  sheet tabs stay left-aligned and the selection-stats readout sits right-aligned in the
  same row, sharing that row's height/background.
- **Contents (default):** `Sum`, `Average`, `Count`, shown as labeled readouts, e.g.
  `Sum: 1,234.50   Average: 246.90   Count: 5`.
- **Min/Max toggle:** clicking anywhere on the stats readout toggles an expanded form that
  **also** shows `Min` and `Max`. The toggle state persists for the session (not saved to
  disk). (GAPS: "Sum · Avg · Count, click for Min/Max.")
- **When shown:** the stats appear only when the selection covers **2+ cells** and
  contains **at least one numeric value**. Otherwise the stats area is **empty** (a
  single-cell selection, an all-text/all-empty selection, or an empty sheet shows no
  numbers). The status-bar row itself is always present (stable layout).
- **Statistic definitions (Excel semantics):**
  - `Count` = number of **non-empty** cells in the selection (text + numbers + booleans +
    errors all count; blanks don't). Matches Excel's "Count".
  - `Sum` = sum of the **numeric** cells only (text/blank/boolean ignored).
  - `Average` = `Sum ÷ (count of numeric cells)`.
  - `Min` / `Max` = over the **numeric** cells only.
  - If the selection contains **no** numeric cells, `Sum`/`Average`/`Min`/`Max` are not
    shown and only… nothing shows (per "when shown" above we require ≥1 numeric).
  - **Errors** in the selection: a cell holding an error value is counted in `Count` but
    excluded from `Sum`/`Average`/`Min`/`Max` (it is not numeric). (Excel propagates the
    error into Sum; we choose the friendlier "ignore for math, still count" — flagged
    below.)
- **Readout number formatting:** a compact **General**-style format — thousands
  separators, trailing zeros trimmed, capped significant digits — independent of the
  selected cells' own number formats. (We are summarizing heterogeneous cells; a single
  neutral format reads best.)
- **Live update:** recomputes whenever the selection changes or an edit changes a value
  inside the current selection.

### Correctness / performance

- Stats must be correct for a selection of **any** size, including a full column
  (1,048,576 rows) or full row — i.e. computed over the selection **intersected with the
  sheet's populated/used range**, not merely the visible viewport. (GAPS' "render-side
  only" note is only true when the whole selection is on-screen; the correct general
  behavior needs the full-sheet values — see `architecture.md` for the worker-aggregate
  approach.) A wrong Sum is worse than no Sum.
- Computation must not stall the UI: it is debounced and runs off the populated cells
  (sparse), never iterating empty cells.

### Out of scope

- Configurable/which-stats picker, `RxC` selection-size readout, per-stat right-click
  menu (Excel-style) — all future. Only the fixed Sum/Avg/Count (+Min/Max toggle) ships.

### Decisions to confirm

- **D1.1** Error cells: **count but exclude from math** (proposed) vs. propagate the error
  into Sum/Average like Excel.
- **D1.2 — RESOLVED (owner, 2026-07-13):** stats live on the **right of the sheet-tab
  bar**, no separate bottom row; the tab bar is refactored to host them.

---

## 2. Cell-area right-click context menu

Right-clicking the grid **cell body** opens a context menu at the cursor (today it just
dismisses any open popover). Header and chart context menus already exist as the pattern.

### Behavior

- **Selection interaction on right-click:**
  - If the right-clicked cell is **outside** the current selection → the selection first
    **moves** to that single cell, then the menu opens (Excel behavior).
  - If the right-clicked cell is **inside** the current selection → the selection is
    **preserved** (so the menu's actions apply to the whole multi-cell selection).
- **Menu items (top to bottom):**
  1. **Cut** ⌘X
  2. **Copy** ⌘C
  3. **Paste** ⌘V — disabled when there is nothing to paste (empty clipboard)
  4. **Paste Values** ⌘⇧V — disabled when the clipboard has no internal payload (see §5)
  5. **Clear Contents** ⌦ — clears values, keeps formatting
  6. — separator —
  7. **Insert row above** / **Insert row below**
  8. **Delete row(s)**
  9. **Insert column left** / **Insert column right**
  10. **Delete column(s)**
  11. — separator —
  12. **Clear Formatting** — resets styles on the selection, keeps values *(include only
      if a style-clear op already exists; otherwise omit for this batch)*
- **Insert/Delete** reuse the **exact existing header-menu commands**, scoped to the
  selection's row/column span, and honor the existing **merge-displacement guard** (the
  same block/reject behavior the header menu already applies).
- **Enable/disable:** Cut/Copy/Clear Contents enabled on any non-empty selection; Paste /
  Paste Values gated on clipboard contents as above.
- **Dismissal:** click-away, `Esc`, or choosing an item closes the menu (standard popover
  behavior, same as the existing menus).

### Out of scope

- A full **Format Cells…** dialog (none exists; the action bar owns formatting). Only the
  lightweight items above. No "Insert cells / shift" (cell-level insert with shift
  direction) — only whole-row/column insert/delete, matching today's header menu.

### Decisions to confirm

- **D2.1** Final item inventory — specifically whether to include **Clear Formatting** now
  (depends on an existing style-clear op) and whether Insert/Delete are **flat items**
  (proposed) or grouped into **Insert ▸ / Delete ▸** submenus.

---

## 3. Fill down / right (⌘D / ⌘R)

Keyboard fill — *the* signature spreadsheet affordance. **Keyboard commands only this
phase; the drag-fill handle stays deferred** (the larger, input-heavy half).

### Behavior (Excel/Sheets ⌘D / ⌘R semantics — a *copy*-fill, not a series)

- **⌘D (Fill Down), multi-cell selection:** the **top row** of the selection is filled
  **down** into every other row of the selection. Each filled cell copies the value/
  formula/format from the cell at the top of its column, with **relative reference
  adjustment** for formulas.
- **⌘R (Fill Right), multi-cell selection:** the **left column** of the selection is
  filled **right** into the rest of the selection, analogously.
- **Copy, not series.** ⌘D/⌘R **copy** the seed (e.g. `A1:A5` with `A1=1` → `1,1,1,1,1`),
  matching Excel. This is intrinsic to seeding from a single row/column: the engine's
  auto-fill only extrapolates a series from a **multi-cell** seed, which this path never
  supplies. (Series autofill — `1,2,3…`, `Jan,Feb…` — belongs to the deferred drag
  handle / Fill Series, not ⌘D/⌘R.)
- **Single-cell selection (Excel "pull from neighbor"):** ⌘D on a single cell copies the
  cell **directly above** it into it; ⌘R copies the cell **directly to the left**. If
  there is no such neighbor (top row / column 0), it is a **no-op**. *(Marked optional —
  see D3.1.)*
- **Overwrite:** fill overwrites existing target content (Excel behavior).
- **Undo:** one step.

### Edge cases

- Selection that is a single row (for ⌘D) or single column (for ⌘R) with >1 cell but only
  the seed line → no-op (nothing to fill into).
- Fill target intersects a merged region → honor the existing merge guard (reject/no-op
  consistently with other structural ops).

### Out of scope

- Drag-fill handle, Fill Series (`1,2,3…`), fill across sheets, ⌘⇧D/other fill variants.

### Decisions to confirm

- **D3.1** Include the **single-cell "pull from above/left"** behavior now (proposed,
  cheap, Excel-expected) or defer it and make single-cell ⌘D/⌘R a plain no-op.

---

## 4. ⌘+arrow → edge-of-data

Change ⌘+arrow (and ⌘⇧+arrow) from jumping to the **sheet edge** to the **edge of the
data region**, matching Excel/Sheets muscle memory. Purely a change to how the existing
`Motion::JumpEdge` / `Motion::ExtendEdge` target is resolved.

### Behavior (exact Excel Ctrl+Arrow algorithm)

From the active cell, moving in the arrow's direction:

- **Active cell is empty:** jump to the **next non-empty** cell in that direction. If none
  exists before the sheet boundary, land on the **sheet edge** (row 0 / last row / col 0 /
  last col).
- **Active cell is non-empty:**
  - If the **immediately adjacent** cell in that direction is **non-empty** → jump to the
    **last non-empty cell of the contiguous run** (the cell just before the first empty
    cell, or the sheet edge if the run reaches it).
  - If the immediately adjacent cell is **empty** → jump **across the gap** to the next
    non-empty cell (or the sheet edge if none).
- **⌘⇧+arrow (`ExtendEdge`):** identical target resolution, but **extends** the selection
  (keeps the anchor) instead of collapsing.
- **Empty sheet / no data in direction:** lands on the sheet edge (unchanged from today).

### Correctness / responsiveness

- Must be correct across the **whole sheet** (a jump can traverse up to ~1M cells), so it
  reads occupancy **beyond the viewport** — the published viewport (≤512×256 cells) is
  insufficient, and there is no UI-side occupancy today. The engine holds the occupancy;
  the resolution **mechanism** (a worker round-trip vs. a published occupancy index that
  keeps movement synchronous) and its latency posture is **architecture decision D4.1**.
  Target UX: a jump feels effectively instant on typical sheets.

### Out of scope

- ⌘A "select current region", jump-by-block selection growing, or any other new motion —
  only the target of the existing edge motions changes.

---

## 5. Paste values (⌘⇧V)

Minimum paste-special: **values only** (no formulas, no formatting). `Shift+V` is already
reserved-but-unbound. Google-Sheets "Paste values only" semantics.

### Behavior

- **⌘⇧V** pastes, for each source cell, its **current evaluated value** as a **literal**:
  - A source **formula** cell pastes as its **computed result** (a static value), not the
    formula.
  - A source **value** cell pastes its value.
  - **Number formats and all other styles are NOT applied** — the target cell keeps its
    own existing formatting. (This is the defining difference from ⌘V.)
- **Source = the internal clipboard** (a prior in-app ⌘C/⌘X). If only an **external TSV**
  is on the system clipboard (no internal payload), ⌘⇧V behaves like a normal paste (TSV
  is already values — nothing to strip).
- **Target sizing** follows the existing paste rules exactly (single-cell source fills the
  selection; block source pastes from the anchor; oversized/mismatched → the same Overflow
  rejection). Values-only changes *what* is written, not *where*.
- **Errors:** a source error value pastes as that error value (literal).
- **Undo:** one step.
- **Menu parity:** exposed both as the ⌘⇧V shortcut and the context-menu "Paste Values"
  item (§2).

### Out of scope

- Full paste-special dialog, "values & number formatting", transpose, skip-blanks,
  add/multiply — all v1.0.

### Decisions to confirm

- **D5.1** Confirm **values only, no number format** (proposed; matches Sheets ⌘⇧V and
  Excel's default Paste-Values) vs. "values + number formatting".

---

## 6. Number-format preset breadth

Widen the number-format dropdown beyond today's 7 presets. **UI-only** — the engine
already renders arbitrary format codes; each preset is just a `(label, code)` pair sent to
the existing set-format command.

### Behavior

- The dropdown is **reorganized into grouped sections/submenus** (a flat list of ~20 is
  unwieldy): **General**, **Number**, **Currency ▸**, **Date ▸**, **Time ▸**, **More ▸**,
  **Text**, **Custom** (Custom stays a display-only reverse-map bucket — the format-code
  **editor** is v1.0).
- **Proposed preset inventory** (final list is D6.1):
  - **Number:** `1234.56` (`0.00`), `1,234.56` (`#,##0.00`, existing), `1,235` (`#,##0`),
    `-1,234.56 in red` (`#,##0.00;[Red]-#,##0.00`).
  - **Currency ▸** (symbol choice): `$`, `€`, `£`, `¥` → `«sym»#,##0.00` (and a negative-in
    -parens accounting-ish variant).
  - **Percent:** `0.00%` (existing), `0%`.
  - **Scientific:** `0.00E+00`.
  - **Fraction:** `# ?/?`. **— DEFERRED (engine limitation, implemented-phase CR 2026-07-13):**
    dropped from the shipped inventory (and the `Fraction` category removed) because IronCalc's
    `?/?` fraction formatting is effectively unimplemented (`1.5` → `"  /2"`, garbled for every
    input). Needs an IronCalc fork implementation — tracked in `PROJECTS.md` +
    `projects/fraction-number-format.md`. Scientific ships.
  - **Currency `£`/`¥` codes (implemented-phase CR 2026-07-13):** shipped as the engine-accepted
    bracket form `[$£]#,##0.00` / `[$¥]#,##0.00` — IronCalc's lexer only accepts the *bare* symbols
    `$`/`€`, so bare `£`/`¥` codes render `#VALUE!`. `$`/`€` stay bare.
  - **Date ▸:** `m/d/yyyy` (existing), `yyyy-mm-dd` (ISO), `d-mmm-yyyy`, `mmm d, yyyy`,
    `m/d/yy`.
  - **Time ▸:** `h:mm AM/PM` (existing), `h:mm:ss AM/PM`, `h:mm` (24-hour), `[h]:mm:ss`
    (elapsed).
  - **Text:** `@` (existing).
- **Action-bar thousands-separator toggle:** add a **comma/1000-separator toggle button**
  next to the existing decimals +/- buttons that adds/removes the `,` grouping from the
  current cell's format code (mirrors the decimals-rewrite logic). *(Optional — D6.2.)*
- Selecting any preset sends the exact code to the existing set-number-format command; the
  reverse-map (code → highlighted category) is extended so an active cell's format shows
  the matching preset selected.

### Out of scope

- Custom format-code **editor** with live preview (v1.0). Locale-specific number/date
  presets (v2.0 localization).

### Decisions to confirm

- **D6.1** Final preset inventory (the list above is the proposal).
- **D6.2** Include the **thousands-separator toggle button** in the action bar this phase.

---

## 7. Autofit column width (double-click header divider)

Double-clicking a **column header's right divider** auto-sizes that column to fit its
content. Pairs with the shipped drag-resize.

### Behavior

- **Trigger:** a **double-click** on the resize hot-zone between two column headers
  (the same divider the drag-resize already uses).
- **Result:** the column's width becomes just wide enough to show the **widest cell's
  content** in that column, plus a small horizontal padding. The width is set as an
  **explicit** column width (undoable; identical to a manual resize; round-trips to xlsx).
- **Measurement scope:** each cell measured at its **own** rendered font
  (family/size/bold) so a bold/larger cell widens the fit correctly. *Which* cells are
  measured — the published/overscanned cells (render-thread-only, cheap, reuses the
  existing measure-and-emit loop) vs. the column's full used range (needs a worker text
  query) — is **architecture decision D7.3**; the trade-off is a wide value scrolled far
  off-screen (beyond the overscan) not being measured.
- **Clamps:** never smaller than a floor (the column-letter header label width /
  configured minimum); never wider than a configured maximum (very-long-content cap).
- **Empty column:** shrinks to the minimum/floor width.
- **Multi-column:** if the double-clicked column is part of a **multi-column selection**,
  autofit **all selected columns** (each to its own content); otherwise just that column.
  *(Optional refinement — D7.1.)*

### Out of scope

- Row-height autofit via double-clicking the **row** divider — deferred (wrap-driven row
  auto-grow already exists; this batch is column-width). *(Reconsider under D7.2.)*
- Autofit-on-type / auto-expanding columns as you enter data.

### Decisions to confirm

- **D7.1** Include **multi-column** autofit (double-click applies to all selected columns)
  now, or single-column only.
- **D7.2** Include **row-height** autofit (double-click the row divider) in this phase, or
  keep it column-width only.

---

## 8. Render-fidelity polish pair *(dedicated late render phase)*

Two cheap, instantly-visible grid-render quality fixes. **This is the only
pixel-suite-in-scope work in the batch**, so per `CLAUDE.md` it is its **own phase after
all other coding**, and it runs the full render suite + refreshes/eyeballs baselines +
dispatches the CI `render` gate.

### 8a. A fill covers its block's interior gridlines

- Within a contiguous block of cells that share the **same resolved fill color**, the
  **interior** gridlines (edges between two same-fill neighbors) are **not** drawn — the
  block reads as one solid rectangle (the Excel look).
- **Rule:** for a filled cell, **skip its right gridline** when the right neighbor has the
  **same** resolved fill; **skip its bottom gridline** when the bottom neighbor has the
  same resolved fill. Gridlines at the block's **outer** boundary (against a different fill
  or an unfilled cell) still draw.
- **Unaffected:** explicit cell **borders** (they always draw), the selection overlay,
  and unfilled cells (normal gridlines everywhere).

### 8b. A full-row selection darkens the row-number header

- A full-**row** selection darkens the left-hand **row-number** header cell(s) of the
  selected row(s) with the selected-header background — **symmetric** with the full-column
  path, which already darkens the column-letter header.
- No other change to the full-row selection (tint + accent border already correct).
- **Verify-first (load-bearing).** The committed header code computes the `selected` flag
  **identically** for the row and column strips (both off the same selection range, both
  feeding `header_element`) — so this may already be correct in source and the GAPS
  observation may stem from render ordering (the accent edge overpainting the tint) or a
  **stale baseline**. Phase 8b therefore **starts by eyeballing the current
  `header_full_row_selected` baseline**: if the row header is already darkened, 8b
  collapses to a baseline refresh + a GAPS correction; otherwise fix the real cause found
  there.

### Render validation (this phase)

- Regenerate + **eyeball** the affected baselines (`cell_fill_covers_gridlines`,
  `header_full_row_selected`, plus any block-fill / full-line-selection cases), commit the
  refreshed baselines, run the **full** suite under a watchdog, and dispatch the CI
  `render` gate to green.

### Out of scope

- Any other render-fidelity item from GAPS (chart residuals, cut-source dimming, etc.).

---

## 9. Sum-section refinements + horizontal scroller control  *(owner feedback, 2026-07-13)*

Owner feedback after using the shipped Phase 1 status bar. All of the below is **one phase**,
built **after** Phases 1–8. Two parts: (A) polish the selection-stats readout, and (B) a new
reusable horizontal-scroller control that makes the stats always-visible and is also reused in
the action bar. **All chrome** (tab bar + action bar) → **out of the pixel suite**; validate
with gpui view tests + `VisualTestContext` paint tests + an Xvfb smoke launch.

### 9A. Selection-stats readout refinements (to the Phase 1 status bar)

1. **Adaptive decimal precision (D9.1).** Today Sum/Average can render absurd precision (e.g.
   `1000000.666667`). Scale the decimal count to the magnitude — by **digits left of the
   decimal point**, using `|value|` so negatives scale the same:
   - `|v| ≥ 100` → **2** decimals
   - `|v| ≥ 10`  → **3** decimals
   - `|v| ≥ 1`   → **4** decimals
   - `|v| < 1`   → **5** decimals

   Applies to Sum, Average, Min, Max (any non-integer stat value). Keep thousands separators
   and trailing-zero trimming (so `2` shows as `2`, not `2.00`). **Count stays an integer.**
2. **Vertical centering.** The readout is not quite vertically centered in the tab bar; make
   line-height track the bar height (`TAB_BAR_H`) so the text is centered.
3. **Leading divider.** Add a divider immediately **before** the stats group, matching the
   action bar's between-group divider style.
4. **Always visible.** The stats group must **never** be pushed off-screen by a long sheet-tab
   strip. Achieved via 9B: the sheet-tab strip scrolls horizontally; the stats group is
   **static**, pinned to the right of the scroller.

### 9B. Horizontal scroller control (new, reusable)

**Premise:** many users lack trackpads for horizontal scrolling and don't discover
scroll-regions that show no scrollbar — so this control is **light and discoverable** while
showing **no visible scrollbar**.

- **Fits horizontally →** renders **exactly as today** — no affordance, no behavior change.
- **Overflows horizontally →** append a **static** right-hand section (does **not** scroll)
  containing: a **divider**, then **left** and **right** chevron buttons — divider + buttons in
  the **same style as the action bar's** dividers/buttons, using the `chevron-left` /
  `chevron-right` **lucide** icons (D9.3). The **content/left** section then scrolls
  horizontally via mouse/trackpad **or** the chevron buttons, with **no visible scrollbar**.
- **Chevron behavior (D9.2):** each click performs an **animated** horizontal scroll of **80%
  of the scroll-viewport width** (left button ← / right button →), clamped at the ends
  (buttons may disable at their limit).

**Two call sites:**
1. **Action bar** — its button groups scroll when the window is too small to fit them.
2. **Sheet-tab strip (bottom row)** — the tabs scroll; the **Sum/Avg/Count group is static to
   the right** of the scroller (this is what implements 9A.4). The leading divider (9A.3) sits
   between the scrollable tabs and the static stats group.

### 9C. Convention note

Add a note to **`CLAUDE.md`** that the project uses **lucide** for icons (recorded while
adding the chevron icons).

### Out of scope (Phase 9)

- Vertical scroll affordances; scrollbar visibility toggles elsewhere; touch-gesture tuning.
- Restyling the action bar or tabs beyond hosting them in the scroller.

---

## 10. Feedback tweaks  *(owner feedback, 2026-07-13 — built after Phase 9)*

A collection of small post-use feedback tweaks. Open-ended (the owner may add more). All
chrome → **not** pixel-suite scope; validate with gpui view tests + `VisualTestContext` paint
tests + an Xvfb smoke launch.

### 10.1 Number-format dropdown: basics-first menu, breadth under "More ▸"

**Regression fixed:** Phase 6's grouped 23-preset dropdown is too long — the common/basic
formats now require scrolling to reach. Restore a short, basics-first menu and demote the full
breadth behind a "More" affordance.

**Behavior:**
- The number-format dropdown's **default content is the original short list** — the ~7 common
  presets that shipped *before* Phase 6 (the pre-Phase-6 `DROPDOWN_FORMATS` set: General,
  Number, Currency, Percent, Date, Time, and the rest of the original seven). Flat, no
  scrolling to see the basics.
- The **last item is "More ▸"**, which opens the **full Phase-6 grouped list** (all 23 presets
  across the 9 groups, with section headers) as a **submenu**. Nothing from Phase 6 is lost —
  just relocated so it isn't in the way of the basics.
- **Reverse-map highlighting still works** across both levels: whichever preset matches the
  active cell's format code is highlighted. If the active format is one that lives only in the
  "More" submenu (not in the basic set), the basic menu still indicates the active state
  sensibly (e.g. "More ▸" is marked active / the submenu opens showing the match) — exact
  behavior per D10.1.
- The **thousands-separator action-bar toggle button** (Phase 6) is unchanged (separate
  control, not in the dropdown). The `More` submenu preserves Phase-6 grouping + the same apply
  path.
- **When it fits:** the basic menu shows without scrolling; the "More" submenu is the only
  place that scrolls (it's the long list).

**Decision — D10.1 (submenu mechanism):** the owner asked for a **submenu under a "More ▸"
item** (a flyout). Preferred = a flyout/side-panel off "More ▸"; **acceptable fallback** if a
true hover-flyout is awkward in the existing custom-`div` popover = a **drill-in** (clicking
"More ▸" swaps the popover content to the grouped list with a "◂ Back" affordance). Pick the
cleaner fit for the existing `render_num_fmt_popover` popover; note the choice.

### 10.2 Action-bar horizontal scroller triggers too early

**Bug:** the action-bar h-scroller (Phase 9) shows its chevrons while every button is still
visible, with empty space to the right of the find button. **Cause:** the button-group
container uses a hand-estimated `min_w(ACTION_ROW_MIN_W = 1152px)` — a drift-prone magic
constant (its own comment shows the running tally 816→896→1080→1145→1152 and warns it will
clip/drift). When 1152 over-estimates the true button width, the surplus becomes trailing
empty space inside the scroll content, so overflow (and the chevrons) trigger before the
controls actually stop fitting.

**Fix:** replace the `min_w` magic constant with `flex_shrink_0` on the button-group content
(flexbox default shrink is 1 — that shrink pressure was the only reason `min_w` was needed to
stop the buttons compressing). The content then sits at its exact natural width (no
compression — Phase 9's "scroll, don't squish" intent preserved), so the chevrons appear
**only** when the buttons genuinely don't fit. Remove `ACTION_ROW_MIN_W`.

### 10.3 Animate the h-scroller chevron scroll

Replace the non-animated clamped jump (Phase 9's sanctioned D9.2 fallback) with a **fast,
clearly-visible animated slide** (D10.2). gpui's `window.request_animation_frame()` repaints
the current view (`ChromeView`) for free, so the tween needs only `&mut Window` (which the
chevron `on_click` already has — no view `Context`/entity plumbing). The slide is a few frames
(quick but obviously a scroll, not a teleport), stops cleanly at the target, and drives frames
only while animating so it never fights manual wheel/trackpad scrolling.

### 10.4 Flyout submenu for "More ▸" — deferred to a GAP (D10.3)

The owner prefers a true flyout for 10.1's "More ▸". gpui-component **does** ship flyout
submenus (`PopupMenu::submenu`), but adopting it *only* for the num-fmt popover is not cheap:
its `scrollable` and submenu modes are mutually exclusive (our card is deliberately
`max_h`+`overflow_y_scroll` for the long grouped list), and it would be the app's **only**
gpui-component menu — diverging in anchoring/dismiss/animation from the **seven** hand-rolled
`div`-popovers beside it (fill, text-color, borders, font-family, font-size, chart, num-fmt).
Doing it well is an **app-wide** menu migration, not a one-popover tweak. So the flyout is
**deferred** and the 10.1 drill-in stays; filed as a GAP (`PROJECTS.md` +
`projects/gpui-component-menus.md`) to adopt gpui-component's `PopupMenu`/`Popover` app-wide
(native flyouts everywhere + consistent anchoring/dismiss, replacing the seven cards).

### Out of scope (Phase 10)

- A custom format-code editor (still v1.0); re-ordering/customizing which presets are "basic".
- Building the flyout now (see 10.4 — deferred to a GAP); a true animated *momentum* scroll
  (10.3 ships a simple fast lerp, not physics).

---

## Render-test scope summary (informs the implementation plan)

| Phase | Pixel-suite in scope? | Validation |
|-------|----------------------|------------|
| 1 Status bar (in tab bar) | No (tab-bar chrome — not a pixel-suite surface) | gpui view tests + Xvfb smoke launch |
| 2 Context menu | No (popover chrome) | gpui view tests + smoke launch |
| 3 Fill ⌘D/⌘R | No (data op) | engine/unit tests |
| 4 Edge-of-data | No (selection logic; overlay position is data-driven) | unit tests for the algorithm |
| 5 Paste values | No (data op) | unit tests |
| 6 Number-format breadth | No (dropdown chrome; values are engine-rendered) | gpui tests (+ subset render check only if a baseline adopts a new preset) |
| 7 Autofit | Lightly (column geometry, like resize) | width-calc unit test + subset render check |
| 8 Render pair | **Yes — intentional baseline moves** | full suite + eyeball + CI `render` gate |
| 9 Sum-section + h-scroller | No (tab-bar + action-bar chrome) | gpui view tests + `VisualTestContext` paint tests + Xvfb smoke launch |
| 10 Feedback tweaks (num-fmt More▸ · action-bar overflow · animated scroll) | No (dropdown + action-bar/tab-bar chrome) | gpui view tests + `VisualTestContext` paint tests + Xvfb smoke launch |

All grid/cell/sheet-pixel-affecting work is concentrated in **Phase 8**, which is the sole
dedicated render-validation phase (Phases 1–7, 9 verify with unit/gpui/subset checks only).
