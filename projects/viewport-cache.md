# Project: Viewport Value/Style Cache

**Status:** Future — **only if we want to push scrolling performance.** Design note
only; not started. Everyday scrolling is likely fine without it (see "Is it worth
it?").

**Relates to:** the SP1 engine↔render seam (`experiments/round-2/01-async-interop/`),
the SP4 styled-read cost (`experiments/round-2/04-styled-read/`), and the Round-3
"FreeCell-owned dirty-tracking" item in `experiments/round-2/SYNTHESIS.md`.

## Problem

In the current SP1 seam, a scroll sends `SetViewport` and the worker republishes by
re-reading **every** cell in the new window. A 1-row scroll re-reads ~1,800 cells to
surface ~30 new ones, and that repeats hundreds of times while dragging. Two costs:

1. **Redundant reads.** Most of a scrolled viewport overlaps the previous one, yet the
   whole window is re-read every move. Styled reads are ~10× a value read (SP4), so
   this is the expensive kind of read being repeated needlessly.
2. **Scroll-during-recompute.** The worker owns the model and is blocked inside a
   multi-second `evaluate()` on a huge edit, so it can't service a scroll until the
   eval finishes (being verified by the SP1 "scroll-during-eval" probe). During that
   window, newly-scrolled-in cells can't be read from the live model.

## Goal

Make scrolling cheap and **live regardless of recompute state**, without settled
values (stale is fine for scrolling).

## Design

A **frontend cache** of cells keyed by address, holding `value` and `style`
separately because they invalidate on completely different events:

- **On scroll:** compute the new viewport → serve the overlap from cache → fetch only
  the **delta** (genuinely-new) cells → update cache → render. Turns an O(viewport)
  read into O(newly-exposed cells).
- **Value invalidation — on generation++ (a recompute completed).** A cascade can touch
  any visible cell, so refresh the visible set's values. If FreeCell later builds its
  own dependency/dirty tracking (Round-3 #1), invalidate only the *actually-changed*
  cells instead of the whole viewport.
- **Style invalidation — NOT on recompute.** Styles don't change during `evaluate()`;
  they change only on an explicit style edit, whose edit-sites IronCalc *does* report.
  So **cache styles across generations** and invalidate them surgically. Since styles
  are the ~10× read (SP4), this is the biggest win: the expensive half almost never
  invalidates.
- **Scroll-during-recompute:** the frontend serves the cached overlap instantly (no
  worker needed). For cells *outside* the cache, a **~3× overscan** published window
  (the SP1 probe's recommended default) absorbs normal interactive scrolling during an
  eval for free; only a scroll past that margin needs an on-demand snapshot read or a
  placeholder until the next publish. This keeps scrolling live while the worker is
  busy evaluating.
- **Pruning:** bound the cache with an LRU / distance-from-viewport threshold so it
  doesn't grow unbounded as the user roams a huge sheet.

**Reading genuinely-new cells *during* an eval — RESOLVED** (SP1 scroll-during-eval
probe, `experiments/round-2/01-async-interop/`): a read on the *live* model is blocked
for the whole eval (`evaluate(&mut self)`'s exclusive borrow excludes any concurrent
`get_cell_value_by_index(&self)`; a scroll issued mid-eval only lands after it, ~1.4s
@1M). Two fixes were measured: (a) a `to_bytes()` **snapshot** serves stale reads
concurrently (p99 ~3µs) but costs **~3.2s to build @1M — worse than the eval it would
paint over**, so it must never be built per-eval; (b) a **~3× overscan** published
window gives ±(k−1)/2·V free scroll headroom (±40 rows / ±12 cols for a 40×12
screenful) at **zero extra memory**. **Decision: default to ~3× overscan; build a
snapshot only on-demand if the user scrolls past the overscan margin mid-eval.**

## Is it worth it? (why this is "future", not "now")

SP4 measured a normal viewport re-read (value+style, ~1,800 cells) at **~1–2 ms** — under
a frame, and on the worker (off the render path). So **uncached scrolling is probably
fine for a typical viewport.** The cache earns its keep specifically for:
- large overscan windows (SP4's 10k-cell read *fails* the frame budget),
- keeping scrolling live *during* a multi-second recompute, and
- cutting worker CPU / contention with evals under rapid dragging.

Cheap to de-risk first: a *delta-read vs full-read on scroll* micro-benchmark plus the
*scroll-during-eval* responsiveness check (partly covered by the SP1 probe). Build the
cache only if those numbers say the complexity pays off.
