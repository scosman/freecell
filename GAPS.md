# FreeCell — Known Gaps & Limitations

A durable, running log of known gaps, deferred behaviors, and limitations across
FreeCell — so nothing gets lost between phases. **Append new gaps here** as they're
discovered; keep each entry short with a pointer to detail where one exists.

- This is a *log of gaps* (things that are missing / partial / deferred).
- For forward-looking **initiatives and design notes**, see [`PROJECTS.md`](PROJECTS.md)
  and [`projects/`](projects/). A gap here often has a matching `projects/*.md` note with
  the full work plan; link it.
- Spec-driven **build artifacts** (per-phase coverage, decisions) live under
  [`specs/projects/`](specs/projects/).

**Adding an entry:** append a row to the relevant section's table (or start a new
section). Give it: what's missing, where the spec/expectation is, severity, current
behavior, root cause, and a home (a `projects/*.md` note, or inline detail if small).
Don't silently drop a gap — record it here first.

---

## MVP — deferred functional-spec behaviors

Deferred from the MVP build (2026-07-04). The MVP is a functional proof of concept
(`specs/projects/mvp/functional_spec.md` §1: "not design-polished"). **None of these are
calculation gaps** — values, number formats, and error results all render correctly;
these are presentation / entry-point behaviors consciously deferred. Each also appears in
`specs/projects/mvp/coverage_matrix.md` (per-behavior map) and
`specs/projects/mvp/DECISIONS_TO_REVIEW.md`.

| # | Behavior | Spec | Severity | Current behavior | Root cause | Detailed home |
|---|----------|------|----------|------------------|------------|---------------|
| 1 | **Type-based default cell alignment** — numbers/dates right, booleans/errors center | §3.6 | Moderate | Grid defaults **every** cell to left; *explicit* alignment still renders correctly | `PublishedCell` carries only a display string, no value type | [`projects/type-aware-alignment.md`](projects/type-aware-alignment.md) |
| 2 | **`[Red]` number-format text color** | §3.6 | Mild | `PublishedCell.text_color` always published `None`; negatives render default color | Worker doesn't publish resolved per-cell color | [`projects/type-aware-alignment.md`](projects/type-aware-alignment.md) |
| 3 | **Input-cap rejection message text** — "Formula too long / too deeply nested" popover | §3.3 | Mild | Danger **border** only; the *safety* behavior (reject, keep focus, cell unmodified) is implemented and tested — just no human-readable reason shown | `DataRowEffect::ShowCapError` is a no-op in the chrome; message-popover not built | *inline below* |
| 4 | **macOS Finder open-file** — double-click / `open -a` / drag-onto-Dock | §2.1 | Moderate | Only the **CLI-argv** open path is wired; the primary-platform "double-click a file" flow does not open it | Pinned gpui rev's `on_open_urls` callback lacks a context (`cx`) arg | *inline below* |
| 5 | **Bundled Inter font** — ship Inter via `add_fonts` at startup | §3.3/§3.6 | Nicety (not a functional gap) | App renders on the platform default font; render baselines pinned to the CI runner image | Fonts not vendored; `register_fonts` is a documented no-op | [`projects/bundled-inter-font.md`](projects/bundled-inter-font.md) |

### Detail for the two without a dedicated note

**#3 — Input-cap rejection message text (§3.3).**
Today an over-cap edit (formula length > 8192 chars or paren-depth > 64) is rejected at
both the `freecell-core::input_cap` validator and the worker-side re-check; the data row
shows a red danger border and keeps the user in edit mode with the cell unmodified
(tested: `chrome/view.rs::cap_reject_keeps_editing_and_flags_error`, plus
`input_cap.rs` unit tests). What's missing is the specced inline message-popover text
telling the user *why* the input bounced. Work when picked up: wire
`DataRowEffect::ShowCapError` to render a tooltip-style popover below the content field
with the reason string (length vs depth). Small, chrome-local change.

**#4 — macOS Finder open-file (§2.1).**
`main.rs` wires only `xlsx_arg` (CLI argv). Opening a `.xlsx` from Finder
(double-click, drag onto the Dock icon, `open -a FreeCell book.xlsx`) does nothing on
macOS — the primary design target. The pinned gpui rev's `on_open_urls` callback
signature lacks the `cx` needed to route the open through `FreeCellApp`. Work when
picked up: this is likely a **spike** first — check whether a newer gpui rev gives
`on_open_urls` a context arg (or an alternative hook), or bridge via an app-global +
deferred dispatch; then map the incoming URLs through the existing `do_open_path`
(canonicalize → dedupe → open) that CLI-argv already uses. Verify on real macOS
(smoke item **M-15** in `specs/projects/mvp/smoke_checklist.md`).

