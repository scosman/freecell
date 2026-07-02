# Project: Viewport Value Cache

**Status:** Future — **optional scroll-perf push.** Design note only; not started.

Styles + geometry are handled by the always-resident `projects/style-cache.md`; **this
project covers only cell _values_** (the one render input that changes on recompute).

**Relates to:** the SP1 seam (`experiments/round-2/01-async-interop/`), the SP4 read
costs, and `projects/style-cache.md`.

## Problem

Cell **values** are the one render input that changes on recompute, so — unlike
styles/geometry — they can't be cached-once. In the current SP1 seam the worker re-reads
the viewport's values on every scroll and republishes; a 1-row scroll re-reads the whole
window's values to surface ~one new row.

## Goal

Make value reads on scroll **incremental** — fetch only newly-exposed cells' values.

## Design

- Frontend cache of cell **values** keyed by address.
- **On scroll:** serve the overlap from cache → fetch only the **delta** (newly-exposed)
  cells' values → update → render. Turns an O(viewport) read into O(newly-exposed).
- **Invalidate on generation++** (a recompute completed — values may have changed). If
  FreeCell builds its own dependency/dirty tracking (SYNTHESIS Round-3 #1), invalidate
  only the *actually-changed* values instead of the whole visible set.
- **Prune** with an LRU / distance-from-viewport bound so it doesn't grow unbounded on a
  huge sheet.
- **During an eval:** the resident style cache already renders the full styled grid; this
  layer serves cached *values* for the overlap instantly, and the adopted **~3× overscan**
  covers small scrolls past it. Cells beyond the overscan show a placeholder until the
  next publish (or an on-demand `to_bytes()` snapshot read — but its ~3.2s build @1M
  exceeds the eval, so **on-demand only**).

## Is it worth it? (why it's optional)

SP4 measured an uncached viewport **value** read at ~1× (cheap — the 10× cost was styles,
now handled by the resident style cache), and the read runs on the worker, off the frame
budget. So uncached value re-reads on scroll are likely fine for typical viewports; this
cache earns its keep only if profiling shows scroll worker-load or publish latency
actually matters. Measure delta-vs-full first.
