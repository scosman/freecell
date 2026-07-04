---
status: draft
---

# MVP Gaps — Core Spreadsheet Feel

The MVP is complete. This project closes core functional gaps to make FreeCell feel
like a real spreadsheet app. The exact feature set is flexible: we want good bang for
our buck in closing the gap with a full spreadsheet app, without high technical risk or
tons of work. No one feature is required — the goal is a good set of high-value
features with balanced risk. (If any item below turns out much larger/riskier than
sized during design, dropping it back to GAPS.md is an acceptable outcome.)

Scope was decided at kickoff (2026-07-04) from the owner's initial list plus a gap
survey of `functional_spec.md §8`, `GAPS.md`, and a merged-cells feasibility
investigation against the pinned IronCalc 0.7.1 source. Features considered but cut
are recorded in `GAPS.md` ("Post-MVP UX features") — nothing was silently dropped.

## Scope

### Cell interaction & editing

- **Type-to-replace**: typing a printable character with a cell selected puts focus in
  the edit bar (data row) and replaces the current content — like Excel/Sheets.
- **Live cell mirror while typing**: as you type in the edit bar, the active cell
  shows the raw text live. *Product call:* no live evaluation before commit — a
  whole-workbook recompute per keystroke is untenable on huge sheets; mirroring raw
  text is the Excel-like behavior.
- **Inline editing**: double-clicking a cell opens an in-cell editor (input overlay
  positioned over the cell, sharing the data-row commit path, caps, and validation).
  IME/international input remains out of scope (`projects/ime-text-input.md`).
- **Range clipboard**: Cut/Copy/Paste of cell ranges — internal (values + formulas
  with reference adjustment as the engine dictates), plus plain-text TSV to and from
  other apps. Rich Excel clipboard interop (HTML/XML flavors, styles) stays a separate
  project (`projects/excel-clipboard.md`).

### Formatting & rendering

- **Font family + size** selection in the action bar; the grid learns to render
  per-cell font family/size (today: one bundled face, one size).
- **Borders**: render cell borders loaded from files, plus a simple borders menu in
  the action bar (common presets: all/outline/top/bottom/left/right/none).
- **Text color** button and **horizontal alignment** controls (left/center/right) in
  the action bar — same palette/popover infra as the existing fill-color button; the
  grid already renders explicit alignment.
- **Number-format dropdown**: common formats (General, Number, Currency, Percent,
  Date) + increase/decrease decimals. Display side is already engine-owned and works.
- **Type-aware default alignment + `[Red]` format color** (GAPS #1/#2): numbers/dates
  right-aligned, booleans/errors centered by default; number-format text color
  published per cell. Per `projects/type-aware-alignment.md` — the publication layer
  carries per-cell type/color.
- **Merged cells — render + selection (tiers a+b)**: merges loaded from `.xlsx` render
  correctly (anchor spans the region, covered cells and interior gridlines
  suppressed) and behave correctly (clicking a covered cell selects the merge, ranges
  expand to whole merges, editing routes to the anchor). Save round-trip already works
  in IronCalc 0.7.1 — add tests. **Zero engine changes needed** (verified against
  pinned source). Merge/unmerge *UI* (tier c) is explicitly out —
  `projects/merged-cells.md`.

### Structure & navigation

- **Row/col resize**: drag header dividers to resize, with the correct resize arrow
  cursors on hover. Geometry model already supports non-uniform sizes.
- **Insert/delete rows & columns UI**: engine + resident cache support was validated
  in round-3; this adds the UI. Entry point: right-click menu on row/col headers (a
  general cell-area context menu stays out of scope — GAPS.md). Must not corrupt
  merged-cell refs (IronCalc doesn't adjust `merge_cells` on structural edits —
  normalize/guard FreeCell-side; see `projects/merged-cells.md`).
- **Header selection**: click a row/col header to select the whole row/col, drag for
  multiple, corner button = select all. (Currently a stubbed no-op in the grid.)
- **Zoom dropdown** in the action row controlling sheet-area zoom (grid is fully
  custom-rendered, so zoom = scale factor through layout + text). *Punt-allowed:* if
  design reveals it's super complex, drop to GAPS.md.

### Chrome & data safety

- **Uniform titlebar grey** (design nit): the title bar is OS-drawn today — FreeCell
  controls no color there. Adopt a client-side/tinted titlebar (Zed-style) so it
  matches the action-bar grey (`CHROME_BG 0xF3F3F3`) for a uniform look. Needs a small
  gpui capability spike at the pinned rev.
- **Cap-error message popover** (GAPS #3): wire `DataRowEffect::ShowCapError` to show
  the specced "Formula too long / too deeply nested" reason; the reject behavior
  itself already works.
- **`.back` backup before first save** (GAPS data-safety item, High): before the first
  save of a document opened from disk, copy the original file to
  `<name>.xlsx.back`, write-once. Cheap insurance while the writer strips
  unmodeled features.

## Explicitly out (recorded, not lost)

Grid cell context menu; fill down/right + fill handle; find/replace; autofit column
width; Cmd+arrow edge-of-data; recent files; freeze panes; sort/filter; text
overflow/wrap; merge/unmerge UI (tier c); rich Excel clipboard; IME; dynamic arrays;
bundled Inter font. All in `GAPS.md` / `PROJECTS.md`.
