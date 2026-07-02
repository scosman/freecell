---
status: complete
---

# Functional Spec: FreeCell MVP

The first real FreeCell application: a macOS, GPU-rendered (GPUI), Excel-compatible
spreadsheet on the IronCalc engine. This MVP is a **workable functional proof of
concept** — it opens/edits/saves real `.xlsx` files, feels fast on huge sheets, and
establishes the app scaffolding, testing infrastructure, and architecture the full
product grows from. It is **not** a design-polished or feature-complete product.

All engine/architecture constraints referenced here were validated in the de-risking
rounds (`experiments/round-2/SYNTHESIS.md`, `experiments/round-3/SYNTHESIS.md`). Verdict
there: CLEAR TO BUILD. This spec builds exactly what those syntheses adopted.

## 1. Product scope at a glance

| Area | In MVP | Explicitly out (P2+) |
|---|---|---|
| Platform | macOS (Metal) only | Windows/Linux |
| Files | Open/Save/Save-As `.xlsx` (IronCalc-native features) | CSV, xlsx feature pass-through (merges/CF/comments/etc. — see §8) |
| Editing | Formula-bar (data-row) editing, delete-to-clear, undo/redo | In-cell editor, IME, clipboard interop, fill handle, autocomplete |
| Formatting | Bold / italic / underline / fill color; engine-side number-format display | Borders UI, fonts UI, alignment UI, number-format UI |
| Structure | Multiple sheets: add / rename / delete / switch | Insert/delete rows & cols UI, row/col resize (P2), hide, freeze, zoom, sort/filter |
| Selection | Single cell + rectangular range; keyboard + mouse | Row/col-header select-all, multi-range (Cmd+click), named ranges |
| Grid scale | Full Excel max: 1,048,576 rows × 16,384 cols, 120 fps scroll budget | — |
| Windows | Welcome window at launch + one window per workbook, standard macOS menus; app quits when last window closes | Tabs-in-window, session restore |
| Formulas | Full IronCalc formula support: entry, recalc, cross-sheet refs, error values | Dynamic arrays/spill (accepted absent for v1), autocomplete |

"Near parity with the big-name spreadsheets" is the **product trajectory**, not the MVP
bar. The MVP bar: a person can open a real Excel file, look around a million-row sheet
smoothly, edit values and formulas, apply basic formatting, and save — without crashes,
data corruption, or UI freezes.

## 2. Application lifecycle & windows

### 2.1 Launch

- Launching the app with no document opens the **Welcome window**.
- Opening a `.xlsx` from Finder (double-click / drag onto Dock icon / `open -a`) opens
  that file directly in a spreadsheet window (macOS open-file events), skipping Welcome.

### 2.2 Welcome window

Small, fixed-size, non-resizable window, centered:

- App name/logo (text is fine).
- **New Spreadsheet** button → creates an empty workbook (one sheet, "Sheet1") in a new
  spreadsheet window; Welcome window closes.
- **Open…** button → native macOS file picker filtered to `.xlsx`; on success the
  workbook opens in a new spreadsheet window and Welcome closes; on cancel, Welcome
  stays.
- No recent-files list in MVP (P2).

### 2.3 Spreadsheet windows

- One window per workbook. Each window owns its full document state (engine model,
  worker thread, caches, selection, dirty flag). **No document state is shared across
  windows**; the only app-global state is window bookkeeping + menu wiring.
- Default size ~1200×800, resizable, standard macOS traffic lights.
- Window title: file name (`Budget.xlsx`) or `Untitled` for new workbooks. The macOS
  document-edited indicator (dot in close button) reflects the dirty flag; if GPUI
  doesn't expose it at our pinned rev, suffix the title with `— Edited` instead.
- Closing a window with unsaved changes prompts: **Save / Don't Save / Cancel**.
- When the last window closes (spreadsheet or Welcome), **the app quits** (product
  call, planning Round 1). Quit via menu/Cmd+Q behaves the same: prompts per-window
  for unsaved changes, any Cancel aborts the quit.

