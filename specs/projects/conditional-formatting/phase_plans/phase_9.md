---
status: complete
---

# Phase 9: GAPS log for deferred CF behaviors

## Overview

Docs-only phase (no code). Record every conditional-formatting option the first pass skips in the
durable `GAPS.md` log so nothing is lost, and tie them into the release-target tier tables.

## What was done

- Added a **"Conditional Formatting — first pass shipped; deferred behaviors (2026-07-17)"** section
  to `GAPS.md` with a table CF1–CF9:
  - CF1–CF3: data bars / icon sets / ratings (deferred visual families → project phases P11–P13; each
    needs a new in-cell grid draw primitive).
  - CF4: `TimePeriod` Between/NotBetween (explicit date-range operands) not authorable.
  - CF5: color-scale `Cfvo::Formula` thresholds not authorable.
  - CF6: theme-colored color scales treated as non-editable (fidelity preservation).
  - CF7: format-editor limited to fill/text-color/bold/italic; plus the P3 load asymmetry (loaded
    `Dxf` underline/strike/alignment applied, num_fmt/border dropped).
  - CF8: whole-sheet CF cache rebuild on value change (perf follow-up; non-CF sheets unaffected).
  - CF9: fork CF-undo-of-sheet-delete fix blocked on push (patch preserved under `fork-fixes/`).
- Updated the existing **v0.5** "Conditional formatting" tier row to "first pass shipped" with a link
  to the new section, and added a **v1.0** "Conditional formatting — completion" tier row for the
  deferred families + residuals.

## Verification

Docs-only — no build/test. Self-reviewed for table formatting + the GitHub anchor link.
