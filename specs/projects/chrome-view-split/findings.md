---
status: complete
---

# Findings: chrome-view-split

Things noticed while moving `chrome/view.rs` into `chrome/view/` and **deliberately not
acted on**. The project is a behaviour-preserving move; fixing anything here would have made
the diff unreviewable, which is the one risk worth managing (`project_overview.md` §Scope).

**Scope caveat, stated plainly:** this was a structural move, not a code audit. Nothing below
came from reading logic for correctness — these are things the *act of splitting* surfaced.
No behavioural bug was found, and none was looked for. A real bug hunt in this file is still
owed.

## 1. The flat test module hid cross-domain coupling — four shared fixtures

The 254 tests lived in one `mod tests` with 27 banner sections, so a helper defined under one
banner could be used from any other with nothing marking it as shared. Splitting forced each
one into the open. Of the module's 44 helpers, eight were the global harness that Phase 1
lifted wholesale and 32 stayed inside a single destination module. **Four more** turned out to
be cross-domain only once the sections were pulled apart, and joined the harness in
`view/test_support.rs` (which holds 12 helpers in total):

| Fixture | Defined under | Also used by |
|---|---|---|
| `numeric_stats` | Selection stats | Horizontal scroller (`shell`) |
| `multi_a1_a3` | Selection stats | Horizontal scroller (`shell`) |
| `two_sheets` | Sheet tab bar | Editing (cross-sheet in-cell seeding) |
| `cf_view` | CF rules list | CF rule editor |

More crossed banner lines without crossing module lines, and they make the point more
strongly than the four that had to move. Of the 36 helpers defined *under* a banner, **sixteen**
are used from a section other than their own; eleven of those also serve their own section, and
**five never do**.

The clearest case: the `// ---- Function autocomplete + signature hints` **test** section
(orig. 15600–15631) contains **no tests at all** — only four helper definitions
(`last_edit_state_autocomplete`, `last_edit_state_ref_highlights`,
`last_edit_state_reference_ready`, `a1`), each serving one of the two sections after it.
`last_edit_state_quick` has the same shape: defined under "Editing feel", called only under
"Quick-edit". At the other end `select_single` is called from five sections of what is now
`formatting.rs`, and three of the eight prelude helpers ahead of the first banner are used
almost everywhere — `upd` from **26 of the 27** banner sections (it misses only the helper-only
autocomplete section above), `one_sheet` from 25, `cell` from 23. The other five prelude
helpers are narrower: `build` and `tall_sheet` reach 5 sections each, `tick` 3, `build_win` and
`build_sized` 1 each (all on the same banner-section basis; `build`, `build_win`, `build_sized`
and `upd` are additionally called from the prelude itself, outside any banner).

Not a defect — but worth knowing that "the tests under this banner" was never a real
boundary, and any future attempt to move a test section will hit the same thing.

## 2. `commit_pending_edit` is an editing method living in the formatting section

It sits under the `// ---- Action row: formatting` banner and moved to `formatting.rs` with it,
but `charts.rs` calls it too — four sites against formatting's six — because those controls
commit a pending edit first and bail if the commit is blocked. It is now `pub(super)`.
(Conditional formatting does *not* call it, in either the split tree or the pre-split file; an
earlier draft of this note claimed it did.)

The banner placement is historical (formatting shipped first). If the domains are ever
tightened, this is the method to look at first: it is the one piece of editing behaviour that
every other domain depends on.

## 3. `set_degraded` had drifted away from `is_degraded`

`set_degraded` sat between the borders section and the sheet-tab section, so it was neither
clearly formatting nor clearly shell. It ended up swept into `formatting.rs` in P6 (once
`tabs.rs` was gone, the preceding banner section ran on past it) and was moved back to
`shell.rs` in P8, next to `is_degraded`, where the plan had it.

