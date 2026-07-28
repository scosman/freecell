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

It sits under the `// ---- Action row: formatting` banner and moved to `formatting.rs` with
it, but it is called from formatting, charts **and** conditional formatting — every mutating
control commits a pending edit first and bails if the commit is blocked. It is now
`pub(super)`.

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

## 6a. `mod.rs`'s import block accumulated 45 single-child orphans — clippy cannot see them

Architecture §3.3 predicted this and made it a **manual** per-phase check: "imports that end up
used by only one child get moved into that child. This is checked per-phase precisely because
it accumulates silently otherwise." That check was never run during the original pass. The
phase-2 code review caught three instances; sweeping the whole block found the scale.

At the point the phase-2 review landed, `mod.rs` imports **120** names — 116 by name plus the
four `as _` traits noted below. Of those, **45 are consumed by exactly one child** and belong in
that child per §3.3; **32 are consumed by two or more** and correctly stay (`ClickEvent` and
`Command` reach eight children each, `Button` seven, `WorkerEvent` five in code and six counting
`formatting.rs`'s intra-doc link — duplicating those into eight files would be strictly worse
than the glob, which is what the shared block is *for*); the remaining 38 `mod.rs` uses itself,
plus the one `pub use` re-export (`ChartPanelSeries`), which is already in its child and cannot
move.

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

That leaves **24**, created by phases 6–8 and moving with those phases' reviews: 15 to
`formatting.rs` (P6), 8 to `editing.rs` (P7), 1 to `shell.rs` (P8). `cf_editor.rs`,
`cf_sidebar.rs`, `charts.rs`, `find.rs`, `stats.rs` and `tabs.rs` are now clear. Landing them
all in one commit labelled for an earlier phase would destroy exactly the per-phase attribution
§4.1 exists to protect.

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

## 6f. Two doc comments were edited during moves, both reverted

`functional_spec.md` §5 permits `//!` module headers only; `architecture.md` §4 rule 1 says
"never reflow a doc comment". Two moved items breached it, and neither was caught by a gate —
only by review:

- **P3** added a `///` to `two_sheets`, which had none. Reverted in `de47ded`.
- **P5** reworded and re-wrapped `cf_view`'s ("A published rule row" → "A published **CF** rule
  row"). Not rustfmt: `wrap_comments` is off, and the de-dent onto file scope can only shorten
  lines, never force a rewrap.

Both restored byte-for-byte. A comment-parity sweep of all eleven files against the baseline's
comment set then found no others, so these were the only two.

Worth naming because this is the one class of edit that defeats machine verification: to a
normalised byte-comparison that strips comments, a reworded comment is invisible; to one that
doesn't, it is indistinguishable from a deliberate change. The check that caught both was a
comment-set diff against the pre-split file — worth running once per phase in any future split.

## 7. Three test leaf names are duplicated crate-wide

`point_count_matches_data`, `shared_domains_cover_all_points` and
`stacked_baselines_are_cumulative` each appear in more than one module. Harmless (full paths
disambiguate), and noted only because the verification tripwire compares leaf-name
**multisets** rather than sets to stay correct in their presence.

## 8. Files still over the F2 ceiling

F2 will add a CI check at 2,000 **production** lines. Every file this project produced is
under it (largest: `cf_editor.rs` at 1,794). A workspace-wide survey at the same commit finds
**five** files that are not:

| Production | Total | File | Owner |
|---:|---:|---|---|
| 6,575 | 10,628 | `freecell-app/src/grid/view.rs` | unowned — see `projects/grid-view-split.md` |
| 3,984 | 9,289 | `freecell-engine/src/worker/run.rs` | **engine-worker-hardening** (parallel project) |
| 2,914 | 2,914 | `freecell-engine/tests/worker_seam.rs` | an integration test file — all "production" by the metric |
| 2,153 | 2,185 | `freecell-app/src/shell/window.rs` | unowned |
| 2,142 | 3,834 | `freecell-engine/src/document.rs` | unowned |

Two things F2 should decide before it lands, neither of which this project can settle:

- **`tests/worker_seam.rs` counts as 2,914 production lines** because it is a `tests/` file
  with no `#[cfg(test)]` inside it — the whole file *is* the test. A rule that excludes
  `#[cfg(test)]` blocks but not `tests/` directories will flag it. Same for
  `view/test_support.rs` here (370 lines on this table's `wc -l`+1 convention, entirely a
  `#[cfg(test)]` module, but the attribute is on the `mod` declaration in the parent, not in
  the file).
- **`grid/view.rs` at 6,575 needs its own unit before F2 can be enforced**, or an exemption.
  Filed as `projects/grid-view-split.md`.
