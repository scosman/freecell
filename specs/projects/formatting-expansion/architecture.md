---
status: complete
---

# Architecture: Formatting Expansion

Technical design for Part 1 (strikethrough, wrap, vertical align) and Part 2 (border
line style + color with the pen modality). All decisions below are resolved — the
coding agent executes, it does not design.

**Scope of the doc:** single `architecture.md` (no separate component designs). The work
touches four layers along the existing seam: `freecell-core` (render model) →
`freecell-engine` (protocol / document / worker / cache) → `freecell-app` (chrome
toolbar + grid render). No IronCalc type crosses the `freecell-engine` boundary.

## 0. Fork verification (done at spec time — no open unknowns)

Verified directly against the pinned fork commit `48b0b235` (= tip of
`scosman/ironcalc@freecell-fixes`, matches `app/Cargo.lock`):

- **`update_range_style` already dispatches all three new Part-1 paths**
  (`base/src/user_model/common.rs` `update_style`): `font.strike` (bool),
  `alignment.vertical` (`top`/`center`/`bottom`/justify/distributed),
  `alignment.wrap_text` (bool). → **Part 1 needs no engine/fork changes**; it rides the
  same generic pass-through as `alignment.horizontal`/`font.color` today.
- **`BorderStyle` is `#[serde(rename_all = "lowercase")]`** (`base/src/types.rs:723`):
  tags `thin` / `medium` / `thick` / `double` / `mediumdashed` (also dotted/dash-dot
  families, deferred). These are the strings the border-item JSON carries.
- **`set_area_with_border` → `update_single_cell_border`** (`base/src/user_model/border.rs`)
  clones the cell's **existing** style and overwrites **only** the edges the `BorderType`
  implies (All=4 edges, Outer=perimeter, Inner=interior, Top/Bottom/Left/Right=one edge,
  None=clear all). Non-targeted edges are preserved. `BorderType` also has **CenterH /
  CenterV** (inner-horizontal/vertical) — unused now, cheap future targets.

→ **The pen modality is a UI-state problem, not an engine problem.** The engine already
does non-destructive per-type application; we only (a) parameterize the border item with
style+color and (b) hold the transient target/pen state in the view.

---

## 1. `freecell-core` — render model changes

`app/crates/freecell-core/src/style.rs`:

- Add fields to `RenderStyle` (parallels `bold`/`italic`/`underline`/`h_align`):
  ```rust
  pub strikethrough: bool,
  pub wrap: bool,
  pub v_align: Option<VAlign>,
  ```
- Add the vertical-alignment enum (parallel to the existing horizontal `Align`):
  ```rust
  pub enum VAlign { Top, Center, Bottom }
  ```
- Update the module doc (currently lists strikethrough/wrap/vertical-align as
  "intentionally absent") — they are now present.

`app/crates/freecell-core/src/border.rs`:

- Add a line-pattern discriminant to `Edge` (keeps existing `weight`, `color`):
  ```rust
  pub enum LinePattern { Solid, Dashed, Double }   // Dotted deferred (GAPS F3)
  pub struct Edge { pub weight: u8, pub color: Rgb, pub pattern: LinePattern }
  ```
  Default/`NONE` semantics unchanged; `effective_edge` (heavier-wins) unchanged — weight
  still decides the winner; the winner carries its own pattern.

## 2. `freecell-engine` — protocol changes

`app/crates/freecell-engine/src/worker/protocol.rs` (engine-free enums only):

- **Strikethrough (toggle):** extend `StyleAttr` with `Strikethrough`. It resolves the
  same way as Bold (§4). Reason it's a `StyleAttr` not a `StylePath`: it needs the
  "any cell lacks it → set all, else clear" toggle semantics.
- **Wrap (toggle):** extend `StyleAttr` with `WrapText`. Same toggle semantics; writes
  the `alignment.wrap_text` path (§4).
- **Vertical align (radio/set):** extend `StylePath` with `AlignVertical`; `as_str()` →
  `"alignment.vertical"`. Value strings `"top"|"center"|"bottom"`. No toggle — a plain
  set, exactly like `AlignHorizontal`.
