# FreeCell — Projects

Forward-looking product/engineering initiatives for FreeCell. This is a lightweight
registry: each entry is a short description plus a pointer to a design note under
[`projects/`](projects/).

> Not to be confused with [`specs/projects/`](specs/projects/), which holds the
> spec-driven **planning + build** artifacts for a phase of work (overview →
> functional spec → architecture → implementation plan). `projects/` here is a
> backlog of *initiatives and design notes* — some future, some speculative.

## Backlog

- **Viewport Value/Style Cache** — *Future, if we want to push scrolling perf.*
  A frontend cache of visible cells (value + style) so scrolling fetches only the
  *delta* cells instead of re-reading the whole viewport each move, keeps scrolling
  live during a recompute, and caches the expensive style reads across recomputes.
  → [`projects/viewport-cache.md`](projects/viewport-cache.md)
