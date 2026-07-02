# Project: All-Styles Resident Cache (grid geometry + styling)

**Status:** **Near-MVP.** Not a "push perf later" optimization — the grid can't render
without it. Tracked here as a design note; expected to be built early in the real app.

**Subsumes** the former grid-size cache (row/col sizes are part of this) and the **style
half** of `projects/viewport-cache.md` (which now covers only cell *values*).

**Relates to:** the SP1 seam + scroll-during-eval probe
(`experiments/round-2/01-async-interop/`) and the SP4 styled-read cost
(`experiments/round-2/04-styled-read/`).

## What

An always-resident **frontend** cache of the full resolved style for the sheet — **not**
viewport-scoped:
- **row heights + column widths** (grid geometry), and
- **fills / highlights, borders / lines, bold / italic / font, number format** (render
  styling).

## Why it's near-MVP (three reasons converge)

1. **Geometry is mandatory.** Virtualizing the grid needs *every* row/col size for the
   cumulative-size prefix-sum + binary-search scroll math (Phase-1 `poc-core`) — you
   can't lazily fetch only the viewport's sizes.
2. **Styles are the expensive read.** SP4 measured `get_style_for_cell` at **~10× a value
   read**. Caching styles once on load removes that cost from every scroll — the worker
   re-pull then fetches only cheap *values*.
3. **It must survive a recompute — and it does, for free.** IronCalc's size/style getters
   take `&self` on the model the worker holds `&mut` during `evaluate()`, so they're
   **blocked for the whole eval** (SP1 scroll-during-eval probe). But styles/sizes
   **don't change during a recompute**, and this cache lives in the frontend (not the
   model) — so it's **always readable**. Result: **you can scroll anywhere during a
   multi-second eval and render a fully-styled grid (geometry + highlights + lines +
   formatting); only cell *values* lag** until the eval publishes.

## Design

- On load, pull the full resolved style + sizes into a compact structure: a **default +
  sparse overrides** per axis (row/col band sizes and styles) and per cell, with
  **interned `StyleId`s** (styles are highly repetitive — SP4/SP5 — so dedup
  aggressively). Sparse + interned keeps it small even at Excel-max.
- Feed the sizes into the renderer's cumulative-size prefix-sum structure.
- **Frontend-resident, independent of the eval worker** → always readable, including
  mid-eval.
- **Invalidate surgically on style edits only** (bold, resize, fill, …); IronCalc reports
  edit-sites for those. Styles never change on recompute, so there is **no per-eval
  invalidation**.

## Relationship to the value layer

This cache handles everything about rendering **except cell values.** Values are the one
eval-dependent input, handled separately by the optional viewport value cache
(`projects/viewport-cache.md`). Together — resident styles/geometry + viewport-delta
values — they give a fully-cached grid that renders live during recomputes.