- **Border pen:** extend `SetBorders` payload:
  ```rust
  SetBorders { sheet, range, preset: BorderPreset,
               line: BorderLine, color: Option<Rgb> }
  ```
  where `BorderLine` is a new engine-free enum mirroring the gallery:
  ```rust
  pub enum BorderLine { ThinSolid, MediumSolid, ThickSolid, Dashed, Double }
  ```
  `BorderPreset` (All/Inner/Outer/Top/Bottom/Left/Right/None) is unchanged and still
  maps to the `BorderType` tag via `border_type_tag()`. `color: None` ⇒ default black.

## 3. `freecell-engine` — document adapter (`document.rs`)

- **`FontFlag`** enum (`Bold|Italic|Underline` → `font.b/i/u`): add `Strike` → `font.strike`.
  `font_flag()` (read) and `set_font_flag()` (write across range) then handle
  strikethrough with zero new logic — it becomes a literal clone of the underline path.
- **Wrap read helper:** add `wrap_flag(sheet, cell) -> bool` reading
  `alignment.wrap_text` from the resolved style (mirrors `font_flag`), for the toggle
  decision. Write goes through the existing `update_style_path` with
  `("alignment.wrap_text", "true"|"false")`.
- **`set_borders` — parameterize the hardcoded item.** Current body hardcodes
  `{"style":"thin","color":"#000000"}`. New signature:
  ```rust
  fn set_borders(&mut self, sheet_idx, range, border_type: &str,
                 style_tag: &str, color_hex: &str) -> Result<(), String>
  ```
  builds `BorderArea` via `serde_json` as
  `{"item": {"style": style_tag, "color": color_hex}, "type": border_type}` and calls
  `self.model.set_area_with_border(...)`. (`BorderArea` fields are `pub(crate)` with no
  constructor at 0.7.1 — the serde-json build stays.)
- Vertical align needs no new document method — it flows through the existing
  `update_style_path`.

## 4. `freecell-engine` — worker dispatch (`worker/run.rs`)

- **`apply_style` (toggle resolver)** currently handles Bold/Italic/Underline via
  `FontFlag`. Extend:
  - `StyleAttr::Strikethrough` → same code path as underline using `FontFlag::Strike`.
  - `StyleAttr::WrapText` → toggle on `wrap_flag`: if any cell in range lacks wrap →
    set `alignment.wrap_text=true` on the whole range, else clear (`=false`). Reuses the
    "any lacks → set all else clear" scan already written for font flags, just with the
    wrap reader/writer.
- **`SetStylePath::AlignVertical`** → already generic: `update_style_path(idx, range,
  "alignment.vertical", value)`. No new arm beyond the enum plumbing.
- **`SetBorders`** dispatch: map `preset.border_type_tag()` + `line.style_tag()` +
  `color.map(to_hex).unwrap_or("#000000")` and call the new `set_borders`. Still returns
  `AppliedKind::StyleOnly` (no recompute); still expands the dirty region by one cell
  ring for the heavier-wins neighbor fix-up (`expand_by_one_cell`) — unchanged.
- `BorderLine::style_tag()` mapping (engine-free → serde tag):
  `ThinSolid→"thin"`, `MediumSolid→"medium"`, `ThickSolid→"thick"`, `Dashed→"mediumdashed"`,
  `Double→"double"`.

## 5. `freecell-engine` — cache resolver (`cache.rs`)

`render_style_from` (IronCalc `Style` → `RenderStyle`) gains:

- `strikethrough = style.font.strike`
- `wrap = style.alignment.map(|a| a.wrap_text).unwrap_or(false)`
- `v_align = style.alignment.and_then(|a| map a.vertical)` via a `v_align_of` helper
  mirroring the existing `h_align_of` (Top/Center/Bottom → `Some`; Justify/Distributed →
  `None`, i.e. treated as unset for the toolbar — out of scope). **Decision C:** IronCalc's
  `VerticalAlignment` default is `Bottom`, so a cell with any alignment record (e.g. only
  horizontal set, or `.xlsx`-loaded) resolves to `Some(Bottom)`; `None` (no alignment
  record) and `Some(Bottom)` both render bottom (§7), so the mapping is coherent — no
  visible split between "unset" and "defaulted-bottom".

