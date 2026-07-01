# Project: Global Row/Column Size Cache (grid geometry)

**Status:** Future — design note. *Likely needed early: the grid cannot lay out without
it. Tracked here so we design it deliberately rather than discovering it mid-build.*

**Relates to:** the SP1 seam + scroll-during-eval probe
(`experiments/round-2/01-async-interop/`), and `projects/viewport-cache.md` (the
per-cell value/style *viewport* cache). This is the geometry layer those sit on top of.

## Problem

To render / virtualize the grid we need **every** row height and column width — not
just the viewport's. The scroll math (cumulative-size prefix sums + binary search to map
a scroll offset to the visible range, as in the Phase-1 `poc-core`) needs the sizes of
*all* rows/cols, including off-screen ones, to know where the viewport sits and how tall
/ wide the scrollable content is. So this is inherently **global**, not viewport-scoped.

And it must be **readable during a recompute.** IronCalc's size getters take `&self` on
the model the worker holds `&mut` inside `evaluate()`, so — exactly like cell reads (SP1
scroll-during-eval probe) — they are **blocked for the whole eval**. If sizes lived only
on the model, the grid couldn't lay out while a recompute runs. Scrolling during an eval
may show **blank values** for a moment, but it **must** still have heights/widths to
render the grid at all.

## Goal

A resident, always-readable cache of **all** row heights + column widths, independent of
the eval worker, so grid geometry is available every frame regardless of recompute
state.

## Design

- On load, pull all row heights + column widths from IronCalc into a compact structure:
  a **default size + a sparse map of non-default rows/cols** (most rows are the default
  height, so this stays small even at Excel-max: 1,048,576 × 16,384).
- Feed the cumulative-size prefix-sum structure the renderer already uses for scroll
  math.
- Update on explicit row-resize / column-resize edits (edit-sites are known; sizes do
  **not** change on recompute).
- Never gated on the eval worker — geometry is constant across a recompute, so it's
  always available for rendering.

## Generalize to a full style cache?

Row/column sizes are part of a cell's resolved **style** (IronCalc stores them alongside
fills, borders, fonts). So this could — and maybe should — be implemented as an
**all-cells style cache**: cache the full resolved style (height, width, fill/highlight,
borders/lines, bold/italic, number format) globally, not just the viewport. One such
cache would serve grid **geometry** *and* render **styling** *and* the style half of
`projects/viewport-cache.md`. And like sizes, styles don't change during a recompute, so
a full style cache is also always-readable mid-eval.

- **Minimum (must-have):** the global size cache — geometry is non-negotiable for
  layout.
- **Superset (nice):** an always-resident full style cache that subsumes the size cache
  and the viewport-cache's style half, leaving only per-cell **values** as the
  eval-dependent, viewport-delta-loaded part.

## Why it's likely near-MVP, not just an optimization

Unlike the viewport *value* cache (an optimization — SP4 showed an uncached viewport
read is only ~1–2ms), the grid **cannot render at all without every row/col size**. So
the minimum size cache is closer to baseline than optional.
