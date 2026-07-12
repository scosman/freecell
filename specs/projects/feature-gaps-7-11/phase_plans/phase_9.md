---
status: complete
---

# Phase 9: Replace All single-undo (ironcalc fork `set_user_inputs` + FreeCell swap)

## Overview

Phase 4 shipped `Command::ReplaceAll` fully working but recording **N** engine undo entries
(one per changed cell — the accepted `SetFont` "K+1" precedent) because IronCalc exposed no
public way to group scattered cell writes into one `diff_list` (`History::push` /
`UserModel::push_diff_list` are `pub(crate)`; the public rectangle pastes clear+rewrite the whole
bounding box, unusable for scattered find/replace matches). See `phase_plans/phase_4.md`
§ROADBLOCK.

This phase closes that gap the CLAUDE.md way — **fix the engine, don't hack FreeCell** — in two
independently-revertible pieces:

1. **ironcalc fork** — a new public `UserModel::set_user_inputs(&[(sheet,row,col,String)])` on its
   **own clean single-feature `fix/batch-set-inputs` branch** (off `main`, upstream-style tests),
   folded into `freecell-fixes`. It applies every listed cell input and records **one**
   `diff_list`, so a single Undo reverts them all and the workbook evaluates once. Mirrors
   `paste_csv_string`'s single-`push_diff_list` pattern, minus the rectangle `range_clear_contents`
   (only the listed cells are touched — unrelated formulas/array formulas/typed values between the
   matches are left alone).
2. **FreeCell** — re-pin `[patch.crates-io]` to the new `freecell-fixes` commit, then swap the two
   isolated ReplaceAll call sites to the batch method so ReplaceAll becomes **one undo step**.
   Find + ReplaceOne unchanged.

**Constraint (this session):** the ironcalc fork branches are committed + pushed (fork process);
FreeCell stays on `claude/feature-gaps-7-11-vit4gy` and is **not** committed/pushed here (the
manager commits the FreeCell side). No upstream PR (owner offers upstream later). No pixel impact
(ReplaceAll is behavior/undo, chrome not baselined).

## Fork — `UserModel::set_user_inputs` (`fix/batch-set-inputs` off `main`)

**Signature** (placed in `base/src/user_model/common.rs`, right after the single-cell
`set_user_input`):

```rust
pub fn set_user_inputs(&mut self, inputs: &[(u32, i32, i32, String)]) -> Result<(), String>
```

**Semantics:**
- Validate every `(sheet, row, column)` up front (sheet exists, `is_valid_column_number`,
  `is_valid_row`) so a bad entry mid-list can't leave a partial, history-less write — all-or-nothing.
- Empty slice ⇒ `Ok(())` no-op (no history entry).
- For each entry: capture the cell's old value (recording a `SpillCell` old value as `None`, exactly
  as single-cell `set_user_input` does — undo re-spills from the anchor), `self.model.set_user_input`
  it, and push one `Diff::SetCellValue` into a single `diff_list`.
- `push_diff_list(diff_list)` **once** (⇒ one history entry, one send-queue entry), then
  `evaluate_if_not_paused()` **once** at the end (paste pattern — a no-op under a paused caller like
  FreeCell's worker, one recompute otherwise).

**Judgment call (recorded in DECISIONS):** the batch omits the per-cell row-height auto-grow that
single-cell `set_user_input` performs, matching `paste_csv_string` (which also omits it). Keeps the
diff_list one `SetCellValue` per cell and the batch semantics predictable; row auto-grow for
replace is out of scope.

**Tests** — new `base/src/test/user_model/test_set_user_inputs.rs`, registered in
`user_model/mod.rs`, mirroring the `test_move_sheet.rs` upstream-style layout:
- `batch_write_is_one_undo_step` — three scattered cells in one call; **one** `undo` clears all,
  `!can_undo` after; **one** `redo` re-applies all.
- `batch_overwrite_undo_restores_prior_values` — single undo restores the prior (non-empty) values,
  the seed edits stay individually undoable.
- `batch_recomputes_dependent_formula` — one evaluate at the end reflects all inputs; undo recomputes.
- `empty_batch_is_a_noop` — no history entry.
- `out_of_range_batch_is_rejected_without_mutating` — an out-of-range entry ⇒ `Err`, no partial
  write, no history entry.
- `batch_across_sheets_is_one_undo_step` — cells on two sheets in one call; one undo clears both.
- `batch_propagates_as_one_diff_list` — the send queue replays onto a fresh peer.

**Fork validation:** `cargo test -p ironcalc_base` green; `make lint` (fmt + strict clippy:
`-W clippy::unwrap_used -W clippy::expect_used -W clippy::panic -D warnings`) clean. Commit on
`fix/batch-set-inputs` (owner author, clean upstream-style message, no AI trailers), merge into
`freecell-fixes` alongside the existing sheet-reorder fix (do NOT rebase it away), push both
branches to `origin`. No upstream PR.

## FreeCell — re-pin + swap the two call sites

- **Re-pin:** `cargo update -p ironcalc -p ironcalc_base` from `app/` to the new `freecell-fixes`
  head (`Cargo.lock` only; the branch pin is unchanged). Confirm `set_user_inputs` is reachable.
  Besides the two `ironcalc`/`ironcalc_base` source-rev bumps, the lock also moved one benign
  transitive edge — `iana-time-zone`'s `windows-core 0.57.0 → 0.58.0` — which is `cfg(windows)`-only
  (never compiled on FreeCell's macOS/Linux targets), harmless churn (same class as the Phase-6a
  note); leaving it as Cargo resolved it is fine.
- **`freecell-engine/src/document.rs::replace_all_matches`** — replace the per-cell `set_cell_input`
  loop with a single `self.model.set_user_inputs(&batch)` call (batch = `(sheet_idx, row, col,
  new_input)` in row-major order). Still returns the changed `CellRef`s. One engine undo entry.
- **`freecell-engine/src/worker/run.rs::apply_replace_all`** — collapse the per-cell
  `Touch::Cells` + `ops_seen += n` bookkeeping to a **single** undo entry: `ops_seen += 1` and push
  one `Touch::Ranges(touched)` covering every changed cell (the `commit_paste` pattern), so a single
  Undo reverts the whole replace. Publish/refresh unchanged.
- Find + ReplaceOne untouched.

## Tests updated (single-undo)

- `document.rs::replace_all_replaces_every_match_and_single_undo_target` — one `doc.undo()` restores
  ALL replaced cells (was: `changed.len()` undos).
- `run.rs::replace_all_command_replaces_reports_count_and_publishes` — asserts **one** new
  `undo_touches` entry for the batch (was: one per replaced cell), plus a new
  `replace_all_is_single_undo_step` worker test that undoes once and asserts every replaced cell is
  restored.

## Validation

From `app/`: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo build --workspace`, `cargo test --workspace` (2 `charts_roundtrip_libreoffice` failures
known-accepted, unrelated). No pixel suite (chrome not baselined). Independently revertible: the
FreeCell swap is a clean self-contained change that could be reverted to the multi-undo interim
without touching Find/ReplaceOne.
