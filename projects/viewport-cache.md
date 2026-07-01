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
  worker needed); genuinely-new cells show a placeholder (or a stale-snapshot value)
  until the next publish. This keeps scrolling live while the worker is busy evaluating.
- **Pruning:** bound the cache with an LRU / distance-from-viewport threshold so it
  doesn't grow unbounded as the user roams a huge sheet.

The exact mechanism for reading genuinely-new cells *during* an eval (a `to_bytes()`
snapshot the frontend can read, vs. simply a large ~3× overscan so small scrolls stay
within already-published data) is the subject of the SP1 scroll-during-eval probe;
fold its result in here before implementing.

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
