---
status: draft
---

# Implementation Plan: Hyperlinks

Phased build order. Details live in `functional_spec.md` (F-items) and `architecture.md`
(§ sections); the `file:line` maps live in `research/`. Render validation is its **own late
phase** per the repo's render-tests policy — earlier phases verify with crate-scoped checks plus
a render **subset**; the full pixel suite and the CI `render` gate run once, at the end.

Phases 1–2 are fork work in `/home/user/IronCalc`; phases 3–7 are FreeCell.

## Phases

- [ ] **Phase 1 — Fork sync.**
  Fast-forward `main` to upstream `91d343c3`; delete the five `fix/*` branches already upstream;
  merge new `main` into `freecell-fixes` (arch §1 — 11 hunks, 8 trivial, the real work is
  `user_model/common.rs`'s `delete_rows`/`delete_columns` restructure vs our frozen-pane +
  merged-cells fields); rebase the three surviving `fix/*` branches. Add the two integration
  tests for the semantic gaps the merge won't flag: row/column move carrying both a merged region
  and a link, and clear-range vs merged-cells. Push `main` + `freecell-fixes`. **Verification
  gate:** confirm theme index 10 resolves to a blue (arch §3.5) and report the answer — it
  decides three lines in Phase 3.

- [ ] **Phase 2 — Fork fix: batch input emits link diffs.**
  Own branch `fix/batch-set-inputs-link-diffs` off the synced `main`, upstream-style tests, then
  merged into `freecell-fixes` (arch §2). Without it, undo after pasting a column of URLs reverts
  the values but leaves the links and their styling (F7.5). Prepare the upstream compare link +
  title + description for the owner — **one fix, one branch, one PR**; do not fold this into
  Phase 1's merge.

- [ ] **Phase 3 — Re-pin + engine facade + link cache.**
  Bump FreeCell's `freecell-fixes` lock; `LinkTarget` in `freecell-core`; `WorkbookDocument::{cell_link,
  set_cell_link, remove_cell_link}`; `Command::{SetCellLink, RemoveCellLink}` + bucketing +
  `apply_one` + `AppliedKind::StyleOnly`; sparse `SheetCache.links` with **all three** agreement
  paths — build, mirror (dynamic links refresh on every `AppliedKind::Cell`), undo-touch-set
  (arch §3). Engine unit tests + the `tests/roundtrip.rs` xlsx cases (F10). No visible UI yet.

- [ ] **Phase 4 — Render links + open them.**
  `LINK` constant consolidated into `grid/mod.rs`; `visible_links` snapshot under the existing
  single read lock; `CellPaint.link`; the three styling call sites that must agree — per-cell
  loop, `resolve_cell_paint`, `SpillPlan`/`spill_element` (F1.5); `.cursor_pointer()` +
  `.tooltip()`; pure click-decision fn + `handle_mouse_up` wiring; `GridEvent::OpenLink` + window
  sink; pure `decide_open` policy with the hostile-input table, the `file:` confirm, and the
  refusal message (arch §4–6, F1–F3); internal-link navigation (arch §7, F4). Verify with
  `render_tests.sh test link_` subset only.

- [ ] **Phase 5 — Authoring: ⌘K dialog, remove, menus, toolbar.**
  `ChromeView` link dialog on the `render_find_bar` lifecycle + `render_delete_confirm` chrome +
  the `BlockMouse` backdrop; pure target-normalization fn tested against the engine's own
  accept/reject table; `InsertLink`/`RemoveLink`/`OpenLink` actions, ⌘K binding, Edit-menu
  placement test, `cell_menu_items` entries (load-bearing — there is no Linux menu bar), action-row
  button + the `link.svg` availability check (arch §8–9, F5–F6). Chrome view tests + Xvfb smoke
  launch.

- [ ] **Phase 6 — Docs + gap closure.**
  **Delete** the `GAPS.md` Hyperlinks row including its `HYPERLINK()` clause (policy: removed, not
  annotated), leaving the distinct external-workbook-links row alone; correct
  `projects/xlsx-preservation.md`; add new rows for the genuine remaining holes — range-apply, and
  `file:` links if F3.2 shipped as a block (arch §12).

- [ ] **Phase 7 — Render validation (dedicated late phase).**
  Full pixel suite under a ~10-minute watchdog; regenerate and **eyeball** the `link_*` baselines;
  commit them; dispatch the CI `render` gate on the branch and confirm it passes (arch §10).
