---
status: draft
---

# Phase 3: Re-pin + engine facade + link cache

## Overview

FreeCell's first hyperlinks phase. It moves the app onto the fork tip that carries the
upstream links engine (Phase 1) and the batch link-diffs fix (Phase 2), then exposes links
through the engine seam: an engine-free [`LinkTarget`]/[`CellLink`] in `freecell-core`, the
three `WorkbookDocument` accessors, two undoable `Command`s, and a **sparse per-sheet link map**
on `SheetCache` kept in agreement with the engine through all three cache paths (build, mirror,
undo/redo touch-set).

Nothing is visible yet — no rendering, no clicking, no authoring. Phase 4 reads the cache; this
phase is what makes that read trustworthy.

**Dropped from the architecture's plan (arch §3.5):** the `LINK` substitution in
`render_style_from` for `Color::Theme(10, _)`. The Phase-1 verification gate came back **YES** —
theme index 10 is the Office `hlink` slot and resolves to `#0563C1` through FreeCell's existing
`resolve_rgb` — so worksheet links get their blue+underline from ordinary engine formatting with
zero FreeCell-side styling (F1.2). Dynamic-link styling (F1.3) is a render-time concern and lands
in Phase 4.

## Steps

### 1. Re-pin the fork

`app/Cargo.toml` pins `ironcalc`/`ironcalc_base` by **exact version** and patches them to the
fork's `freecell-fixes` branch. The Phase-1 upstream sync carried a version bump (`0.7.1` →
`0.8.3`), so the exact pin no longer matches the branch and cargo **silently drops the patch**
(`warning: patch … was not used in the crate graph`) and builds crates.io `0.7.1` instead. Bump
both pins to `=0.8.3` with a comment recording the coupling, then `cargo update -p ironcalc -p
ironcalc_base` so `Cargo.lock` names `195f11e6` (the Phase-2 fork tip).

Expect upstream drift to reconcile on the FreeCell side (`CLAUDE.md`: "expect incidental drift").
It amounted to exactly one line: upstream accepted our `set_worksheet_index` contribution under
the name `UserModel::move_sheet`, so `document.rs::move_sheet` calls the new name.

### 2. `LinkTarget` + `CellLink` — `freecell-core/src/link.rs`

A new module, re-exported at the crate root. `freecell-core` forbids IronCalc (its
`tests/dependency_rule.rs`), so this is the mirror of `ironcalc_base::types::Link`, converted at
the `freecell-engine` boundary exactly as `CfRuleView` already is.

```rust
pub enum LinkTarget {
    External { target: String, tooltip: Option<String> },
    Internal { location: String, tooltip: Option<String> },
}

pub struct CellLink { pub target: LinkTarget, pub dynamic: bool }
```

**Deviation from arch §3.2/§3.3** (`links: HashMap<(u32,u32), LinkTarget>`): the cached value is
a `CellLink`, i.e. the target **plus** the engine's `dynamic` flag. F6.1/F6.3 gate the Remove
affordance on the link being a *worksheet* link, and the Phase-5 dialog pre-fills synchronously
from the cache — an async round-trip to the worker to ask "is this dynamic?" would be a
regression on a dialog that must open filled in. Carrying the flag now costs one `bool` and
avoids re-threading all three agreement paths in Phase 5.

Accessors (`destination`, `tooltip`, `is_internal`) so downstream code never matches the enum
just to read the string.

### 3. `SheetCache.links` — `freecell-core/src/cache.rs`

A sparse map shaped like `merges`, **not** a `RenderStyle` field (arch §3.3: `RenderStyle` is
interned, so a link field would give every distinct link its own `StyleId`).

```rust
links: HashMap<(u32, u32), CellLink>,
```

- `SheetCache::cell_link(row, col) -> Option<&CellLink>` — the per-cell read Phase 4 uses.
- `SheetCache::links() -> &HashMap<(u32,u32), CellLink>` — the whole map, for the
  `resolve_frame` snapshot.
- `SheetCache::set_links(map) -> bool` — the **mirror primitive**: replaces the map wholesale,
  returning whether it changed (so the worker only emits `StyleCacheUpdated` on a real change).
- `SheetCacheBuilder::push_link` / `.link(...)` (non-consuming + fluent, matching `push_merge` /
  `.merge(...)`).

### 4. `WorkbookDocument` — `freecell-engine/src/document.rs`

Free converters `link_target_from(&Link) -> LinkTarget` / `link_from(&LinkTarget) -> Link` beside
the other engine↔core conversions, then three methods (each opening with
`crate::instrument::record_engine_call()`):

