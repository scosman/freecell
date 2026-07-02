# FreeCell — Phase 2 (Round-2) Synthesis

> Stage-3 input. Consolidates the five Round-2 experiments (SP1–SP5) into a
> **build / adjust / pivot** recommendation for FreeCell. Evidence lives under
> `experiments/round-2/NN-*/findings.md`; the plan and criteria are in
> `specs/projects/freecell-phase-2/`. All numbers are a 4-core / ~15 GB Linux
> container **floor** (real hardware is faster); UI is unmeasured here (GPUI was
> validated in Phase 1 and deliberately out of Phase-2 scope).

## Verdict: **BUILD (proceed), with a concrete engineering agenda**

IronCalc cleared **every** Phase-2 bar. None of the three off-ramp conditions fired.
The engine is confirmed viable to build FreeCell on. What Phase 2 surfaced is not a
set of blockers but a **well-scoped list of things FreeCell must own or design around**
— because IronCalc is *correct and complete-enough, but single-threaded,
non-incremental, has no change-stream, and no dynamic arrays.* Several "big / live"
behaviors therefore cost seconds or need FreeCell-side work. All are tractable.

A well-evidenced "the conditions hold" is the successful outcome here — and they hold.

## What each experiment established

| # | Experiment | Result vs bar | The load-bearing lesson |
|---|---|---|---|
| **SP1** | Non-blocking recompute & interop seam | **PASS** (gate-proven with a negative control) | Worker owns the (`Send`) model; edits **coalesce** (30→1 eval); render tick stays <8.3ms during a multi-second eval. **But IronCalc exposes no evaluated-cell change stream** (diff-list = edit-sites only) → wait-then-repull → **staleness = one eval duration (~1.3s@1M, ~7s@10M).** App never freezes; it just isn't instantly "live" on huge edits. |
| **SP2** | Large styled `.xlsx` open | **PASS** (seconds, sane memory) | 105 MB / 12.7M-cell styled file opens in **~22s**, peak RSS **2.5 GB ≈ 5× uncompressed**. Cost is a **single-threaded ~18s parse** (opaque in the API); first-paint ≈ 18s (cached values, no recompute needed). |
| **SP3** | Function-parity audit | **PASS** (credible, not off-ramp) | **96.4%** golden-correctness (error semantics flawless), **81.5%** common-function coverage. The one structural hole: **dynamic arrays / spilling = 0/17** (no FILTER/SORT/UNIQUE) — plus a few missing scalars (SUMPRODUCT/TRANSPOSE) and a TRIM bug. |
| **SP4** | Styled viewport read + style-API coverage | **PARTIAL PASS** | A realistic ~1,800-cell viewport reads value+style in **<2ms**; but `get_style_for_cell` is **~10× a value read**, so a 10k-cell overscan **fails** the frame budget. Binding-layer constraint, not an engine issue. **Style API fully covers per-cell + row/col band + empty-cell → the engine-native formatting decision STANDS; no side-store forced.** |
| **SP5** | Long-tail style-roundtrip fidelity | **PASS** (common long tail faithful) | 50/59 attributes survive: exact `#RRGGBB` colors, 8/9 border styles, all number-format families, full alignment matrix, font long tail. Losses are minor/edge: dotted→thin border, theme/indexed color-*references* flatten to resolved RGB, diagonal-direction flags, a few exotic borders, indent, rich-text runs. |

## The cross-cutting picture
IronCalc's **correctness and file/format fidelity are strengths** (SP3 semantics, SP5
round-trip, SP4 style-API completeness). Its **architecture is the source of every
caveat**: no incremental recalc + no change stream (SP1), single-threaded parse (SP2),
no dynamic arrays (SP3), per-cell style resolution cost (SP4). These are consistent
and understood, and each has a FreeCell-side or upstream path.

## Adopted baseline decisions (plan of record)

Decided for the real build — **not** "maybe later" (distinct from the Round-3 agenda
below):

