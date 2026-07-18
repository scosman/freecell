---
status: draft
---

# Implementation Plan: Merged Cell UI

Phased build order. Details live in `functional_spec.md`, `ui_design.md`, and
`architecture.md` (§ references below). Render validation is its **own late phase** per the
repo's render-tests policy — earlier phases verify with crate-scoped checks + a render
**subset**; the full pixel suite + CI `render` gate run once, at the end.

## Phases

- [ ] **Phase 1 — Re-pin + engine wrapper + resident MergeMap + retire interim guard.**
  Bump the `freecell-fixes` lock to the merge tip (arch §1); add `Document::merge_cells` /
  `unmerge_cells` / `merged_regions` / `merge_would_lose_data` (§3, §8); add
  `Command::{MergeCells,UnmergeCells}` + `WorkerEvent::MergeNeedsConfirm` and `apply_one`
  arms (§3); build the `MergeMap` from `merged_regions` in the cache (§2); add pure
  `freecell-core/src/merge.rs` (§2); remove the insert/delete guard + menu flags/footnote,
  keep the fill guard (§5). Worker + pure unit tests; no visible UI change yet.

- [ ] **Phase 2 — Render merged regions as one box.**
  `visible_merges` snapshot in `resolve_frame`; skip covered cells; region-draw pass via
  `span_rect`; interior-gridline suppression; explicit-border + text-spill handling; span the
  active outline / range overlay / in-cell editor (arch §6, ui §3–5). Verify with
  `render_tests.sh test merge_` subset (full suite deferred to Phase 5).

- [ ] **Phase 3 — Merge-aware selection & editing.**
  `snap_cell`, `effective_range`, merge-aware `apply_motion` (enter/exit/shift-extend fixpoint),
  click/drag snapping at the input call-sites; edit/clear route to the anchor (arch §7,
  fspec F4–F5). Heavy pure unit-test coverage (the delicate phase).

- [ ] **Phase 4 — Merge/Unmerge control + data-loss dialog.**
  Action-row toggle (icon/states/tooltip), Edit-menu item + ⌃⌘M, `toggle_merge` decision,
  `MergeCells{confirmed}` + `MergeNeedsConfirm` round-trip, new `ActiveModal::Confirm`, fill
  message re-word (arch §8–9, ui §1–2,6). Chrome view tests + Xvfb smoke launch.

- [ ] **Phase 5 — Render validation + round-trip (dedicated late phase).**
  Full pixel suite under a ~10-min watchdog; regenerate + **eyeball** `merge_*` baselines,
  commit them; dispatch the CI `render` gate on the branch and confirm green; add the
  in-app-created-merge xlsx round-trip test (arch §10).
