# Project: `.xlsx` Preservation on Save (don't destroy what we don't model)

**Status:** **Pre-build decision.** Flagged by the 2026-07-02 pre-build spec review
(`specs/pre-build-spec-review-2026-07-02.md` §2, blind spot 2) as make-or-break for
trust: the policy call (and a cheap de-risk) should land before the build commits,
even if the implementation lands later.

**Relates to:** round-3 B needed-API audit (`experiments/round-3/B-api-audit/`,
comments dropped on export; merges/CF have no API), SP2 large-open
(`experiments/round-2/02-xlsx-open/` — all fixtures were IronCalc-authored), SP5
fidelity (`experiments/round-2/05-style-fidelity/`), and the long-standing
"owning `.xlsx` writing is ~10× scope" trap (round-2/round-3 SYNTHESIS).

## The problem

FreeCell saves through IronCalc's writer, which emits only what IronCalc models.
Everything else in an opened workbook is **silently dropped on save**:

- **Confirmed dropped / unmodeled:** comments (source-confirmed dropped on export),
  data validation, hyperlinks, merged cells, conditional formatting.
- **Never examined in three rounds:** charts, pivot tables, images/drawings,
  tables/autofilter, external links, VBA/macros.

So the most basic trust scenario — *open a colleague's workbook, fix one cell,
save* — destroys their charts, pivots, and comments without warning. The corpus
framed merges/CF as features FreeCell can't *offer*; the sharper framing is
**data loss on re-save of other people's files**. Also: no real Excel-authored
file was ever opened during de-risking (every fixture was synthetic or written by
IronCalc itself).

## Why this is (probably) not optional for v1

For an "Excel-compatible" app, destructive save is the #1 trust-killer — worse
than a missing feature, because the user finds out after the damage. Some
preservation strategy is arguably **mandatory for v1**, and the choice changes the
file-layer architecture, so it must be decided before the build, not during.

## Options (the decision)

1. **Warn-and-strip** — detect unmodeled OOXML parts on open; on save, warn
   ("saving will remove: 2 charts, 1 pivot…") and/or force save-as. Cheapest;
   honest; still lossy.
2. **Zip-level unknown-part pass-through** — carry the original package's
   untouched parts (charts, drawings, pivot caches, VBA…) through to the saved
   file, splicing in IronCalc-written parts for what FreeCell *does* model.
   Tractable without owning a full writer, but needs care with relationships
   (`[Content_Types].xml`, rels) and parts that reference edited data (a chart
   pointing at changed cells goes stale rather than lost).
3. **Own `.xlsx` writing** — full fidelity ceiling, ~10× scope (the known trap).
   Not a v1 move unless forced.

A likely v1 shape: **2 for inert parts + 1 as the fallback** where pass-through
can't be made safe.

## Cheap de-risk (days, before the build)

1. Assemble a corpus of **30–50 real files** (Excel-authored, plus
   LibreOffice/Sheets exports) with charts, pivots, comments, CF, validation,
   drawings, macros.
2. For each: open → save via IronCalc → **diff the OOXML part inventory**
   (what parts/rels vanished?). This also gives the first-ever robustness data on
   real Excel-authored input (SP2 only ever read IronCalc's own output).
3. Prototype zip-level pass-through on one chart-bearing file; confirm Excel
   re-opens the result cleanly.
4. Record the v1 policy decision (per part type: preserve / warn-strip / defer).
