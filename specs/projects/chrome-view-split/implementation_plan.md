---
status: complete
---

# Implementation Plan: chrome-view-split

Ordered by risk: the rename first (so git records every later move as a move), then the
mechanics-proving extraction, then domains from most independent to most entangled. `shell.rs`
is last because it is the residue — whatever has not moved out.

Every phase ends green on the full gate (functional spec §6): `cargo build -p freecell-app`,
`cargo test -p freecell-app --lib` with the test-name multiset check against
`baseline_tests.txt`, `cargo fmt --all --check`, `cargo clippy -p freecell-app --all-targets
-- -D warnings`. A phase that cannot go green is reverted, not patched forward.

Details live in `architecture.md` §7 (the source-range → destination mapping) and §3
(privacy). No phase re-decides anything settled there.

## Phases

- [x] **Phase 1: Directory + test support.** `git mv view.rs view/mod.rs` as its own commit
      (arch §4.1), then extract `test_support.rs` — the `Harness`, eight harness helpers,
      two body stubs, and the 15 `#[cfg(test)]` seams. Proves the `pub(super)` + `use super::*`
      mechanics (arch §3) on a small surface before any production code moves.
- [x] **Phase 2: `stats.rs` and `find.rs`.** The two smallest, least-coupled domains (~110 and
      ~410 production lines). First phase to move production code *and* its tests together;
      validates the banner-to-banner cut end to end.
- [x] **Phase 3: `tabs.rs`.** Sheet tab bar, reorder drag, rename, context menu, delete
      confirm, plus `TabDrag`/`TabSpan` and the tab-geometry free functions.
- [x] **Phase 4: `charts.rs`.** Insert-chart menu, chart edit panel and its P20 chrome, and the
      `ChartPanel`/`ChartPanelSeries` types — which must stay `pub` and keep re-exporting
      through `chrome/mod.rs` unchanged.
- [x] **Phase 5: `cf_sidebar.rs` and `cf_editor.rs`.** The largest domain and the only one that
      splits in two (arch §7.3). Done as one phase because the sidebar/editor seam is only
      verifiable with both sides moved.
- [x] **Phase 6: `formatting.rs`.** Style toggles, merge, fill, text colour, number format,
      font, borders, their popovers and free helpers.
- [x] **Phase 7: `editing.rs`.** Data row, in-cell editor, quick-edit, autocomplete, signature
      hints. Largest test payload (~1,930 lines across eight banner sections).
- [x] **Phase 8: `shell.rs` + project close-out.** Move the residue (`Render`/`Focusable`,
      `render_action_row`, overlays, `on_worker_event`, selection plumbing, degrade) out of
      `mod.rs`, leaving it the struct + `new` + shared constants. Then, once: verify every
      file is under the 2,000 production-line ceiling, run `cargo test -p freecell-app` (all
      targets) and a workspace build, run the Xvfb smoke launch, write `findings.md`, and file
      the `grid/view.rs` backlog entry (functional spec §9).

## Not run

The pixel render suite — full or subset. Nothing in `chrome/view.rs` has a baseline
(functional spec §7). If a phase finds itself editing `grid/` or `chart/` render code, that is
evidence the move was not pure: stop rather than regenerate a baseline.
