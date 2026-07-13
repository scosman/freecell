---
status: complete
---

# Phase 4: Find / replace

## Overview

A dismissible find/replace bar scoped to the current sheet (`functional_spec.md §4`,
`architecture.md §4`, `ui_design.md §1–2`). Opened via ⌘F / Ctrl+F or an action-bar search
button; it renders directly below the data/formula row and pushes the grid down. Find scans the
active sheet's used range **in the worker** (huge-sheet safe, off the render publication) and
returns row-major matches; the current match is selected + scrolled into view. Replace-one and
Replace-all mutate through the worker.

Split across three layers, exactly as the architecture prescribes:

- **Worker/engine** — `freecell_core::find` pure predicate/replace helpers; `Command::Find` →
  `WorkerEvent::FindResults`; `Command::ReplaceOne`; `Command::ReplaceAll` →
  `WorkerEvent::ReplacedCount`; a `document.rs` used-range scan over `sheet_data` (populated cells
  only, row-major).
- **Chrome** — find-bar state on `ChromeView`, `render_find_bar`, the action-row search button, and
  all UI wiring (open/focus, next/prev wrap, counter, replace, escape/X close, sheet-switch
  re-scope).
- **Shell** — `OpenFind` action + `primary()+f` keybinding + Edit-menu item, the window action
  handler, worker-event routing to the chrome, and a grid select-and-reveal request.

Chrome/tabs are **not** baselined by the pixel suite → no pixel run; validate with gpui view/unit
tests + a single Xvfb smoke launch (`architecture.md §8`, CLAUDE.md).

## ROADBLOCK → resolved as Phase 9 (its own standalone ironcalc fork fix)

`functional_spec.md §4.4` and the task require Replace All to be **one undoable batch** (a single
Undo reverts every replacement), and the task says: *"If IronCalc genuinely cannot group multiple
writes into one undo step (verify against how paste does it), return a roadblock rather than
shipping N undo steps."*

**Owner decision:** the single-undo fork fix is **NOT** folded into Phase 6 (sheet-reorder keeps its
own separate fork branch). It becomes its **own new standalone final phase — `implementation_plan.md`
Phase 9 — with its own clean single-feature ironcalc `fix/` branch + upstream PR, independently
revertible.** The verified analysis + the exact swap follow.

**Verified: IronCalc cannot, from FreeCell's accessible API.** How paste achieves single-undo:
`UserModel::paste_csv_string` / `paste_from_clipboard` build ONE `Vec<Diff>` and push it via
`History::push` / `self.push_diff_list(..)`. Both `History::push` and `UserModel::push_diff_list`
are **`pub(crate)`** in `ironcalc_base` — not callable from FreeCell. The two *public* multi-cell
single-undo methods are unusable for scattered find/replace matches:

- `paste_csv_string(area, csv)` `range_clear_contents` the **entire** bounding rectangle then
  rewrites every cell — it would round-trip (and risk corrupting) non-matching formulas / array
  formulas / typed values across the whole used range, and produce an O(area) diff. That is a
  compensating FreeCell hack CLAUDE.md forbids ("fix upstream, don't hack FreeCell").
- `paste_from_clipboard(.., clipboard: &ClipboardData, ..)` takes a **sparse** map and builds one
  diff-list, but `ClipboardData`'s `ClipboardCell` has private fields with no public constructor,
  and it applies formula-reference displacement — not constructible/usable from FreeCell.

