---
status: complete
---

# Phase 2: Shared color map + grid highlights

## Overview

Ships the first user-visible half of the feature: while a **formula** edit is open, every distinct
same-sheet cell reference already typed is drawn on the grid as a rich colored **fill + border**
highlight, updated live per keystroke and per caret move. No point-mode routing yet (Phase 3), no
in-editor token coloring (v1.0), no gpui-component / vendored-widget change.

This phase promotes the formula-feature state onto the shared edit layer ([`EditController`]) — the
Q3 consolidation — so all of it (autocomplete, sig-hints, the token→color map, the pending-ref
span) has **one** owner, driven off a single per-transition recompute. It then plumbs the derived
state to the grid through `ChromeGridRequest::EditState` / `GridView::set_edit_state` and paints the
same-sheet highlights in the grid overlay pass. `reference_ready` / `pending_ref` are pushed now but
only **consumed** by the grid in Phase 3 — plumbed here so the payload lands once
(`architecture.md §3.1`).

Builds on Phase 1's pure foundation (`freecell_core::{RefToken, assign_ref_colors,
is_reference_ready, palette::{REF_HIGHLIGHT_PALETTE, ref_color}}` + `freecell_engine::lex_formula_refs`).

## Steps

### A. `EditController` (chrome/edit.rs) — promote the formula-feature state (§2.4, §6, component §1.2–1.3)

1. **Relocate `Autocomplete`** struct from `chrome/view.rs` into `chrome/edit.rs` (pub within the
   crate, pub fields `matches`/`highlight`/`token_start`), re-exported from `chrome/mod.rs` so the
   `view.rs` autocomplete methods + tests can still name it.
2. **New state on `EditController`:** `ref_tokens: Vec<RefToken>`, `ref_colors: Vec<u8>`,
   `pending_ref: Option<Range<usize>>`, `autocomplete: Option<Autocomplete>`,
   `sig_hint: Option<&'static str>`, `reference_ready: bool` (cached). Init all empty/None/false in
   `EditController::new`.
