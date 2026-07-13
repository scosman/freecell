---
status: complete
---

# Implementation Plan: gaps_closing_7_12

Nine independent phases (see `functional_spec.md` + `architecture.md` for detail). Ordered
so dependencies land first (paste-values before the context menu that lists it) and the
sole pixel-suite phase (Phase 8) lands before the final chrome phase. Each phase: build
crate-scoped, `cargo fmt --all --check`, unit/gpui tests + (where noted) a render subset,
commit, then move on. Full render suite + CI `render` gate happen **once**, in Phase 8.
Phase 9 (owner feedback, added 2026-07-13) is chrome-only → no pixel suite.

Unresolved decisions carry a **recommended default** (architecture.md "Consolidated
decisions"); proceed on the default unless the owner overrides at phase start.

## Phases

- [x] **Phase 1 — Status bar with selection stats.** Worker aggregate
      (`Command::SelectionStats` → `document.rs::selection_stats`) + stats readout on the
      **right of the tab bar** (`render_tab_bar` refactor), Sum·Avg·Count with a click
      toggle for Min·Max. Decides D1.1. Non-pixel (tab-bar chrome) → gpui tests + smoke.
- [x] **Phase 2 — Fill down / right (⌘D / ⌘R).** Keyboard commands only (drag handle
      deferred); new command → fork `auto_fill_rows/columns` (copy-fill from the top
      row/left col). Decides D3.1. **Check out the fork** (`add_repo scosman/ironcalc`) to
      bind the exact `auto_fill_*` signature. Engine tests.
- [x] **Phase 3 — ⌘+arrow → edge-of-data.** Pure Excel edge algorithm in `freecell-core`,
      resolved worker-side (D4.1 Option A). Route only `JumpEdge`/`ExtendEdge` to the async
      query; other motions unchanged. Exhaustive algorithm unit tests.
- [x] **Phase 4 — Paste values (⌘⇧V).** Bind the reserved `Shift+V`; paste the internal
      clipboard's computed-value TSV at target (values-only, one undo step; D5.1/D5.2).
      Exposes `GridEvent::PasteValues` for Phase 5. Unit tests incl. the `"=x"` edge case.
- [x] **Phase 5 — Cell-area right-click context menu.** Clone `chart_menu_elements` →
      `cell_menu_elements`, open from the cell-body arm of `handle_right_mouse_down`
      (select-move-if-outside); items reuse existing Copy/Cut/Paste/Clear/Insert/Delete +
      Paste-Values (Phase 4). Decides D2.1. gpui tests.
- [x] **Phase 6 — Number-format preset breadth.** Grouped preset model + restructured
      `render_num_fmt_popover`; optional thousands-separator toggle button; extended
      reverse map. UI-only (engine renders codes). Decides D6.1/D6.2. Unit + gpui tests.
- [x] **Phase 7 — Autofit column width.** Double-click the column resize hotspot →
      measure widest published cell (`measure_incell_text_width`) → reuse
      `SetColumnWidths`. Decides D7.1/D7.2/D7.3. Width-calc unit test + a render **subset**
      check.
- [ ] **Phase 8 — Render-fidelity polish pair (dedicated render phase).** 8a: fill skips
      interior gridlines vs. same-fill neighbors (`cell_element`). 8b: **investigate the
      `header_full_row_selected` baseline first** — the source looks symmetric, so this may
      be a baseline/ordering issue, not a code fix. Then: regenerate + **eyeball** affected
      baselines, run the **full** render suite (watchdog), commit baselines, and dispatch
      the CI `render` gate to green.
- [ ] **Phase 9 — Sum-section refinements + horizontal scroller (owner feedback).** 9A:
      adaptive stats decimals (`format_stat_value`: by |value|, ≥100→2/≥10→3/≥1→4/<1→5 dp;
      D9.1), vertical-center the readout, add a leading divider. 9B: a new reusable
      **horizontal-scroller** control (`chrome/h_scroller.rs`) — unchanged when content fits;
      on overflow adds a static divider + lucide `chevron-left/right` buttons (action-bar
      style, no visible scrollbar) that animate-scroll 0.8× viewport width (D9.2/D9.3). Use it
      in the **action bar** and the **sheet-tab strip** (stats group static to its right → the
      always-visible fix, 9A.4). 9C: note in `CLAUDE.md` that we use lucide for icons.
      Chrome-only → gpui view tests + `VisualTestContext` paint tests + Xvfb smoke; **no pixel
      suite.**
