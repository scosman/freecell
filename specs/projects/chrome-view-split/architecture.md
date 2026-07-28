---
status: complete
---

# Architecture: chrome-view-split

## 0. Scope of this document

There is no data model to design and no runtime behaviour to specify — the compiled program
is meant to be identical before and after. What this document specifies is:

- the target module graph and what each module owns (§2),
- the **one** genuinely non-trivial technical problem: Rust privacy across the new boundary
  (§3),
- the mechanical procedure for moving code so that a reviewer can verify a move *is* a move
  (§4),
- the exact source-range → destination mapping the implementer executes (§7).

**Single-document decision.** The skill's 300-line heuristic points at component designs, but
per-module component docs would be ten files each restating "this module holds the ranges
listed in §7 of the architecture." The complexity here is not in any component — every module
is a bag of moved methods — it is in the *mapping* and the *privacy mechanics*, which are
global. Those live here. No `components/` directory.

## 1. Prior art in this codebase

The overview asks to follow the pattern that already worked. Two things were previously
extracted out of `chrome/view.rs`, and one existing directory is the structural precedent.
They are not the same pattern, and the distinction matters:

- **`chrome/edit.rs` (`EditController`) and `chrome/cond_fmt.rs` (`CondFmtPanel`,
  `CfEditorState`)** — *state* extractions into **sibling** modules. These moved fields out of
  `ChromeView` and required a deliberate public accessor surface, which is why they are small
  (283 and 118 lines). **This pattern is out of scope here**: moving state is E2's job, and
  the overview explicitly forbids it. Cited so a reviewer does not ask why we didn't do it.

- **`grid/`** (`mod.rs`, `input.rs`, `layout.rs`, `chart_layer.rs`, `fixtures.rs`, `view.rs`)
  — a *file* split into a directory, with `mod.rs` holding shared constants and re-exports.
  **This is the applicable precedent** and the target shape. Note `grid/mod.rs` re-exports
  with `pub use view::{GridDataSources, GridView};` and `pub(crate) use
  view::caret_intent_modifiers;` — the same re-export discipline `chrome/mod.rs` already uses
  for `view`.

The split here is cheaper than either, because it moves **no state at all**. It relies on a
property neither prior extraction could use: a child module can already see its parent's
private fields.

## 2. Target module graph

```
chrome/
├── mod.rs                 (unchanged — still `mod view;` + `pub use view::{...}`)
├── client.rs              (untouched)
├── cond_fmt.rs            (untouched — the CF *state* types)
├── edit.rs                (untouched — EditController)
├── h_scroller.rs          (untouched)
├── sidebar.rs             (untouched)
└── view/
    ├── mod.rs             struct ChromeView (69 fields) · new() · shared consts ·
    │                      Anchor · module declarations · re-exports
    ├── shell.rs           impl Render / Focusable · render_action_row · render_overlays ·
    │                      backdrop · anchored_trigger · on_worker_event · on_selection_changed ·
    │                      set_grid_body · refresh_active_style · degrade
    ├── editing.rs         data row · in-cell editor · quick-edit · commit/cancel ·
    │                      autocomplete · signature hints · cap-error popover
    ├── formatting.rs      style toggles · merge · fill · text colour · number format ·
    │                      font family/size · borders · their popovers and free helpers
    ├── charts.rs          ChartPanel(+Series) · insert-chart menu · chart edit panel + chrome
    ├── cf_sidebar.rs      CF open/close/refresh/re-scope · rules list · row render · raise/
    │                      lower/delete
    ├── cf_editor.rs       CF rule editor state · operands · formats · colour scales ·
    │                      validation · save · CF constant tables
    ├── tabs.rs            TabDrag/TabSpan · tab bar · reorder drag · rename · context menu ·
    │                      delete confirm
    ├── find.rs            find/replace bar + behaviour
    ├── stats.rs           selection-stats readout
    └── test_support.rs    #[cfg(test)] Harness · build helpers · the 15 test seams
```

