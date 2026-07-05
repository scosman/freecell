---
status: complete
---

# Component: Action Bar (formatting controls)

## Purpose and scope

The reworked action row: font family/size dropdowns, text color, borders presets,
alignment toggles, number-format dropdown + decimals ±, alongside the existing
B/I/U/Fill. Covers control state derivation and command emission. NOT responsible
for: rendering the applied styles (style_render.md), the engine mechanics
(architecture §3), titlebar (architecture §7.1).

Lives in `chrome/view.rs` (+ small helpers in `freecell-core` for testable logic).
Layout/visuals: ui_design §2.

## State derivation (what the controls display)

Single source: the **active cell's** resolved `RenderStyle` from the UI-side
`SheetCache` snapshot — the same mechanism the B/I/U pressed-state uses today
(chrome/view.rs:660-673). New fields consumed: `font_family` (id → name via
`cache.font_families`), `font_size_q` (0 ⇒ "11"), `text/font color`, explicit
horizontal alignment, `num_fmt` (id → string via the cache's num-fmt side table —
see style_render.md; the string is needed for category display and decimals ±).

Derivation helpers (unit-testable, `freecell-core/src/format_ui.rs`):

```rust
pub fn num_fmt_category(code: &str) -> Category;   // exact-match table → General/Number/
                                                   // Currency/Percent/Date/Time/Text/Custom
pub fn adjust_decimals(code: &str, delta: i8) -> Option<String>;
    // scans the LAST `0(\.0+)?` group: +1 appends a 0 (creating ".0" if none),
    // -1 removes one (dropping the "." when empty); None (no-op) for General/Text/
    // Date/Time/Custom-without-decimal-group; min 0 decimals. Also None (gate off) for
    // any format the last-group scan can't safely edit: multi-section (`;`), scientific
    // (`E`/`e`), or quoted/escaped (`"…"`, `\`) — editing those would corrupt the code
    // (functional_spec §3.4 only guarantees the dropdown-native numeric formats).
pub fn font_size_display(q: u16) -> String;        // 0 → "11"; else q/4 (trim .0)
```

Multi-cell selections: state reflects **nothing** (toggles unpressed, number format shows
General, decimals ± disabled), matching the shipped B/I/U toggles' multi-select behavior —
the derived state clears when the selection isn't a single cell. Commands still apply to the
full selection. (Reconciled with the shipped-consistent behavior; DECISIONS_TO_REVIEW #5
flags whether a uniform anchor-reflect for *all* action-bar controls is wanted later.)

## Command emission (control → worker)

| Control | Command | Value |
|---|---|---|
| Font family pick | `SetFont { area, family: Some(name), size_pt: None }` | "System Default" → `family: Some("")` = clear |
| Font size pick | `SetFont { area, family: None, size_pt: Some(pt) }` | fixed list 8–36 |
| Text color swatch | `SetStylePath { area, "font.color", "#RRGGBB" }` | **Automatic** → value `""` (clear) |
| Text color Custom… | same, from ColorPicker (existing Fill pattern) | |
| Alignment L/C/R | `SetStylePath { area, "alignment.horizontal", "left|center|right" }` | pressing the pressed one → value `"general"` (clears horizontal only — never the `""` whole-alignment nuke, which would drop wrap/vertical loaded from file) |
| Border preset | `SetBorders { area, preset }` | `BorderPreset ∈ {All, Inner, Outer, Top, Bottom, Left, Right, None}` — worker maps 1:1 to the serde-built `BorderArea` (architecture §3.4) |
| Number format pick | `SetStylePath { area, "num_fmt", code }` | codes per architecture §3.1; General → the engine's `general` |
| Decimals ± | `SetStylePath { area, "num_fmt", adjust_decimals(current, ±1) }` | computed UI-side from the cached string; `None` ⇒ button no-ops (and renders disabled when category makes it a no-op) |

`area` = `area_of(selection)` — full-extent exactness for header selections is
load-bearing (band fast path, architecture §3.1/§5.2). Select-all: worker clamps
per-command per the architecture's clamping table.

All emissions reuse the existing `SetStyleAttr` dispatch path in the shell (one
window→worker call site; commands are fire-and-forget with log-only errors, except
`SetFont`'s too-large reply which dialogs).

## Internal design notes

- **Font family dropdown data**: `cx.text_system().all_font_names()` fetched once at
  window build, cached in chrome state, "System Default" prepended. Plain scrolling
  menu (gpui-component), no search field in this project.
- **Layout**: groups per ui_design §2 with existing divider styling; window min-width
  raised to the row's natural width (compute once from the rendered row; constant is
  fine — record the value in DECISIONS_TO_REVIEW). No overflow behavior.
- **Disabled logic**: degraded/read-only mode disables all mutating controls
  (existing flag); decimals ± additionally disabled when `adjust_decimals` returns
  `None` for the active cell.
- **Popovers**: text color reuses the Fill palette popover component with the "No
  fill" slot relabeled **Automatic**; borders popover is a 4×2 icon grid of buttons
  (no state — presets are actions, not toggles).
- Alignment pressed-state uses the **explicit** style only — a right-aligned number
  via type-default shows *no* pressed button (matches Excel).

## Dependencies

Depends on: SheetCache new fields + num-fmt table (style_render.md), worker commands
(architecture §2), gpui-component menu/popover/ColorPicker (existing patterns),
`all_font_names` (verified available). Depended on by: nothing downstream — leaf UI.

## Test plan

Unit (`format_ui`):
- `category_exact_matches_all_seven` + `category_custom_fallback`.
- `adjust_decimals_adds_and_removes` (`#,##0.00`→`#,##0.000`; `0.0`→`0`; `0`→`0.0`).
- `adjust_decimals_noop_on_general_text_date_time`.
- `adjust_decimals_currency_keeps_prefix` (`$#,##0.00` → `$#,##0.0`).
- `font_size_display_default_and_halves`.
Chrome-level (existing chrome test harness):
- `alignment_toggle_emits_clear_on_repress` (value `general`).
- `text_color_automatic_emits_empty_value`.
- `decimals_disabled_for_date_format`.
- `font_dropdown_shows_active_cell_family_and_size`.
- `controls_disabled_in_degraded_mode` (new controls included).
Engine integration: covered per-path in architecture §9 (band write, num_fmt apply);
this component adds `numfmt_dropdown_roundtrip` — set Currency via command, cache
refresh shows Currency category on the control.
Render suite: action-bar states are not pixel-tested (chrome excluded from the cell
render suite, as in MVP).
