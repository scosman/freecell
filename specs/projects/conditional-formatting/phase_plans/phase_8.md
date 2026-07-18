---
status: complete
---

# Phase 8: Persistence + deferred-rule handling + round-trip verification

## Overview

P1–P7 built the whole CF engine seam, worker protocol, value-dependent cache, and the sidebar. CF
is part of the IronCalc worksheet model, so — unlike charts — it needs **no special FreeCell save
handling**: it saves/loads through IronCalc's native `.xlsx` writer/reader. This phase does **not**
add production code; it **verifies + locks in** the two persistence-adjacent behaviors the earlier
phases already implement, with headless (engine-side) tests:

1. **xlsx round-trip through `WorkbookDocument`.** A highlight (CellIs) rule and a color-scale rule
   authored via `add_cond_fmt`, saved via FreeCell's real save API (`WorkbookDocument::save` — the
   exact call the worker's chart-less `save_workbook` branch makes, funnelling through IronCalc's
   `save_xlsx_to_writer`), reopened into a fresh `WorkbookDocument` via `open`, must come back with
   the **same rules** (range / kind / priority; the highlight rule's format survives) and the
   effective render style (`extended_render_style`) must still reflect them (the `> 100` cell keeps
   its fill).

2. **Deferred-family handling.** A loaded deferred-family rule (DataBar) — constructed directly via
   the engine's `UserModel::add_conditional_formatting` (there is no `CfRuleSpec` variant for it) —
   must surface in `cond_fmt_rules` as a non-editable `Badge` (`editable: false`, `spec: None`) and
   must **not** corrupt its cell's render: `extended_render_style` for a cell in the rule's range
   returns a valid base style (the bar decoration is intentionally dropped this pass), never
   garbage / a panic.

### Why this is verification, not new code

The architecture is already sound end-to-end and I confirmed each link against the pinned fork
(`scosman/ironcalc#freecell-fixes`, checkout `81feec4`):

- **Writer emits CF.** `save_xlsx_to_writer` (used by both `WorkbookDocument::save` and
  `to_xlsx_bytes`) serializes each worksheet's `conditional_formatting` via
  `get_conditional_formatting_xml` (`xlsx/src/export/worksheets.rs`). The fork's own
  `xlsx/tests/test_conditional_formatting.rs::test_conditional_formatting_lists` already proves a
  full save→load round-trip preserves CF lists.
- **Loader evaluates CF.** The model constructor `load_from_xlsx` funnels into calls
  `model.evaluate_conditional_formatting()` (`base/src/model.rs:1729`), which populates `cf_cache`;
  `get_extended_style_for_cell` reads that cache. `UserModel::from_model` moves the model in as-is
  (`base/src/user_model/common.rs:248`), preserving `cf_cache`. So after `WorkbookDocument::open`
  (which does **not** call `evaluate()` — SP2 cached-values open), the reopened document's
  `extended_render_style` already reflects CF without any extra step.
- **Deferred rules already route to Badge.** P1's `cf_rule_to_view` maps `CfRule::{DataBar, IconSet,
  IconRating}` → `badge_view` (`editable:false`, `Badge`, `spec:None`); `extended_render_style`
  drops `ExtendedStyle.{icon,data_bar,rating}` and returns only `.style`.

Given all links check out, the expectation is both tests pass with no code change. If the round-trip
does **not** hold (writer strips CF, or loader doesn't re-evaluate), that is a blocker to report per
CLAUDE.md (engine gaps get fixed in the fork), **not** something to work around FreeCell-side.

## Steps

1. **Add the round-trip test** to the existing in-crate test module in
   `app/crates/freecell-engine/src/document/cond_fmt.rs` (`#[cfg(test)] mod tests`). It must be
   in-crate (not `tests/`) because `add_cond_fmt` / `cond_fmt_rules` / `extended_render_style` /
   `has_cond_fmt` / `workbook_theme` are `pub(crate)`. Test body:
   - `let mut doc = WorkbookDocument::new_empty()`; set source values: `A1=150`, `A5=50` (highlight
     domain over `A1:A10`), `C1=0`, `C2=50`, `C3=100` (color-scale domain over `C1:C3`).
   - Author a **highlight** rule via `add_cond_fmt(0, "A1:A10", CellIs Gt "100" with fill=RED,
     text_color=BLUE, bold=true)`.
   - Author a **2-color scale** via `add_cond_fmt(0, "C1:C3", ColorScale[Min=GREEN, Max=RED])`.
   - Capture `rules_before = doc.cond_fmt_rules(0)`; sanity: `len()==2`, `fill_at(A1)==Some(RED)`.
   - Save with FreeCell's real save API: `doc.save(&path)` into a `tempfile::tempdir()` path; reopen
     `let reopened = WorkbookDocument::open(&path)`.
   - Assert **rules survive**: `reopened.cond_fmt_rules(0).len()==2`; a `range→priority` map is
     unchanged before/after (locks priority); the highlight row (keyed by range `A1:A10`) is
     `editable` and its `spec == Some(highlight)` (this equality proves op **and** format —
     fill/text-color/bold — survived); the scale row (`C1:C3`) is `editable` and its
     `spec == Some(scale)`.
   - Assert **effective style survives**: `fill_at(reopened, A1)==Some(RED)` and its `font_color ==
     Some(BLUE)` and `bold`; `fill_at(reopened, A5)==None`; the scale midpoint `C2` gets some
     interpolated fill (`is_some()`).