Every module except `mod.rs` and `test_support.rs` contributes `impl ChromeView { … }` blocks
and its own `#[cfg(test)] mod tests`. `ChromeView` remains a single type with a single
definition; only its `impl` blocks are distributed. Rust permits inherent `impl` blocks for a
type in any module of the defining crate, which is what makes this legal without a trait or a
newtype.

### 2.1 What stays in `mod.rs`

`mod.rs` is the shared root, deliberately kept thin:

- The `struct ChromeView` definition with all 69 fields and their doc comments, verbatim.
- `ChromeView::new()` (lines 560–746) — it constructs every field across every domain, so it
  cannot belong to any one domain without arbitrarily privileging it.
- The chrome look constants (`CHROME_BG`, `DIVIDER`, `HAIRLINE`, `MUTED_TEXT`, `DANGER`,
  `TOOLTIP_*`, …) used by more than one domain.
- `enum Anchor` + `impl Anchor` + `ANCHOR_COUNT` — the dropdown-anchor index, read by
  `shell.rs` (`anchored_trigger`), `formatting.rs`, `charts.rs` and the test seams.
- `mod` declarations and the shared import block.

Domain-specific constants move to their domain: `FONT_SIZES`/`SYSTEM_DEFAULT_FAMILY` →
`formatting.rs`; `CHART_INSERT_*`/`CHART_MENU` → `charts.rs`; `CF_*` + `enum CfMenu` →
`cf_editor.rs`; `TAB_*` → `tabs.rs`; `FIND_*` → `find.rs`; `REF_BOX_W`/`DATA_ROW_*` →
`editing.rs` (with `DATA_ROW_CONTENT_LEFT` staying wherever the compiler shows it is shared).

## 3. The one real technical problem: privacy

This is the part that is not obvious, and getting it wrong is the only way this project
produces a diff that is not a pure move.

### 3.1 What is free

Rust visibility is "visible in this module **and all its descendants**." `ChromeView`'s 69
fields are private to the module that defines the struct — which after the split is
`chrome::view`. Every new child (`chrome::view::tabs`, …) is a *descendant* of
`chrome::view`, so **every child can read and write every field with no annotation at all**.
This is the load-bearing fact behind the whole project: 69 fields, zero changes.

The same applies to everything else that stays in `mod.rs`: the look constants, `Anchor`, and
the struct itself are all visible to every child for free.

### 3.2 What is not free

Privacy does **not** flow sideways or upward. A private item that moves *into*
`chrome::view::tabs` becomes invisible to `chrome::view` and to sibling
`chrome::view::formatting`.

Three categories are affected:

1. **Private methods called across domains.** `impl ChromeView { fn foo() }` written inside
   `view::tabs` is a method whose *visibility* is `view::tabs`. If `view::shell` calls
   `self.foo()`, it fails to compile (E0624).
2. **Private free functions** — e.g. `hsla_to_rgb`, `cf_stops_from_colors`,
   `tab_insertion_index`.
3. **Private types and constants** — `TabDrag`, `TabSpan`, `CfMenu`, the `CF_*` tables.

**Resolution rule (uniform, no judgement per item):** any such item gets **`pub(super)`**.

`pub(super)` on an item in `chrome::view::tabs` resolves to `pub(in crate::chrome::view)` —
visible throughout the `view` subtree and nowhere else. That is *exactly* the visibility the
item had when everything lived in `view.rs` (private to `view`). So the change is
visibility-preserving in effect, even though it is a textual change. Nothing gains
`pub(crate)` or `pub`.

**Already-`pub` methods need nothing.** An inherent `pub fn` on `ChromeView` is visible
wherever `ChromeView` is visible, regardless of which module the `impl` block sits in. Since
~half the production methods are already `pub` (they are the view's action surface, called by
`shell::window` and the tests), those move untouched.

**Discovery is the compiler's job, not the planner's.** We do not attempt to predict the
cross-domain call set. The loop is: move a range, build, and let `rustc` name every E0624 /
E0603; annotate exactly those with `pub(super)`; rebuild. Guessing up-front would both miss
cases and over-widen items that did not need it.

### 3.3 Imports and the glob