Recorded because it is a small example of the real cost of the old layout: with 8,248
production lines under 17 banners, an item's domain was decided by whatever banner happened to
precede it, and nothing checked that.

## 4. `on_worker_event` is a fan-out point that will keep growing

An 84-line `match` that routes every worker event to every domain — content replies, selection
stats, CF rule lists, chart snapshots, find results, degrade. It stayed in `shell.rs` because
it belongs to no single domain.

It is the one place the split does not help: adding a feature still means editing this method.
Nothing is wrong with it today; it is simply the file's remaining central seam, and a
reasonable thing to watch.

## 5. `DATA_ROW_H` / `TAB_BAR_H` are imported by path from a sibling

`chrome/sidebar.rs` does `use super::view::{ACTION_ROW_H, ACTIVE_TAB_BG, DATA_ROW_H, HAIRLINE,
MUTED_TEXT, TAB_BAR_H, TEXT};`. That pins those constants to `chrome::view`'s root — they
cannot move into a child module without changing a path outside this module. They stayed in
`mod.rs` for exactly that reason (as did `action_divider`, which is `pub(super)` *for
`chrome`*, a visibility a child module cannot express).

Layout constants shared between the chrome shell and a sidebar container arguably want their
own `chrome::metrics` module rather than living in the view. Out of scope here.

## 6. Test-only imports must sit inside `mod tests`, not at module level