```rust
pub fn cell_link(&self, sheet: u32, cell: CellRef) -> Result<Option<CellLink>, CellQueryError>;
pub(crate) fn sheet_links(&self, sheet_idx: u32)
    -> Result<HashMap<(u32, u32), CellLink>, String>;
pub(crate) fn set_cell_link(&mut self, sheet_idx: u32, cell: CellRef,
                            target: &LinkTarget, label: Option<&str>) -> Result<LinkWrite, String>;
pub(crate) fn remove_cell_link(&mut self, sheet_idx: u32, cell: CellRef) -> Result<bool, String>;
```

- `cell_link` is the **effective** link (worksheet wins, else dynamic). Upstream has no per-cell
  call covering both — `get_cell_link` is worksheet-only — so it reads the worksheet link first
  and falls back to scanning the sheet's union list. Returns `Option<CellLink>` rather than the
  arch's `Option<LinkTarget>` for the reason in step 2. `pub` (like `cell_content`) because it is
  the agreement contract's fresh-re-read oracle and the round-trip tests' reader.
- `sheet_links` is the cache's source: `get_links_list(sheet)` (the union, worksheet-wins) keyed
  0-based.
- **`LinkWrite`** is the load-bearing return type. `UserModel::set_cell_link` **suppresses no-ops**
  (same link + same label ⇒ no `push_diff_list` ⇒ **no IronCalc undo entry**), and it also
  **writes the cell's value** when `label` differs from the current display text. The worker-side
  touch history is 1:1 with IronCalc's undo stack, so the facade must report which of the three
  happened rather than always claiming "applied":

  ```rust
  pub(crate) enum LinkWrite { NoOp, LinkOnly, LabelChanged }
  ```

  The facade mirrors the engine's own conditions (link differs ‖ label differs) *before* calling,
  which is the only way to know — the engine returns `()`.
- `remove_cell_link` returns `false` when the cell had no link (`delete_cell_link` is likewise a
  diff-free no-op there).

### 5. `Command::{SetCellLink, RemoveCellLink}` — `worker/protocol.rs` + `worker/run.rs`

```rust
SetCellLink { sheet: SheetId, cell: CellRef, target: LinkTarget, label: Option<String> },
RemoveCellLink { sheet: SheetId, cell: CellRef },
```

- **Bucketing** (`run.rs` drain loop): the **edits** arm — they are undoable and ride the
  coalesced eval + publish + undo/redo machinery.
- **`apply_one`**: `SetCellLink` maps `LinkWrite::NoOp → AppliedKind::NoOp`, `LinkOnly →
  StyleOnly`, `LabelChanged → Cell`. Arch §3.4 says "`StyleOnly` — a link never changes a value",
  which is true of the link itself but **not** of `label` (F5.3: the label is the cell's content,
  set in the same undo step) — a `LabelChanged` write must force the batch's recompute.
  `RemoveCellLink` maps `false → NoOp`, `true → StyleOnly`.
- **`op_of`**: `AppliedOp::Cells { sheet, range: CellRange::single(cell) }`, like `SetStyleAttr` —
  so the mirror re-reads the cell's style (the engine's blue+underline) and the undo/redo
  touch-set re-read covers it.
- **`pre_validate` — the abort-class gate.** `SetCellLink`'s `label` is written with
  `Model::set_user_input`, so it reaches IronCalc's *recursive* formula parser exactly as a typed
  formula does, and must go through `validate_input` alongside `Command::SetCellInput`. This is not
  belt-and-braces: the parser has no depth cap and an overflow is a **SIGABRT that `catch_unwind`
  cannot catch**, which kills the process and the user's unsaved workbook. Verified directly during
  this phase — a throwaway probe calling `set_cell_link` with a depth-490 label produced "fatal
  runtime error: stack overflow, aborting" (signal 6). Not user-reachable until Phase 5's ⌘K
  dialog, but the fix belongs with the command that opens the hole. The standing rule: **every
  command that can carry user text into `set_user_input` must appear in that gate.**

### 6. The three agreement paths — `freecell-engine/src/cache.rs` + `worker/run.rs`

The load-bearing contract (`freecell-engine/src/lib.rs`): the mirrored cache must provably agree
with a fresh engine re-read. Links need all three paths, and they are **whole-sheet**, not
per-cell — `get_links_list` is a per-sheet union and dynamic links are rebuilt from scratch on
*every* evaluation.