Each child module begins with `use super::*;`. This pulls down `ChromeView`, the shared
constants, `Anchor`, and — because `use` declarations are themselves private items visible to
descendants, and a glob from a descendant picks up the parent's private imports —
`mod.rs`'s entire import block. That avoids duplicating a 55-line `use` header ten times.

Two consequences to manage:

- **Unused imports.** CI runs `clippy --workspace -- -D warnings`, so a stranded import in
  `mod.rs` fails the build. After each phase, `cargo clippy -p freecell-app --all-targets --
  -D warnings` must be clean; imports that end up used by only one child get moved into that
  child. This is checked per-phase precisely because it accumulates silently otherwise.
- **Ambiguity.** If a child both globs `super::*` and explicitly imports the same name, the
  explicit import shadows the glob — legal and warning-free. Two *globs* offering the same
  name would be an error, which is why children glob only `super::*`.

Test modules do the same: `mod tests { use super::*; }` inside a child transitively reaches
`mod.rs`'s imports through the same mechanism. This is the pattern the file already uses; it
just gains one more level of nesting.

### 3.4 Test support

The current test module is one flat `mod tests` (lines 8380–16099) whose 254 tests share nine
helpers and a `Harness` struct, plus 15 `#[cfg(test)]` test seams on `ChromeView` at
8249–8379 (`test_type`, `test_press_enter`, `test_find_type`, `incell_text`, `anchor_x_of`, …).

These become `chrome/view/test_support.rs`, declared `#[cfg(test)] mod test_support;`. Its
items — `Harness` (and its fields), `cell`, `build`, `build_win`, `build_sized`, `one_sheet`,
`tall_sheet`, `upd`, `tick` — and the 15 seam methods all take `pub(super)`, by the same rule
and for the same reason as §3.2.

Each domain's `mod tests` then opens with:

```rust
use super::*;
use crate::chrome::view::test_support::*;
```

**Why a `#[cfg(test)]` module rather than `#[cfg(test)]` items inside `mod.rs`:** keeping the
harness in one addressable place means each domain's test module imports it by one line, and
the seams stay grouped as they are today rather than scattering across ten files by which
widget they poke.

## 4. Move discipline — how we keep it a move

The risk this project manages is not "does it compile" but "can a reviewer confirm nothing
changed." Four rules make the diff mechanically checkable:

1. **Cut on item boundaries, including the doc comment.** A method's `///` block moves with
   it. Never cut mid-item, never reflow a doc comment.
2. **Preserve relative order within a destination.** Items arrive in a module in the same
   order they appeared in `view.rs`. A reviewer reading the new file top-to-bottom reads the
   old file's sections in the old sequence.
3. **Move the banner comments too.** The `// ---- Action row: SetBorders (pen popover)
   ----` banners move with their sections and become the section headers of the new files.
   They are the evidence that a section arrived intact.
4. **Body bytes are identical.** The only permitted edits to a moved item are (a) a
   `pub(super)` prefix, (b) whatever `cargo fmt` does to re-indent. Nothing else. In
   particular: no import-path rewriting inside bodies (the glob keeps paths valid), no
   renaming, no clippy fixes.

Rule 4 makes the review tractable: apart from visibility prefixes, `git show --stat` should
show large deletions from `view.rs` and matching insertions elsewhere, and a reviewer can spot
check by diffing extracted ranges.

**Verification is per-phase, not per-project** (functional spec §6): build, test with the
name-multiset check, `fmt --check`, `clippy -D warnings`. A phase that cannot go green is
reverted rather than patched forward.

### 4.1 Why `view.rs` → `view/mod.rs` first

Phase 1 does the rename (`git mv view.rs view/mod.rs`) **and nothing else**. Git records it as
a pure rename; every subsequent phase then shows as a move *out of* `mod.rs`. Doing the rename
in the same commit as a content move would make git render both as unrelated
add/delete blobs and destroy the reviewability the whole project is trying to buy.

## 5. Risks and how each is handled

