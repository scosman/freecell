# FreeCell — Projects

Forward-looking product/engineering initiatives for FreeCell. This is a lightweight
registry: each entry is a short description plus a pointer to a design note under
[`projects/`](projects/).

> Not to be confused with [`specs/projects/`](specs/projects/), which holds the
> spec-driven **planning + build** artifacts for a phase of work (overview →
> functional spec → architecture → implementation plan). `projects/` here is a
> backlog of *initiatives and design notes* — some future, some speculative.

## Backlog

- **All-Styles Resident Cache (grid geometry + styling)** — *Near-MVP.*
  An always-resident cache of the full resolved style for the sheet — **all** row/col
  sizes (geometry) + fills/lines/bold/number-format — **not** viewport-based. Needed to
  render the grid at all (geometry), takes the ~10× style read (SP4) off the scroll path,
  and — since styles/sizes don't change during a recompute and it's frontend-resident —
  lets the grid render **fully-styled during an eval** (only cell values lag).
  → [`projects/style-cache.md`](projects/style-cache.md)

- **`.xlsx` Preservation on Save** — *Future (post-MVP by product call, 2026-07-02).*
  IronCalc's writer silently drops everything it doesn't model (comments, validation,
  hyperlinks, merges, CF — and charts/pivots/drawings/VBA were never examined), so
  "open a colleague's file, fix one cell, save" is destructive. MVP ships this
  behavior with no warning (decided in MVP planning Round 1); this project adds the
  warn-and-strip UX first, then weighs zip-level unknown-part pass-through vs owning
  the writer, plus the real-file-corpus de-risk.
  → [`projects/xlsx-preservation.md`](projects/xlsx-preservation.md)

- **IME / International Text Input** — *Future (post-MVP by product call, 2026-07-02).*
  Full IME (CJK composition), dead keys, layouts, decimal-comma entry for the custom
  raw-gpui cell editor. What GPUI exposes at the pinned rev is unknown — carries a cheap
  probe to run before the editor architecture hardens.
  → [`projects/ime-text-input.md`](projects/ime-text-input.md)

- **Excel Clipboard Interop** — *Future (post-MVP by product call, 2026-07-02).*
  Rich range copy/paste with Excel via TSV + HTML/XML-Spreadsheet clipboard flavors.
  All FreeCell-side work (IronCalc's clipboard isn't externally chainable); plain TSV
  values may ride along with the editor build.
  → [`projects/excel-clipboard.md`](projects/excel-clipboard.md)

- **Viewport Value Cache** — *Future, optional scroll-perf push.*
  Delta-load only newly-exposed cells' *values* on scroll (styles/geometry come from the
  resident style cache above); invalidate on recompute. Optional — SP4 showed uncached
  value reads are cheap. → [`projects/viewport-cache.md`](projects/viewport-cache.md)
