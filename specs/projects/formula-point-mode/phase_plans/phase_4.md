---
status: complete
---

# Phase 4: Consolidation cleanup + v1.0 GAP entries

## Overview

The final **coding** phase (render validation is Phase 5). Phases 1–3 already landed the
consolidation — the token→color map + point-mode state live on the shared `EditController`, and both
host adapters (grid overlay; chrome data-row bar) consume only pushed primitives. This phase closes
out that consolidation: it **verifies** the two adapters carry zero formula logic, removes the one
genuinely-dead method left over from the Phase 2 CR (`EditController::set_sig_hint`), and records the
two v1.0 GAPS.md entries the v0.5 deferrals (`functional_spec.md §6`) point at.

No behaviour change, no new state, no render-relevant pixel change — so the pixel suite is **not**
run here (deferred to Phase 5). Confirmation of the consolidation state was done by source audit
before touching anything (findings below).

## Consolidation audit (findings — no change needed)

- **Grid host adapter = primitives + paint (zero formula logic).** `grid/` contains **no** call to
  `lex_formula_refs` / `assign_ref_colors` / `is_reference_ready` / `fn_edit_context` /
  `enclosing_fn_name`. It only *stores* the three pushed primitives (`reference_ready`,
  `pending_ref`, `ref_highlights` — `grid/view.rs:372,376,380`), paints the highlight overlay + the
  point-drag preview, resolves merge geometry against the grid's own `cache.merges()`, and emits
  `GridEvent::InsertReference`. Merge geometry is grid-native (the same `merges()` the
  selection/render path uses), not formula logic.
- **Data-row bar host adapter = render only.** `ChromeView` no longer holds `autocomplete` /
  `sig_hint` fields (relocated onto `EditController` in Phase 2); its formula methods are thin
  delegators that read the driving `InputState`, call `EditController`, and write results back. The
  bar renders `content_input` + the autocomplete/sig-hint popovers off `edit`-owned state.
- **No now-dead scattered fields** on `ChromeView` or `EditController` other than the one method
  below. Every formula accessor on `EditController` (`ref_tokens`, `ref_colors`, `ref_highlights`,
  `autocomplete`, `autocomplete_mut`, `take_autocomplete`, `sig_hint`, `reference_ready`,
  `clear_formula_state`, `recompute_formula`, `pending_ref`, `set_pending_ref`) has external callers.

## Steps

1. **`chrome/edit.rs` — remove the dead `set_sig_hint`.** Delete `EditController::set_sig_hint`
   (`edit.rs:215-218`) and its doc comment. It is genuinely dead: the Phase 2 CR reimplemented
   `accept_autocomplete` to re-derive the signature hint via `recompute_formula_edit_state`
   (`chrome/view.rs:1565`), so nothing calls it — verified across the whole repo (only match is its
   own definition; no test caller). **Keep `set_pending_ref`** (`edit.rs:167-170`): it is live
   forward-plumbing consumed by Phase 3's `insert_reference` (`chrome/view.rs:1626`).

2. **`GAPS.md` — confirm (a), add (b).**
   - **(a) Styled text-input control — already logged, no change.** The v1.0 tier row (GAPS.md
     `### v1.0 …` table, "In-editor rich text formatting … via a FreeCell-owned styled text-input
     control", split from `formula-point-mode` 2026-07-18) already states it owns in-editor
     ref-token coloring + rich formatting and consumes this project's token→color map, and points at
     [`projects/styled-text-input-control.md`](../../../projects/styled-text-input-control.md).
     Confirmed accurate; left as-is.
   - **(b) Cross-sheet point-mode insertion — MISSING, add it.** No "cross-sheet" entry exists in
     GAPS.md. Append a **v1.0 tier-table row** (matching the sibling styled-control row's format: a
     single tier row, `NEW` tag, pointer to its home), placed immediately after the styled-control
     row so the two `formula-point-mode` deferrals sit together. It records: click another sheet's
     tab mid-formula → insert `Sheet2!A1`; and the **asymmetry** — the color map still colors
     already-typed cross-sheet refs for the future in-editor control, but v0.5 draws **no**
     cross-sheet grid highlight and inserts **no** cross-sheet reference (only same-sheet insertion +
     same-sheet highlights ship). Home: `functional_spec.md §6`.

## Tests

No new tests (dead-code removal + a docs-only GAPS entry; behaviour unchanged). The migration-step-1
regression gate is re-asserted by running the existing suites:

- **Regression gate:** the full crate-scoped `freecell-app` lib test run stays green — all shipped
  autocomplete/sig-hint view tests + the `data_row` unit tests + the Phase 1–3 formula tests. This is
  the migration-step-1 "keep green" gate from `components/formula_editor.md §1.8`.

## Checks (run cargo from `app/`)

- `cargo build -p freecell-app`
- `cargo test -p freecell-app --lib` (full crate-scoped run — the touched crate; engine untouched, so
  no `-p freecell-engine`)
- `cargo fmt --all --check`