| Risk | Handling |
|---|---|
| A test module silently dropped during a move | Test-name multiset diff per phase (functional spec §6.1) |
| Over-widening visibility to `pub`/`pub(crate)` | Uniform `pub(super)` rule; grep the diff for `pub fn`/`pub(crate)` additions before commit |
| Stranded import fails CI clippy | `clippy --all-targets -D warnings` per phase, not just at the end |
| An accessor moved to the "wrong" domain | Harmless (compiles, no behaviour change); accepted rather than agonised over |
| A domain turns out to need a state change to separate | **Stop and report.** Per the overview this is the one legitimate blocker. Assessed in §6 — none found |
| Scope creep into logic fixes | Bugs go to `findings.md`; rule 4 above |
| `cf_editor.rs` drifting over 2,000 later | It lands at ~1,730 with ~270 lines of headroom; F2 will police it |

## 6. Separability assessment (the overview's named unknown)

The overview asks to raise it if a domain cannot be separated without changing state
ownership. **Checked against all nine domains: none require it.** The reason is structural
rather than lucky — every domain's methods are `&mut self` methods on `ChromeView` reading
whatever fields they need, and §3.1 gives child modules unrestricted access to all 69 fields.
Cross-domain coupling (e.g. `formatting` calling the editing domain's `commit_pending_edit`
before applying a style, `charts` and `cf_sidebar` sharing the right dock so each closes the
other) resolves entirely through `pub(super)` on the callee. Coupling is *not* a blocker for
this project — it would be for E2, which is why E2 is a separate project.

No blocker to report.

## 7. Source-range → destination mapping

The executable specification. Ranges are 1-based inclusive against `chrome/view.rs` at
`6e05e76`. Cuts land on item boundaries (doc comment included), so the exact numbers shift by
a line or two; the item names are authoritative where they disagree.

### 7.1 Production

| Source range | Items | → Destination |
|---|---|---|
| 1–75 | imports | `mod.rs` (redistribute per §3.3 as clippy dictates) |
| 76–122 | chrome look constants | `mod.rs` |
| 123–137 | `enum CfMenu` | `cf_editor.rs` |
| 138–171 | `CHART_INSERT_COLS/ROWS`, `CHART_MENU` | `charts.rs` |
| 172–189 | `enum Anchor`, `impl Anchor`, `ANCHOR_COUNT` | `mod.rs` |
| 190–199 | `FONT_SIZES`, `SYSTEM_DEFAULT_FAMILY` | `formatting.rs` |
| 200–210 | `DATA_ROW_*`, `TAB_*` constants | `editing.rs` / `tabs.rs` (split by prefix) |
| 211–224 | `REF_BOX_W`, `FIND_*`, `DATA_ROW_CONTENT_LEFT` | `find.rs` / `editing.rs` |
| 225–288 | `ChartPanelSeries`, `ChartPanel`, `impl ChartPanel` | `charts.rs` (stay `pub`; re-exported) |
| 289–329 | `TabDrag`, `TabSpan`, `tab_insertion_index`, `move_target_for_gap` | `tabs.rs` |
| 330–555 | `struct ChromeView` (69 fields) | `mod.rs` |
| 560–746 | `new()` | `mod.rs` |
| 747–838 | `set_grid_body`, `refresh_active_style`, `on_selection_changed` | `shell.rs` |
| 839–922 | 5 selection-stats methods | `stats.rs` |
| 923–1682 | commit/escape, pending edit, in-cell, quick-edit, autocomplete | `editing.rs` |
| 1683–1767 | `on_worker_event` | `shell.rs` |
| 1768–1897 | reducer sync, content events, data/eval effects, fetch timer | `editing.rs` |
| 1898–2159 | formatting toggles, fill, `SetStylePath` group | `formatting.rs` |
| 2160–2293 | insert-chart menu, chart panel open/close | `charts.rs` |
| 2294–2883 | CF sidebar + rule editor behaviour | split `cf_sidebar.rs` / `cf_editor.rs` (§7.3) |
| 2884–3152 | chart type/range/chrome from panel, chart input events | `charts.rs` |
| 3153–3206 | `SetFont` (family + size) | `formatting.rs` |
| 3207–3376 | `SetBorders` pen popover + colour-picker events | `formatting.rs` |
| 3377–3398 | `set_degraded` | `shell.rs` |
| 3399–3725 | sheet tab bar + reorder drag | `tabs.rs` |
| 3726–3927 | read accessors | **distributed** (§7.2) |
| 3928–4081 | `hsla_to_rgb`, `border_target_icon*`, `border_line_preview`, `format_size_pt` | `formatting.rs` |
| 4082–4150 | `impl Focusable`, `impl Render`, `anchored_trigger` | `shell.rs` |
| 4151–4557 | `render_action_row` | `shell.rs` |
| 4558–4644 | `render_data_row` | `editing.rs` |
| 4645–5044 | `render_find_bar` + find behaviour | `find.rs` |
| 5045–5122 | `render_tab_bar` | `tabs.rs` |
| 5123–5149 | `render_selection_stats` | `stats.rs` |
| 5150–5250 | `render_tab` | `tabs.rs` |
| 5251–5338 | `render_overlays`, `backdrop` | `shell.rs` |
| 5339–5454 | cap-error popover, autocomplete popover + row, sig-hint popover | `editing.rs` |
| 5455–5795 | fill / text-colour / number-format popovers + menus | `formatting.rs` |
| 5796–5941 | `render_chart_menu`, `render_chart_panel` | `charts.rs` |
| 5942–6862 | CF sidebar / list / row / editor renders | split `cf_sidebar.rs` / `cf_editor.rs` (§7.3) |
| 6863–7104 | chart type row, range body, legend row, series colours, data labels | `charts.rs` |
| 7105–7226 | font family + size popovers | `formatting.rs` |
| 7227–7455 | `render_borders_popover` | `formatting.rs` |
| 7456–7572 | `render_context_menu`, `render_delete_confirm` | `tabs.rs` |
| 7573–8248 | CF free functions + constant tables | split `cf_sidebar.rs` / `cf_editor.rs` (§7.3) |

