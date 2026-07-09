---
status: draft
---

# Functional Spec: Charts (production)

Builds on the PoC (`../chart-proof-of-concept/`). Covers the *production* behaviors the PoC
skipped (`SYNTHESIS.md §8`): in-app rendering, live binding, performance, production
save/restore, OOXML fidelity — **plus** the authoring + editing extensions confirmed at the
overview review. Where the PoC settled a behavior, this references it rather than restating.

**Shape of the project:** a shippable **v1 core** (§1–§5: display + live + preserve, read-only
in-place, with compatibility warnings) followed by **end-phase extensions** (§6: authoring,
then chrome editing). v1 can ship before the extensions land.

---

## 1. Display charts from opened files (v1 core)

- On open, discover every embedded chart (worksheet → drawing → chart chain, implemented in
  the PoC) and render it **in the grid at its anchored position** (`xdr:twoCellAnchor`
  from/to cells → pixel rect), floating above cells, clipped to the grid viewport.
- A chart **scrolls and zooms with the sheet** (anchor is in sheet coordinates).
- Supported types (v1): **line, column, bar, area (incl. stacked/100%), pie, doughnut,
  scatter, bubble.** Type handling follows the **parse contract (§5)**: a type with no
  faithful representation renders a **graceful placeholder** (title + anchor rectangle +
  "unsupported chart type"); a **3D** type degrades to its 2D equivalent with a compatibility
  warning (§5). Never a crash, never a blank hole; the workbook still opens.

## 2. Live data binding (v1 core)

- A chart's series values come from the **current** worksheet cells its `c:f` ranges resolve
  to (against IronCalc), not the file's cached snapshot. Editing a cell in a chart's source
  range **re-renders** the chart.
- The file's `numCache`/`strCache` is the **first paint** (instant, no eval) and the
  **fallback** when a range can't be resolved; live values replace it once resolved.
- Re-render is **incremental + coalesced**: only charts whose source ranges intersect the
  edit recompute, batched per frame. Charts update **when the async recompute lands** — no
  explicit "stale/updating" badge (default; flag if you want one).

## 3. Save / restore (v1 core)

- **Round-trip fidelity:** open → edit → save → reopen preserves every chart, in **Excel and
  LibreOffice**, not just our loader. Extends the PoC single-sheet byte-preservation to
  **multi-sheet** (sheet→part map via `workbook.xml.rels`) + carries chart-aux parts
  (`styleN`/`colorsN`).
- **Edit-reflow:** for a chart whose source cells changed since load, its cached values are
  **refreshed** from IronCalc's current values on save (so Excel shows current data); charts
  on untouched sheets are **bit-stable**.
- The existing **`.back` backup** (mvp-gaps) before first save applies — a recovery path if a
  save ever loses fidelity.

## 4. Per-type fidelity — what "correct" means (v1 core)

Baseline: everything the PoC validated (`SYNTHESIS §2`) — correct geometry, multi-series,
grouping/stacking, synthesized palette, nice numeric axis, legend, title/axis-titles — now at
**production quality**. Plus the prioritized `ooxml-coverage-matrix.md` features the thin PoC
model dropped:
- **P1 (v1 must-have):** `c:dPt` per-point / per-slice colors (esp. pie), series colors incl.
  **theme colors** (`schemeClr`+tint), value-axis title **rotated** vertical, horizontal-bar
  category order matching Excel.
- **P2 (v1 target, deferrable per item):** data labels (`c:dLbls` val/percent/name), number
  formats (`c:numFmt`), axis scaling (min/max, reversed), gridline toggles, markers,
  `c:smooth`, gap/overlap, pie rotation/explosion. **A P2 feature we don't implement sets the
  compatibility flag (§5) rather than being silently dropped.**
- **Out (this project):** log/date axes, pattern fills, combo, and the §10 items.

**Interaction in the v1 core:** charts are **read-only, in-place** — not selectable, movable,
resizable, or deletable, and no hover/tooltips. Manipulation arrives with authoring (§6.A).

## 5. Compatibility warnings & the parse contract (v1 core)

To be **honest about fidelity** without failing, every chart-model property / OOXML feature
seen at parse time falls into exactly one of three buckets:

1. **Supported** — parses into our real model and renders faithfully.
2. **Degraded → sets the compatibility flag** — a *non-essential* feature we don't model
   (most P3, and any P2 we haven't implemented): we parse what we can, ignore the rest, and
   set a **per-chart `compatibility_warning` flag**. The chart still renders — just not
   exactly as authored.
3. **Essential-unsupported → error** — a feature essential enough that drawing the chart
   without it would be *misleading*: the chart does not render as itself and falls back to the
   **placeholder** (§1). "Error" is a per-chart render-fallback, **never an app crash**.

**UI signal:** when a chart's compatibility flag is set, render a small, unobtrusive
**"May not display as intended"** warning in a **corner** of the chart (exact affordance in
`ui_design.md`). Category-3 charts show the full placeholder instead (a stronger signal).

**3D types degrade to 2D + flag.** `bar3DChart`/`line3DChart`/`pie3DChart`/`area3DChart`
render as their **2D equivalent** (bar/line/pie/area) with the compatibility flag set — **not**
a placeholder. Types with no 2D equivalent in our set (surface, radar, ofPie, stock, `cx:`)
remain category-3 → placeholder.

**Where it lives:** the parser (engine) assigns each chart a `Supported | Degraded |
Unsupported` outcome + the flag; it rides on the chart's model/spec and is consumed by the
render/UI layer. The `ooxml-coverage-matrix.md` priorities map onto the buckets — **P1
unsupported tends to category 3**, **P2/P3 unsupported → category 2**.

## 6. Authoring & editing (end-phase extensions — after the v1 core ships)

Two **separate, sequenced** stages. These require a **write-from-model** path (synthesize
chart XML from `chart-model`); the writer is built in Stage A.

### 6.A — Minimal authoring (first extension stage)
- **Insert a chart:** select a data range → *Insert chart* → choose an in-scope type →
  FreeCell creates a chart with **sensible defaults** (series/categories inferred from the
  range shape, default palette, auto axes, legend when multi-series), anchored near the
  selection.
- **Manipulate:** **select / move / resize / delete** a chart object in the grid.
- **Change type** (among in-scope types) and **re-range** (re-pick the source range).
- **Authored-chart styling is FreeCell-native** (our palette/defaults) — a valid `.xlsx` that
  opens correctly in Excel/LibreOffice but does **not** pixel-match Excel's default theme.

### 6.B — Chrome editing (second extension stage)
- Edit **title** text; **legend** on/off + position; **axis titles**; **series colors**;
  **data-label** toggles (show value / percent / name).

### Edit contract (editing a chart that was *loaded from a file*)
- Editing a loaded chart switches it from byte-preservation to **written-from-our-model** on
  save. Styling our model does not capture (gradients, theme effects, unmodeled fields) **may
  be lost** on that first edit — accepted, documented behavior (the `.back` backup gives a
  recovery path). Untouched loaded charts remain byte-preserved.

## 7. Edge cases & error handling

- Unsupported / parse-failed chart → placeholder (§1/§5); workbook still opens.
- `c:f` range unresolvable (deleted sheet / bad ref) → fall back to cached values; if none →
  placeholder.
- Source range edited to empty / non-numeric → render what's valid, blank the rest, no crash.
- Row/col insert-delete shifting a chart's ranges → live binding resolves against current cell
  addresses (range adjustment is IronCalc's job; we read current values).
- Very large series (10k+ points) → down-sample / cap for **paint** (perf); full data retained
  for **save**.
- Multi-sheet save where IronCalc's output part order differs from input → explicit sheet→part
  remap; **fail loudly** rather than silently drop a chart (PoC risk #8).
- (Authoring) insert with no/invalid selection → prompt / no-op, never a broken chart.
- (Editing) editing a loaded chart → per the §6 edit contract; `.back` backup on first save.

## 8. Constraints

- **Performance (north star):** lazy parse off the open critical path; off-screen charts cost
  ~nothing; an edit recomputes only the intersecting charts, coalesced per frame; scroll stays
  at frame budget with charts present. Explicit **p50/p99** targets (first-paint, edit
  re-render, scroll frame time with K charts) set + measured **at the line-chart checkpoint**,
  per the repo bench convention (foreground `timeout`, forced + asserted, env-stamped).
- **Compatibility:** classic `c:` charts; pinned gpui/gpui-component/IronCalc (mirror
  `app/Cargo.toml`); saved files valid in Excel + LibreOffice.
- **Robustness:** a malformed/unexpected chart part never breaks workbook open or the grid —
  degrade to placeholder + log. Tolerate real-world XML variety (odd namespaces, richer
  styling) — a real-file corpus is required (PoC risks #10/#11).

## 9. Success criteria

- **Line-chart checkpoint** (implementation_plan): a real `.xlsx` line chart opens, renders
  in-grid at its anchor, updates live on edit, survives save/reopen in Excel + LibreOffice,
  and meets the agreed perf targets — validated by a human review/tuning pass.
- **v1 core done:** all in-scope types at that bar; the P1 fidelity set honored; the parse
  contract + compatibility warning working (incl. 3D→2D degrade); a real-file corpus loads
  without breakage; a perceptual-diff regression suite guards rendering.
- **Extensions done (each stage):** 6.A — insert/move/resize/delete/retype/re-range produce
  valid charts that round-trip; 6.B — chrome edits apply and round-trip.

## 10. Out of scope (this project)

Log & date axes; pattern fills; **combo / stock / radar / surface / ofPie / multi-ring
doughnut / `cx:` extended** chart types (placeholder via §5); **true 3D** rendering (3D types
degrade to 2D + warning, §5); hover/tooltips; rich Excel-parity chart editor (beyond 6.A+6.B);
live collaboration; chart printing/export beyond `.xlsx` round-trip.
