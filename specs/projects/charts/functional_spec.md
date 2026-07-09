---
status: draft
---

# Functional Spec: Charts (production)

> ⚠️ **STRAWMAN — not yet reviewed.** Written ahead of process during a batch drift; parked
> as raw input pending the step-gated `/spec new_project` review. Only `project_overview.md`
> is under review right now — do **not** treat anything here as vetted or settled.

Builds on the PoC (`../chart-proof-of-concept/`). This spec covers the *production*
behaviors the PoC deliberately skipped (`SYNTHESIS.md §8`): in-app rendering, live binding,
performance, production save/restore, and OOXML fidelity. Where the PoC already settled a
behavior, this doc references it rather than restating it.

## 1. Core behaviors

### 1.1 Display charts from opened files
- On opening an `.xlsx`, FreeCell discovers every embedded chart (worksheet → drawing →
  chart chain, already implemented) and renders it **in the grid at its anchored position**
  (`xdr:twoCellAnchor` from/to cells → pixel rect), floating above the cells, clipped to the
  grid viewport.
- A chart **scrolls and zooms with the sheet** (its anchor rect is in sheet coordinates).
- Supported types (v1): **line, column, bar, area (incl. stacked/100%), pie, doughnut,
  scatter, bubble.** An out-of-scope or unparseable chart renders a **graceful placeholder**
  (title + anchor rectangle + "unsupported chart type" — never a crash, never a blank hole).

### 1.2 Live data binding
- A chart's series values come from the **current** worksheet cells its `c:f` ranges point
  at (resolved against IronCalc), not the file's cached snapshot. Editing a cell in a
  chart's source range **re-renders** the chart.
- The file's `numCache`/`strCache` is used for the **first paint** (instant, no eval) and as
  a **fallback** when a range can't be resolved; live values replace it once resolved.
- Re-render is **incremental and debounced** — a keystroke storm coalesces; only charts whose
  source ranges intersect the edit recompute. (Perf contract in §3.)

### 1.3 Save / restore
- **Round-trip fidelity:** open → edit → save → reopen preserves every chart, in Excel and
  LibreOffice, not just our own loader. Extends the PoC's single-sheet byte-preservation to
  **multi-sheet** workbooks (sheet→part mapping via `workbook.xml.rels`) and carries all
  chart-aux parts (`styleN`/`colorsN`).
- **Edit-reflow:** when the user has edited a chart's source cells, the saved chart's cached
  values are **refreshed** from IronCalc's current values (so a colleague opening the file in
  Excel sees current data), while unedited charts are byte-preserved as-is.
- Charts on sheets the user never touched must be **bit-stable** (no spurious diffs).

### 1.4 Per-type fidelity (what "correct" means)
Baseline: everything the PoC validated (`SYNTHESIS §2`) — correct geometry, multi-series,
grouping/stacking, synthesized palette, nice numeric axis, legend, title/axis-titles — now
at **production quality** (not relaxed-rigor). Plus the **P1/P2** features from
`ooxml-coverage-matrix.md` that the thin PoC model dropped, prioritized:
- **P1:** `c:dPt` per-point / per-slice colors (esp. pie), series colors incl. **theme
  colors** (`schemeClr`+tint), value-axis title **rotated** vertical, horizontal-bar
  category order matching Excel.
- **P2:** data labels (`c:dLbls` show val/percent/name), number formats (`c:numFmt`), axis
  scaling (min/max, reversed), gridline toggles, markers, `c:smooth`, gap/overlap, pie
  rotation/explosion, live `c:f` (see §1.2).
- **Out (this project):** log/date axes (HEAVY), pattern fills, combo, and all §8 items.

## 2. User-facing surface & interaction

- **v1 = read-only, in-place.** Charts render and update; they are **not** selectable,
  movable, resizable, or deletable in v1 (fast-follow phase, `implementation_plan.md`).
- **No authoring** (create/insert/retype/re-range a chart) in this project.
- **Hover/tooltips:** out for v1 (the PoC noted candlestick/pie lack them; not a v1 need).
  Value read-off comes from axis + optional data labels.

## 3. Constraints

- **Performance (FreeCell's north star — huge sheets):**
  - Chart discovery/parse is **lazy / off the open path's critical section** — opening a
    workbook with charts must not regress open time meaningfully; parse on first paint of the
    owning sheet region.
  - A chart **off-screen** costs ~nothing (no paint, no recompute).
  - An edit that touches N charts' ranges recomputes **only those N**, coalesced per frame;
    the scroll path must stay at the grid's frame budget with charts present.
  - Establish explicit p50/p99 targets (per repo bench convention) at the checkpoint:
    first-paint latency, re-render latency on edit, and scroll frame time with K charts
    visible.
- **Compatibility:** classic `c:` charts; pinned gpui/gpui-component/IronCalc versions
  (mirror `app/Cargo.toml`). Saved files valid in Excel + LibreOffice.
- **Robustness:** a malformed / unexpected chart part **never** breaks workbook open or the
  grid — it degrades to the placeholder and logs. Real-world chart XML variety (odd
  namespaces, richer styling) must be tolerated (PoC risk #11 — real-file corpus needed).

## 4. Edge cases & error handling

- Unsupported/parse-failed chart → placeholder (§1.1), workbook still opens.
- Chart `c:f` range can't be resolved (deleted sheet, bad ref) → fall back to cached values;
  if none, placeholder.
- Chart source range edited to empty / non-numeric → render what's valid, blank the rest, no
  crash.
- Merged/inserted/deleted rows or columns shifting a chart's ranges → live binding resolves
  against current cell addresses (range adjustment is IronCalc's job; we read current values).
- Very large series (10k+ points) → down-sample or cap for paint (perf), full data preserved
  for save.
- Multi-sheet save where IronCalc's output part order differs from input → explicit
  sheet→part remap; **fail loudly** rather than silently drop a chart (PoC risk #8).

## 5. Success criteria

- The **line-chart checkpoint** (implementation_plan): a real `.xlsx` line chart opens,
  renders in-grid at its anchor, updates live on edit, survives save/reopen in Excel +
  LibreOffice, and meets the agreed perf targets — validated by a human review/tuning pass.
- **v1 done:** all in-scope types reach the same bar; the fidelity P1 set is honored; a
  real-file corpus loads without breakage; a perceptual-diff regression suite guards
  rendering.

## 6. Out of scope (this project)

Chart authoring/editing UI; selection/move/resize (deferred to a named fast-follow phase,
not v1); hover/tooltips; log & date axes; pattern fills; combo/stock/radar/surface/3D/ofPie/
multi-ring-doughnut/`cx:` types; live collaboration; chart printing/export beyond `.xlsx`
round-trip.