### Intentional MVP scope exclusions (NOT gaps — deliberate, listed for completeness)

- **Silent `.xlsx` fidelity strip on save, no warning** (§5.2) — intentional MVP
  decision; the warn-and-strip UX is [`projects/xlsx-preservation.md`](projects/xlsx-preservation.md).
- **Dynamic arrays / spill absent** (§8) — accepted absent for v1; the engine surfaces
  an error. Out of MVP scope by product call.

### When picking these up

Items #1 and #2 share a root cause (the publication carries no per-cell type/color) and
should be done together — see [`projects/type-aware-alignment.md`](projects/type-aware-alignment.md)
for the publication → grid threading plan. #3 is a small chrome-local change. #4 needs a
gpui-capability spike before estimating. None are blocked by the others.

---

## Engine (IronCalc 0.7.1) — `.xlsx` import fidelity bugs

Bugs in the pinned IronCalc's **import** path, found while opening real Excel/Numbers files
(a mortgage calculator with a custom purple theme + accounting number formats; a Numbers
export with a custom indexed colour palette). IronCalc evaluates every formula correctly;
these are **import/presentation** defects — except E4, which is a hard **parse rejection**
that stops the file opening at all. FreeCell corrects the common cases at open time in two
best-effort adapters, both driven from `WorkbookDocument::open`:
[`open_repair`](app/crates/freecell-engine/src/open_repair.rs) runs **pre-parse** (repairs
the bytes and retries on a specific parse failure) and
[`open_fixups`](app/crates/freecell-engine/src/open_fixups.rs) runs **post-parse** (corrects
the loaded model). Entries marked *worked around* are fixed for the observed cases; the
*residual* rows are the parts our fix does not cover.

