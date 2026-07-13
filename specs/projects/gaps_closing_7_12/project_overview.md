---
status: complete
---

# gaps_closing_7_12 — v0.5 low-hanging-fruit gap batch

A gap-closing batch that closes a set of **v0.5-tier** gaps from
[`GAPS.md`](../../../GAPS.md) (§"v0.5 — table stakes for 'pretty good'"). Selection bar:
**best bang-for-buck, good user gain, low risk, ~1 phase each, no upfront spike or
hands-on decision.** In the spirit of `feature-gaps-7-11` — a batch of independent,
independently-shippable phases.

## Selection criteria applied

Kept only v0.5 gaps that are **engine-ready or FreeCell-only** (no multi-PR fork
campaign), fit **one coherent phase**, have **low blast radius**, and need **no upfront
spike / product fork**. Dropped anything that needs a capability spike, a fork
modelling/round-trip campaign, or "deserves its own focused project."

## Scope — 7 feature phases + 1 render-validation phase (approved 2026-07-13)

Ordered roughly by bang-for-buck. Each is intended to stand alone as one phase.

1. **Status bar with selection stats** — Sum · Avg · Count on the current selection,
   click-to-toggle Min/Max. Values already flow in the published viewport, so this is
   **render-side only** and lands as new chrome (not a pixel-baseline surface).
2. **Cell-area right-click context menu** — cut/copy/paste/clear, insert/delete
   rows/cols, format entry points. Header + chart context menus already exist as the
   pattern to copy; reuses existing clipboard/insert/delete commands. Today the cell
   body's `handle_right_mouse_down` just dismisses.
3. **Fill down / right (⌘D / ⌘R)** — keyboard fill of the selection from its top/left
   edge. Engine is ready and undoable (`UserModel::auto_fill_rows/auto_fill_columns`,
   incl. sequence detection). **Scope note:** ship the keyboard commands only; the
   **drag-fill handle** (the larger, input-heavy half) stays deferred.
4. **⌘+arrow → edge-of-data** — change `Motion::JumpEdge`/`ExtendEdge` from jumping to
   the sheet edge to the edge of the contiguous data block (Excel/Sheets behavior).
   Confined to selection logic plus a cheap occupied-extent query.
5. **Paste values (⌘⇧V)** — minimum paste-special: values-only (no formulas, no
   formatting). `Shift+V` is already reserved-but-unbound; clipboard infrastructure
   exists. (Full paste-special dialog stays v1.0.)
6. **Number-format preset breadth** — widen the number-format dropdown beyond today's 7
   presets (`DROPDOWN_FORMATS`): thousands-separator style, a currency-symbol choice,
   more date/time forms, scientific/fraction. **UI-only** — the engine already renders
   arbitrary format codes. (Custom format-code **editor** stays v1.0.)
7. **Autofit column width** — double-click the header divider to size a column to its
   content. Pairs with the shipped drag-resize; needs text measurement over the column's
   cells. (Wrap-driven row auto-grow already exists.)
8. **Render-fidelity polish pair** *(dedicated late render phase)* —
   (a) a fill covers the interior gridlines of its block (Excel "solid block" look);
   (b) a full-row selection darkens the row-number header (symmetric with the
   full-column path). Both cheap and instantly visible. **This is the only
   pixel-suite-in-scope change in the batch**, so per `CLAUDE.md` it runs as its own
   phase **after** all coding: full render suite (with watchdog), regenerate + eyeball
   baselines, commit, and dispatch/confirm the CI `render` gate.

## Cut from scope

- **Warn-before-strip on save** — **cut (owner, 2026-07-13).** The write-once `.back`
  backup is our accepted data-safety method (already shipped); a warn dialog is not
  wanted. Removed from GAPS.md's v0.5 table in the same change. Full pass-through
  preservation remains a separate v1.0 item.

## Held back (recorded, not in this batch)

- **CSV/TSV import + export** — carries a small product fork (open-as-untitled-workbook
  vs. true csv save-in-place). Not taken as a stretch add.
- **Function autocomplete / point-mode + formula-range highlighting** — real
  formula-UX projects; more design than low-hanging fruit.
- **Missing everyday scalar functions + TRIM bug** — **fork work**
  (one-fix-one-branch-one-PR ×~14); belongs to the ironcalc-upstreaming track, not a
  FreeCell phase.
- **Hide/unhide rows & cols** — needs fork modelling (`Col` has no `hidden` field) →
  not pure no-hands-on.
- **Freeze panes / merged cells (render+selection) / conditional formatting** — each
  "deserves its own focused project" per GAPS (split-viewport, delicate-input UX trap,
  and rule-editor+render+round-trip respectively).
- **macOS Finder open-file** — needs a gpui `on_open_urls` capability spike first →
  explicitly hands-on; excluded.
