---
status: complete
---

# Implementation Plan: Feature Gaps 7_11

Ordered checklist; details live in `functional_spec.md`, `ui_design.md`, and
`architecture.md` (§ references below). The features are **mutually independent** and meant
to be coded **async** — the ordering below is a suggested sequence (quick wins first), not a
hard dependency chain. Only two hard constraints exist:

1. **Phase 6a (IronCalc fork API) must land before Phase 6b (reorder wiring).**
2. **Auto-grow (Phase 7) is the last coding phase and independently revertible; the render
   validation phase (Phase 8) runs after it.**

Each coding phase: write a phase plan, implement, run the project's checks + the relevant
render **subset** (spill/auto-grow only), code-review, commit. Record judgment calls in a
`DECISIONS_TO_REVIEW.md` in this folder.

## Phases

- [x] **Phase 1 — Font-warning fix** (§1): add `gpui::svg_renderer=error` to the default
      `EnvFilter` (`shell/main.rs`); keep `RUST_LOG` override working; smoke-launch to
      confirm the two WARN lines are gone. *(Trivial quick win. No pixel impact.)*

- [x] **Phase 2 — Quick-edit mode** (§5): `quick_edit` flag on `ChromeView`; set in
      `begin_typed`, cleared in `begin_in_cell` / formula-bar focus; arrow interception in
      the data-row + in-cell `capture_key_down` handlers → `commit_and_move`; cancel on
      mouse-caret / Home / End / modified-arrow; thread `quick_edit` into
      `ChromeGridRequest::EditState`. gpui view/unit tests. *(No pixel impact.)*

- [x] **Phase 3 — Text spill** (§2): gpui-free neighbor-scan + direction helper in
      `grid/layout.rs` (unit-tested); spill-rect element in `build_grid_layers`/`cell_element`;
      text-only, wrap-off, alignment-aware (rightward must-have, left/center gated together
      and punt-able); coverage-edge safety. Iterate with `render_tests.sh test spill_`.
      *(Moves pixels — baselines refreshed in Phase 8.)*

- [x] **Phase 4 — Find/replace** (§4): `render_find_bar` under the data row + `find_open` /
      field / toggle / match state on `ChromeView`; `search.svg` icon + action-row button;
      worker `Command::Find` + `WorkerEvent::FindResults`, `Command::ReplaceAll` (one undo
      batch) + single-replace via `SetCellInput`; `OpenFind` action + `⌘F` keybinding +
      window handler; Escape/X close; sheet-switch re-scope. Worker unit tests + gpui view
      tests + smoke launch. *(No pixel impact — chrome not baselined.)*

- [x] **Phase 5 — Verify right-click insert/delete** (§7): Xvfb smoke — confirm header
      right-click shows Insert/Delete with correct counts + applies, merge guard intact.
      File a bug only if a real gap surfaces; else close as verified. *(No code expected.)*

- [x] **Phase 6a — IronCalc fork: sheet-reorder API** (§6.1): on `scosman/ironcalc`, added an
      undoable, xlsx-order-preserving `UserModel::set_worksheet_index` (wrapping
      `Model::move_sheet`) with upstream-style tests on a clean `fix/sheet-reorder` branch
      (commit `21cde33`); folded into `freecell-fixes` (`a49cfd60`); both fork branches pushed,
      **no upstream PR opened** (owner offers upstream later). FreeCell re-pinned
      `#48b0b23 → #a49cfd60` (`Cargo.lock` only — branch pin unchanged); builds + fmt + clippy
      clean; `cargo test --workspace` green modulo the 2 known LibreOffice failures. See
      `phase_plans/phase_6a.md`. *(Separate repo — see CLAUDE.md /
      `specs/projects/ironcalc-upstreaming` operating model. Gates 6b.)*

- [x] **Phase 6b — Sheet reorder wiring + tab drag** (§6.2–6.3): `Command::MoveSheet` +
      worker dispatch + `document.rs::move_sheet` + republish `SheetsChanged`; tab drag state
      + threshold + drop indicator + lift in `render_tab`; drop → `MoveSheet`; active follows
      `SheetId`. Worker test + gpui view test (index-compute helper as pure fn) + smoke
      launch. *(No pixel impact — tab bar not baselined.)*

- [x] **Phase 7 — Auto-grow rows** (§3) *(LAST coding phase; independently revertible)*:
      manual-rows set (`SheetCache`), marked on user resize only; UI-thread wrap measurement
      of dirty visible wrap-on cells; `Command::AutoGrowRowHeights` (auto rows only, no undo
      spam, doesn't mark manual); oscillation guard; cap. Retain existing font/newline
      auto-grow. Unit + worker + `render_tests.sh test autogrow_`. *(Moves pixels — baselines
      in Phase 8.)*

- [ ] **Phase 8 — Render validation + closeout** (§8): full pixel suite under a ~10-min
      watchdog; regenerate + **eyeball** the intentional `spill_` + `autogrow_` baseline
      changes and commit them; dispatch the CI `render` gate on the branch and confirm green;
      final Xvfb smoke of the chrome features (find bar, tab drag, quick-edit); update
      `GAPS.md` (spill/overflow F1, wrap+auto-grow) + note the shipped features; sweep
      `DECISIONS_TO_REVIEW.md`.

- [ ] **Phase 9 — Replace All single-undo (ironcalc fork)** (§4.4): make Replace All ONE undo step.
      Phase 4 shipped `Command::ReplaceAll` working but with N engine undo entries (one per changed
      cell — the `SetFont` "K+1" precedent) because IronCalc exposes no public way to group scattered
      cell writes into one `diff_list` (`History::push`/`push_diff_list` are `pub(crate)`; the public
      rectangle pastes are unusable for scattered matches). Per CLAUDE.md, add
      `UserModel::set_user_inputs(&[(sheet,row,col,String)])` (one `diff_list`, no rectangle clear) to
      the `scosman/ironcalc` fork on its **own clean single-feature `fix/` branch** (upstream-style
      tests, one focused upstream PR — do NOT reuse Phase 6's sheet-reorder branch); fold into
      `freecell-fixes`; re-pin FreeCell's `[patch.crates-io]` + bump `Cargo.lock`. Then swap the two
      isolated FreeCell call sites to the batch method so a single Undo reverts the whole replace:
      (1) `document.rs::replace_all_matches` (per-cell loop → one batch call) and
      (2) `worker/run.rs::apply_replace_all` (collapse the per-cell `Touch`/`ops_seen` pushes to a
      single undo touch/op). Keep it **independently revertible**. *(Separate repo — see CLAUDE.md /
      `specs/projects/ironcalc-upstreaming`. No pixel impact — chrome not baselined.)*
