# Decisions to Review — MVP spec (made autonomously during planning)

Judgment calls made while speccing without human input, per the project's autonomy
instructions. None block implementation; review at leisure and amend specs if any call
is wrong. Implementation phases will append their own entries below the line.

## Product-level calls

1. **Dynamic arrays: accept absence for v1.** Round-3 explicitly required a product
   decision (accept / build spill / upstream). Chose **accept**: FILTER/SORT/UNIQUE
   surface whatever error the engine returns. (`functional_spec.md §8`)
2. **`.xlsx` fidelity policy: warn-and-strip.** `projects/xlsx-preservation.md` flags
   this as a pre-build decision. Chose the cheapest v1: one-time-per-document warning
   dialog on first save over a file opened from disk; Save Anyway / Cancel. No
   unknown-part pass-through, no owned writer. (`functional_spec.md §5.2`)
3. **No clipboard at all in MVP** — not even internal single-cell copy/paste (Excel
   interop was already post-MVP by product call; internal-only clipboard felt like a
   half-feature not in the overview's list). Data-row text field keeps normal
   NSText-style editing. (`functional_spec.md §8`)
4. **No in-cell editing; typing a character does not start an edit.** The overview
   only specifies data-row editing; the in-cell editor drags in IME/editor
   architecture that's explicitly post-MVP. All editing goes through the formula bar.
   (`ui_design.md §6`)
5. **Sheet delete included** (context-menu, confirm when non-empty, never the last
   sheet). The overview listed only add/rename; add-without-delete felt broken for a
   "workable" MVP. (`functional_spec.md §3.6`)
6. **Structural edits (insert/delete rows/cols) fully out of MVP UI** — the validated
   round-3 A shift design is deliberately *not built* yet; MVP cache sync only needs
   build + style-edit mirror + undo re-read. (`architecture.md §11`)
7. **macOS-only MVP** stated as explicit scope (implied everywhere in the de-risking
   record, now written down). (`functional_spec.md §1`)
8. **Welcome window reappears when the last document window closes**; app keeps
   running per macOS convention. (`functional_spec.md §2.3`)
9. **Undo does not clear the dirty flag** (op-counter monotonic; undo counts as an
   op). Simpler than exact save-point matching. (`architecture.md §2`)
10. **Fill palette = fixed ~10 swatches + No fill**, exact hexes chosen at
    implementation from a standard set. No custom color picker. (`ui_design.md §3.1`)

## Technical calls

11. **Formula input cap = depth ≤ 64, length ≤ 8192** (Excel-compatible bounds; the
    round-3 synthesis's recommended numbers, chosen over D's alternative depth-128
    suggestion). (`functional_spec.md §3.3`)
12. **`Publication` includes each cell's `raw_content`** alongside the display string
    so the formula bar is instant and never blocks on an in-flight eval; payload cost
    accepted (~overscanned-viewport strings). (`architecture.md §2`)
13. **Undo/redo cache sync via touch-set re-read** (worker re-reads styles for cells
    recorded against the history entry) instead of round-3 A's inverse-op mirror —
    simplest correct path; A's agreement-contract tests still enforced. The
    inverse-mirror + sub-ms shift design activates with structural edits (P2).
    (`components/style_cache.md`)
14. **Cache read model lives in `freecell-core`** (engine-free) so grid/render-test
    tracks build in parallel with the engine track. (`architecture.md §3`)
15. **Perf gates kept at the Phase-1 bar**: 120 fps (frame p99 ≤ 8.33 ms, worst
    ≤ 16.67 ms), cell-load p99 < 2 ms — the gates the POC already passed with ~4×
    margin. (`functional_spec.md §7`)
16. **Locale/timezone for the engine: `en` / system tz** at open/new. No locale UI.
    (`components/engine_worker.md`)
17. **Custom-drawn scrollbars** (two rects + drag) rather than adapting
    gpui-component's to external virtual extents. (`components/grid.md`)
18. **`arc_swap` + `parking_lot` + `tempfile`** added as small utility deps.
    (`components/engine_worker.md`)
19. **Render-suite thresholds start at round-3 C's validated values** (12/255
    tolerance, 0.5% fraction), re-tuned once against real-grid baselines.
    (`components/render_test_harness.md`)
20. **Finder open-with and traffic-light-close interception are best-effort** against
    the pinned GPUI rev's API surface; if the rev lacks the hooks, ship without and
    record here. (`components/app_shell.md`)

## To be filled in during implementation (placeholders)

- gpui-component pinned SHA + Rust toolchain version (Phase 1).
- Exact fill-palette hexes (Phase 9).
- Perceptual-diff thresholds after first real baselines (Phase 7).

---
*Entries below this line are appended by implementation phases.*
