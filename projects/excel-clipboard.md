# Project: Excel Clipboard Interop (rich copy/paste)

**Status:** **Future (post-MVP).** Product call (2026-07-02): rich Excel clipboard
interop is **not MVP-level**. Tracked so the decision and scope are recorded.
Origin: 2026-07-02 pre-build spec review
(`specs/pre-build-spec-review-2026-07-02.md` §2, blind spot 4).

## What

Copy/paste of **ranges** between FreeCell and Excel (and Sheets/LibreOffice) via
the system clipboard's rich flavors:

- **Paste from Excel:** read TSV (values) and HTML / XML-Spreadsheet flavors
  (values + formatting) into a FreeCell range.
- **Copy to Excel:** write the same flavors so a FreeCell range pastes into Excel
  with values (and eventually formatting) intact.

## Why it's tracked

- It's the most-used bridge for anyone trialing a new spreadsheet — a likely
  first-five-minutes action — so its absence shapes first impressions even if it
  isn't MVP.
- It's **all FreeCell-side work of unknown size**: round-3 A found IronCalc's
  internal clipboard isn't externally chainable, and GPUI's clipboard API richness
  (multiple flavors? HTML?) has zero corpus coverage.

MVP posture: plain-text TSV paste/copy (values only) is cheap and may ride along
with the editor build; this project is the *rich* (formatted, multi-flavor,
Excel-verified) story.

## Cheap probe (when picked up)

1–2 days: prototype a two-way TSV + HTML clipboard bridge; verify against real
Excel both directions (paste a formatted Excel range in; copy a FreeCell range
out and paste into Excel); record which flavors GPUI can read/write at the pinned
rev and what needs a platform-specific fallback.