Border pattern mapping — extend the existing `border_weight` region into an
`edge_from`/`border_spec_from` that also sets `Edge.pattern`:

| `BorderStyle` | weight | pattern |
|---|---|---|
| Thin | 1 | Solid |
| Medium | 2 | Solid |
| Thick | 3 | Solid |
| MediumDashed | 2 | Dashed |
| Double | 3 | Double |
| Dotted, dash-dot family, SlantDashDot | (existing weight) | **Solid** (fallback; deferred styles render solid, unchanged from today) |

This keeps files that already contain deferred styles rendering exactly as they do now.

## 6. `freecell-app` — chrome toolbar (`chrome/view.rs`)

### Part 1 buttons
- Reuse the `toggle(...)` closure for **Strikethrough** and **Wrap** — append after the
  Underline `.child(...)`, before the group divider. New readers `strikethrough_active()`
  / `wrap_active()` off `active_style` (identical to `bold_active()`); dispatch via
  `toggle_style(StyleAttr::Strikethrough)` / `toggle_style(StyleAttr::WrapText)`.
- Reuse the `align_btn(...)` closure shape for **vertical align** as a new divider group
  after the horizontal-align group. New `apply_valign(VAlign)` → `apply_style_path`
  (`StylePath::AlignVertical`, value string), and `valign_active(VAlign)` reads
  `active_style.v_align == Some(x)`.
- Glyphs per `ui_design.md` §1.2 (Unicode, finalized at baseline-eyeball; div-icon
  fallback available).

### Part 2 borders popover — pen state
- New view fields:
  ```rust
  border_target: Option<BorderPreset>,   // None on open
  border_line:   BorderLine,             // pen style, default ThinSolid
  border_color:  Rgb,                    // pen color, default black
  border_color_picker: Entity<ColorPickerState>,  // reuse pattern (like fill/text)
  ```
- `toggle_borders_popover` (open): reset `border_target=None`, `border_line=ThinSolid`,
  `border_color=black`.
- `render_borders_popover` rebuilt per `ui_design.md` §2:
  - Region A: 8 target icon buttons (new `border_target_icon(preset)` element, §7);
    `.selected(self.border_target == Some(preset))`; `.on_click` →
    `select_border_target(preset)`.
  - Region B: 5 line-style preview buttons; `.selected(self.border_line == variant)`;
    `.on_click` → `set_border_line(variant)`.
  - Region C: `FILL_PALETTE` swatches + `ColorPicker::new(&self.border_color_picker)`
    inline (verbatim reuse of the fill popover); change → `set_border_color(rgb)`.
- Handlers (no popover close except None/click-away):
  - `select_border_target(p)`: if `p == None` → send `SetBorders{preset:None,…}` and set
    `border_target=None`; else set `border_target=Some(p)` and **paint**:
    `send SetBorders{ preset:p, line:self.border_line, color:Some(self.border_color) }`.
    `borders_open` stays true. `cx.notify()`.
  - `set_border_line(l)` / `set_border_color(c)`: update the pen field; **if
    `border_target` is `Some(p)`**, re-paint that target with the new pen (send
    `SetBorders`). If `None`, update pen only (no send) — MVP; P2 (GAPS F2) upgrades this
    to restyle-all.
- `apply_borders` (old immediate-apply-and-close) is replaced by the above; the backdrop
  click-away dismiss and `.occlude()` card stay.

## 7. `freecell-app` — grid rendering (`grid/view.rs`, `grid/layout.rs`)

- **Strikethrough:** where underline is painted for a cell's text, also paint a strike
  line at text vertical-middle when `RenderStyle.strikethrough`. Same color as text.
- **Wrap (2A):** when `RenderStyle.wrap`, the cell text element wraps to the cell content
  width and renders multiple lines, **clipped** to the row height (no overflow into
  neighbors, no auto-grow — GAPS F1). When unset, current single-line behavior unchanged.