### 7.2 Distribution of the read-accessor block (3726–3927)

The block is a grab-bag grouped by "accessor," not by domain. Each goes to the domain whose
state it reads. All are `pub` already, so none need annotation.

| Accessors | → |
|---|---|
| `ref_box_text`, `content_text`, `data_mode` | `editing.rs` |
| `bold_active`, `italic_active`, `underline_active`, `strikethrough_active`, `wrap_active` | `formatting.rs` |
| `active_sheet_merges`, `merge_active`, `merge_disabled` | `formatting.rs` |
| `align_active`, `valign_active`, `num_fmt_category`, `num_fmt_category_label` | `formatting.rs` |
| `increase_decimals_enabled`, `decrease_decimals_enabled`, `decimals_enabled`, `active_numeric_decimals` | `formatting.rs` |
| `is_degraded` | `shell.rs` |
| `text_color_open`, `num_fmt_open`, `fill_open` | `formatting.rs` |
| `eval_spinner_visible`, `fetch_spinner_visible`, `cap_error_visible`, `cap_error_message` | `editing.rs` |
| `active_sheet`, `sheets`, `rename_target`, `rename_error`, `confirm_delete_target`, `context_menu_target` | `tabs.rs` |

### 7.3 The conditional-formatting cut

Split at the sidebar/editor seam (functional spec §3.1), not at a line number:

**→ `cf_sidebar.rs`** — `cond_fmt_open`, `cond_fmt_sheet`, `toggle_cond_fmt_sidebar`,
`open_cond_fmt`, `close_cond_fmt`, `refresh_cond_fmt`, `rescope_cond_fmt_if_open`,
`raise_cf_rule`, `lower_cf_rule`, `delete_cf_rule`; renders `render_cond_fmt_sidebar`,
`render_cf_list`, `render_cf_row`; helpers `CfRowControls`, `cf_row_controls`,
`cf_rule_intersects_selection`, `render_cf_preview`, `cf_color`.

