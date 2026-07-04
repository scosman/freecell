---
status: complete
---

# MVP Gaps — Core Spreadsheet Feel

The MVP is complete. This project closes core functional gaps to make FreeCell feel
like a real spreadsheet app. The feature set is flexible: good bang for our buck in
closing the gap with a full spreadsheet app, without high technical risk or tons of
work. No one feature is required — the goal is a good set of high-value features with
balanced risk. If any item proves much larger than designed, dropping it back to
GAPS.md is an acceptable outcome.

Scope was decided at kickoff (2026-07-04) from the owner's initial list plus a gap
survey (`functional_spec.md §8` of the MVP, `GAPS.md`), then **scoped back** after two
source-level audits of the pinned dependencies (IronCalc 0.7.1 UserModel APIs; gpui
pinned rev capabilities). Audit findings are baked into `architecture.md`. Everything
cut is recorded in `GAPS.md` — nothing silently dropped.

## In scope

**Editing feel**
- Type-to-replace: typing with a cell selected focuses the edit bar and replaces content.
- Live cell mirror: the active cell shows the raw text as you type (no live evaluation
  before commit — a whole-workbook recompute per keystroke is untenable on huge sheets).
- In-cell editing: double-click (or F2) opens an editor overlay on the cell, sharing
  the data-row commit path and caps. IME stays out (`projects/ime-text-input.md`).
- Range clipboard: Cut/Copy/Paste of ranges — engine-native internal (values +
  formulas with reference adjustment + styles, undoable, verified present in 0.7.1),
  plus TSV text to/from other apps. Rich Excel interop stays a separate project.

**Formatting**
- Font family + size in the action bar (installed-font list + fixed size list); grid
  renders per-cell family/size; row height auto-grows for larger fonts (never shrinks).
- Borders: render cell borders loaded from files + a fixed-preset borders menu
  (all/inner/outer/top/bottom/left/right/none — thin black). No style/color picker.
- Text color button + horizontal alignment controls (left/center/right).
- Number-format dropdown (General, Number, Currency, Percent, Date + more-decimals /
  fewer-decimals).
- Type-aware default alignment (numbers/dates right, booleans/errors center) + `[Red]`
  number-format text color (GAPS #1/#2).

**Structure & navigation**
- Row/col resize by dragging header dividers, correct resize cursors on hover;
  resizing a header inside a multi-header selection resizes all selected rows/cols.
- Row/col header selection (click, drag, Shift+extend) + select-all corner; whole
  row/col formatting uses the engine's band styles (no per-cell explosion).
- Insert/delete rows & columns via a header right-click menu — **guarded**: blocked
  with a clear message when the operation would displace merged cells present in the
  file (IronCalc doesn't adjust merge refs; merges round-trip through save today even
  though we don't render them).

**Chrome & data safety**
- Uniform titlebar (macOS): client-drawn titlebar row in the action-bar grey
  (`CHROME_BG 0xF3F3F3`) with repositioned traffic lights — capability confirmed at
  the pinned gpui rev. Linux keeps server decorations (unchanged).
- Cap-error message popover (GAPS #3).
- `.back` backup before first save of a disk-opened file (GAPS data-safety item).

## Cut during scope-back (recorded in GAPS.md)

- **Zoom** — the most cross-cutting item: a scale factor through perf-gated geometry,
  hit-testing, text sizing, and pixel baselines. Punt pre-authorized by owner.
- **Merged cells (all tiers)** — render-only is a UX trap without selection snapping;
  with snapping it drags fixpoint logic through the most delicate input code and
  couples to insert/delete. Investigation (2026-07-04) showed tiers a+b need zero
  engine changes — parked ready-to-build in `projects/merged-cells.md`. Only the
  insert/delete **guard** ships now.
- Earlier cuts: grid cell context menu, fill down/right + fill handle (engine
  `auto_fill_*` exists — cheap later), find/replace, autofit, recent files, freeze
  panes (engine API exists), sort/filter, overflow/wrap, Cmd+arrow edge-of-data.
