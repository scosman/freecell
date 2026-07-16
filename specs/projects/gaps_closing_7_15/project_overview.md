---
status: draft
---

# gaps_closing_7_15

Third gap-closing round (after `mvp-gaps` and `gaps_closing_7_12`). Goal, per the owner:

> Read GAPS.md. A suggested set of gaps targeting **v0.5** that are lower-hanging fruit we
> could implement in **one phase each**. Best bang-for-buck: good user gain, low risk. As
> many as are reasonable to implement in a phase, and that don't need hands-on guidance.

## Proposed scope — one gap = one phase

Selected from the open rows of the GAPS.md v0.5 tier table (`gaps_closing_7_12` already
closed: status bar, ⌘D/⌘R fill, edge-of-data jumps, paste-values, cell context menu,
number-format presets, autofit column width, render polish pair).

1. **Function autocomplete + signature hints** — type `=SU` → SUM/SUMIF… list; on accept,
   show the arg template. FreeCell-side only (static function list; the engine's enum is
   private). The "don't have 80 functions memorized" bar — highest formula-UX gain of the
   open set, no engine risk.
2. **CSV import + export** — opening a downloaded `.csv` is a top-3 home task. Import
   reuses the TSV-paste parsing (open-as-untitled workbook — the simple recommended
   default); export walks the used range of the active sheet. No fork work.
3. **Drag fill handle + series autofill** — *the* signature spreadsheet affordance; its
   absence reads instantly as "not a real spreadsheet". Engine fully ready and undoable
   (`auto_fill_rows/columns` incl. 1,2,3… / Jan,Feb… sequence detection — ⌘D/⌘R already
   ride it). Work is the selection-corner handle + drag interaction in the grid
   (pixel-suite in-scope → render subset while iterating, full suite at the end).
4. **Hide / unhide rows & columns** — header context-menu entries. Fork half per policy
   (two clean branches: row-hidden `UserModel` setter; column-hidden modelling +
   round-trip); FreeCell half renders hidden as zero-size in the axis geometry.
5. **Everyday scalar functions batch + TRIM bug (fork)** — SUMPRODUCT, PROPER, REPLACE,
   CHAR, CODE, CLEAN, DOLLAR, ADDRESS, plus the TRIM internal-runs fix. Each is an
   independently-implementable engine function = one `fix/` branch = one upstream PR
   (per the fork policy); pure engine + tests, fully autonomous. SUMPRODUCT alone bites
   real home sheets. (TRANSPOSE/XMATCH/percentile-quartile stay out — array/semantics
   heavier.)
6. **Basic sort (A→Z / Z→A)** — selection/column sort, header-aware, via the context
   menu. **The borderline one:** needs a new fork range-sort op (values + styles move
   together, one undo step) — the largest single phase here; flagged so the owner can cut
   it if this round should stay strictly low-risk.
7. **Autofit row height** (double-click a row divider) — small; pairs with the shipped
   autofit column width and reuses the wrap-measurement machinery from row auto-grow.

Plus a final **render-validation phase** (required by convention: items 3, 4, and 7 touch
grid geometry/overlay pixels): full pixel suite + baseline eyeball + CI `render` gate,
once, after all coding phases.

## Explicitly excluded (and why)

- **Conditional formatting** — engine-side done in the fork, but rule-editor UI + grid
  render (data bars, color scales) + round-trip is a multi-phase project of its own.
- **Merged cells (render + selection)** — plan exists (`projects/merged-cells.md`) and
  says it deserves its own focused project; drags selection-fixpoint logic through
  delicate input code.
- **Freeze panes** — split-viewport rendering in the custom grid; real complexity.
- **Formula range highlighting + point-mode** — pairs with autocomplete but is the
  deeper, riskier half of formula-entry UX (editor + click routing); own round.
- **macOS Finder open-file** — needs a gpui capability spike and hands-on verification
  on real macOS; can't be validated autonomously in this container.

## Constraints

- Existing conventions apply: crate-scoped checks per phase, `cargo fmt --all --check`,
  render subsets while iterating, one late full render phase, commit + push regularly.
- Fork work follows the standing policy: one fix = one `fix/` branch = one clean upstream
  PR; FreeCell pins `freecell-fixes`.