**→ `cf_editor.rs`** — everything else CF: `cf_editor_open`, `open_cf_editor`,
`cancel_cf_editor`, `seed_cf_inputs`, `on_cf_input_event`, `sync_cf_scale_values`,
`seed_cf_stop_inputs`, `cf_input_texts`, `with_cf_editor`, `toggle_cf_menu`, `select_cf_kind`,
the `set_cf_*` / `select_cf_*` / `toggle_cf_*` setter family, `show_cf_editor_error`,
`save_cf_editor`; renders `render_cf_editor`, `cf_dropdown`, `render_cf_type_dropdown`,
`render_cf_operands`, `render_cf_format_editor`, `render_cf_color_row`,
`render_cf_scale_editor`, `render_cf_stop_color`; `enum CfMenu`, `enum StopPos`, all `CF_*`
constant tables, and the label/validation/spec helpers (`cf_stops_from_colors`,
`cf_fmt_stop_value`, `cf_*_label`, `cf_inline_error`, `cf_segmented`, `cf_validate`,
`cf_build_spec`, `cf_state_from_spec`, `cf_stop_pos`, `cf_stop_label`,
`cf_stop_needs_value`, `cf_threshold_kind_label`).

`cf_sidebar.rs` calls into `cf_editor.rs` (opening a row's editor) and vice versa (save
refreshes the list) — resolved with `pub(super)`, per §3.2.

### 7.4 Tests (8249–16099)

Test sections are moved to the module holding the code they exercise, using the test-half
banner comments as the cut points (functional spec §2.1).

| Test source range | Section | → |
|---|---|---|
| 8249–8379 | the 15 `#[cfg(test)]` test seams | `test_support.rs` |
| 8380–8518 | `Harness`, `cell`, `build`, `build_win`, `build_sized`, `one_sheet`, `tall_sheet`, `upd`, `tick` | `test_support.rs` |
| 8519–8614 | Data row: fetch / reply / disable | `editing.rs` |
| 8615–8810 | Selection stats | `stats.rs` |
| 8811–9134 | Horizontal scroller (action bar + tab strip) | `shell.rs` |
| 9135–9329 | Data row: edit / commit / escape / cap | `editing.rs` |
| 9330–9420 | Action row: toggles + fill | `formatting.rs` |
| 9421–9620 | Merge / Unmerge toggle | `formatting.rs` |
| 9621–9755 | Insert chart (P17) | `charts.rs` |
| 9756–9921 | CF sidebar (P4) | `cf_sidebar.rs` |
| 9922–10584 | CF rules list (P5) | `cf_sidebar.rs` |
| 10585–11677 | CF rule editor (P6) | `cf_editor.rs` |
| 11678–11838 | Chart edit panel (P19) | `charts.rs` |
| 11839–12173 | Chart edit panel: chrome (P20) | `charts.rs` |
| 12174–12384 | Chart edit panel: post-v1 Batch 2 | `charts.rs` |
| 12385–12805 | `SetStylePath` | `formatting.rs` |
| 12806–12902 | BUG A/B: popover item clicks apply | `formatting.rs` |
| 12903–13315 | Phase 10.1 number-format drill-in | `formatting.rs` |
| 13316–13608 | `SetBorders` | `formatting.rs` |
| 13609–13702 | `SetFont` | `formatting.rs` |
| 13703–13777 | The two 250 ms spinners | `editing.rs` |
| 13778–14128 | Find / replace | `find.rs` |
| 14129–14348 | Sheet tab bar | `tabs.rs` |
| 14349–14533 | Sheet-tab reorder drag | `tabs.rs` |
| 14534–14806 | Editing feel | `editing.rs` |
| 14807–15599 | Quick-edit mode | `editing.rs` |
| 15600–15631 | Autocomplete + signature hints | `editing.rs` |
| 15632–15735 | Reference highlighting | `editing.rs` |
| 15736–16099 | Point-mode insertion | `editing.rs` |

The multi-line explanatory comment above the BUG A/B section (12806–12820) moves with it — it
is the only place the occlusion behaviour is documented.

## 8. Testing strategy

No tests are written, changed, or deleted. The 254 relocated tests **are** the strategy: they
are behaviour tests over `ChromeView`'s public action surface driven through a `RecordingClient`
double, and they pass identically before and after or the move was not pure.

The per-phase gate is functional spec §6. The one addition worth stating: the test-name
multiset check runs **every** phase rather than at the end, because a lost `mod tests`
declaration is invisible to a green build and to a passing test run — it only shows as a
smaller test set, and catching it three phases later means bisecting the wrong commits.
