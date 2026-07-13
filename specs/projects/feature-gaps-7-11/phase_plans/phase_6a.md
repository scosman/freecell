---
status: complete
---

# Phase 6a: IronCalc fork — sheet-reorder API (§6.1)

## Overview

Cross-repo fork work that **gates Phase 6b** (sheet-reorder wiring + tab drag). Per
`functional_spec.md §6.3` and `architecture.md §6.1` and the CLAUDE.md standing process
("fix upstream, don't hack FreeCell"), this phase adds an undoable, xlsx-order-preserving
worksheet-reorder API to our IronCalc fork (`scosman/ironcalc`) on a clean single-feature
`fix/sheet-reorder` branch, folds it into the `freecell-fixes` integration branch, and
re-pins FreeCell to build against it. **No new FreeCell code uses the API yet** — that is
Phase 6b; the bar here is the fork's own tests + a FreeCell build/clippy against the new pin.

**Outcome: done.** API added + tested green in the fork; both fork branches pushed (no PR
opened); FreeCell re-pinned `48b0b23` → `a49cfd60` and builds clean.

## The API

`UserModel::set_worksheet_index(sheet_index: u32, new_index: u32) -> Result<(), String>`
(the symbol `architecture.md §6.1` names), wrapping a new low-level
`Model::move_sheet(sheet_index, new_index)`. The worksheet at `sheet_index` is moved so it
ends up at **exactly** `new_index`, shifting the intervening sheets. Naming/typing follows
the existing `delete_sheet`/`rename_sheet`/`duplicate_sheet` convention (index-addressed;
Model + UserModel methods share the low-level name, like `duplicate_sheet`).

Implementation (fork commit `21cde33`, off `main` @ `cedba4e`):

- **`base/src/new_empty.rs`** — `Model::move_sheet`: bounds-checks both indices, no-ops on
  same index, then `worksheets.remove(from)` + `worksheets.insert(to, ..)` + a single
  `reset_parsed_structures()`. Mirrors `delete_sheet` exactly — the reparse is what keeps
  references valid (see below).
- **`base/src/user_model/history.rs`** — new `Diff::MoveSheet { sheet_index, new_index }`
  (symmetric: a reverse move is its own inverse, so no worksheet snapshot is needed, unlike
  `DeleteSheet`).
- **`base/src/user_model/common.rs`** — `UserModel::set_worksheet_index`: validates, no-ops
  cleanly (no history), moves, remaps the selection by identity, pushes the diff. Plus a pure
  helper `selected_sheet_after_move(selected, from, to)`.
- **`base/src/user_model/undo_redo.rs`** — `MoveSheet` arms in both `apply_diff_list` (redo:
  move `from`→`to`) and `apply_undo_diff_list` (undo: move `to`→`from`), each re-remapping
  the selection. (Both matches are exhaustive, so the new variant is compiler-forced to be
  handled in both directions.)

### Why the four requirements hold

- **Undoable/redoable** — the `MoveSheet` diff rides `push_diff_list`/`History` like every
  other `UserModel` mutation; undo/redo apply the inverse/forward move.
- **xlsx-order-preserving** — the xlsx export iterates `workbook.worksheets` in vector order
  (`xlsx/src/export/workbook.rs` writes `<sheet>` entries in that order), so the new order
  round-trips through save/load with no extra work.
