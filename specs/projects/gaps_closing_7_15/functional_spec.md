---
status: draft
---

# Functional Spec: gaps_closing_7_15 (v0.5 low-hanging-fruit batch, round 3)

Five independent, independently-shippable v0.5 gap closures (after `mvp-gaps` and
`gaps_closing_7_12`). Each numbered section is one phase. Behavior is specified to
Excel / Google-Sheets parity except where a deliberate FreeCell deviation is called out.
Mechanism (worker vs. render thread, engine/fork APIs) is deferred to `architecture.md`;
this document fixes **what the user sees**.

Cross-cutting conventions (apply to every feature):

- **Shortcuts** are written with ⌘ (macOS, the primary target); the keymap layer already
  maps ⌘→Ctrl on other platforms. No per-feature restatement.
- **Undo:** every data-mutating action below is **one** undo step (rides a single engine
  op). Pure view/geometry changes (autofit, auto-grow) follow the existing cache-only
  posture and are called out per feature.
- **Selection semantics** use the existing `SelectionModel` (single rectangular range +
  full-row / full-column selections). Multi-area (⌘-click) is v1.0 and out of scope.
- **Pixel suite:** features **3, 4, 5** move grid pixels (selection handle, hidden-track
  geometry, row height). They verify with a render **subset** while iterating; the **full**
  suite + CI `render` gate run **once** in the final render-validation phase (§6).
  Features **1, 2** are chrome/non-grid and out of pixel-suite scope (gpui view tests +
  smoke launch).

**Highest-impact open decisions** (each has a recommended default below; owner may
override at review): **D4.1** (does hide/unhide round-trip to `.xlsx` via a fork fix, or
ship session-only first?), **D1.1** (signature-hint depth), **D2.2** (what values CSV
export writes). All decisions are collected per-feature under "Decisions to confirm".

---

## 1. Function autocomplete + signature hints

While editing a **formula** (leading `=`), typing a function-name prefix shows a dropdown
of matching function names; accepting one inserts it with its open paren, and a one-line
**signature hint** shows the argument template. This is the "don't have 80 functions
memorized" bar (GAPS.md v0.5, §364).

### Behavior

- **Trigger.** The completion list appears when **all** hold: (a) the edit text starts
  with `=` (the existing formula predicate, `input_cap.rs:80`); (b) the caret is at the
  end of an **identifier token** being typed (letters/digits/`.`/`_`, the leading char a
  letter) that is in *function position* — i.e. not inside a string literal and not
  immediately following a cell reference; (c) the typed prefix is **≥1 character**
  (so a bare `=` does not dump all 345 names). Matching is **case-insensitive**,
  **prefix** match against the function name (`=su` → `SUM`, `SUMIF`, `SUMIFS`,
  `SUMPRODUCT`, …).