| # | Bug (IronCalc) | Symptom | Our status | Detail |
|---|----------------|---------|------------|--------|
| E1 | **Theme colours resolved against a hardcoded default Office palette, ignoring the file's `xl/theme/theme1.xml`.** `import::colors::get_themed_color` uses a fixed 12-colour array and discards the theme index + tint, storing only the (wrong) resolved RGB. | Every theme-indexed fill/font colour is wrong. On this file (whose theme swaps `dk1`/`lt1` and uses a purple `dk2`) the purple header rendered navy, white label cells rendered solid black, lavender bands rendered blue-grey. | **Worked around (our bug fixed).** `open_fixups::correct_theme_colors` re-reads `theme1.xml` + `styles.xml`, recomputes each theme-indexed colour against the *file* palette (OOXML dark/light swap + §18.8.3 tint), and overwrites the resolved RGB. | Unit + crafted-zip tests in `open_fixups`. Verified end-to-end on the real file. |
| E2 | **`DEFAULT_NUM_FMTS` table (`ironcalc_base::number_format`) maps standard built-in `numFmtId`s to garbage codes** — e.g. id 39 (`#,##0.00_);(#,##0.00)`) → `"t0.00"`, ids 41–44 (accounting) → `"t0"/"t0.00"/…`. `get_formatted_cell_value` then returns **`#VALUE!`** for a perfectly valid number. | Currency/accounting/number cells show `#VALUE!` even though the underlying value is correct (proven: `NPER` over those cells returns the right number; raw `get_cell_value_by_index` is correct). On this file all loan/payment/total cells (id 39) showed `#VALUE!`. | **Worked around (our bug fixed).** `open_fixups::inject_builtin_num_fmts` injects the correct ECMA-376 built-in code (ids 5–8, 37–49) for ids the workbook references but does not itself define, so `get_num_fmt` picks it up ahead of the broken table. IronCalc's formatter handles the correct code fine. | Unit tests in `open_fixups`; values now match Excel's CSV export exactly. |
| E3 | **Residual: date/time and other built-in `numFmtId`s IronCalc mis-maps are not corrected.** Our E2 fix deliberately covers only the locale-independent numeric/currency/accounting/misc block (ids 5–8, 37–49). IronCalc's table is also wrong/garbage for ids 11–13 (spacing) and 23–36, and dates 14–22 are locale-sensitive. | A file relying on those specific built-in ids (rare vs. the E2 block) may still format wrong. Not seen in the test file. | **Tracked (IronCalc limitation).** Extend `STANDARD_BUILTIN_NUM_FMTS` if a real file needs it, or upstream a fix to IronCalc's `DEFAULT_NUM_FMTS`. | — |
| E4 | **Styles parser wrongly *requires* the optional `xfId` on every `<cellXfs>`/`<xf>`.** `import::styles.rs` reads it with `get_attribute(&xfs, "xfId")?` (mandatory), but per OOXML §18.8.10 `xfId` on a `cellXfs/xf` is **optional** (default: none). IronCalc reads it optionally on `cellStyleXfs` but not here. | The **entire open fails** with `XML Error: Missing "xfId" XML attribute` — the file never loads. Hits Numbers, LibreOffice, and various generators that omit `xfId`. Reproduced on the committed `numbers_table.xlsx` fixture (22 `<cellXfs>` `<xf>`, none with `xfId`). | **Worked around (our bug fixed).** `open_repair::try_repair_and_reload` runs *only* on this specific error: it re-reads the zip, injects `xfId="0"` into `<cellXfs>` `<xf>` elements that lack it (scoped strictly to the `cellXfs` block — `cellStyleXfs` untouched), and reloads from the repaired bytes via `load_from_xlsx_bytes`. Reactive, so files that already parse are untouched; on any repair/reload failure the original typed error is surfaced. | Unit + crafted-zip tests in `open_repair`; integration test opens the fixture through `WorkbookDocument::open` and asserts its values. |
| E5 | **Custom indexed colour palette ignored.** `import::colors::get_indexed_color` uses a hardcoded legacy default indexed palette and never reads the workbook's `<colors><indexedColors>` override (OOXML §18.8.27). So an `indexed="n"` fill/font colour resolves to Excel's *default* colour n, not the file's redefined colour n. | Fills/fonts/borders that use `indexed=` render the wrong colour. On `numbers_table.xlsx`: column A's light-grey label band (custom index 12 = `#DBDBDB`) renders **bright blue** (default 12 = `#0000FF`); the light-grey header band (custom index 9 = `#BDC0BF`) renders white (default 9 = `#FFFFFF`); the TOTAL cells render default index 13/14 (`#FFFF00` yellow / `#FF00FF` magenta) instead of the file's `#FFD931` gold / `#FE634D` red-orange. Values are unaffected — the file opens and all numbers/labels are correct. | **Worked around (our bug fixed).** `open_fixups::correct_indexed_colors` re-reads `<colors><indexedColors>` from `styles.xml` and — **only when the file supplies that override** — overwrites each `indexed=`-derived fill/font colour with the file's palette entry (walking `<fills>`/`<fonts>` by index exactly as `correct_theme_colors` does). Guards: out-of-range index and the system indices 64/65 (auto fg/bg) are left as IronCalc parsed them; explicit `rgb=`/`auto=`/`theme=` are untouched. A standard-palette file (no `<indexedColors>`) is left entirely to IronCalc, so it can never regress. | Unit + crafted-zip tests in `open_fixups`; a fixture colour-regression test opens `numbers_table.xlsx` and asserts the resolved fills (`#BDC0BF` header, `#DBDBDB` label column, `#FFD931`/`#FE634D` TOTALs) through the grid/cache accessor. Verified end-to-end via an Xvfb+lavapipe capture. **Residual:** `indexed=` **border** colours are not corrected — the MVP grid does not render borders, so there is no visual impact; only the save round-trip keeps IronCalc's (wrong) default border colour. |

**Upgrade note:** E1/E2/E4/E5 are compensations for IronCalc import bugs. If IronCalc is bumped to
a release that resolves file themes, reads the file's `<indexedColors>` palette, ships a correct
built-in number-format table, and accepts an `xfId`-less `cellXfs`, `open_fixups` + `open_repair`
(and the `zip`/`roxmltree` deps they add) can be deleted.

## Data safety & robustness

| Gap | Severity | Why it matters | Sketch |
|-----|----------|----------------|--------|
| **Save a `.back` backup before the first save** | High (we're alpha) | The save path can lose data (IronCalc's writer silently strips anything it doesn't model; we're early and bugs are likely). A one-time backup of the original bytes means an overwrite can never be the *only* copy. | Before the **first** save of a document opened from disk, copy the original file to `filename.xlsx.back` (write-once — do **not** re-back-up / overwrite the backup on subsequent saves, so the backup always holds the pristine original). Applies to files opened from disk; a never-saved new workbook has nothing to back up. **Picked up in `specs/projects/mvp-gaps`.** |

---

## Post-MVP UX features — surveyed for `mvp-gaps`, deferred (2026-07-04)

Candidate features surveyed while scoping the **`specs/projects/mvp-gaps`** project (the
first post-MVP gap-closing round). These did **not** make that project's scope; recorded
here so they aren't lost. All were deliberate MVP scope cuts (`functional_spec.md §8`),
now re-triaged. Severity here = product gap vs. a "real spreadsheet app", not a defect.