**The fix (Phase 9)** is the CLAUDE.md standard: add a small public `UserModel` method to
`scosman/ironcalc` — `set_user_inputs(&[(sheet,row,col,String)])` — that applies the writes and
pushes **one** `diff_list` (mirroring `paste_csv_string`'s pattern, minus the rectangle clear) — on
its **own** clean single-feature `fix/` branch with upstream-style tests + one focused upstream PR,
folded into `freecell-fixes`, then re-pin FreeCell's `[patch.crates-io]` + bump `Cargo.lock`. This is
a fork + FreeCell-pin change out of scope for this single-branch coding phase; it is scheduled as its
own phase and stays independently revertible.

**Interim shipped in this phase:** `Command::ReplaceAll` is fully functional — one paused-eval + one
`evaluate()` + one publish — but records **N** engine undo entries (one per `set_user_input`),
exactly like the accepted `SetFont` "K+1 undo steps" precedent (`run.rs` `apply_set_font`, already in
`DECISIONS_TO_REVIEW`).

**Phase 9 swaps TWO isolated FreeCell call sites** (not one) so ReplaceAll collapses to a single undo
step; both are the sole places that need to change:
1. `document.rs::replace_all_matches` — the per-cell `set_user_input` loop becomes one
   `set_user_inputs(&batch)` call.
2. `worker/run.rs::apply_replace_all` — it currently pushes **one `Touch::Cells` + one `ops_seen`
   increment per changed cell** (kept 1:1 with the N engine undo entries); with the batch method it
   must **collapse to a single undo touch/op** (one `Touch` covering the changed cells, `ops_seen +=
   1`) so a single Undo reverts the whole replace.

Everything else in Phase 4 is complete, tested, and green.

## Steps

### Worker / engine

1. **`freecell-core/src/find.rs` (new)** — pure, gpui/engine-free predicate + replace:
   - `pub fn cell_matches(content: &str, query: &str, match_case: bool, whole_cell: bool) -> bool`
     — empty query ⇒ false; `whole_cell` ⇒ equality; else substring; case per `match_case`.
   - `pub fn replace_in_cell(content, query, replacement, match_case, whole_cell) -> Option<String>`
     — `None` if no match; whole-cell ⇒ `Some(replacement)`; else replace **every** occurrence
     (char-vector scan so case-insensitive replacement preserves original-case surrounding text).
   - Register `pub mod find;` in `lib.rs`. Unit tests cover case/whole-cell/substring/empty/
     multi-occurrence/formula-text (a `"=A1+A2"` content string).

2. **`worker/protocol.rs`** — add to `Command`:
   - `Find { sheet: SheetId, query: String, match_case: bool, whole_cell: bool }` (a read).
   - `ReplaceOne { sheet, cell: CellRef, query, replacement, match_case, whole_cell }` (worker
     recomputes from fresh `cell_content` — race-free; a single-cell edit is inherently one undo
     step).
   - `ReplaceAll { sheet, query, replacement, match_case, whole_cell }`.
   Add to `WorkerEvent`:
   - `FindResults { matches: Vec<CellRef> }`.
   - `ReplacedCount { n: usize }`.

3. **`document.rs`** — `pub(crate)` scans over the sheet's populated cells:
   - `find_matches(sheet_idx, query, match_case, whole_cell) -> Vec<CellRef>` — iterate
     `worksheet.sheet_data` (populated cells; it is a **`HashMap`**, so iteration order is arbitrary
     and the results are **sorted explicitly by `(row, col)`** for row-major order), `cell_content`
     each, `cell_matches`, collect `CellRef`s.
   - `replace_all_matches(sheet_idx, query, replacement, match_case, whole_cell) -> Vec<CellRef>` —
     same scan; for each populated cell, `replace_in_cell`; when `Some(new)` and it differs,
     `set_user_input` (the N-undo interim — see ROADBLOCK). Returns the changed cells (sorted
     row-major). Collect matches first, then write (avoid mutating while iterating the borrowed
     `sheet_data`).
   - `replace_one(sheet_idx, cell, query, replacement, match_case, whole_cell) -> bool` — read
     `cell_content`, `replace_in_cell`; when `Some` and it differs, `set_user_input`; returns whether
     it wrote (a same-text replacement is a skipped no-op).

4. **`worker/run.rs`** — classify + dispatch the three new commands in the drain loop
   (exhaustive match — no catch-all):
   - `Find` → bucket like the `reads` (GetCellContent) path: resolve sheet, `find_matches`, emit
     `FindResults`. No eval, no publish.
   - `ReplaceOne` / `ReplaceAll` → standalone ops (like clipboard ops): guarded (`catch_unwind`)
     paused-eval → mutate → one `evaluate()` → publish + `Published` → push undo touch entry(ies)
     + `refresh_cache_cells` → emit `ReplacedCount { n }`. Degraded-guarded (emit
     `EditRejected{Degraded}` and reply `ReplacedCount{0}`).

### Chrome (`chrome/view.rs`, `chrome/mod.rs`)

5. **State on `ChromeView`** (near the other open-flags): `find_open: bool`,
   `find_input: Entity<InputState>`, `replace_input: Entity<InputState>`, `match_case: bool`,
   `whole_cell: bool`, `matches: Vec<CellRef>`, `match_idx: Option<usize>`, plus a
   `find_query_dirty`-style guard is not needed. Construct the two inputs + subscribe to them
   (mirror the `content_input` subscription).

6. **`render_find_bar(cx)`** — inserted in `render` between `render_data_row` and the grid body
   (`flex_col` pushes the grid down). Height `DATA_ROW_H`, `CHROME_BG`, bottom `HAIRLINE`. Layout
   per `ui_design §1`: find field (~220px) · replace field (~220px) · `Aa` match-case toggle ·
   match-entire-cell toggle · `DIVIDER` · prev/next chevron icon buttons (`icons/chevron-up.svg` /
   `icons/chevron-down.svg`, bundle) · match counter (min-width, "3 of 12" / muted "No results" /
   empty) · `flex_1` spacer · Replace + Replace All ghost text buttons (disabled 40% when nothing
   to act on) · dismiss `icons/square-x.svg`. Only rendered while `find_open`.

7. **Action-row search button** — small ghost icon button at the trailing end of
   `render_action_row` (after insert-chart, behind an `action_divider()`), `icons/search.svg`
   (resolves from the gpui-component bundle — see DECISIONS; not vendored because the bundle already
   ships it and gpui-component itself renders it, so vendoring would shadow a bundle icon). Tooltip
   "Find & Replace (⌘F)". `selected` while `find_open`; click toggles the bar.

8. **Behavior methods** (plain, unit-testable):
   - `toggle_find(window, cx)` / `open_find` / `close_find` — open focuses the find field
     (retains prior text); close returns focus to the grid (`ChromeGridRequest::FocusGrid`) and
     keeps the text.
   - `recompute_matches` — sends `Command::Find`; on `FindResults` store `matches`, pick
     `match_idx` = first match at/after the current selection, reveal.
   - `select_current_match` — `ChromeGridRequest::SelectAndReveal(cell)` (new variant).
   - `next_match` / `prev_match` — advance/retreat `match_idx` with wraparound; reveal.
   - `replace_current` — `Command::ReplaceOne` for the current match cell; re-Find on the reply.
   - `replace_all` — `Command::ReplaceAll`; show count on `ReplacedCount`.
   - `on_find_input_event` / `on_replace_input_event` — find Change ⇒ recompute; find
     PressEnter{shift} ⇒ prev/next; replace PressEnter ⇒ replace_current.
   - Escape on the bar div ⇒ `close_find`.
   - `on_worker_event` — handle `FindResults` + `ReplacedCount`.
   - `adopt_active_sheet` already re-points the chrome on a sheet switch; extend
     `on_selection_changed`/the switch path so an **open** bar re-scopes (re-send Find, reset idx).

9. **`chrome/mod.rs`** — add `ChromeGridRequest::SelectAndReveal(CellRef)`.

### Shell (`shell/mod.rs`, `shell/menus.rs`, `shell/window.rs`, `grid/view.rs`)

10. `actions!` gains `OpenFind`; `menus::bind_keys` binds `primary()+f`; the Edit menu gains a
    "Find…" item.
11. `WorkbookWindow::render` gains `.on_action(OpenFind → chrome.toggle_find)`.
12. `make_chrome_grid_sink` handles `SelectAndReveal(cell)` (deferred, like the others):
    `grid.select_and_reveal(cell)` + mirror the selection into `sink_shared.last_selection` and the
    chrome (`on_selection_changed`), keeping the find field focused (no `FocusGrid`).
13. `on_worker_event` routes `FindResults` + `ReplacedCount` to the chrome (exhaustive-match arm).
14. `grid/view.rs` — `pub fn select_and_reveal(&mut self, cell, window, cx)` = single selection +
    `reveal_and_announce`.

## Tests

Worker/core unit tests:
- `cell_matches_*` — case on/off, whole-cell vs substring, empty query, formula-text content.
- `replace_in_cell_*` — whole-cell replacement, multi-occurrence substring, case-insensitive
  preserves surrounding case, no-match ⇒ None, formula-text replacement.
- `find_matches_scopes_used_range_row_major` — matches only populated cells, row-major order,
  case/whole-cell honored.
- `replace_all_replaces_every_match_and_reports_count` + `replace_all_undo_restores_all` (undo the
  recorded count of steps — documents the N-undo interim; flips to a single undo when the fork
  method lands).
- `replace_one_rewrites_matching_cell_only`.

gpui chrome view tests (headless, `RecordingClient`):
- `cmd_f_opens_and_focuses_find_bar` / `search_button_toggles_bar`.
- `toggles_flip_match_case_and_whole_cell` and re-send Find.
- `find_results_set_counter_and_select_first`.
- `next_prev_wrap_around`.
- `escape_and_x_close_and_keep_text`.
- `sheet_switch_rescopes_open_bar`.
- Assets: `search.svg` resolves (bundle), `chevron-up/down.svg` resolve, `square-x.svg` resolves.

Validation: `cargo fmt/clippy/build/test --workspace`; one Xvfb smoke launch.