- **List contents & order.** Draws from a FreeCell-static list of the **engine-registered
  function names** (the 345 IronCalc supports; anything else would just error). Ordered:
  exact-prefix matches, then **importance rank** (the canonical CSV's `common` set first),
  then alphabetical. Cap the visible list to **~10** rows with internal scroll for the
  rest; show the count is not required.
- **Each row** shows the **function name** (matched prefix may be emphasized) and, space
  permitting, a short **argument template** (e.g. `SUMIF(range, criteria, [sum_range])`).
- **Keyboard.** While the list is open:
  - **↑ / ↓** move the highlighted row (wrapping optional; recommend clamp, no wrap).
  - **Tab** or **Enter** **accepts** the highlighted row (see "on accept").
  - **Esc** **closes the list only** — the edit continues, nothing is committed or
    reverted. A second Esc then behaves as today (revert/exit edit).
  - Any other key (typing more of the name, moving the caret with ←/→, etc.) updates or
    dismisses the list per the trigger rule; it does **not** get swallowed.
  - If no row is highlighted (or the list is closed), Tab/Enter/Esc behave exactly as
    today (commit-and-move / revert). The list must not change the meaning of these keys
    when it is not showing.
- **Mouse.** Clicking a row accepts it; clicking elsewhere dismisses the list (the edit
  stays active). Hover highlights a row.
- **On accept.** The typed prefix is replaced by `NAME(` and the caret is placed
  **immediately after the `(`** so the user types arguments next. (Excel/Sheets insert the
  open paren and leave the closing paren to autoclose / commit-time; we do **not** insert a
  trailing `)` here — see D1.2.) After acceptance the **signature hint** for `NAME` shows
  (below).
- **Signature hint.** A single-line, passive (non-interactive) hint showing the accepted
  function's argument template — e.g. `SUMIF(range, criteria, [sum_range])`, optional
  args in brackets. It shows: (a) right after accepting a completion, and (b) whenever the
  caret sits inside the parentheses of a recognized function call. It dismisses when the
  caret leaves the call or the edit ends. **Which argument is "current" is not tracked /
  bolded in this round** (D1.1) — the whole template shows.
- **Both editing surfaces.** Identical behavior in the **data row** (formula bar) and the
  **in-cell editor**; the two share one pending edit, so the list/hint is driven off the
  same text and anchored under whichever editor has focus.
- **Anchor & appearance.** The list is a small popover anchored directly **below** the
  active editor (under the data-row field, or under the measured in-cell editor box),
  matching the existing cap-error popover placement. It never covers the text being typed.
  No modal backdrop (typing continues underneath).

### Edge cases

- Prefix matches **nothing** → no list shown (not an empty box).
- Prefix exactly equals a full name **and** the user keeps typing `(` → the list dismisses
  (they've moved past the name into arguments); the signature hint may then show.
- Editing a **non-formula** cell (no leading `=`) → never triggers.
- Text is over the input cap → the cap-error popover (existing) takes precedence; the
  completion list does not fight it for the same anchor.

### Out of scope (this round; tracked as v0.5/v1.0 follow-ons)

- **Point-mode / colored range highlighting** while editing (GAPS.md:365) — the deeper
  half of formula-entry UX; separate round.
- **Current-argument highlighting** in the signature hint (bolding the arg the caret is
  in) — needs formula tokenization; D1.1 defers it.
- **Defined-name / named-range** completion, cell-reference completion, snippet-style
  arg placeholders you Tab through.

### Decisions to confirm

- **D1.1 — Signature-hint depth.** *Recommended:* **static** whole-template hint (no
  current-arg tracking). Alt: tokenize the formula to bold the active argument — richer,
  but needs the engine lexer surfaced (heavier, riskier) and pushes this out of "one
  low-risk phase."
- **D1.2 — Paren insertion on accept.** *Recommended:* insert `NAME(` only, caret after
  `(`, no auto-closing `)` (matches "type the rest yourself"; avoids caret-management
  complexity if the pinned gpui-component `InputState` lacks an insert-at-caret API —
  see architecture). Alt: insert `NAME()` with caret between parens (needs a confirmed
  caret API).
- **D1.3 — Name set.** *Recommended:* the **345 engine-registered** names (superset of
  what evaluates). Alt: only the "common" canonical subset (fewer, cleaner, but hides
  working functions).
- **D1.4 — Trigger threshold.** *Recommended:* **≥1** identifier char after `=`. Alt: ≥2
  (less noise, but misses single-letter starts).

---

## 2. CSV import + export

Open a `.csv` file as a new spreadsheet, and export the current sheet to `.csv`. Opening a
downloaded CSV is a top-3 home task; export is the round-trip partner.

### Import behavior

- **Entry points.** (a) **File ▸ Open** (and the welcome-window "Open…", and Open Recent,
  and double-click / CLI argv): if the chosen file has a `.csv` extension, it is
  **imported** rather than run through the `.xlsx` loader. (b) An explicit **File ▸ Import
  CSV…** menu item opens the same picker scoped to the action. (No new keyboard shortcut.)
- **Result — open as a new untitled workbook (D2.1).** The CSV becomes a **new, untitled**
  single-sheet workbook (title `Untitled`, dirty from creation is **not** required — treat
  like a freshly-opened doc with unsaved = false until edited). The source `.csv` path is
  **not** adopted as the document path: the first **Save** routes through **Save As** and
  proposes an `.xlsx` name. No `.back` backup is created (there is no in-place `.xlsx`
  origin to protect). The imported `.csv` is added to Recent Files (it's a file the user
  opened).
- **Parsing (RFC 4180).** Comma-delimited; fields may be double-quoted; quoted fields may
  contain commas, embedded newlines, and doubled `""` quotes. Each row becomes a grid row;
  each field is applied to a cell **as user input** — so a field that looks like a number
  becomes a number, `TRUE`/`FALSE` a boolean, a leading `=` a formula, everything else
  text. (Same "apply as user input" semantics as TSV paste.)
- **Cell placement.** Row/column origin is `A1`. **Empty fields clear** their cell (the
  target starts empty, so this is automatically satisfied — no leftover-token issue).
- **Ragged rows** (varying field counts) are accepted — short rows leave trailing cells
  empty.
- **Delimiter detection.** *This round is comma-only* (`.csv`). Auto-detecting `;` (some
  European locales) or `\t` is **out of scope** (D2.4); a semicolon file imports as
  single-column text, which is acceptable for v0.5.
- **Size / overflow.** Import is bounded by the Excel-max grid; a CSV exceeding
  1,048,576 rows or 16,384 cols is **rejected** with a clear dialog ("This CSV is larger
  than the maximum sheet size"), not silently truncated. Encoding is **UTF-8** (with BOM
  tolerated); non-UTF-8 bytes surface a readable error dialog rather than mojibake, if
  cheaply detectable (else best-effort lossy decode — D2.5).

### Export behavior

- **Entry point.** **File ▸ Export as CSV…** (window-scoped, like Save As). Opens a native
  save panel proposing `<sheetname>.csv` (or `<workbookname>.csv`). No new shortcut.
- **Scope — active sheet only.** CSV is single-sheet; export writes the **active sheet's
  used range** (`A1` through the sheet's occupied extent). Other sheets are not written
  (no multi-file export this round).
- **What is written (D2.2).** *Recommended:* each cell's **displayed value** — the
  engine-formatted string the user sees (numbers with their number format applied, dates
  as displayed, `TRUE`/`FALSE`, error text like `#DIV/0!`), **not** raw stored values or
  formulas. This matches "export what's on screen" and Excel's CSV behavior for most cells.
  (Alt in D2.2: raw values / formulas.)
- **Serialization (RFC 4180).** Comma-delimited, UTF-8, `CRLF` line endings; a field
  containing a comma, double-quote, or newline is wrapped in double-quotes with internal
  quotes doubled. Trailing empty cells in a row are omitted (no trailing commas beyond the
  used range).
- **Atomicity.** Written via the existing atomic temp-file-then-rename path; a write
  failure surfaces the standard save-error dialog and leaves any existing file intact.
- **Does not change document state.** Export is a side output: it does **not** alter the
  workbook's dirty flag, path, or title (you exported a copy; the `.xlsx` document is
  unchanged).

### Edge cases

- Exporting an **empty sheet** writes an empty (0-byte or single-newline) file — no error.
- A cell whose display string is very long is written verbatim (no cap on CSV export).
- Import of a **0-byte / header-only** CSV yields a valid one-sheet workbook (possibly with
  one row).

### Out of scope (tracked)

- True **CSV save-in-place** (a document whose canonical format is `.csv`, so ⌘S rewrites
  the `.csv`) — v1.0+; this round is import-as-`.xlsx` + export-a-copy.
- Multi-sheet export (one CSV per sheet / zip), custom delimiter/encoding pickers,
  semicolon/`\t` auto-detection, import "text-to-columns" style options dialog.

### Decisions to confirm

- **D2.1 — Import target.** *Recommended:* **untitled workbook** (simple; Save→Save-As to
  `.xlsx`). (Owner already leaned this way in the overview.)
- **D2.2 — Export values.** *Recommended:* **displayed/formatted** strings. Alt: raw
  stored values (loses number formatting) or formula text (rarely wanted in CSV).
- **D2.3 — Menu shape.** *Recommended:* Open **auto-detects** `.csv` by extension **and** a
  dedicated **Import CSV…** item exists; Export is its own **Export as CSV…** item. Alt:
  export lives under Save As with a `.csv` type instead of a separate item.
- **D2.4 — Delimiter.** *Recommended:* comma-only both directions this round.
- **D2.5 — Non-UTF-8 import.** *Recommended:* attempt UTF-8 (BOM-tolerant); on invalid
  bytes show an error dialog. Alt: lossy-decode and import anyway.

---

## 3. Drag fill handle + series autofill

A small **fill handle** at the bottom-right corner of the selection; dragging it extends
the selection's content into the swept cells — **copying** a single-cell seed, or
**extrapolating a series** (1, 2, 3…; Jan, Feb…; Mon, Tue…) from a multi-cell seed. This
is *the* signature spreadsheet affordance; its absence reads instantly as "not a real
spreadsheet" (GAPS.md v0.5, §352).

### Behavior

- **The handle.** When a range is selected, a small square handle (~the chart-handle size)
  draws at the **bottom-right corner** of the selection rectangle. It shows for any
  selection (single cell or range). It is hidden while editing a cell and while another
  drag (selection/resize/chart) is in progress.
- **Cursor.** Over the handle the cursor becomes a crosshair/`+` affordance (if the pinned
  gpui exposes cursor styling for a sub-region; otherwise no cursor change — cosmetic,
  non-blocking).
- **Drag gesture.** Mouse-down on the handle begins a **fill drag**. As the pointer moves,
  a **preview rectangle** shows the target fill region (the selection extended along the
  dominant drag axis). Fill is **one axis at a time** (Excel behavior): whichever of
  vertical / horizontal the pointer has moved farther determines the direction; the other
  axis stays pinned to the seed's span.
  - **Down / Right** extend past the seed (the common case).
  - **Up / Left** extend before the seed (also supported).
- **Auto-scroll.** Dragging near a viewport edge auto-scrolls (reusing the existing
  cell-drag auto-scroll) so you can fill beyond the visible area.
- **On release — what fills:**
  - **Single-cell seed** → **copy** the seed's value + formatting into every target cell,
    with relative-reference adjustment for formulas (identical to today's ⌘D/⌘R copy).
  - **Multi-cell seed** → the engine's **series/progression detection** runs over the seed:
    a linear numeric progression (`1,2 → 3,4,5`; `5,10 → 15,20`), and known text sequences
    the engine recognizes (months, weekdays) **extrapolate**; a non-detectable seed
    **copies/repeats** (tiling the seed pattern), matching the fork's `auto_fill_*`
    behavior. Formatting from the seed carries to the filled cells.
  - Direction is respected (dragging **up/left** reverses the progression).
- **Undo.** The whole fill is **one** undo step.
- **No-op.** Releasing without leaving the seed region (target == selection) does nothing
  (no undo entry). Dragging **inward** (shrinking below the seed) is **out of scope** this
  round — treated as a no-op, not a clear (D3.3).
- **Selection after fill.** The selection expands to cover the seed **plus** the filled
  region (Excel behavior), so a follow-up action applies to the whole run.
- **Overflow guard.** A fill whose target exceeds the sheet or a sane cell-count cap is
  rejected the same way large paste/fill is today.

### Edge cases

- Seed contains **formulas** → relative references adjust per destination cell (engine
  behavior), consistent with ⌘D/⌘R.
- Seed spans **mixed** content (numbers + text) → the engine's per-column/row detection
  applies; where no progression is found it repeats.
- Dragging exactly onto the seed's own edge → no fill.

### Out of scope (tracked)

- **Double-click the handle** to auto-fill down to the neighbor column's data extent
  (Excel convenience) — separate small follow-on.
- **Drag-inward to clear** (D3.3), **fill-handle right-drag menu** (Excel's "Fill Series /
  Copy Cells / Fill Formatting Only" context menu on right-drag), **Flash Fill**.
- Fill **across/into** merged cells (merged-cell support is its own project).

### Decisions to confirm

- **D3.1 — Direction model.** *Recommended:* **single dominant axis** per drag (Excel).
  Alt: allow rectangular 2-D fill (rarely wanted, more preview/behavior complexity).
- **D3.2 — Series vs copy trigger.** *Recommended:* single-cell seed = **copy**;
  multi-cell seed = **series if the engine detects one, else repeat** (native fork
  behavior). No separate UI toggle this round.
- **D3.3 — Drag inward.** *Recommended:* **no-op** (fill only). Alt: implement
  shrink-to-clear (Excel) — defer.
- **D3.4 — Handle visibility.** *Recommended:* show for **all** selections incl. single
  cell. Alt: hide for whole-row/column selections (where a corner is off-screen) — treat
  as an implementation clamp, not a behavior change.

---

## 4. Hide / unhide rows & columns

Hide selected rows or columns (collapse them to zero visible size, preserving their data
and their prior size), and unhide them again — via the header context menu. Files with
hidden rows/cols are everywhere; today FreeCell has no hide concept at all.

### Behavior

- **Hide.** Right-clicking a **row-number** or **column-letter** header (or a selection of
  them) shows the existing header context menu with new items:
  - **Hide** (rows / columns, label reflects the axis and count, e.g. "Hide 3 rows").
    Collapses the selected track(s) to **zero visible width/height**. Their data is
    untouched; formulas referencing them still compute.
  - The clicked track follows the existing rule: if it's outside the current header
    selection, the selection moves to it first; if inside, the whole selection is hidden.
- **Unhide.** When a selection **spans** hidden track(s) — e.g. selecting columns C:E where
  D is hidden, or selecting the whole sheet — the header menu offers **Unhide** ("Unhide
  N rows/columns"), which restores each hidden track in the span to its **prior** size
  (the size it had before hiding; a track hidden at default size returns to default).
- **Rendering of hidden tracks.** A hidden track occupies **zero** pixels: no cell, no
  header, no gridline; the neighbors abut. Clicking/selection cannot land **on** a hidden
  track (you can't select into it directly), but a range **spanning** it includes it (so
  copy/delete over the span still covers hidden cells — Excel behavior).
- **No visible "hidden here" affordance this round (D4.2).** FreeCell does not draw a
  thick divider / gap marker between non-adjacent visible headers. Unhide is reached by
  **spanning-selection → right-click → Unhide** (and Select-All → Unhide to reveal
  everything). A marker is a tracked follow-on.
- **Distinct from resize (D4.3).** Hide is an **explicit command**, not "drag to 0px".
  Dragging a divider to minimum still clamps to the existing `MIN_*` (it does not hide);
  hiding is separately tracked so Unhide can restore the pre-hide size.
- **Persistence / round-trip (D4.1).** *Recommended:* hidden state **round-trips to
  `.xlsx`** — a file saved with hidden rows/cols reopens with them hidden (in FreeCell and
  in Excel), and a file **opened** with hidden rows/cols shows them hidden. This requires
  the fork half (see below).
- **Undo.** Hide and Unhide are each **one** undo step.

### Engine / fork work (per the fork policy — one fix = one branch = one PR)

Two independent capabilities are missing in the engine and must be added in the fork and
upstreamed (not worked around in FreeCell):

1. **Row hidden setter** — an undoable `UserModel` method to set/clear a row's hidden flag
   (IronCalc models `Row.hidden` on import but exposes no setter). One `fix/` branch.
2. **Column hidden modelling + round-trip** — `Col` has **no** hidden field; add it, parse
   it on import, and emit it on export, with an undoable setter. One `fix/` branch.

FreeCell reads the hidden flags on open (currently `build_sheet_cache` ignores them) and
renders hidden tracks as zero-size.

### Edge cases

- Hiding **all** columns or **all** rows is disallowed (Excel disallows leaving a sheet
  with nothing visible) — the Hide item is disabled if it would hide every remaining
  visible track. (If cheaply detectable; otherwise clamp to leave ≥1 visible.)
- Hiding across a **frozen** boundary — freeze panes are not in this batch, no interaction.
- A track hidden then **resized via the model** externally — on reopen, hidden wins
  (zero-size) until unhidden, then restores the file's stored size.
- Inserting/deleting rows/cols adjacent to hidden ones — the existing insert/delete path
  shifts indices; hidden flags move with their tracks (engine responsibility).

### Out of scope (tracked)

- The **gap/marker affordance** between non-adjacent headers (D4.2).
- **Hide/unhide sheets** (that's the separate "sheet management extras" v1.0 row).
- Group/outline (collapse) — v2.0.

### Decisions to confirm

- **D4.1 — Round-trip vs session-only.** *Recommended:* **round-trip via the two fork
  fixes** (the point of the feature; files with hidden tracks are common). This makes §4
  the heaviest phase in the batch (two fork branches + FreeCell render/UI). *If the owner
  wants this round strictly light*, the fallback is **session-only hide** (no fork work:
  hidden state lives in FreeCell, not saved) shipped now, with round-trip as a follow-on —
  but that ships a feature that silently loses hidden state on save, which is a poor v0.5
  signal. **Flagging prominently.**
- **D4.2 — Unhide discoverability.** *Recommended:* spanning-selection + Select-All →
  Unhide, **no** header marker this round. Alt: add the thick-divider marker (new header
  render work).
- **D4.3 — Hide vs 0px resize.** *Recommended:* explicit command, **distinct** from resize.

---

## 5. Autofit row height (double-click a row divider)

Double-clicking the divider **below a row-number header** resizes that row to fit its
tallest cell — the row-height twin of the shipped autofit **column** width. Small; reuses
the column-autofit pattern and the wrap-measurement machinery.

### Behavior

- **Gesture.** Double-click the row-resize hotspot (the divider between two row-number
  headers) → the row above the divider **autofits** to its content height. (Single-click +
  drag on the same hotspot still resizes manually, exactly as today — only the
  double-click branch is new. The column hotspot already does this; the row hotspot
  currently lacks the double-click branch.)
- **Multi-row.** If the double-clicked row is part of a multi-row header selection, **all**
  selected rows autofit (each to its own content). A whole-sheet selection autofits only
  the divider's row (the same whole-sheet guard the column autofit uses).
- **Measurement.** The fitted height is the max over the row's **populated** cells of that
  cell's rendered text height at the cell's column width — accounting for **explicit
  newlines** and, for **wrap-on** cells, soft-wrapped line count; single-line cells
  contribute their font's line height. Empty cells contribute the default. The result is
  clamped to `[DEFAULT_ROW_HEIGHT_PX, MAX_AUTO_ROW_HEIGHT_PX]` (the same 24…240px clamp as
  wrap auto-grow) plus the standard vertical padding.
- **Result & undo.** Autofit sets the row height as **one undo step** (like the column
  autofit / a manual resize), reusing the existing `SetRowHeights` path. A no-op autofit
  (height unchanged) adds no undo entry (existing no-op guard).
- **Interaction with wrap auto-grow (D5.1).** *Recommended:* a double-click autofit sets an
  explicit height and **marks the row manual** — consistent with the column autofit and
  with a manual drag-resize — which means the row is thereafter **exempt from automatic
  wrap re-grow** until its content-driven height is re-established. (This matches the
  "manual resize wins" posture already in the code.) This is a slight departure from
  Excel, where double-click autofit returns the row to auto-tracking; called out in D5.1.

### Edge cases

- Autofit on an **empty** row → default height (no shrink below default).
- A row with a very tall cell (many wrapped lines / large font) → clamped at
  `MAX_AUTO_ROW_HEIGHT_PX`; content beyond clips within the cell (matching wrap auto-grow).
- Mixed font sizes in the row → fitted to the tallest.

### Out of scope (tracked)

- **Autofit-all** (double-click the corner / a menu item to autofit every row) — follow-on.
- Making autofit **re-enable** live wrap auto-grow for that row (the Excel auto-track
  behavior) — D5.1 defers.

### Decisions to confirm

- **D5.1 — Manual vs auto after autofit.** *Recommended:* autofit **marks the row manual**
  (one undo step, consistent with column autofit; wrap auto-grow no longer overrides it).
  Alt: autofit stays "auto" (cache-only, no undo, keeps live wrap tracking) — closer to
  Excel but inconsistent with the shipped column autofit and the manual-row model.
- **D5.2 — Measure scope.** *Recommended:* measure **all populated cells** in the row
  (wrap-on and wrap-off), honoring explicit `\n` and soft-wrap. (No alt — this is the
  correct behavior.)

---

## 6. Render validation (final phase — no new user-facing behavior)

Not a feature: the mandatory late render-validation pass required because §3 (fill
handle), §4 (hidden-track geometry), and §5 (row height) move grid pixels. After all
coding phases: run the **full** pixel suite (with a ~10-min watchdog), regenerate and
**eyeball** any intentionally-changed baselines (and add new cases for the fill handle and
hidden-track rendering), commit baseline updates, and dispatch the CI `render` gate to
green. Details in `architecture.md` / the phase plan.

---

## Phase ↔ feature map (for the implementation plan)

| Phase | Feature | Pixel suite? | Fork work? |
|------|---------|--------------|------------|
| 1 | Function autocomplete + signature hints | No (chrome) | No |
| 2 | CSV import + export | No (chrome/IO) | No |
| 3 | Drag fill handle + series autofill | Yes (subset) | No (generalize existing wrapper) |
| 4 | Hide / unhide rows & columns | Yes (subset) | **Yes — 2 branches** (row setter; col modelling) |
| 5 | Autofit row height | Yes (subset) | No |
| 6 | Render validation (full suite + CI gate) | **Yes (full)** | No |

Ordering rationale: the two no-pixel, no-fork chrome features (1, 2) land first; the three
grid features (3, 4, 5) follow, each verifying with a render **subset**; the single full
render run + CI gate is the dedicated final phase (6). §4 carries the only fork work and is
sequenced so its two upstream branches can be opened independently.