3. **New methods:**
   - `pending_ref(&self) -> Option<Range<usize>>` / `set_pending_ref(&mut self, ...)` (used by
     Phase 3's `insert_reference`; added now so the state has one owner).
   - `ref_tokens(&self)` / `ref_colors(&self)` accessors.
   - `ref_highlights(&self) -> Vec<(CellRange, u8)>` — the `same_sheet` subset of tokens mapped to
     `(target, slot)` (cross-sheet excluded; §3.1/§4.1).
   - `autocomplete(&self)` / `autocomplete_mut(&mut self)` / `take_autocomplete(&mut self)` /
     `sig_hint(&self)` / `set_sig_hint(&mut self, ...)` — accessors so the `view.rs` autocomplete
     nav/accept/display methods keep working against the relocated state.
   - `recompute_formula(&mut self, text: &str, caret: usize, active_sheet_name: &str, keep_pending:
     bool) -> bool` — the consolidation seam: lex refs (`lex_formula_refs`) → colors
     (`assign_ref_colors`) → reference-ready (`is_reference_ready`) → autocomplete + sig-hint
     (relocated `fn_edit_context`/`complete`/`enclosing_fn_name`/`signature` logic, preserving the
     same-token highlight-carry). Clears `pending_ref` unless `keep_pending`. Returns + caches
     `reference_ready`.
   - `reference_ready(&self) -> bool` (cached last result).
   - `clear_formula_state(&mut self)` — zero tokens/colors/reference_ready/autocomplete/sig_hint/
     pending_ref (commit/cancel/cap-error).

### B. `ChromeView` (chrome/view.rs) — delegate + generalize (component §1.4)

4. Remove the `autocomplete`/`sig_hint` fields + their `new()` init; route every read/write through
   `self.edit` accessors.
5. `active_sheet_name(&self) -> String` — resolve `self.active_sheet` against `self.sheets`.
6. Rename `recompute_autocomplete` → `recompute_formula_edit_state(cx)`: on cap-error →
   `edit.clear_formula_state()`; else read driving `text`/`caret` + `active_sheet_name`, call
   `edit.recompute_formula(text, caret, &sheet, false)`. Update every caller (both Change handlers,
   the caret-move recompute, `begin_typed`, `begin_in_cell`). `escape_edit`/`commit_and_move` clear
   via `edit.clear_formula_state()`.
7. `autocomplete_display`/`autocomplete_nav`/`autocomplete_dismiss`/`accept_autocomplete`/
   `autocomplete_accept_at` read/mutate via the `edit` accessors. `accept_autocomplete` ends by
   recomputing formula edit state (so the token map + sig-hint stay in lockstep after the splice).
8. `refresh_edit_grid_state`: also compute `reference_ready = editing && edit.reference_ready()`,
   `pending_ref = editing && edit.pending_ref().is_some()`, `ref_highlights = editing.then(||
   edit.ref_highlights()).unwrap_or_default()`; add them to the `EditState` emit.

### C. `ChromeGridRequest::EditState` (chrome/mod.rs) + window plumbing

9. Add `reference_ready: bool`, `pending_ref: bool`, `ref_highlights: Vec<(CellRange, u8)>` to the
   `EditState` variant (import `CellRange`). Update the `shell/window.rs` destructure + the
   `set_edit_state` forward.

### D. `GridView` (grid/view.rs) — store + paint (§4.1)

10. Three new fields: `ref_highlights: Vec<(CellRange, u8)>`, `reference_ready: bool`,
    `pending_ref: bool`. Init in `new`; clear in `set_active_sheet`.
11. `set_edit_state`: append `reference_ready`, `pending_ref`, `ref_highlights` params; store them.
    Update every call site (the window forward + the grid test call sites → defaults).
12. Overlay pass: after the selection overlay (active-cell border) and before the fill handle /
    in-cell overlay, iterate `self.ref_highlights`; clip each `target` to the visible frame exactly
    like the selection overlay; push one `rect_div` per range with a translucent `.bg(...)` fill +
    `.border_2().border_color(...)` in the slot color. Slot→color via a new
    `ref_slot_border(slot, is_dark) -> u32` helper over `freecell_core::palette::ref_color`; the
    grid renders on the white `CELL_BG` always, so `is_dark = false` (helper stays parametrized for
    the future theme-aware / in-editor control).

## Tests

**gpui view tests (chrome/view.rs):**
- `formula_edit_pushes_same_sheet_ref_highlights`: `=A1+B2` on Sheet1 → last `EditState` push has
  2 `ref_highlights` with distinct slots (`(A1,0)`, `(B2,1)`).
- `repeated_ref_shares_highlight_color`: `=A1+A1` → 2 highlights, both slot 0.
- `cross_sheet_ref_absent_from_highlights_but_in_color_map`: `=Sheet2!A1` on Sheet1 → `ref_highlights`
  empty; `edit.ref_tokens()` still has 1 (colored) token.
- `commit_clears_ref_highlights`: after `=A1` then Enter/commit → last `EditState` push has empty
  `ref_highlights`.
- `escape_clears_ref_highlights`: after `=A1` then Escape → empty `ref_highlights`.
- `reference_ready_pushed_for_open_formula`: `=A1+` → `reference_ready` true in the push; `=A1` →
  false (plumbed-now / consumed Phase 3).
- Relocation regression: all shipped autocomplete/sig-hint view tests pass unchanged against the
  `edit`-owned state (retargeted to the accessors).

**grid view tests (grid/view.rs):** existing `set_edit_state` call sites updated to the new arity;
a highlight-store smoke assertion (`set_edit_state` with a `ref_highlights` vec stores it).

## Checks (run cargo from `app/`)

- `cargo build -p freecell-app -p freecell-engine`
- `cargo test -p freecell-app --lib` (+ `-p freecell-engine --lib`)
- render **subset** while iterating: `app/render-tests/scripts/render_tests.sh test formula_ref`
  (setup via `setup_render_env.sh` if first run) — no new baseline yet (deferred to Phase 5), just
  confirm no unexpected diff on existing grid cases.
- `cargo fmt --all --check`
