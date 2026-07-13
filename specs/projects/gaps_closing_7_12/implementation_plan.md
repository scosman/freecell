---
status: complete
---

# Implementation Plan: gaps_closing_7_12

Eight independent phases (see `functional_spec.md` + `architecture.md` for detail). Ordered
so dependencies land first (paste-values before the context menu that lists it) and the
sole pixel-suite phase is last. Each phase: build crate-scoped, `cargo fmt --all --check`,
unit/gpui tests + (where noted) a render subset, commit, then move on. Full render suite +
CI `render` gate happen **once**, in Phase 8.

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
- [ ] **Phase 3 — ⌘+arrow → edge-of-data.** Pure Excel edge algorithm in `freecell-core`,
      resolved worker-side (D4.1 Option A). Route only `JumpEdge`/`ExtendEdge` to the async
      query; other motions unchanged. Exhaustive algorithm unit tests.
- [ ] **Phase 4 — Paste values (⌘⇧V).** Bind the reserved `Shift+V`; paste the internal
      clipboard's computed-value TSV at target (values-only, one undo step; D5.1/D5.2).
      Exposes `GridEvent::PasteValues` for Phase 5. Unit tests incl. the `"=x"` edge case.
- [ ] **Phase 5 — Cell-area right-click context menu.** Clone `chart_menu_elements` →
      `cell_menu_elements`, open from the cell-body arm of `handle_right_mouse_down`
      (select-move-if-outside); items reuse existing Copy/Cut/Paste/Clear/Insert/Delete +
      Paste-Values (Phase 4). Decides D2.1. gpui tests.
- [ ] **Phase 6 — Number-format preset breadth.** Grouped preset model + restructured
      `render_num_fmt_popover`; optional thousands-separator toggle button; extended
      reverse map. UI-only (engine renders codes). Decides D6.1/D6.2. Unit + gpui tests.
- [ ] **Phase 7 — Autofit column width.** Double-click the column resize hotspot →
      measure widest published cell (`measure_incell_text_width`) → reuse
      `SetColumnWidths`. Decides D7.1/D7.2/D7.3. Width-calc unit test + a render **subset**
      check.
- [ ] **Phase 8 — Render-fidelity polish pair (dedicated render phase).** 8a: fill skips
      interior gridlines vs. same-fill neighbors (`cell_element`). 8b: **investigate the
      `header_full_row_selected` baseline first** — the source looks symmetric, so this may
      be a baseline/ordering issue, not a code fix. Then: regenerate + **eyeball** affected
      baselines, run the **full** render suite (watchdog), commit baselines, and dispatch
      the CI `render` gate to green.
