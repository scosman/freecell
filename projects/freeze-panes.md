# Freeze Panes

**Status:** Future — deferred from `feature-gaps-7-11` planning (2026-07-12); the engine
side is ready, the grid-render side is the real work.

## Goal

Right-click a row or column header → **Freeze** freezes that row/column and everything
above/left of it, so the frozen band stays pinned while the rest of the sheet scrolls. If
the right-clicked row/col is already the current freeze boundary, the item reads
**Unfreeze**. (Excel's Freeze Panes, header-driven rather than the "freeze at active
cell" variant.)

## Engine side — READY, no IronCalc fork change

IronCalc already models frozen panes end-to-end (confirmed by the round-3 API audit,
`experiments/round-3/B-api-audit/findings.md:136-139`):

- `UserModel::set_frozen_rows_count` / `set_frozen_columns_count` (+ getters) —
  `ironcalc_base` `common.rs:1143/1157/1126/1135`, **undoable**.
- `Worksheet.frozen_rows` / `frozen_columns` (`types.rs:115/116`).
- xlsx `<pane>` import **and** export — frozen state round-trips through open→save.

So the only engine-layer work is thin wiring: a `Command::SetFrozen { sheet, rows, cols }`
in `freecell-engine/src/worker/protocol.rs` + `document.rs` wrappers over the IronCalc
calls, and surfacing the current counts through the publication/`SheetCache` so the grid
can read them per frame.

## Why it's deferred — the grid is a custom viewport

Nothing in the render layer knows about frozen regions today (`GAPS.md:141`: *"Viewport-split
rendering + scroll clamping in the custom grid — real complexity"*). The grid currently has:

- a **single** content rect + a **single** scroll pair per sheet
  (`scroll: HashMap<SheetId,(f64,f64)>`, `grid/view.rs:194`; clamp math in
  `grid/layout.rs`);
- `resolve_frame` / `build_grid_layers` that compute one visible row/col range and paint
  one content layer (`grid/view.rs:753`, `2071`).

Freeze requires splitting the viewport into up to four quadrants (frozen corner, frozen
top band, frozen left band, scrolling body), each with its own visible-range + offset math,
and clamping scroll so the frozen bands never move. That touches the grid's most
performance-sensitive and most-tested code (the render hot path + `layout.rs` geometry +
the render-baseline suite). It's self-contained but sizeable, and it interacts with the
header hit-testing and the resize/selection drag code.

## Sketch (when picked up)

1. **Engine wiring:** `Command::SetFrozen` + `document.rs` wrappers + getters; publish
   `frozen_rows`/`frozen_columns` into the read model (`SheetCache`/publication).
2. **Header menu:** add **Freeze / Unfreeze** items to the existing `header_menu_elements`
   (`grid/view.rs:2592`) — emit a new `GridEvent::SetFrozen{…}`; label flips to Unfreeze
   when the clicked run's boundary equals the current freeze count.
3. **Grid render:** split `resolve_frame`/`build_grid_layers` into frozen + scrolling
   quadrants; per-quadrant visible-range + offset; clamp scroll (`layout.rs`) so frozen
   bands are pinned; draw the freeze divider line(s).
4. **Interactions:** header/cell hit-testing across quadrants; scroll-to-reveal must not
   scroll a target under a frozen band; resize + selection drags across the boundary.
5. **Render suite:** new baseline cases for a frozen-row sheet, frozen-col sheet, both,
   and scroll-offset states.

**Size:** L (grid viewport rework). **Risk:** render-hot-path regressions; baseline churn;
edge cases in hit-testing/scroll-reveal near the freeze boundary. Engine risk: low
(IronCalc API exists + round-trips).