1. **Build** — `build_sheet_cache` pushes `doc.sheet_links(sheet_idx)` into the builder, beside
   the merges. This also covers every **full rebuild** (structural edits, CF refresh,
   sheet activation).
2. **Mirror** — `cache::changed_links(doc, sheet_idx, cache, evaluated)`, driven from
   **`Worker::refresh_cache_cells`** — the single chokepoint every per-cell mirror flows through
   (the edit batch, paste, replace, and `SetFont` all call it). It refreshes **every resident
   sheet**, not just the touched ones: a `HYPERLINK()` on Sheet1 can be re-created by an
   evaluation an edit on Sheet2 triggered, so a touched-sheet-only refresh would go stale.
   Free on a link-free workbook (both maps empty ⇒ no change ⇒ no event).

   Because it runs on **every** mirror — every keystroke, paste, undo — and F10.4 supports a
   10 000-cell range hyperlink import, the shape is cost-driven: compare **in place** against the
   engine's list and materialize the map only on a real change (by move, not clone); collapse to
   the allocation-free `worksheet_links_match` when the batch did not evaluate (no evaluation ⇒ no
   dynamic link can have moved; `evaluated: true` is the conservative default); and compute every
   resident sheet's update under the shared read lock before installing them all under **one**
   write lock, rather than taking the render thread's contended lock per sheet.
3. **Undo/redo touch-set** — falls out of (2): `Command::Undo`/`Redo` route through
   `apply_edit_batch` → `apply_cache_refresh` → `refresh_cache_cells`.

`assert_cache_agrees` (the test-only re-read oracle) gains a link comparison over the probe grid,
so every existing agreement test now also proves link agreement.

### 7. Fixture + xlsx round-trip — `fixtures.rs`, `tests/roundtrip.rs`

`fixtures::links()`: an external link, an external link **with a tooltip** (Excel ScreenTip), an
internal link (`Sheet1!A30`), an auto-linked typed URL, and a `=HYPERLINK(...)` dynamic link.
Built through the same public engine APIs the app uses, so a saved fixture is a faithful `.xlsx`.

`tests/roundtrip.rs` gains the F10 cases, reading back through the public `cell_link`.

## Tests

**`freecell-core` (`link.rs`, `cache.rs`)**

- `link_target_accessors_read_both_variants` — `destination`/`tooltip`/`is_internal` on both
  variants, with and without a tooltip.
- `links_round_trip_through_builder` — builder-pushed links survive `build()` and read back per
  cell; a fresh cache has none.
- `set_links_reports_whether_the_map_changed` — the mirror primitive's changed-flag is what gates
  `StyleCacheUpdated`: replacing with an equal map returns `false`, a different target / a
  changed `dynamic` flag / an added or removed cell returns `true`.

**`freecell-engine` unit (`document.rs`)**

- `cell_link_reads_a_worksheet_link` — set → read back target + tooltip, `dynamic == false`.
- `cell_link_reads_a_dynamic_hyperlink` — `=HYPERLINK("http://x","label")` → `dynamic == true`,
  and the cell **displays** the friendly name (F9.1).
