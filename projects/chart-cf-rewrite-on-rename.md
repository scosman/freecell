# Rewrite chart `c:f` on sheet rename

**Status: Future (deferred from charts P10, 2026-07-10).**

## Goal

Keep an embedded chart's **live data link** correct after an in-session worksheet rename, so a
saved chart both shows the right cached data *and* refreshes correctly in Excel/LibreOffice.

## Current behavior (the limitation)

Chart save is **source-first** (charts/architecture §5, implemented in `freecell-engine::chart::save`):

- An **unedited** chart is byte-preserved (its `xl/charts/chartN.xml` carried verbatim).
- An **edited-loaded** chart is targeted-patched (`patch_chart_source`) — it reflows only the
  `numCache`/`strCache` cached values, **keeping every `c:f` data reference byte-for-byte**.

Charts are **invisible to IronCalc** (it has no chart model), so IronCalc's `rename_sheet` — which
rewrites all *formula* references — does **not** touch a chart's `c:f`. The result: after renaming
`Data`→`Data2`, a chart whose series reads `Data!$B$2:$B$5` keeps that literal `c:f`:

- **Cached values are correct.** The save patches `numCache` from the chart's current live values
  (P10 live binding resolved them while the sheet was still `Data`, or before the rename), so on
  reopen the chart draws the right numbers.
- **The live link dangles.** In Excel/LibreOffice the `c:f` now points at a nonexistent sheet
  `Data`, so a data refresh can't resolve it.

P10 places such a chart on the **correct renamed worksheet** (the `<drawing>` re-injection is
`SheetId`-anchored, rename-safe) and preserves its cached data — only the internal `c:f` prefix is
stale. The narrow edge of an *unparsed* (unsupported) chart on a renamed host sheet is a documented
best-effort drop.

## Sketch

- On a `RenameSheet` (or at save time), reflow every chart's `c:f` sheet-name **prefix** from the
  old name to the new — including the quoting rules (`'Old Name'!` ↔ `'New Name'!`) and unqualified
  refs (which are already anchor-relative and need no change).
- Do it as a **targeted `c:f` text patch** in the retained source (same second-pass style as
  `patch_chart_source`), so all unmodeled styling stays byte-preserved. A parsed→rebuilt round-trip
  is explicitly NOT wanted (it would lose fidelity).
- Track the old→new name mapping the worker already has (stable `SheetId` → name), and apply it to
  each chart bound to (or referencing) the renamed sheet.

## Why deferred

Out of P10's scope (save/restore correctness + no-silent-drop). The dangling link is a fidelity
nicety, not data loss — the chart is present, on the right sheet, with the right values. Sequence it
with the chart authoring/editing phases, where `c:f` rewriting (re-range) is built anyway.