- **Vertical align (decision C):** position the cell's text block at top / middle / bottom
  of the row for `Some(Top)` / `Some(Center)` / `Some(Bottom)`. The grid's **default**
  placement (the flex container's `items_*`) is **bottom** — Excel-faithful — so `None`
  (and `style: None` mirror cells) render bottom too. This intentionally moves the unset
  baseline center → bottom for essentially every cell, so **all** render baselines are
  regenerated + eyeballed with this change (not just the vertical-align cases).
- **Border patterns:** `vertical_edge_quad` / `horizontal_edge_quad` branch on
  `Edge.pattern`:
  - `Solid` → today's single filled strip (unchanged).
  - `Dashed` → a run of short filled rects with gaps along the edge (dash length/gap
    constants; endpoints clamped to the edge span).
  - `Double` → two thin (1px) parallel strips separated by a gap, spanning the weight.
- **Border target icons (§6 Region A):** one parameterized element
  `border_target_icon(preset) -> AnyElement` drawing a ~22px 2×2 mini-grid: all gridlines
  thin light-grey; the segments the preset targets drawn solid dark. Built from `div`
  rectangles (same primitive as the edge quads). Driven by a per-preset edge mask table
  (matches `update_single_cell_border`'s per-type edges).

## 8. Undo / redo

IronCalc-native, one undoable diff-list per applied op (`set_area_with_border`,
`update_range_style`). **No coalescing of consecutive border paints** — each target
paint and each pen tweak is its own undo step, by design (functional spec §2). Part-1
toggles/sets are each one undo step, like existing formatting.

## 9. Error handling

- Style/border ops are `StyleOnly` (no eval); failures surface through the existing
  worker reply/degraded path. Degraded/read-only worker disables the toolbar group and
  force-closes the borders popover (existing behavior, extended to the new controls).
- Value strings sent to `update_range_style` are always from our own fixed enums
  (`top/center/bottom`, `true/false`, style tags) — the engine validates and errors on
  unknown, but we never send unknown values.

## 10. Testing strategy

Mirror the existing formatting tests (the exploration mapped exact analogs):

- **Engine round-trip (`document.rs` `#[cfg(test)]`):** clone `roundtrip_styles_preserved`
  to assert `font.strike`, `alignment.vertical`, `alignment.wrap_text` survive
  save→reopen. Extend `set_borders_applies_all_and_none_clears` to assert the written
  `BorderItem.style`/`.color` match the requested pen (e.g. `mediumdashed` + `#FF0000`),
  and that painting Outer over a cell with an existing interior border leaves the
  interior edge intact.
- **Worker (`worker/run.rs` `#[cfg(test)]`):** clone the Bold-toggle tests for
  Strikethrough and WrapText (send `SetStyleAttr`, assert flag set then cleared); clone
  the AlignHorizontal test for AlignVertical; extend the `SetBorders` tests to pass a
  line+color and assert the resolved edges.
- **Cache (`cache.rs` `#[cfg(test)]`):** extend `render_style_from` field checks for
  strikethrough/wrap/v_align; extend the all-9-`BorderStyle` mapping test to assert
  `Edge.pattern` (Solid/Dashed/Double + solid fallback for deferred styles).
- **UI (`chrome/view.rs` `#[cfg(test)]`):** the borders popover no longer closes on a
  target click (assert `borders_open` stays true, `border_target` set); switching target
  keeps the pen; reopening resets `border_target=None`; None clears + deselects.
- **Render tests (manual gate):** new cases for each visible change — the 5 toolbar
  buttons, strike line, wrapped text, each vertical alignment, the redesigned popover,
  the 2×2 icons, the gallery previews, and dashed/double borders. Regenerate + **eyeball**
  baselines for the intentional changes; then **dispatch the CI `render` gate and confirm
  green** before merge (per the project's render-test policy). This is an explicit step
  in the implementation plan, not implicit.

## 11. Sequencing

Part 1 and Part 2 are independent and ship in that order (Part 1 is small/low-risk,
Part 2 carries the popover redesign + new grid paint). Detailed in the implementation
plan. **No IronCalc fork changes are required for either part.**
