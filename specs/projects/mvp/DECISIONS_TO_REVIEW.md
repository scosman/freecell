# Decisions log — MVP planning

## Ratified in planning Round 1 (human product calls, 2026-07-02)

These are decided — no longer open for implementation-time relitigating:

1. **Formula-bar editing only**; no in-cell editing; typing with grid focus does not
   start an edit.
2. **No cell clipboard at all in MVP** (internal or Excel-interop).
3. **Dynamic arrays: accept absence for v1** (FILTER/SORT/UNIQUE surface engine
   errors).
4. **Save fidelity: silently strip in MVP, no warning dialog.** Warn-and-strip UX is
   the post-MVP `projects/xlsx-preservation.md` project (PROJECTS.md updated).
5. **macOS-only MVP.**
6. **Last window closes → app quits** (no Welcome reappearance).
7. **Undo does not clear the dirty flag**; only save clears it.
8. **Evaluating spinner: top-right of the action row, appears only after an eval has
   been in flight > 250  ms, hides on completion.**
9. **Formulas explicitly in scope**: full IronCalc function set, cross-sheet refs,
   error values, recalc on every commit (functional_spec §3.4).
10. Proposed additions accepted: keyboard navigation set, Delete-clears, undo/redo,
    sheet delete, unsaved-changes prompts, loading states + open-error dialogs,
    formula input cap (64 depth / 8192 chars).

## Ratified in UI round (human calls, 2026-07-02)

11. **Bundled Inter** as the grid/cell font (registered via `add_fonts`; also
    stabilizes render baselines). Chrome keeps the gpui-component theme font.
12. **Selection/accent = gpui-component primary blue** everywhere.
13. **Fill palette = the 10 Office-theme colors** (Background 1 `#FFFFFF`, Text 1
    `#000000`, Background 2 `#E7E6E6`, Text 2 `#44546A`, Accent 1 `#4472C4`, Accent 2
    `#ED7D31`, Accent 3 `#A5A5A5`, Accent 4 `#FFC000`, Accent 5 `#5B9BD5`, Accent 6
    `#70AD47`) + **No fill** + **Custom…** via gpui-component's ColorPicker.
14. **Light theme only** for MVP.

## Planning-agent judgment calls (still open to review; will be re-surfaced in the
## architecture round)

- `Publication` includes each cell's `raw_content` so the formula bar never blocks on
  an in-flight eval (payload cost accepted). (`architecture.md §2`)
- Undo/redo cache sync via touch-set re-read instead of round-3 A's inverse-op mirror
  (simplest correct; agreement contract still enforced; inverse-mirror activates with
  structural edits P2). (`components/style_cache.md`)
- Cache read model lives in `freecell-core` (engine-free) for track parallelism.
  (`architecture.md §3`)
- Perf gates kept at the Phase-1 bar: frame p99 ≤ 8.33 ms / worst ≤ 16.67 ms /
  cell-load p99 < 2 ms. (`functional_spec.md §7`)
- Engine locale/tz defaults: `en` / system tz. (`components/engine_worker.md`)
- Custom-drawn scrollbars (two rects + drag). (`components/grid.md`)
- Small utility deps: `arc_swap`, `parking_lot`, `tempfile`.
- Render-suite diff thresholds start at round-3 C's validated 12/255 + 0.5%, re-tuned
  after first real baselines.
- Finder open-with and traffic-light-close interception are best-effort against the
  pinned GPUI rev's API surface.

## To be filled in during implementation (placeholders)

- gpui-component pinned SHA + Rust toolchain version (Phase 1).
- Exact fill-palette hexes (Phase 9 in the draft plan).
- Perceptual-diff thresholds after first real baselines.

---
*Entries below this line are appended by implementation phases.*