- **No dangling references** — sheet order is only a **vector position**; formulas key off
  the sheet **name** (stored in each worksheet's `shared_formulas` R1C1 strings) and defined
  names off the sheet **id**. `reset_parsed_structures()` re-resolves every reference by name
  against the reordered vector, exactly as `delete_sheet` already relies on. Verified by test
  (`=Sheet2!A1` still resolves, and a dependent recompute still flows, after the referenced
  sheet is moved).
- **Bounds/no-op** — out-of-range `sheet_index`/`new_index` are rejected with an error; moving
  to the current index is a clean no-op that records no history.

**Selection follows the sheet by identity** across the move (and undo/redo) rather than
pointing at whichever sheet lands in the old slot — this also gives Phase 6b's "dragged sheet
stays active" for free.

## Tests (upstream-style, mirror `test_add_delete_sheets.rs`)

`base/src/test/user_model/test_move_sheet.rs` (registered in `test/user_model/mod.rs`):

- `move_reorders_sheets` — front→end, end→front, middle moves land at the expected order.
- `move_undo_redo` — undo restores prior order, redo re-applies.
- `move_to_same_index_is_noop` — no order change **and** no history entry.
- `move_out_of_range_is_rejected` — both indices out of range error; order + history untouched.
- `move_preserves_cross_sheet_references` — `=Sheet2!A1` resolves after moving the referenced
  sheet; a dependent recompute flows; undo keeps references intact (the **no-dangling-ref**
  proof).
- `move_keeps_the_same_sheet_selected` — selection follows a non-moved and a moved sheet by
  identity, across move and undo.
- `move_propagates` — the `MoveSheet` diff replays through `flush_send_queue` /
  `apply_external_diffs` onto a fresh peer (mirrors `new_sheet_propagates`).

Pure-helper unit test in `common.rs`'s `#[cfg(test)]` module:
`test_selected_sheet_after_move` (moved sheet followed to dest; between-span shift; outside-span
unchanged).

xlsx round-trip in `xlsx/src/export/test/test_export.rs`:
`test_move_sheet_order_roundtrips` — reorder via `Model::move_sheet`, `save_to_xlsx` →
`load_from_xlsx`, assert the saved order is preserved **and** a cross-sheet reference still
resolves after reload (the **xlsx-order-preserving** proof).

**Fork test results (branch `fix/sheet-reorder`, then verified on `freecell-fixes`):**
`ironcalc_base` lib **2197 passed / 0 failed**; `ironcalc` (xlsx) all suites green
(**33 + 219 + 15 + …**, 0 failed); `cargo fmt -- --check` clean; strict clippy
(`-W unwrap_used -W expect_used -W panic -D warnings`) clean on `base` + `xlsx`; whole fork
workspace (incl. wasm/python/nodejs bindings) builds.

## Follow-up (record for when we offer this upstream — do NOT change the fork now)

- **Harden the undo/redo regression net** before opening the upstream PR: add a deeper
  multi-step test — 3+ stacked `set_worksheet_index` moves fully unwound (undo back to the
  original order and redo back to the final order), plus an explicit `from > to` (rightward→
  leftward) redo — on top of the current single-move `move_undo_redo`. Cheap, and it guards the
  symmetric-inverse `MoveSheet` diff against future churn. Not needed for FreeCell's Phase 6b;
  queued for the upstream-PR polish pass.

## Fork branch / integration / push (per the operating model)

- `fix/sheet-reorder` — single-feature branch off `main` @ `cedba4e`, one clean commit
  `21cde33` (`feat(base): add UserModel::set_worksheet_index to reorder worksheets`), authored
  `Steve Cosman <848343+scosman@users.noreply.github.com>`, **no AI/model/Co-Authored-By
  trailers**. Pushed to `origin` @ `21cde33`.
- `freecell-fixes` — fetched `origin/freecell-fixes` @ `70f512f` (carried the prior
  not-yet-upstreamed E5 `<indexedColors>` fix; the old E2 numFmt fix is now upstreamed and
  present in `main` @ `cedba4e`). Merged `fix/sheet-reorder` in with `--no-ff` (clean, no
  conflicts) → `a49cfd60`. Pushed to `origin` (`70f512f..a49cfd60`).
- **No pull request opened** to `ironcalc/IronCalc` (the owner will offer it upstream later);
  `fix/sheet-reorder` is left pushed and ready.

## FreeCell re-pin (the Phase 6a FreeCell deliverable)

- `app/Cargo.toml` `[patch.crates-io]` pins `ironcalc`/`ironcalc_base` by
  `branch = "freecell-fixes"` (unchanged — a branch pin), so only `app/Cargo.lock` moved:
  `cargo update -p ironcalc -p ironcalc_base` from `app/` bumped both from
  `#48b0b23` → `#a49cfd60`. The only other line the lock touches is one benign transitive
  edge: `iana-time-zone` flips its dependency from `windows-core 0.62.2` → `windows-core
  0.57.0`. That edge is `cfg(windows)`-only (never compiled on FreeCell's macOS/Linux
  targets) and **both** `windows-core` versions already exist in the lock, so no crate is
  added/removed and nothing on our build changes — it is cosmetic churn, not a real dep bump.
- **The re-pin advances the whole `freecell-fixes` baseline, not just sheet-reorder.** The old
  `48b0b23` is **not** an ancestor of `a49cfd60` — `freecell-fixes` was rebased onto a newer
  upstream `main` (old base `29daa42` → new base `cedba4e`) — so ~25 `base`/`xlsx` source files
  differ from the previously-pinned engine beyond this phase's change. That is **expected** per
  the "sync `main` periodically" operating model (`specs/projects/ironcalc-upstreaming`), and the
  green FreeCell `cargo test --workspace` below is the safety net that the incidental upstream
  drift is benign. (The upstreamed E2 numFmt fix survives the advance — verified in the
  checked-out source and by the green suite.)
- Confirmed the new API is reachable: Cargo's checkout at `a49cfd60` contains both
  `UserModel::set_worksheet_index` and `Model::move_sheet`.
- **FreeCell not committed** (manager owns FreeCell commits); FreeCell stays on
  `claude/feature-gaps-7-11-vit4gy`. Only `Cargo.lock` is dirty.

## Project checks (FreeCell, against the new pin)

- `cargo build --workspace` → clean (exit 0).
- `cargo fmt --all --check` → clean.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo test --workspace` → green **except** the 2 `charts_roundtrip_libreoffice` tests
  (`libreoffice_reopens_freecell_saved_line_chart`,
  `libreoffice_reopens_freecell_authored_line_chart`), which fail on the `soffice
  --convert-to` step in this headless container (known-accepted, unrelated). Totals:
  freecell-app 373, freecell-chart-model 93, freecell-core 158, freecell-engine 243, plus
  dependency_rule 5, charts_corpus 8, and the other suites — all passing.

(No numFmt regression from the re-pin: the new pin `a49cfd60` still carries the ECMA-376
built-in table + the id-39 regression fix, verified in the checked-out source and confirmed by
the green suite.)

## Conclusion

The reorder API lands cleanly in the fork with undo/redo, xlsx-order-preservation, and
reference-safety all test-proven; the fork's own suite + strict lint are green; both fork
branches are pushed (no upstream PR); and FreeCell is re-pinned and builds/lints/tests clean
against it (modulo the 2 known LibreOffice failures). **Phase 6b is unblocked.**