`cargo clippy --all-targets` compiles the lib and the lib-test target separately. An import
used only by tests is "unused" in the lib target and warns — and CI runs `clippy -D warnings`.
One import (`cf_row_controls`, used by `cf_editor`'s tests) had to move inside the test module
for this reason.

Worth knowing before the next split: it is not enough for an import to be used *somewhere in
the file*.

## 6a. `mod.rs`'s import block accumulated 47 single-child orphans — clippy cannot see them

Architecture §3.3 predicted this and made it a **manual** per-phase check: "imports that end up
used by only one child get moved into that child. This is checked per-phase precisely because
it accumulates silently otherwise." That check was never run during the original pass. The
phase-2 code review caught three instances; sweeping the whole block found the scale.

At the point the phase-2 review landed, `mod.rs` imports **120** names — 116 by name plus the
four `as _` traits noted below. Of those, **47 are consumed by exactly one child** and belong in
that child per §3.3; **31 are consumed by two or more** and correctly stay (`ClickEvent` and
`Command` reach eight children each, `Button` seven, `WorkerEvent` five in code and six counting
`formatting.rs`'s intra-doc link — duplicating those into eight files would be strictly worse
than the glob, which is what the shared block is *for*); the remaining 37 `mod.rs` uses itself,
plus the one `pub use` re-export (`ChartPanelSeries`), which is already in its child and cannot
move. (47 + 31 + 37 + 1 = 116.)

**The sweep has one trap, and it caught the first attempt.** `mod.rs`'s own module doc contains
intra-doc links — `` [`validate_sheet_name`] ``, `` [`DataRow`] `` — and a scan that treats the
whole file as "`mod.rs`'s body" reads those as real consumers, so the name looks used and never
surfaces. Excluding `//!`, `///` and `//` lines before testing raised the count from 41 to 45;
the four it hid were `BASIC_FORMATS`, `CfEditorState`, `NUM_FMT_GROUPS` and
`validate_sheet_name`.

The exclusion is **asymmetric**, which is easy to get wrong in the other direction: strip
comments from `mod.rs`'s *own* body only. A doc mention there is not a reason to keep an import
— you de-link it, as `validate_sheet_name` required when it moved to `tabs.rs` — but an
intra-doc link in a *child* resolves only because the name is in scope there, so it genuinely
counts as a consumer. `formatting.rs`'s `` [`WorkerEvent::MergeNeedsConfirm`] `` is the live
example: it resolves only through the glob and would break if `WorkerEvent` moved. A plain
`` `WorkerEvent::CondFmtUpdated` `` code span, as in `cf_sidebar.rs`, is *not* a consumer —
that distinction is the difference between six and seven.

The child side needs the same care, and a naive word-boundary match over the whole child file
gets it wrong in **both** directions. `EditRejectedReason` looked multi-child — hits in
`cf_editor.rs`, `shell.rs` and `tabs.rs` — but `cf_editor.rs`'s is a plain code span in a doc
comment and `tabs.rs`'s is written `freecell_engine::EditRejectedReason::…`, fully qualified and
so independent of `mod.rs`'s import. Only `shell.rs` actually consumes it, which makes it a
single-child orphan the first sweep missed. A correct sweep strips comments from the child too,
and ignores fully-qualified paths.

And **string literals**, which is how `Category` hid: `mod.rs` contains `.placeholder("Category
axis")`, so a raw scan sees a use and keeps the import. It has no code use in `mod.rs` at all —
its only consumer is `formatting.rs`'s `num_fmt_category` return type.

The fourth trap runs the other way, and is the only one that can break a build. A **trait
imported by name whose sole use is method resolution** has no textual occurrence at its call
sites at all, so the sweep reports it as consumer-less and "safe" to move. `Focusable` is the
live case: this document previously listed it as a `shell.rs` orphan, and acting on that would
have produced two `E0599`s — `mod.rs:465` and `find.rs:202` both call
`InputState::focus_handle(cx)`, which exists only through `impl Focusable for InputState`, and
`Focusable` is not in `gpui::prelude`. The `as _` carve-out does not help: `Focusable` is
imported by name, so it passes the carve-out and still looks unused.

So: the scan must see *code*, on both sides, and nothing else — and for any **trait**, the
absence of textual hits proves nothing. Verify a trait move by compiling it, never by grepping.

A second trap, for anyone automating the "assert nothing has zero consumers" check: the four
trait imports brought in `as _` (`ButtonVariants`, `Disableable`, `Selectable`, `Sizable`) have
no name in code *by construction*, so a name-based scan reports them as consumer-less every
time. They need an explicit carve-out or the assertion fires spuriously.

The reason it accumulates invisibly is worth stating: a child's `use super::*;` re-globs
`mod.rs`'s private imports, so every orphan still resolves and `clippy -D warnings` stays
green. There is no latent build failure and nothing is dead — a sweep found **zero** names with
no consumer at all. It is a locality problem, not a correctness one, which is why it was scored
Mild.

**Status: being cleared per phase, with the CR of each phase.** Moved so far:

| Phase | Orphans returned to their owner |
|---|---|
| 2 | `format_stat_count`, `format_stat_value` → `stats.rs`; `close_button` → `find.rs` |
| 3 | `CursorStyle`, `MouseMoveEvent`, `MouseUpEvent`, `validate_sheet_name` → `tabs.rs` |
| 4 | `AnchorCell`, `ChartAnchor`, `ChartAxisKind`, `ChartChromeEdit`, `DataLabelToggles`, `LegendPosition`, `limits` → `charts.rs` |
| 5 | `Checkbox`, `CfColorStop`, `CfFormat`, `CfPeriod`, `CfRuleSpec`, `CfTextOp`, `CfThresholdKind`, `CfValueOp`, `CfEditorKind`, `CfEditorState` → `cf_editor.rs` |
| 6 | `BASIC_FORMATS`, `Category`, `ColorPicker`, `ColorPickerEvent`, `Hsla`, `NUM_FMT_GROUPS`, `StylePath`, `adjust_decimals_cell`, `displayed_decimals`, `effective_range`, `font_size_display`, `is_more_only_num_fmt`, `num_fmt_category`, `region_at`, `regions_intersecting`, `toggle_thousands` → `formatting.rs` |
| 7 | `AutocompleteDisplay`, `AutocompleteRow`, `DataRowEffect`, `EvalEffect`, `Motion`, `Position`, `caret_intent_modifiers`, `functions` → `editing.rs` |
| 8 | `EditRejectedReason` → `shell.rs` |

The ledger is now **closed**: `mod.rs` imports 74 names — **38** it uses itself (including
`Focusable`, whose only use is method resolution and which therefore has *no* textual hit), 31
reach two or more children, 4 are `as _` trait imports, 1 is the `pub use` re-export, and
**none is a single-child orphan**. The
zero-consumer class is empty — asserted, not inferred from a green clippy run. Clearing them per
phase rather than in one sweep kept the per-phase attribution §4.1 exists to protect.

Two things for whoever closes it out: apply the predicate as *two* parts (zero uses in
`mod.rs`'s own body **and** exactly one child consumer — a name `mod.rs` itself uses stays
regardless), and **assert** the zero-consumer class is empty rather than inferring it from a
green clippy run, which proves nothing here. A one-line comment above the block saying it is
consumed by children via `use super::*` would make the next reader's manual check tractable.

## 6b. Five broken intra-doc links, pre-existing and inherited by the split

`cargo doc -p freecell-app --no-deps --document-private-items` reports five warnings inside
`chrome/view/`: `render_chart_menu`, `render_chart_type_row` and `open_chart_panel`
(`charts.rs`), and `Self::accept_autocomplete` and `GridEvent::InsertReference` (`editing.rs`).

**None is a split artifact.** All five doc lines moved verbatim from the pre-split file, and all
five were already broken there: the three `charts.rs` ones link *bare* to inherent methods on
`ChromeView`, which never resolves without a `Self::`/`ChromeView::` qualifier; `GridEvent`
appears exactly once in the whole of the old `view.rs` — in that doc line — and was never
imported; and the `editing.rs` one is a `private_intra_doc_links` warning whose two visibilities
are unchanged by the move.

Left alone deliberately: fixing them is a doc change to items this project only relocates, and
CI has no `cargo doc` gate. Worth a follow-up sweep for whoever adds one.

## 6c. Three items did not land where the plan put them

All three are recorded so the per-phase diffs aren't read as tidier than they were. The first
two were relocated later and are where the plan wanted them at HEAD; the third was deliberately
left where it landed, and the plan is what's out of date.

- **`impl ChartPanel`** (holding the `#[cfg(test)] skeleton` constructor) stayed in `mod.rs`
  when P4 moved `ChartPanel` and `ChartPanelSeries`, leaving an inherent `impl` for a type its
  own module no longer defined — legal, but exactly the "code in the wrong place" this project
  exists to remove, and a deviation from `architecture.md` §7.1, whose mapping row for source
  225–288 lists the `impl` alongside the two structs. Moved to `charts.rs` in P8 (`3475998`).
- **`set_degraded`** drifted into `formatting.rs` in P6 and returned to `shell.rs` in P8 — see
  §3.
- **`CF_SWATCH_W`, `CF_SWATCH_H`, `CF_BADGE_BG`** landed in `cf_sidebar.rs`, though
  `architecture.md` §2.1 and §7.3 both assign the `CF_*` constants to `cf_editor.rs`. Left as
  built — they exist for `render_cf_preview`, which is the sidebar's — but it is why
  `cf_editor.rs` carries a cross-child `use super::cf_sidebar::{cf_color, CF_SWATCH_H,
  CF_SWATCH_W}` that §7.3 doesn't predict.

The first two share a cause: the cut was driven by a banner section's extent rather than by the
item list in §7, and a banner's extent changes as earlier phases remove the sections around it.
The third is a different miss — the §7 mapping was simply not followed for three scalars whose
doc comments point at the sidebar's preview swatch, and on review the constants are where they
belong and §7 is what's wrong. Two lessons, then: cut from §7's item list rather than from
whatever a banner happens to enclose at the time, and when the code disagrees with the mapping,
check which one is right before moving anything.

## 6d. Five of the ten child files break source order once; five don't

`architecture.md` §4 rule 2 says items arrive in a module in the order they appeared in
`view.rs`, so a reviewer can read the new file top-to-bottom and get the old file's sequence.
Measured by anchoring every item to its definition line in the baseline and counting descents:

| Child | Seams | Where |
|---|---:|---|
| `charts.rs`, `editing.rs`, `find.rs`, `stats.rs`, `tabs.rs` | 0 | — |
| `cf_sidebar.rs` | 1 | `render_cf_preview` (8189) → `cond_fmt_open` (2298) |
| `cf_editor.rs` | 1 | `cf_state_from_spec` (8050) → `cf_editor_open` (2420) |
| `formatting.rs` | 1 | `format_size_pt` (4070) → `toggle_style` (1902) |
| `shell.rs` | 1 | `backdrop` (5312) → `focus_handle` (4083) |
| `test_support.rs` | 1 | `two_sheets` (14131) → `cf_view` (9935) |

So **at most one** seam per file, not exactly one — and they are three different things, which
matters if you are using this as a checklist:

1. **Free items hoisted above the `impl`** (`cf_sidebar`, `cf_editor`, `formatting`) — the CF
   free functions and the border-icon helpers sat *after* the impls in source. A file that put
   free helpers 1,700 lines down would be worse, so the house shape wins.
2. **Trait impls grouped after the inherent impl** (`shell`) — `impl Focusable`/`impl Render`
   sat between the two original `impl ChromeView` blocks.
3. **Late-arriving shared fixtures appended** (`test_support`) — the four helpers that phases
   2/3/5 discovered to be cross-domain land after phase 1's harness, in phase order.
   `architecture.md` §3.4 anticipated this.

Not a defect in any case, but §4 rule 2 should be read as "within each block", and a reviewer
diffing ranges should expect at most one seam and know which kind to expect where.

## 6d1. Five production banner sections lost the item-to-header relationship rule 3 promises

`architecture.md` §4 rule 3 says the `// ----` banners move with their sections and become the
new files' section headers — the evidence that a section arrived intact. That works whenever a
section had one destination. Five production sections did not:

| Section (baseline banner line) | Items went to | Banner ended up |
|---|---|---|
| `Read accessors (tests + render)` (3726) | `formatting.rs` 19, `editing.rs` 7, `tabs.rs` 6, `shell.rs` 1 | `shell.rs:189` |
| `Selection + data-row plumbing` (773) | `shell.rs` 1, `stats.rs` 5, `editing.rs` 3 | **nowhere** — see below |
| `Action row: SetBorders` (3207) | `formatting.rs` 13, plus `set_degraded` → `shell.rs` | `formatting.rs:501` |
| `Function autocomplete + signature hints` (1375) | `editing.rs` 20, plus `on_worker_event` → `shell.rs` | `editing.rs:499` |
| `Conditional-formatting sidebar (P4)` (2294) | `cf_editor.rs` 32, `cf_sidebar.rs` 10, `charts.rs` 2 | `cf_sidebar.rs:143` |

Row 5 is the one where the banner's extent overshot the *planned* cut: §7.3 divides CF between
the sidebar and the editor, but the section also swept up `set_chart_type_from_panel` and
`apply_chart_range_from_selection`, which §7.1 sends to `charts.rs` — a third destination §7.3
never contemplated.

Three **test** sections split too, each donating one or two shared fixtures to `test_support.rs`
(§1 covers them): `Selection stats` (8615), `CF rules list` (9922), `Sheet tab bar` (14129).

Two consequences worth knowing before diffing ranges:

- **`editing.rs` and `stats.rs` open with header-less preludes.** Items that arrived from a
  split section have no banner above them; in `formatting.rs` the same applies mid-file, where
  19 accessors run straight on from `on_border_color_picker_event`. (`shell.rs` did too, until
  the phase-8 review relocated this section's banner into it — see below.)
- **`set_degraded` now sits under the wrong banner.** It landed in `shell.rs:194`, beneath
  `Read accessors`, while its own `SetBorders` banner went to `formatting.rs`. It is in the
  right *module* (§3, §6c); it is under the wrong *heading*.

**One banner was left dangling, and has been moved.** `// ---- Selection + data-row plumbing`
sat at `mod.rs:564` immediately before the `impl` block's closing brace — P8 took
`on_selection_changed` out from under it and left the header behind, heading nothing. The
phase-8 review relocated it to `shell.rs:45`, immediately above `on_selection_changed`, the item
it introduced in the baseline — not above `set_grid_body`/`refresh_active_style`, which preceded
it in source and were never under it. That is rule 3 being *completed* rather than a comment being edited: the section
moved and the banner had simply failed to travel with it, and the comment multiset across the
tree is unchanged (44 banners in the baseline, 44 now). Row 2 of the table above reads
"nowhere" for the state P8 left; it is `shell.rs` today.

## 6e. One deliberate exception to "body bytes are identical"

`architecture.md` §4 rule 4 allows a moved item exactly two edits: a `pub(super)` prefix and
whatever `cargo fmt` does. `cf_sidebar.rs`'s `CfRowControls` now carries four added `//` lines
explaining why `edit`/`delete` are `pub(super)` while `move_up`/`move_down` are not.

That is a rule-4 violation, and it landed in the same change that *restored* a doc comment for
breaching the same rule (§6f). Kept anyway, for one reason: mixed visibility across a four-field
POD reads as an oversight without it, and the next reader's instinct is to "fix" it by widening
the other two — which is the exact over-widening review caught twice (here and `BodyStub` in
P1). A comment that prevents a regression is worth more than byte-parity on a four-line struct.

Recorded rather than left silent, because the value of rule 4 is that deviations are declared.

## 6f. Three doc comments were edited during moves; two reverted, one kept

`functional_spec.md` §5 permits `//!` module headers only; `architecture.md` §4 rule 1 says
"never reflow a doc comment". Two moved items breached it, and neither was caught by a gate —
only by review:

- **P3** added a `///` to `two_sheets`, which had none. Reverted in `de47ded`.
- **P5** reworded and re-wrapped `cf_view`'s ("A published rule row" → "A published **CF** rule
  row"). Not rustfmt: `wrap_comments` is off, and the de-dent onto file scope can only shorten
  lines, never force a rewrap.

Both restored byte-for-byte.

A **third** breach exists and was deliberately kept: `mod.rs`'s `num_fmt_more_open` field doc had
its `` [`BASIC_FORMATS`] `` and `` [`NUM_FMT_GROUPS`] `` links re-qualified to full paths and the
paragraph re-wrapped, when those two imports moved to `formatting.rs` in the phase-6 review.
Reverting it would leave two dangling intra-doc links, so the edit is the lesser evil — but it
*is* a rule-1 breach, and it went unrecorded for a commit because the sweep that found the first
two was run before it existed. Recording it here rather than pretending the count is two.

(This is a different call from §6b, which leaves five *pre-existing* broken links alone: those
were broken before the split and fixing them is outside a move's remit. This one the split
itself would have created.)

Worth naming because this is the one class of edit that defeats machine verification: to a
normalised byte-comparison that strips comments, a reworded comment is invisible; to one that
doesn't, it is indistinguishable from a deliberate change. The check that caught both was a
comment-set diff against the pre-split file — worth running once per phase in any future split.

## 6g. One test is filed under the wrong domain, by the plan rather than by accident

`multiselect_disables_field` lives in `shell.rs` (`:1087`), among the horizontal-scroller cases.
It asserts `data_mode()`, `content_text()` and `ref_box_text()` — three accessors that are now
`editing.rs`'s — and touches nothing about the scroller. It is there because it was the last
test under the `// ---- Horizontal scroller` banner in the old file, so `architecture.md` §7.4's
range mapping sent it to `shell.rs`, and the phase followed the mapping correctly.

Left where it is: moving a test across modules on a domain judgement is not a pure move, and
this project's whole premise is that it doesn't make those. Recorded because it is the sharpest
concrete instance of §1's thesis — "the tests under this banner" was never a real boundary —
and because a future reader looking for the multiselect-disable behaviour will not find its
test next to the code.

## 6h. Nine single-domain constants stayed in `mod.rs`, deliberately

`architecture.md` §2.1's rule is that `mod.rs` keeps "the chrome look constants … used by more
than one domain". Nine are not: `SPINNER_DELAY`, `TOOLTIP_BG`, `TOOLTIP_TEXT`, `AUTOCOMPLETE_HL_BG` and
`AUTOCOMPLETE_MIN_W` (editing only), `STATS_DEBOUNCE` (stats only), and `TARGET_ICON_PX`,
`TARGET_ICON_GREY`, `TARGET_ICON_DARK` (formatting only — they exist for
`border_target_icon`).

They stayed because §2.1 contradicts itself here — its own parenthetical names `TOOLTIP_*` as a
`mod.rs` resident, and §7.1 routes the 76–122 constant range to `mod.rs` (seven of the nine;
`SPINNER_DELAY` and `STATS_DEBOUNCE` sit just above it, at baseline 70 and 74). With the plan
ambiguous and `mod.rs` at 571 lines, splitting a coherent look-constant block to satisfy one
reading of a rule that also says the opposite is not worth a diff.

Recorded rather than silently left, because the phase-7 review set the opposite precedent for
`REF_BOX_W`/`DATA_ROW_FIELD_H`/`DATA_ROW_CONTENT_LEFT` — there §2.1 and §7.1 agreed, so the
constants moved. The distinction is whether the plan is self-consistent, not whether the
constant is single-domain.

## 7. Three test leaf names are duplicated crate-wide

`point_count_matches_data`, `shared_domains_cover_all_points` and
`stacked_baselines_are_cumulative` each appear in more than one module. Harmless (full paths
disambiguate), and noted only because the verification tripwire compares leaf-name
**multisets** rather than sets to stay correct in their presence.

## 8. Files still over the F2 ceiling

F2 will add a CI check at 2,000 **production** lines. Every file this project produced is
under it (largest: `cf_editor.rs` at 1,794). A workspace-wide survey at the same commit finds
**five** files that are not. Both columns are `wc -l`; production is the line before the file's
first top-level `#[cfg(test)]`:

| Production | Total | File | Owner |
|---:|---:|---|---|
| 6,575 | 10,627 | `freecell-app/src/grid/view.rs` | unowned — see `projects/grid-view-split.md` |
| 3,984 | 9,288 | `freecell-engine/src/worker/run.rs` | **engine-worker-hardening** (parallel project) |
| 2,913 | 2,913 | `freecell-engine/tests/worker_seam.rs` | an integration test file — all "production" by the metric |
| 2,153 | 2,184 | `freecell-app/src/shell/window.rs` | unowned |
| 2,142 | 3,833 | `freecell-engine/src/document.rs` | unowned |

Two things F2 should decide before it lands, neither of which this project can settle:

- **`tests/worker_seam.rs` counts as 2,914 production lines** because it is a `tests/` file
  with no `#[cfg(test)]` inside it — the whole file *is* the test. A rule that excludes
  `#[cfg(test)]` blocks but not `tests/` directories will flag it. Same for
  `view/test_support.rs` here (369 lines, entirely a
  `#[cfg(test)]` module, but the attribute is on the `mod` declaration in the parent, not in
  the file).
- **`grid/view.rs` at 6,575 needs its own unit before F2 can be enforced**, or an exemption.
  Filed as `projects/grid-view-split.md`.
