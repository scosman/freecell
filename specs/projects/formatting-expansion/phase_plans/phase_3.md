---
status: complete
---

# Phase 3: Part 2 — Border UI (pen popover)

## Overview

Redesign the borders popover from the Phase-2 8-button apply-and-close preset grid into
the **pen model** (`functional_spec.md §2`, `ui_design.md §2`, `architecture.md §6`):
transient view state (a selected *target* + a *pen* = line style + color) that resets on
every open, three stacked regions (target icons / line-style gallery / color), and
handlers that **paint the pen onto just the picked target's edges and keep the popover
open**. The Phase-2 engine foundation (`SetBorders { line, color }`, `BorderLine`,
patterned edge rendering) is already committed; this phase is UI-only in
`freecell-app/src/chrome/view.rs` plus one grid render case.

The render harness renders the **grid only, not the chrome** (verified: no `ChromeView`
reference under `app/render-tests/src`), so the popover itself has **no pixel coverage** —
its state machine is covered by chrome `#[cfg(test)]` unit tests, and a pen-applied border
is covered by one grid render case (resolved `BorderSpec`).

## Steps

1. **View fields** (`ChromeView`, near `borders_open`): add the transient pen state
   - `border_target: Option<BorderPreset>` (None on open)
   - `border_line: BorderLine` (pen style, default `ThinSolid`)
   - `border_color: Rgb` (pen color, default black `Rgb::new(0,0,0)`)
   - `border_color_picker: Entity<ColorPickerState>` (reuse the fill/text pattern)
   Initialize all four in `new()`; build `border_color_picker` with
   `cx.new(|cx| ColorPickerState::new(window, cx))` and subscribe with a new
   `on_border_color_picker_event` (mirrors `on_color_picker_event`).

2. **`toggle_borders_popover` reset-on-open**: when the toggle transitions to open, reset
   `border_target = None`, `border_line = BorderLine::ThinSolid`,
   `border_color = Rgb::new(0,0,0)`. (Reopen resets target + pen — spec §2.1.)

3. **Handlers** (replace `apply_borders`):
   - `send_border_paint(preset, window, cx) -> bool` (private): degraded-guard +
     `commit_pending_edit` (same rule as the other action-row controls), then
     `client.send(Command::SetBorders { sheet, range, preset, line: self.border_line,
     color: Some(self.border_color) })`. Returns whether it dispatched. (For `None` the
     line/color are irrelevant — the engine clears — but harmless to pass.)
   - `select_border_target(preset, window, cx)` (pub): call `send_border_paint`; if it
     dispatched, set `border_target = (preset != None).then_some(preset)` and `notify`.
     **Popover stays open** (no `borders_open = false`). `None` clears + leaves no target.
   - `set_border_line(line, window, cx)` (pub): `self.border_line = line`; if a target is
     selected, `send_border_paint(target)`; `notify`. No-target → pen only, no send (MVP;
     P2 restyle-all deferred, GAPS F2).
   - `set_border_color(rgb, window, cx)` (pub): symmetric to `set_border_line`.
   - `on_border_color_picker_event`: on `Change(Some(hsla))` → `set_border_color`.

4. **Icon element** `border_target_icon(preset) -> AnyElement` (free fn, like
   `hsla_to_rgb`): a ~22px square drawing a 2×2 mini-grid from absolutely-positioned `div`
   rectangles. Six line segments (outer top/bottom/left/right + inner mid-H/mid-V); each is
   **dark 2px** if the preset affects it else **light-grey 1px** (context). Per-preset dark
   masks per `ui_design.md §2.2`: All=all 6, Inner=inner cross, Outer=perimeter, None=none,
   Top/Bottom/Left/Right=that one outer edge. Grey lines painted first, dark on top so the
   dark wins at crossings.

5. **Line-style preview element** `border_line_preview(line) -> AnyElement` (free fn): a
   short horizontal sample of the real line — solid bar 1/2/3px (thin/med/thick), a row of
   dashes (dashed), two 1px bars (double), all dark, vertically centered in a ~34px box.

6. **Rebuild `render_borders_popover`** (`ui_design.md §2.1`): card with
   - muted section label "Which lines"
   - Region A: two rows of icon `Button`s — row1 All/Inner/Outer/None, row2
     Top/Bottom/Left/Right — each `.ghost().small()`, `.child(border_target_icon(preset))`,
     `.tooltip(name)`, `.selected(self.border_target == Some(preset))`, `.debug_selector`
     keeping the existing `border-all` … `border-right` ids, `.on_click` →
     `select_border_target`.
   - a thin `HAIRLINE` divider
   - muted label "Line"; Region B: 5 gallery `Button`s
     (`.child(border_line_preview(line))`, `.tooltip`, `.selected(self.border_line ==
     line)`, ids `border-line-thin|medium|thick|dashed|double`) → `set_border_line`.
   - muted label "Color"; Region C: the `FILL_PALETTE` 5×2 swatch grid (verbatim from the
     fill popover) with a **selected ring** when `swatch.rgb == self.border_color`, +
     "Custom…" `ColorPicker::new(&self.border_color_picker).small()` inline. Swatch
     `on_mouse_down` → `set_border_color`.
   Keep the full-window backdrop (click-away → `borders_open = false; notify`) and the
   `.occlude()` card, anchored under `Anchor::Borders` — same shell as the other popovers.

7. **Render case** (`app/render-tests/src/cases.rs`): add `border_pen_outer_dashed_red` — a
   2×2 block whose four corner cells carry only their two **outer** edges as **dashed red
   (2px)** (interior bare), mirroring "select Outer with a dashed + red pen". Regenerate +
   eyeball this baseline; confirm no unrelated baselines move; `render_tests.sh test` green.

8. **Tests**: rewrite the chrome border tests for the pen model (see below); update the
   imports/usages that referenced the removed `apply_borders`.

## Tests

Chrome `#[cfg(test)]` (`view.rs`):
- `borders_popover_toggles` — unchanged (open/close flag).
- `borders_reopen_resets_target_and_pen` — set target + non-default pen, close, reopen →
  `border_target == None`, `border_line == ThinSolid`, `border_color == black`.
- `select_border_target_paints_and_stays_open` — select_single, open, `select_border_target(Outer)`
  → one `SetBorders { preset: Outer, line: ThinSolid, color: Some(black) }` over the
  selection **and `borders_open` still true**, `border_target == Some(Outer)`.
- `border_target_icon_click_stays_open` — via `press_popover_button("border-all")`: asserts
  the click dispatches `SetBorders { All }` and (unlike the old test) the popover stays open
  with `border_target == Some(All)`.
- `set_border_line_with_target_repaints` — select target Outer, `set_border_line(Dashed)`
  → repaints `SetBorders { Outer, line: Dashed, color: Some(black) }`; pen carries.
- `set_border_color_with_target_repaints` — after target, `set_border_color(red)` →
  `SetBorders { Outer, Dashed?, color: Some(red) }`.
- `pen_carries_across_target_switch` — set Dashed+red on Outer, then
  `select_border_target(Top)` → `SetBorders { Top, line: Dashed, color: Some(red) }`.
- `set_border_line_without_target_updates_pen_only` — no target: `set_border_line(Thick)`
  dispatches **nothing**; `border_line == Thick` (next paint uses it).
- `border_none_clears_and_deselects` — target Outer selected, `select_border_target(None)`
  → `SetBorders { None }` and `border_target == None` (popover stays open).
- `borders_disabled_in_degraded_mode` — degraded force-closes; `select_border_target`
  dispatches nothing.

Render (`cases.rs`): `border_pen_outer_dashed_red` baseline (eyeballed).