| Feature | Severity | Notes |
|---|---|---|
| **Grid cell right-click context menu** (cut/copy/paste/clear/…) | Moderate | Cheap once range clipboard lands (it's in `mvp-gaps`). Note: a *header* right-click menu for insert/delete rows/cols **is** in `mvp-gaps`; this row is the general cell-area menu. |
| **Fill down/right (Cmd+D / Cmd+R) + drag fill handle** | Moderate | Fill-down/right is small once range ops exist; the drag handle is the larger half. Engine support exists and is undoable: `UserModel::auto_fill_rows/auto_fill_columns` incl. sequence detection (verified in the 2026-07-04 0.7.1 audit) — cheap when picked up. |
| **Zoom control (sheet-area zoom dropdown)** | Moderate | Cut in `mvp-gaps` scope-back (punt pre-authorized): a scale factor cross-cutting perf-gated geometry (`Axis`, hit-testing, scrollbars), text sizing, and all pixel baselines — high blast radius for a mid-size win. |
| **Merged-cell rendering + selection ("tiers a+b")** | Moderate | Cut in `mvp-gaps` scope-back. Investigated and **ready to build with zero engine changes** (merges already round-trip open→save at 0.7.1); render-only without selection snapping is a UX trap, and the pair drags selection-fixpoint logic through delicate input code — deserves its own focused project. Full plan: [`projects/merged-cells.md`](projects/merged-cells.md). Meanwhile `mvp-gaps` ships a guard blocking insert/delete rows/cols that would displace merges. |
| **Find (Cmd+F) / replace** | Moderate | Find-only would cover most usage; replace adds engine-write fan-out. |
| **Autofit column width** (double-click header divider) | Mild | Pairs with the resize UI shipping in `mvp-gaps`; needs text measurement over the column's cells. |
| **Cmd+arrow jumps to edge-of-*sheet*, not edge-of-*data*** | Mild | MVP behavior (spec §3.2) is the nonstandard one; edge-of-data needs a cheap occupied-extent query. |
| **Recent files on Welcome window** | Mild | Spec'd out of MVP (§2.2); needs a small persisted MRU store. |
| **Freeze panes** | Moderate | Viewport-split rendering + scroll clamping in the custom grid — real complexity, defer until asked for. Engine side is trivial when picked up: `UserModel::set_frozen_rows_count/set_frozen_columns_count` exist and are undoable (2026-07-04 audit). |
| **Sort / filter** | Moderate | Large feature (engine ops + UI + selection semantics); own project when picked up. |
| **Text overflow into empty neighbors + wrap** | Moderate | Spec §3.6 clips at cell boundary; overflow needs neighbor-emptiness lookups on the render path, wrap needs row-height interaction. |
| **Merge/unmerge UI** ("tier c") | Moderate | Blocked on an IronCalc `UserModel` merge API (fork or upstream PR); *rendering* file-loaded merges is in `mvp-gaps`. See [`projects/merged-cells.md`](projects/merged-cells.md). |

### `mvp-gaps` — accepted behavior deviations (owner-approved 2026-07-04)

Product judgment calls baked into the `mvp-gaps` specs, reviewed and accepted at
planning sign-off. Each ships as specced; listed here so the follow-up path isn't
lost if one bites in practice.

| Deviation | Vs. Excel | Follow-up if needed |
|---|---|---|
| **Cut has no visual indicator** | Excel shows marching ants; Esc cancels a cut | Cmd+X looks like copy; source clears at paste time. Cheap cue later: dim the cut source range. |
| **Font family/size on full-row/col doesn't apply to future cells** | Excel sets a row/col-level font | No font band API at IronCalc 0.7.1 (`update_range_style` has no `font.name`/absolute-size path); we clamp to the used range via `on_paste_styles`. Fix = upstream a font band path, then swap the clamp for a band call. |
| **External TSV paste skips empty tokens instead of clearing cells** | Excel blanks the target cell | Engine `paste_csv_string` behavior. Fix = FreeCell pre-clears the target area (one extra undoable step) if this bites. |
| **`.back` backup failure blocks the save** | n/a (our feature) | Data-safety-wins call: "Couldn't create backup — file not saved." The annoying case (unwritable dir) mostly implies the atomic save would fail too. Could soften to warn-and-continue. |
| **No action-bar overflow; window min-width rises to fit the control row** | Excel ribbon collapses | Could feel restrictive on small/split screens. Fix = overflow menu for trailing groups. |
