---
status: complete
---

# Phase 4: Formatting controls (SetStylePath, text color, alignment, number formats)

## Overview

Adds the first batch of the reworked action bar: a generic `SetStylePath` worker
command that drives text color, horizontal alignment, and number-format changes, plus
the number-format dropdown with category display and a decimals ± pair. This closes the
formatting-controls half of `components/action_bar.md` (fonts land in Phase 5, borders
in Phase 6) and the number-format render-side dependency of `components/style_render.md`.

The rendered cell effects (text color, alignment, engine-formatted numbers, `[Red]`
negatives) already exist from Phase 1 and are already pixel-baselined; this phase only
adds the *controls* that emit the commands, plus the resident-cache plumbing the
controls read to display the active cell's number-format category and compute decimals.

Key facts verified against the pinned IronCalc 0.7.1 source before building:
- `update_range_style(&Area, "font.color", value)` — `color("")` returns `Ok(None)`
  (clears); `#RRGGBB` sets. (`user_model/common.rs:128,76`)
- `update_range_style(&Area, "alignment.horizontal", "general")` → `HorizontalAlignment::General`
  and mutates only `alignment.horizontal`, preserving vertical/wrap (`common.rs:159,93`) —
  so re-press clears horizontal without the `""` whole-alignment nuke.
- `update_range_style(&Area, "num_fmt", code)` clones the code straight in (unvalidated)
  (`common.rs:150`).

## Steps

### Data model (freecell-core)

1. **`style.rs` — `RenderStyle`**: replace `num_format_is_default: bool` with
   `num_fmt: u16` (index into the cache's `num_fmts` side table; `0` = "general").
   `Default` sets `num_fmt: 0`. Update the module tests. (Nothing in the render path
   reads the old bool — grep confirms only cache build + the render-test scene copy it;
   the field was pure interning identity. The index is strictly more informative and
   keeps the interning identity.)

2. **`cache.rs` — `SheetCache` + `SheetCacheBuilder` num-fmt side table**: add
   `num_fmts: Vec<Arc<str>>` (seeded `[0] = "general"`) to both. Add:
   - `fn intern_num_fmt(&mut self, code: &str) -> u16` on both — `code` that is
     case-insensitively `"general"` → 0; else linear-scan/append (the table is tiny —
     a handful of distinct formats per sheet; document that).
   - `pub fn num_fmt_code(&self, id: u16) -> &str` on `SheetCache` — `id` out of range
     or 0 → `"general"`.
   - Thread `num_fmts` through `SheetCacheBuilder::build()` into the `SheetCache`.
   - `SheetCache` stays `Copy`-free but `Send + Sync` (`Arc<str>` is both — the existing
     compile-time guard still holds). Deviation from style_render.md's `SharedString`:
     freecell-core is deliberately gpui-free, so `Arc<str>` is the headless analog
     (record in DECISIONS).

3. **`format_ui.rs` (NEW) + `lib.rs` export**: pure, unit-tested helpers:
   ```rust
   pub enum Category { General, Number, Currency, Percent, Date, Time, Text, Custom }
   impl Category { pub fn label(self) -> &'static str; }
   pub fn num_fmt_category(code: &str) -> Category;   // exact-match dropdown codes → else Custom
   pub fn adjust_decimals(code: &str, delta: i8) -> Option<String>; // last `0(.0+)?` group
   pub fn font_size_display(q: u16) -> String;        // 0 → "11"; else q/4 (trim .0)
   ```
   `adjust_decimals` scans (no regex dep) for the last `0` optionally followed by `.` and
   one-or-more `0`s; +1 appends a `0` (creating `.0` if none), −1 drops one (dropping `.`
   when empty), min 0 → `None`; no matchable group → `None` (covers General/Text/Date/Time
   whose canonical codes carry no `0` placeholder).

### Worker seam (freecell-engine)

4. **`worker/protocol.rs`**: add `pub enum StylePath { FontColor, AlignHorizontal, NumFmt }`
   with `fn as_str(&self) -> &'static str`, and
   `Command::SetStylePath { sheet: SheetId, range: CellRange, path: StylePath, value: String }`.
   (Typed path instead of the architecture's raw `String` — safer, self-documenting, no
   IronCalc type crosses the seam; record in DECISIONS.) Re-export `StylePath` from
   `worker/mod.rs` + crate `lib.rs`.

5. **`document.rs`**: add
   `pub(crate) fn update_style_path(&mut self, sheet_idx, range, path: &str, value: &str) -> Result<(), String>`
   → `user_model.update_range_style(&area_of(sheet_idx, range), path, value)` (mirrors
   `set_fill`/`set_font_flag`).

6. **`worker/run.rs`**: bucket `SetStylePath` into the `edits` group (245-ish); `apply_one`
   handles it → `doc.update_style_path(idx, *range, path.as_str(), value)` → `StyleOnly`;
   `op_of` → `AppliedOp::Cells { sheet, range }`. Refresh + StyleCacheUpdated ride the
   existing per-cell `refresh_cell` plumbing (bounded UI ranges — non-band).

7. **`freecell-engine/cache.rs`**: `render_style_from(&Style)` now sets `num_fmt: 0`
   (field rename only). `build_sheet_cache` + `refresh_cell` set
   `rs.num_fmt = intern_num_fmt(&style.num_fmt)` **before** the `!= default` check (so a
   cell whose only styling is a custom format is still stored). `assert_cache_agrees`
   compares num-fmt **strings** (`cache.num_fmt_code(idx)` vs `style.num_fmt`, general
   normalized) since the index is cache-local; structural compare zeroes `num_fmt`.
   Update the affected unit tests.

### Action bar (freecell-app)

8. **`chrome/client.rs`**: add `fn num_fmt_code(&self, sheet, cell) -> Option<String>` to
   `ChromeClient`; `DocumentClient` resolves via the cache
   (`render_style` → idx → `num_fmt_code`); `RecordingClient` gains a `num_fmts` map +
   `set_num_fmt`.

9. **`chrome/view.rs`** — state + commands + getters + render:
   - Fields: `active_num_fmt: Option<String>` (refreshed with `active_style` in
     `on_selection_changed` + `refresh_active_style`), `text_color_open: bool`,
     `text_color_picker: Entity<ColorPickerState>` (+ subscription → `apply_text_color`),
     `num_fmt_open: bool`, `degraded: bool` + `set_degraded`.
   - Commands (all `commit_pending_edit` first, log-only, reuse `SetStylePath`):
     `apply_text_color(Option<Rgb>)` (None → value `""`), `apply_alignment(Align)`
     (re-press active → `"general"`, else `left|center|right`), `apply_num_fmt(&str)`,
     `bump_decimals(delta)` (compute via `adjust_decimals(active_num_fmt, delta)`; None →
     no-op).
   - Getters: `align_active(Align)` (explicit `h_align` only), `num_fmt_category_label`,
     `increase_decimals_enabled` / `decrease_decimals_enabled`, popover open flags.
   - Render (`render_action_row`) reworked to the ui_design §2 group order with existing
     divider styling; text-color + number-format popovers mirror the existing fill
     popover; alignment is a 3-button toggle group; decimals are two small buttons
     disabled per-direction. Every mutating control (incl. existing B/I/U/Fill) is
     `.disabled(self.degraded)`.
   - Window min-width raised to the row's natural width (constant — record value in
     DECISIONS).

