---
status: complete
---

# Implementation Plan: Formatting Expansion

Phased build order. Each phase is a coherent, independently-reviewable unit and ends
with committed code + refreshed/eyeballed render baselines. See `functional_spec.md`,
`ui_design.md`, and `architecture.md` for detail. **No IronCalc fork changes are needed.**

## Phases

- [x] **Phase 1 — Part 1: Text formatting (strikethrough, wrap, vertical align).**
  `RenderStyle` fields (`strikethrough`, `wrap`, `v_align`) + `VAlign` enum; protocol
  (`StyleAttr::Strikethrough`, `StyleAttr::WrapText`, `StylePath::AlignVertical`,
  `FontFlag::Strike`); document (`Strike` flag, `wrap_flag` reader); worker `apply_style`
  toggles + vertical-align set; cache resolver (`render_style_from` + `v_align_of`);
  toolbar buttons (strikethrough + wrap in the B/I/U group, vertical-align group after
  horizontal align); grid render (strike line, wrapped multi-line clipped to row height,
  vertical text placement). Tests: engine round-trip, worker, cache, chrome; render cases
  + eyeballed baselines.

- [ ] **Phase 2 — Part 2: Border engine + rendering foundation.**
  `Edge.pattern` + `LinePattern` enum; protocol (`SetBorders` gains `line: BorderLine` +
  `color`; `BorderLine` enum + `style_tag()`); document (`set_borders` parameterized —
  drop the hardcoded thin-black item); worker `SetBorders` dispatch; cache
  `BorderStyle`→(weight, pattern) mapping with solid fallback for deferred styles; grid
  render dashed + double edge patterns. Tests: engine round-trip (pen style/color written;
  Outer preserves interior), worker, cache; render cases for dashed/double + eyeballed
  baselines.

- [ ] **Phase 3 — Part 2: Border UI (pen popover).**
  Borders popover redesign: transient `border_target` + pen (`border_line`,
  `border_color`) view state reset on open; the parameterized 2×2 `border_target_icon`
  component; the line-style gallery previews; reused color picker (`border_color_picker`
  + `FILL_PALETTE`); pen-model handlers (`select_border_target`, `set_border_line`,
  `set_border_color`) with the stays-open / repaint-selected-target behavior; replace the
  old apply-and-close path. Tests: chrome (popover stays open on target click, pen carries
  over, reopen resets, None clears+deselects); render cases for popover/icons/gallery +
  eyeballed baselines.

## Cross-cutting: render gate

Every phase moves pixels. Each phase: regenerate + **eyeball** the affected baselines with
`app/render-tests/scripts/render_tests.sh generate`, commit them with the change, and run
`render_tests.sh test` green locally. Before the project is considered done, **dispatch the
CI `render` gate on the branch and confirm it passes** (per `CLAUDE.md` render policy).
