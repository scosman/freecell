# Chart edit-panel range picking under click-away-close

**Status: Future (deferred from charts post-v1 Batch 2, item 12, 2026-07-11).**

## Goal

Restore a smooth "point at the data block in the grid" flow for setting/changing a chart's data
range from the edit panel, now that the panel closes on click-away.

## Current behavior (the rough edge)

Batch 2 item 12 made the chart edit panel **close on click-away** (a click on a cell / header /
empty grid, routed through `ChromeView::on_selection_changed` — reversing P19's deliberate
no-backdrop). That is the behavior the user asked for. Its side effect on the **Data range**
section (`render_chart_range_body` → `apply_chart_range_from_selection`, authored charts only):

- The old flow was **open panel → drag a range in the grid → "Use selection (A1:B10)"**. Under
  click-away-close, the drag's first mouse-down changes the grid selection, which closes the panel
  *before* the user can click "Use selection". So that specific order no longer works.
- **A workable order survives:** select the range **first**, *then* click the chart. Clicking a
  chart emits `GridEvent::ChartSelected` (not `SelectionChanged`), which preserves the current grid
  selection and opens the panel with "Use selection (A1:B10)" already reflecting it — one click sets
  the range.
- **The remaining rough edge** is a **freshly-inserted** chart whose panel auto-opens (the
  insert→shape flow): there's no pre-made selection to "Use", and dragging one closes the panel.

Note: Batch 3's **item 8** (default the data range to the current selection at chart *creation*)
already mitigates the most common freshly-inserted case — insert with the data pre-selected and the
new chart is bound immediately, no panel range-pick needed.

## Sketch (options — pick when built)

- **A "pick range" mode:** a button in the Data-range section that temporarily **suspends
  click-away** (and visually arms a range-picker), lets the user drag a block in the grid, then
  commits it as the range and disarms — the panel stays open throughout. Cleanest UX; needs a
  panel-state flag consulted by `on_selection_changed` so a click during pick-mode feeds the range
  instead of closing.
- **An in-panel hint:** document the **select-first** order right in the Data-range section
  ("select the data, then click the chart") — near-zero code, just guidance.
- Combine: ship the hint now (trivial) and the pick-range mode later if the hint proves
  insufficient.

## Why deferred

Tracking only — not on Batch 2's path (the 5 edit-panel fixes). The primary create-time case is
covered by Batch 3 item 8; this note captures the residual edit-time range-repick ergonomics so the
click-away decision doesn't silently degrade the "Use selection" flow.