10. **`shell/window.rs`**: in the `WorkerDegraded` handler, also
    `self.chrome.update(cx, |c, cx| c.set_degraded(true, cx))`.

### Render-tests

11. **`scene.rs`**: `Inject::Align` arm drops the explicit `num_format_is_default:` line
    (the `..base` spread already carries `num_fmt`). No new render cases, no baseline
    changes (the struct rename is render-neutral; the phase's rendered effects — text
    color, alignment, engine-formatted numbers, `[Red]` — are already baselined by
    Phase 1: `cell_fill_dark_text_contrast`, `cell_align_*`, `cell_number_*`,
    `cell_number_negative_red`; the action bar is chrome, excluded from the pixel suite).

## Tests

Unit (`freecell-core::format_ui`):
- `category_exact_matches_all_seven` + `category_custom_fallback`.
- `adjust_decimals_adds_and_removes` (`#,##0.00`→`#,##0.000`; `0.0`→`0`; `0`→`0.0`).
- `adjust_decimals_noop_on_general_text_date_time`.
- `adjust_decimals_currency_keeps_prefix` (`$#,##0.00`→`$#,##0.0`) + percent keeps suffix.
- `font_size_display_default_and_halves`.

Unit (`freecell-core::cache`):
- `num_fmt_interning_dedups_and_general_is_zero` (case-insensitive general → 0; distinct
  codes get distinct ids; `num_fmt_code` round-trips; out-of-range → "general").

Engine (`freecell-engine::cache`):
- `build_carries_num_fmt_from_file` — a custom-format cell resolves to a non-zero
  `num_fmt` whose `num_fmt_code` is the engine string; a general cell stays 0.
- existing `assert_cache_agrees` fixtures stay green (num-fmt string compare).

Worker (`freecell-engine::worker::run`):
- `set_style_path_num_fmt_applies_and_cache_reflects` — `SetStylePath{NumFmt,"$#,##0.00"}`
  on a number cell → cache `num_fmt_code` is Currency after refresh; StyleOnly (no eval).
- `set_style_path_align_and_color_apply`.

Chrome (`chrome/view.rs` tests, `RecordingClient`):
- `alignment_toggle_emits_clear_on_repress` (value `"general"` when re-pressing active).
- `text_color_automatic_emits_empty_value`.
- `text_color_swatch_emits_hex`.
- `num_fmt_pick_emits_currency_code`.
- `decimals_buttons_emit_adjusted_code` (increase `#,##0.00`→`#,##0.000`).
- `decimals_disabled_for_date_format` (both directions).
- `num_fmt_category_reflects_active_cell`.
- `controls_disabled_in_degraded_mode`.

## Automated checks (from `app/`, iterate until clean)

`cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` ·
`cargo build --workspace` · `cargo test --workspace` (without `FREECELL_RENDER`).