- `cell_link_prefers_the_worksheet_link_over_the_dynamic_one` — both on one cell; worksheet wins
  (the engine's documented precedence, which the cache inherits).
- `cell_link_is_none_on_a_plain_cell`.
- `set_cell_link_with_a_label_sets_the_cell_content_in_one_undo_step` — F5.3: one `undo()`
  reverts **both** the link and the label.
- `set_cell_link_reports_no_op_when_nothing_changes` — the same link + the same label twice ⇒
  `LinkWrite::NoOp`, and `can_undo()` is unchanged. This is the property that keeps the worker's
  touch stack aligned with IronCalc's.
- `set_cell_link_reports_label_changed_only_when_the_label_differs` — `LinkOnly` vs
  `LabelChanged`, since only the latter needs a recompute.
- `remove_cell_link_reports_whether_there_was_one` — `true` then `false`; the second call records
  no undo entry.
- `remove_cell_link_cannot_remove_a_dynamic_link` — F6.3.
- `remove_cell_link_keeps_the_content_and_the_link_formatting` — F1.6/F6.2: content and the
  blue+underline stay.
- `a_reopened_hyperlink_formula_recreates_its_link` — the half of the round-trip story that needs
  the crate-private `evaluate`: nothing about a dynamic link is persisted, and the first recompute
  on the reopened workbook brings it back.
- `engine_link_styling_resolves_to_blue` — pins the Phase-1 gate that this phase's design rests
  on: after `set_cell_link`, `render_style_from` gives `underline == true` and a **blue**
  `font_color` (`resolve_rgb` on `Color::Theme(10, 0.0)`), with no FreeCell-side substitution. If
  this ever fails, arch §3.5's `LINK` substitution is back on the table.
- `sheet_links_returns_the_worksheet_and_dynamic_union`.

**`freecell-engine` unit (`cache.rs`)**

- `build_sheet_cache_populates_links` — worksheet + dynamic links land in the built cache with
  the right `dynamic` flags; a link-free sheet builds an empty map.
- `refresh_links_mirrors_a_new_link` / `..._a_removed_link` / `..._a_dynamic_link_re_evaluation`
  — the mirror primitive against each of the three ways a link map can move, each asserting the
  returned changed-flag too.
- `the_no_eval_fast_path_still_sees_every_worksheet_link_change` — the soundness condition of the
  allocation-free path, walked over all four ways a worksheet link moves (add / retarget /
  **tooltip-only** / remove). Tooltip-only is the case a target-only comparison would miss.
- `the_no_eval_fast_path_does_not_disturb_a_cached_dynamic_link` — the fast path compares only the
  worksheet half, so it must leave a cached dynamic entry alone rather than treating it as a
  divergence.
- `link_agreement_survives_build_and_mirror` — the extended `assert_cache_agrees` over a
  link-carrying fixture, on both paths.
- `negative_control_skipping_the_link_mirror_diverges` / `..._catches_a_retarget` — the
  **negative controls** for the extended oracle: change the engine's links without refreshing and
  the contract must `Err`, for a *removal* and for a *retarget* (a presence-only comparison would
  pass the second). Without these the new assertion could be vacuous.

**`freecell-engine` worker (`run.rs` inline)**

- `set_cell_link_command_populates_the_cache_and_styles_the_cell` — end to end through the worker:
  the resident cache's link map carries it, the cell's `RenderStyle` is underlined + coloured (the
  engine's own styling, mirrored by the existing style path), and a `StyleCacheUpdated` ships.
- `remove_cell_link_command_clears_the_cache_entry`.
- `undo_redo_of_a_link_edit_restores_the_cache` — the undo/redo touch-set path: undo drops the
  link from the cache, redo puts it back.
- `a_no_op_link_command_records_no_undo_entry` — the alignment property: a repeated identical
  `SetCellLink` and a `RemoveCellLink` on an unlinked cell commit no op and push no touch entry,
  so one `Undo` lands **link-free** rather than one step behind.
- `a_link_label_edit_recomputes_dependent_formulas` — the `LabelChanged → AppliedKind::Cell`
  mapping: `StyleOnly` would leave `=A1` stale.
- `typing_a_url_auto_links_and_undo_removes_it` — F7.1 + F7.5 end to end over the fork fix.
- `a_dynamic_link_follows_a_re_evaluation_of_its_argument` — the reason the mirror is
  whole-sheet: editing A2 changes A1's `=HYPERLINK(A2)` link.
- `a_dynamic_link_on_another_sheet_follows_a_cross_sheet_edit` — the reason the mirror covers
  **all resident** sheets, not just the touched one.
- `clearing_a_cell_clears_its_link` (F8.5) and `an_internal_paste_carries_the_link_into_the_cache`
  (F8.3) — engine behaviours the cache must follow; the paste one also proves the mirror
  chokepoint covers the clipboard path, which never enters `apply_edit_batch`.
- `a_link_survives_a_row_insert_that_displaces_it` (F8.1) — and, being a structural edit, proves
  the **full-rebuild** path carries links too.
- `a_degraded_worker_refuses_link_edits` — links inherit the degraded gate (F5.9 leans on it).
- `a_link_label_goes_through_the_input_cap_gate` — the **abort-class** guard: a link's `label` is
  written with `set_user_input`, so it reaches IronCalc's recursive parser and needs the same cap
  as a typed formula. Over-cap labels (depth and length) are rejected and reach neither the engine
  nor the undo stack, while a long *plain* label still applies (the cap is scoped to formulas).
  The crash itself cannot be asserted — it is a SIGABRT — but it was verified directly with a
  throwaway probe: a depth-490 label aborts the process (signal 6).
- `undo_of_sheet_delete_drops_the_sheets_links` — **`#[ignore]`d**, an executable record of the
  engine hole described under *Carried forward*.

**`freecell-engine` integration (`tests/roundtrip.rs`, F10)**

- `roundtrip_links_preserved` — F10.1/F10.2/F10.3: external, tooltip, `mailto:`, internal, and
  auto-created links survive save → reopen with their targets, tooltips, and kinds intact.
- `roundtrip_link_formatting_preserved` — the reopened link colour is **identical to the live
  one** (a differential, so it pins preservation rather than "some colour survived") and is a
  blue, with an unlinked control cell carrying none. The **underline** half is unreachable from an
  integration test (`published_style` carries colour only), so it is covered in-crate by
  `a_reopened_link_keeps_its_underline_and_blue`.
- `roundtrip_dynamic_link_is_not_persisted` — a `HYPERLINK()` cell writes no link at all; only
  the formula persists (the re-creation half is the `document.rs` test above).

## Deferred / not in this phase

- Anything visible: rendering, cursor, tooltip, click-to-open, `decide_open`, ⌘K, menus, toolbar
  (Phases 4–5).
- `fix/undo-set-cell-style-clears-inherited` — Phase 2's carried-forward upstream bug (undo of an
  auto-link materializes `Style::default()` over an inherited row/column style). FreeCell starts
  hitting it visibly in Phase 4; capturing it in `PROJECTS.md` / `GAPS.md` is Phase 6's job.

## Carried forward

- **Undo of a sheet delete drops that sheet's hyperlinks** — a pure engine hole this phase makes
  *matter*. `Diff::DeleteSheet` snapshots the whole worksheet into `old_data` (a `Box<Worksheet>`,
  so `links` is right there), but the undo arm in `base/src/user_model/undo_redo.rs` restores nine
  fields and copies back neither `links` nor `conditional_formatting`. Recorded as the
  `#[ignore]`d, executable `undo_of_sheet_delete_drops_the_sheets_links` (it fails precisely on the
  link assertion — the cell content *is* restored, so it isolates the hole rather than the undo).
  It is the exact twin of the pre-existing `undo_of_sheet_delete_repopulates_cf_map`: **one** fork
  fix — "undo of `Diff::DeleteSheet` restores the full worksheet" — is a candidate to close both.
  No `fix/` branch opened here; that is a separate upstream contribution for the owner to place.
  Phase 6 carries the `GAPS.md` / `PROJECTS.md` row.

  While confirming this I found the **CF twin's `#[ignore]` note was wrong**: it claimed the engine
  fix "is committed on the fork's `freecell-fixes` branch" and said to re-pin then un-ignore. The
  re-pin has now happened and the test still fails, because the fix was never pushed. The note is
  corrected in place to describe the verified reality and to point at the shared root cause.

- **A complete "no links anywhere" fast gate needs a small fork accessor.** The CF precedent
  short-circuits on an empty published map; the link equivalent can only do the *worksheet* half
  for free (`Model::get_links` returns a reference). The dynamic half lives in `Model::links`,
  which is `pub(crate)` with no public accessor, so `has_dynamic_links()` / `get_dynamic_links()`
  is a candidate fork addition. Everything achievable without it is done (see below).

## Found while building

- **The exact version pin silently disabled the fork patch.** `app/Cargo.toml` pinned
  `ironcalc = "=0.7.1"` while `freecell-fixes` is now `0.8.3`, so cargo emitted only a *warning*
  ("patch … was not used in the crate graph") and resolved crates.io `0.7.1` — i.e. a re-pin that
  looks like it worked but silently drops every fork fix. Fixed by bumping both pins, with a
  comment recording that the two must move together. Worth knowing for the next fork sync.
- **One drift point in the whole fork sync**: upstream took our `set_worksheet_index` PR and
  renamed it `UserModel::move_sheet`. A one-line rename in `document.rs::move_sheet`; nothing else
  in FreeCell failed to compile against the 0.7.1 → 0.8.3 jump.
- **`undo_of_sheet_delete_repopulates_cf_map` stays `#[ignore]`d.** Its note says to un-ignore
  after a re-pin, but the fork CF-undo fix it waits on was never pushed — `freecell-fixes`
  `195f11e6` still lacks the `Diff::DeleteSheet` CF restore, and the test still fails when run
  with `--ignored`. The note remains accurate; out of scope here.
- **Two environment gaps, neither caused by this change.** Linking any gpui binary needs
  `libxkbcommon-dev` / `libxkbcommon-x11-dev`, absent from a fresh container (the crate *libs* and
  all tests build fine without them; only the `freecell` and `render-tests` **binaries** fail to
  link). And `tests/charts_roundtrip_libreoffice.rs` fails here because the container's
  LibreOffice cannot load any `.xlsx` — reproduced by running `soffice --convert-to xlsx` on the
  committed, unmodified fixture.
