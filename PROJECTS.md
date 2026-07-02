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

- **Viewport Value Cache** — *Future, optional scroll-perf push.*
  Delta-load only newly-exposed cells' *values* on scroll (styles/geometry come from the
  resident style cache above); invalidate on recompute. Optional — SP4 showed uncached
  value reads are cheap. → [`projects/viewport-cache.md`](projects/viewport-cache.md)
