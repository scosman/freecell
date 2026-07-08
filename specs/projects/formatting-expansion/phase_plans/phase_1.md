---
status: complete
---

# Phase 1: Part 1 — Text formatting (strikethrough, wrap, vertical align)

## Overview

Adds three text-formatting controls that ride the existing formatting seam end-to-end
(`freecell-core` render model → `freecell-engine` protocol/document/worker/cache →
`freecell-app` chrome toolbar + grid render), with no IronCalc fork changes
(`architecture.md §0`):

- **Strikethrough** — a toggle after Underline in the B/I/U group (`font.strike`).
- **Wrap text** — a single toggle after Strikethrough (`alignment.wrap_text`); text wraps to
  the column width, clipped to the row height (auto-grow deferred → GAPS F1).
- **Vertical alignment** — a new Top/Center/Bottom group after the horizontal-align group
  (`alignment.vertical`), radio-style (plain set, no re-press-to-clear).

## Steps

1. **`freecell-core/src/style.rs`** — add `VAlign { Top, Center, Bottom }` enum; add
   `strikethrough: bool`, `wrap: bool`, `v_align: Option<VAlign>` to `RenderStyle`; update the
   module/struct docs; extend the default-is-plain unit test. Export `VAlign` from `lib.rs`.
2. **`freecell-engine/src/worker/protocol.rs`** — add `StyleAttr::Strikethrough` +
   `StyleAttr::WrapText` (toggles); add `StylePath::AlignVertical` (`as_str()` →
   `"alignment.vertical"`).
3. **`freecell-engine/src/document.rs`** — add `FontFlag::Strike` → `font.strike` (read via
   `font_flag`, write via `set_font_flag` — zero new logic); add `wrap_flag(sheet, cell)`
   reading `alignment.wrap_text`.
4. **`freecell-engine/src/worker/run.rs`** — extend `apply_style`: `Strikethrough` reuses the
   font-flag toggle via `FontFlag::Strike`; `WrapText` toggles `wrap_flag` and writes
   `alignment.wrap_text` via `update_style_path`. Factor the "any cell lacks it" scan into a
   shared `any_cell_lacks` helper. `AlignVertical` needs no new arm (generic `SetStylePath`).
5. **`freecell-engine/src/cache.rs`** — `render_style_from` sets `strikethrough`/`wrap`;
   add `v_align_of` (Top/Center/Bottom → `Some`, Justify/Distributed/no-record → `None`) —
   mirrors `h_align_of`. Documented Excel-faithful reading of IronCalc's `Bottom` default.
6. **`freecell-app/src/chrome/view.rs`** — readers `strikethrough_active`/`wrap_active`/
   `valign_active`; handler `apply_valign` (plain set); `toggle`-closure buttons for S + wrap
   in the B/I/U group; a `valign_btn` closure + Top/Center/Bottom group after horizontal align.
   Bump `ACTION_ROW_MIN_W` (896 → 1080) for the wider row; widen the test window (900 → 1200).
7. **`freecell-app/src/grid/view.rs`** — in `cell_element`: `line_through()` when
   `strikethrough`; vertical placement via the flex container — base default `items_end`
   (**bottom**, decision C — Excel-faithful, so unset + `style: None` cells render bottom), with
   `Some(Top/Center/Bottom)` → `items_start/center/end`; when `wrap`, attach the text inside a
   full-width `whitespace_normal` content box (text-align = h_align) so gpui has a definite width
   to wrap into, clipped by the cell's `overflow_hidden`.
8. **`render-tests`** — `Scene::strikethrough`/`wrap` (real `SetStyleAttr` commands) +
   `Scene::v_align` (cache injection, like `align`); 7 new cases + registrations in
   `render_suite.rs`. Decision C moves text center → bottom for essentially every case, so
   **all** baselines are regenerated + eyeballed, not just the new ones.

## Tests

- **core `style.rs`**: `render_style_default_is_plain` extended for the new fields.
- **engine `cache.rs`**: `render_style_from_maps_each_attribute` extended — `font.strike`,
  `alignment.wrap_text`, `alignment.vertical` (top/center/bottom → `Some`, justify → `None`,
  default → `None`).
- **engine `document.rs`**: `roundtrip_styles_preserved` extended — strike/wrap/vertical survive
  save→reopen (fixture `styles()` gains D1/D2/D3 cells).
- **worker `run.rs`**: `strikethrough_toggle_sets_all_then_clears`,
  `wrap_toggle_sets_all_then_clears` (clone of the bold toggle), and
  `set_style_path_vertical_align_applies` (clone of the align test, with cache agreement).
- **chrome `view.rs`**: `strikethrough_and_wrap_toggles_send_setstyleattr`,
  `strikethrough_and_wrap_reflect_active_style`, `vertical_alignment_sets_and_reflects`.
- **render cases (manual gate)**: `cell_strikethrough`, `cell_strikethrough_underline`,
  `cell_wrap_multiline_clipped`, `cell_valign_top/middle/bottom`, `cell_wrap_valign_bottom` —
  baselines regenerated + eyeballed; full `render_tests.sh test` green (no existing baseline
  disturbed).