### 2.4 Menus (macOS menu bar)

- **FreeCell** (app menu): About FreeCell, Quit (Cmd+Q).
- **File**: New (Cmd+N — new workbook window), Open… (Cmd+O), Save (Cmd+S), Save As…
  (Cmd+Shift+S), Close Window (Cmd+W).
- **Edit**: Undo (Cmd+Z), Redo (Cmd+Shift+Z). (No clipboard items in MVP; see §8.)
- Menu items enable/disable by context (e.g., Save disabled when Welcome is frontmost;
  Undo/Redo track the focused window's history availability).

## 3. Spreadsheet window layout & behavior

Vertical stack (details in `ui_design.md`):

1. **Action row** (toolbar, shared grey background): Bold, Italic, Underline toggle
   buttons; Fill-color button with a small palette dropdown.
2. **Data row** (formula bar, same grey background): cell-reference box (read-only in
   MVP, e.g. `B7`) + single-line text input showing the active cell's raw content.
3. **Grid** — full-bleed custom spreadsheet component (the one surface we design well).
4. **Sheet tab bar**: one tab per sheet + a `+` button.

### 3.1 Grid & scrolling

- Renders the full Excel-max sheet (1,048,576 × 16,384) with fixed column headers
  (`A`…`XFD`) and row headers (`1`…`1048576`), gridlines, and cell content.
- Scrolling both axes: trackpad/scroll-wheel with pixel precision, matching the
  raw-gpui demo app (`experiments/04-ui-poc/raw-gpui`) in feel and mechanism. Scrolling
  clamps at sheet edges. No scroll animation/kinetics beyond what the OS events give us.
- Scrollbars on both axes (proportional thumb; draggable). Keyboard navigation scrolls
  the viewport to keep the active cell visible.
- Rendering stays under the frame budget (p99 < 8.33 ms) while scrolling, including
  during a background recompute — styles/geometry come from the resident cache, values
  from the published viewport (~3× overscan). Scrolling past the overscan mid-eval shows
  blank values (never stale wrong values, never a freeze) until the next publish.
- Row heights / column widths honor values loaded from the file (per-row/col overrides
  + defaults). MVP has no resize UI (P2), but the geometry model supports non-uniform
  sizes from day one.

### 3.2 Selection

- **Click** a cell → single active cell, outlined (2px accent border, Excel-style).
- **Drag** → rectangular range selection. **Shift+click / Shift+arrows** extend the
  range from the anchor. Range shows a light accent overlay + border around the range;
  the anchor cell stays visually distinct (no overlay).
- **Arrow keys** move the active cell; **Cmd+arrow** jumps to the sheet edge in that
  direction (MVP: edge of sheet, not edge-of-data). **Tab/Shift+Tab** move right/left;
  **Enter/Shift+Enter** move down/up (after committing any pending data-row edit).
- Selection is per-sheet and restored when switching back to a sheet (in-session only;
  not persisted to file).
- Single selection: data row shows the active cell's content; action-row toggles show
  that cell's state.
- Multi selection: data row is **disabled** (greyed, shows the anchor cell's content is
  NOT required — it shows empty); the cell-reference box shows the range (`B2:D9`).
  Action-row buttons remain enabled and apply to every cell in the range.

### 3.3 Data entry (data row)

- The data row shows the active cell's **raw content**: the formula text (`=SUM(A1:A5)`)
  for formula cells, the literal for value cells, empty for empty cells.
- Typing in the data row edits a pending value. **Enter** commits: the input string is
  handed to the engine as user input (engine parses numbers, booleans, formulas, text),
  the evaluate loop runs (§4), and the active cell moves down one row. **Escape**
  reverts the data row to the cell's current content and cancels the edit. Clicking a
  different cell while an edit is pending commits the pending edit first (Excel
  behavior), then moves selection.
- **Delete/Backspace** with the grid focused (not editing) clears the content of all
  selected cells (one undo step).
- **Formula input cap** (load-bearing, from round-3 D): before any input string reaches
  the engine, reject formulas with **length > 8192 chars** or **nesting depth > 64**
  (matching Excel's own limits, well under the measured abort ceilings). Rejection keeps
  focus in the data row and shows an inline error ("Formula too long / too deeply
  nested"); the cell is not modified.

### 3.4 Formulas (explicit — this is the point of a spreadsheet)

- Input starting with `=` is a formula; the engine parses and evaluates it. The full
  IronCalc function set applies (345 built-ins, 96.4% golden-correctness per SP3) —
  FreeCell adds no formula logic of its own and imposes no allowlist.
- References work as the engine defines them: relative/absolute (`A1`, `$A$1`),
  ranges (`A1:B9`), cross-sheet (`Sheet2!A1`), defined-name references resolve if
  present in the file (no UI to create them in MVP).
- Editing any cell triggers the evaluate loop (§4): the whole workbook recomputes
  (IronCalc is non-incremental) and dependent cells update on publish.
- Error results (`#DIV/0!`, `#NAME?`, `#VALUE!`, `#CIRC!`, …) are values, rendered
  in-cell as the engine's display text. Circular references resolve to `#CIRC!` in
  milliseconds (validated) — never a hang.
- Known absence: dynamic arrays/spill (FILTER/SORT/UNIQUE) — accepted for v1
  (planning Round 1); such formulas surface whatever error the engine returns.
- A cell's formula text (not its result) is what the data row shows and edits.

### 3.5 Formatting actions

- **Bold / Italic / Underline** toggle buttons. Multi-cell toggle semantics (Excel):
  if any selected cell lacks the attribute → set it on all; if all have it → clear it
  on all. One undo step per click.
- **Fill color**: button opens a small fixed palette (~10 colors + "No fill"). Applies
  the background fill to all selected cells. The button is a stock gpui-component
  control; no custom color wheel in MVP.
- Formatting applies to cells (not ranges-as-objects); applying to a multi-cell
  selection writes per-cell styles through the engine's style API so they persist to
  `.xlsx`.
- Action-row button state reflects the **active cell** (pressed = attribute set). For
  multi selections, state still reflects the anchor/active cell (cheap, predictable).

### 3.6 Cell rendering (what the grid can draw in MVP)

- Text runs with **bold / italic / underline** (and their combinations) in the app's
  single default font family/size.
- **Background fill** color (solid).
- **Display text is engine-owned**: the grid renders the string from the engine's
  formatted-value API (number formats, dates, percentages, currency, thousands
  separators) — FreeCell implements **no** number-format logic. The engine's `[Red]`-
  style format color is applied to the text when present.
- Error values render as their engine text (`#DIV/0!`, `#N/A`, `#CIRC!`, …).
- Horizontal alignment: explicit alignment from the cell style if set; otherwise
  Excel's defaults (text left, numbers/dates right, booleans/errors center).
- Text longer than the cell clips at the cell boundary in MVP (no overflow into empty
  neighbors, no wrap — P2). Vertical alignment: centered.
- Not rendered in MVP (silently ignored on screen, preserved in the engine and saved):
  borders, font family/size/color overrides, strikethrough, wrap, indent, rotation.

### 3.7 Sheets & tab bar

- One tab per sheet, in workbook order; the active tab is visually distinct. Click to
  switch. Each sheet keeps its own scroll position + selection in-session.
- **`+` button** appends a new empty sheet named `SheetN` (smallest N avoiding
  collisions) and switches to it.
- **Double-click** a tab title → inline rename (text field in place). Commit on
  Enter/focus-loss; Escape cancels. Validation (xlsx rules): non-empty, ≤31 chars, no
  `: \ / ? * [ ]`, not just an apostrophe-wrapped blank, case-insensitively unique.
  Invalid rename reverts with a brief inline error state.
- **Right-click** a tab → context menu: Rename, Delete. Delete requires >1 sheet
  (disabled otherwise) and shows a confirmation dialog when the sheet has any content.
- Sheet operations run through the engine (they affect formulas that reference the
  sheet and round-trip to file) and are undoable via the engine's history where the
  engine supports it.

## 4. The evaluate loop (data-update behavior)

The contract users observe; the mechanism is in `architecture.md` (the validated SP1
worker seam):

- Commits (cell edit, clear, formatting, sheet ops, undo/redo) are sent to a
  per-window **engine worker**; the UI thread never runs an evaluation.
- Rapid successive edits **coalesce** — pending edits queue while an evaluation runs and
  apply as a batch before the next evaluation (30 edits → 1 eval, per SP1).
- When an evaluation completes, the worker **publishes** a fresh viewport snapshot
  (values + a generation counter); the grid re-pulls and repaints. Staleness bound:
  values may lag by **at most one evaluation** (~1.3 s at 1M cells on the container
  floor; faster on real hardware). Styles, geometry, selection, and scrolling are
  **never** stale or blocked — only values can lag.
- **Evaluating spinner** (product call, planning Round 1): a small spinner in the
  **top-right of the action row** that appears only when an evaluation has been in
  flight for **> 250 ms**, and stays until it completes. Small sheets never see it;
  huge sheets read as working, not broken. No modal blocking, no other progress UI.
- Edits made **during** an eval are accepted (optimistic UI: the edited cell shows its
  new raw input immediately as pending) and evaluated in the next cycle.

## 5. File operations

### 5.1 Open

- Native picker (or Finder event), `.xlsx` only.
- Opening a file that's already open in a window focuses the existing window instead of
  opening a duplicate (match by canonical path).
- Large files: the window opens immediately with a **loading state** (spinner +
  "Opening <name>…"); parse happens off the UI thread. 100 MB-class files take ~20 s
  (measured floor) — the app stays responsive and the window can be closed to cancel.
- First paint uses the file's **cached values** (no recompute on open) — matching SP2:
  first paint ≈ parse time, no full evaluate needed to show the sheet.
- Failure (corrupt zip, not-an-xlsx, unreadable, password-protected): the loading
  window closes (or never leaves Welcome) and a dialog reports the file name + a
  human-readable reason. Never a crash.

### 5.2 Save / Save As

- Save writes through the IronCalc writer. Save on an `Untitled` workbook = Save As.
  Save As shows the native save panel, defaulting to `Untitled.xlsx` / current name,
  enforcing the `.xlsx` extension.
- Writes are **atomic**: write to a temp file in the destination directory, then rename
  over the target. A failed write never destroys the existing file. Errors (permissions,
  disk full) surface in a dialog; the document stays dirty.
- **Save fidelity (product call, planning Round 1): out of MVP scope.** IronCalc's
  writer only persists what it models — merges, conditional formatting, comments,
  validation, hyperlinks, charts, pivots, drawings, VBA are silently dropped on save.
  The MVP ships this behavior as-is, with **no warning dialog**. The warn-and-strip
  UX (and any smarter preservation) is a tracked post-MVP project:
  `projects/xlsx-preservation.md` / `PROJECTS.md`.
- Save clears the dirty flag; any edit (data, style, structure, rename) sets it.

## 6. Errors, robustness & edge cases

- **Formula errors** are data, not failures: rendered as engine error text, never
  dialogs.
- **Circular references**: engine returns `#CIRC!` in ms (validated) — no hang, no
  special UI.
- **Input cap** (§3.3) eliminates the engine parser's stack-overflow abort at the
  source. Defense in depth (both cheap, from round-3 D): the engine worker runs with a
  **64 MiB stack**, and applies edits + evaluate inside `catch_unwind` — a caught panic
  drops the offending edit, restores the previous published state, and shows "That
  change couldn't be applied" (document intact, app running).
- **Worker death** (should be unreachable): the window shows a non-dismissable error
  bar offering Save As (from the last good model state if reachable) — never silent
  data loss.
- Sheet-name validation per §3.7; formula-bar junk input is engine-handled (becomes
  text or a typed error — validated in round-3 D, never a panic).
- Opening read-only locations works; Save then fails with a clear dialog → user Save-As
  elsewhere.
- Unsaved-changes prompts on window close and app quit (§2.3).

## 7. Performance requirements (gates, not aspirations)

Carried from the validated experiment gates; CI-benchmarked where marked:

| Metric | Budget | Where enforced |
|---|---|---|
| Scroll frame time, 1M×100 styled sheet | p99 ≤ 8.33 ms (120 fps), worst-case ≤ 16.67 ms | perf harness (run-test scenario from the POC), CI on macOS runner |
| Viewport cell load (value+style read for visible cells) | p99 < 2 ms | perf harness |
| Style/geometry lookups during scroll | resident cache only — zero engine calls on the scroll path | code review + harness assertion |
| Edit → UI ack (pending state visible) | < 1 frame | manual + unit |
| Staleness during recompute | ≤ 1 evaluation duration; UI interactive throughout | worker-seam tests |
| 100 MB styled open | responsive UI throughout; loading state ≤ parse time + 2 s | manual |

Memory: resident style/geometry cache is O(styled cells + row/col overrides), not
O(sheet area) (per the locked round-3 A design). No per-cell allocation for the empty
expanse.

## 8. Explicitly out of scope (MVP)

Recorded so "near parity" doesn't creep in during implementation. Each is either P2/P3
or an already-tracked project (`PROJECTS.md`):

- **In-cell editing** (edit directly in the grid cell), IME/international input
  (`projects/ime-text-input.md`), autocomplete/function hints.
- **Clipboard**: no Cut/Copy/Paste of cells in MVP, internal or Excel-interop
  (`projects/excel-clipboard.md`). (Text-field-level clipboard inside the data row —
  standard NSText-style editing — works as normal.)
- **Structural edits UI**: insert/delete rows/columns (the engine + cache design fully
  support it — validated in round-3 A — but no MVP UI). Row/col **resize by dragging is
  P2** (first follow-up), per the overview.
- **Dynamic arrays / spill**: product decision **accept absence for v1** (0/17 in
  IronCalc; FILTER/SORT/UNIQUE return errors as the engine emits them). Logged in
  DECISIONS_TO_REVIEW.md.
- **Merged cells, conditional formatting, comments, validation, hyperlinks**: no
  IronCalc public API; files containing them open but these features don't render and
  are silently stripped on save (§5.2). The warn-and-strip dialog + preservation
  options are the post-MVP `projects/xlsx-preservation.md` project.
- CSV import/export; recent-files; printing; find/replace; sort/filter; freeze panes;
  hide rows/cols; zoom; charts/images; named-range UI; multi-range selection;
  row/col-header selection; fill handle; window session restore; autosave;
  non-macOS platforms.

## 9. Testing & quality bar (functional view)

Full strategy in `architecture.md`; the functional requirements:

- **Every phase ships tested well enough to not need human review** (per the overview's
  autonomy goal): unit tests for logic, integration tests for the worker seam and file
  round-trips, and the render suite for pixels.
- **Cell-render test suite** (first-class deliverable): automated
  render-cell-to-PNG-vs-reference tests on the macOS CI runner via GPUI offscreen Metal
  capture + the perceptual diff validated in round-3 C. One test per formatting feature
  and most meaningful permutations (`bold`, `italic`, `bold_italic`,
  `bold_italic_underline`, `fill_red`, `bold_fill_yellow`, `number_format_currency`,
  `error_value`, `clipped_text`, alignment cases, …) with interpretable snake_case
  names. A `generate_baselines` script regenerates references; the suite README
  documents the human process (run, visually verify, commit). Infra is reusable so every
  future rendering feature adds tests cheaply.
- **File round-trip tests**: open→save→reopen preserves values, formulas, styles,
  number formats, sheet structure for IronCalc-native features (leaning on SP5's
  measured fidelity).
- **Worker-seam tests**: coalescing, generation ordering, staleness bound, input cap,
  catch_unwind recovery — the SP1/round-3 A test patterns, ported to the real app.
- **CI**: GitHub Actions — fmt + clippy (warnings deny) + GPUI-free crate tests on
  Linux; full build + all tests + render suite + perf smoke on a macOS runner.
