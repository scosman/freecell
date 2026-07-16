---
status: complete
---

# Implementation Plan: gaps_closing_7_15

Six phases. Build order front-loads the **self-contained, no-fork, no-pixel** features
(autocomplete, CSV), then the grid features that move pixels (autofit row, fill handle),
then the **fork-dependent** hide/unhide, and finally the single render-validation phase.
Each coding phase: build **crate-scoped**, `cargo fmt --all --check`, unit/gpui tests +
(grid phases) a render **subset**, commit + push. The **full** render suite + CI `render`
gate run **once**, in Phase 6.

Section numbers below reference `functional_spec.md` / `architecture.md` (§1–§5). Decisions
D1.1/D2.2/D4.1 are owner-resolved; all other Dx use the recommended defaults fixed in the
architecture.

## Phases

- [x] **Phase 1 — Function autocomplete + signature hints (§1).** New
      `freecell-core/functions.rs` (static 345-name catalog + `complete`/`signature` +
      `fn_edit_context`/`enclosing_fn_name` lexical heuristics); `ChromeView` autocomplete
      state + per-keystroke recompute + keyboard interception (data-row interceptor +
      in-cell capture via new `GridEvent`s) + accept (`insert NAME(` via
      set_value+set_cursor_position) + list/sig-hint popovers. No fork, no pixel suite
      (chrome). Unit + gpui view tests.
- [x] **Phase 2 — CSV import + export (§2).** `freecell-engine`: add `csv` dep,
      `DocumentSource::ImportCsv` + `WorkbookDocument::import_csv` (untitled, RFC-4180,
      overflow guard, `LoadError::BadCsv`), `Command::ExportCsv` + `export_csv` (used-range
      → raw `value_token` values, atomic write). `freecell-app`: widen argv/open branch on
      `.csv`, `ImportCsv`/`ExportCsv` actions + File-menu items. No fork, no pixel suite.
      Engine + shell tests.
- [x] **Phase 3 — Autofit row height (§5).** `grid/view.rs`: add the double-click branch to
      the row-resize hotspot; `autofit_row` + `autofit_height_for_row` (measure all populated
      cells, wrap-aware, clamp 24…240); reuse `SetRowHeights` (one undo/row, marks manual per
      D5.1). No fork. Unit + gpui tests + render **subset**.
- [ ] **Phase 4 — Drag fill handle + series autofill (§3).** `grid/view.rs`: fill-handle
      square in the selection overlay; `fill_drag` state machine (hit-test, dominant-axis
      preview, auto-scroll reuse, up/left support); `GridEvent::FillDrag` →
      `Command::FillDrag` → new `document.fill_drag` seeding `auto_fill_*` with the **full
      multi-cell seed** (series detection; 1-cell → copy). **Check out the fork** to bind the
      exact `auto_fill_rows/columns` arg shape + up/left behavior (no fork *change*). One undo
      step. Unit + gpui tests + render **subset**.
- [ ] **Phase 5 — Hide / unhide rows & columns (§4).** **Two fork branches** (D4.1):
      `fix/row-hidden-setter` (undoable `set_rows_hidden` + export) and `fix/column-hidden`
      (`Col.hidden` model + parse + export + undoable `set_columns_hidden`); integrate on
      `freecell-fixes`, re-pin FreeCell. `freecell-engine`: read hidden flags in
      `build_sheet_cache` (hidden set + preserved sizes), `SetRows/ColumnsHidden` commands.
      `grid/`: zero-size axis rendering for hidden tracks; Hide/Unhide header-menu items
      (disable-hide-all guard; unhide over spanning selection). Fork upstream-style tests +
      engine + gpui tests + render **subset**. **Heaviest phase.**
- [ ] **Phase 6 — Render validation (§6, dedicated late phase).** Regenerate + **eyeball**
      baselines for the fill handle + drag preview (P4), hidden-track zero-size (P5), and any
      row-height shifts (P3); add new cases; run the **full** pixel suite (timeout +
      ~10-min watchdog); commit refreshed baselines; dispatch the CI `render` gate to green.

## Notes for the build

- **Fork policy:** Phase 5's two capabilities are **separate** `fix/` branches = separate
  upstream PRs (never combined). Push the fork branches + integrate on `freecell-fixes` +
  re-pin FreeCell so it builds; **do not open the upstream ironcalc PRs automatically**
  (leave them ready to open) unless the owner asks — consistent with "no PRs unless
  explicitly requested." Same for any FreeCell PR.
- **Ephemeral container:** commit + push after **every** phase (and mid-phase for the big
  ones).
- **Efficiency:** crate-scoped checks per phase; reserve `--workspace` for the final
  pre-Phase-6 validation; run cargo from `app/`.