2. **Add the deferred-family test** to the same module. Because there is no `CfRuleSpec::DataBar`,
   construct the rule through the raw engine via `doc.user_model_mut()` (`pub(crate)`):
   - `let mut doc = WorkbookDocument::new_empty()`; set `A1=10, A2=50, A3=100`.
   - `doc.user_model_mut().add_conditional_formatting(0, "A1:A3", CfRuleInput::DataBar { min: None,
     max: None, positive_color: Color::Rgb("#638EC6"), negative_color: Color::Rgb("#FF0000"),
     is_gradient: true, show_value: true })` (imports scoped locally in the test:
     `ironcalc_base::cf_types::CfRuleInput`, `ironcalc_base::types::Color`).
   - Assert the list shows it as a **Badge**: `cond_fmt_rules(0).len()==1`; the row is
     `!editable`, `spec.is_none()`, `matches!(preview, CfPreview::Badge(_))`, `range=="A1:A3"`; and
     `has_cond_fmt(0)` is `true`.
   - Assert the cell render is **not corrupted**: `extended_render_style(0, A2, theme) ==
     RenderStyle::default()` (the base style; the data-bar decoration is dropped) — no panic, no
     garbage.

3. **No production-code change is expected.** If step 1 fails, stop and report the persistence gap
   as a blocker (with the exact failing assertion + evidence), and — only if it is an engine bug —
   capture a fork patch under `specs/projects/conditional-formatting/fork-fixes/` (the container
   cannot push to the fork; preserve as a patch + report).

## Tests

- `cond_fmt_round_trips_through_xlsx_save` — highlight + color-scale rules survive
  `save` → `open` (count, range→priority map, `editable`, and full `spec` equality incl. format);
  `extended_render_style` on the reopened doc still yields the highlight fill/text-color/bold on the
  `>100` cell, no fill on the `50` cell, and an interpolated fill on the scale midpoint.
- `loaded_deferred_family_rule_is_badge_and_renders_base_style` — an engine-constructed `DataBar`
  rule lists as a non-editable `Badge` with `spec: None`, keeps `has_cond_fmt` true, and its
  in-range cell's `extended_render_style` returns the base style (`RenderStyle::default()`), not
  garbage/panic.

## Validation

Engine-side / headless — **no pixel render suite** (this phase moves no grid/cell/sheet/titlebar
pixels). Run cargo from `app/`, crate-scoped:
- `cargo build -p freecell-engine`
- `cargo test -p freecell-engine`
- `cargo clippy -p freecell-engine --all-targets -- -D warnings`
- `cargo fmt --all --check`