- **Formatting = IronCalc-native styles**, no FreeCell side-table (SP4 confirmed the
  public style API covers per-cell + row/column band + empty-cell resolution).
- **Recompute seam = worker-owned (`Send`) model + coalesced edits + publish-on-
  completion** (SP1); the render loop watches a **generation counter** for cheap,
  on-demand viewport re-pulls (not per-frame full reads).
- **~3× overscan published viewport** so scrolling stays live *during* a multi-second
  recompute — **free** (zero extra memory; it just widens the published window) and the
  SP1 scroll-during-eval probe's measured recommendation over a `to_bytes()` snapshot
  (whose ~3.2s build @1M *exceeds* the eval). Build a snapshot only **on-demand** if the
  user scrolls past the overscan margin mid-eval.
- **An always-resident full style + geometry cache** (all row/col sizes + fills/borders/
  fonts/number-format for the sheet, held in the frontend, invalidated only on style
  edits — never on recompute). **Near-MVP:** grid layout needs all row/col sizes; it
  takes the ~10× style read (SP4) off the scroll path; and because styles/sizes don't
  change during a recompute and the cache is frontend-resident, **the grid renders
  fully-styled during an eval — only cell values lag.** See `projects/style-cache.md`.

## Round-3 / real-build agenda (ranked — this is the real output)

1. **FreeCell-owned dirty-tracking + a viewport value cache.** Because IronCalc gives no
   downstream-dirty signal, build FreeCell's own dependency/dirty tracking to shrink
   SP1's post-eval repaint from "whole viewport" to "only actually-changed cells." Pair
   it with an **optional viewport value-delta cache** — on scroll, fetch only
   newly-exposed cells' *values* (invalidate on generation++). Values are now the *only*
   eval-dependent render input; **styles + geometry are handled by the always-resident
   style cache (adopted, above).** Optional because SP4 showed uncached value reads are
   ~1× and cheap — measure delta-vs-full first. See `projects/viewport-cache.md`.
2. **Dynamic arrays / spilling (0/17).** The biggest perceived-compat risk for
   modern-Excel users (FILTER/SORT/UNIQUE are everyday). **Explicit decision needed:**
   accept absence for v1, build spill support, or contribute upstream. Not a
   contributable scalar function — it's a structural capability.
3. **Large-file open latency.** ~18s single-threaded parse for 100 MB. Parallelize /
   stream / lazy-load, or accept seconds-scale opens with a first-paint-fast progress
   UX (cached values already paint before recompute).
4. **Styled-read binding layer.** Cap the synchronous styled window (~≤3k cells) and/or
   route the styled read through the SP1 worker (off the frame budget) + a style
   projection cache — the SP4 10k-overscan failure dissolves once the read isn't on the
   render thread.
5. **Minor fidelity losses (SP5).** dotted→thin border, theme/indexed color references,
   diagonal-direction flags, hair/dashed/dashDot borders, indent, rich text, double
   underline — each a small upstream fix or a FreeCell-side shim; prioritize by real
   use.
6. **Merges + conditional formatting — still OPEN.** No IronCalc public API; persisting
   either likely forces FreeCell to take over `.xlsx` writing (~10× scope). Major
   features that each need their own technical design; **not** designed in Phase 2.
7. **Pre-distribution / polish.** GPL #55470 fix before shipping a proprietary binary
   (tracked); SUMPRODUCT/TRANSPOSE/TRIM parity fixes; CSV/load-API ergonomics.

## Bottom line
**Proceed to build FreeCell on IronCalc + GPUI.** Everyday behavior is proven fast and
correct; file fidelity and formatting are strong; the extremes are non-blocking and
understood. The remaining work is an ordered engineering agenda — most of it centered
on **FreeCell owning a dirty-tracking + viewport-cache layer** to turn IronCalc's
"correct but seconds-scale / non-incremental" behavior into a live-feeling app. The one
item deserving an explicit product decision before or early in the build is **dynamic
arrays**.
