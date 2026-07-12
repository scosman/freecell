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
| 1 | **Type-based default cell alignment** — numbers/dates right, booleans/errors center | §3.6 | Moderate | ✅ **Resolved (mvp-gaps Phase 1)** — `PublishedCell.kind` is published and the grid aligns by type when no explicit alignment is set; explicit alignment still wins | `PublishedCell` carries only a display string, no value type | [`projects/type-aware-alignment.md`](projects/type-aware-alignment.md) |
| 2 | **`[Red]` number-format text color** | §3.6 | Mild | ✅ **Resolved (mvp-gaps Phase 1)** — the worker resolves per-cell `text_color` (explicit font colour → number-format colour); `[Red]` negatives render red | Worker doesn't publish resolved per-cell color | [`projects/type-aware-alignment.md`](projects/type-aware-alignment.md) |
| 3 | **Input-cap rejection message text** — "Formula too long / too deeply nested" popover | §3.3 | Mild | ✅ **Resolved (mvp-gaps Phase 1 + 2)** — a tooltip-style popover shows the length/depth reason under the **data row** (Phase 1) and the **in-cell editor** (Phase 2); dismisses on the next keystroke/focus change | `DataRowEffect::ShowCapError` was a no-op in the chrome; message-popover not built | *inline below* |
| 4 | **macOS Finder open-file** — double-click / `open -a` / drag-onto-Dock | §2.1 | Moderate | Only the **CLI-argv** open path is wired; the primary-platform "double-click a file" flow does not open it | Pinned gpui rev's `on_open_urls` callback lacks a context (`cx`) arg | *inline below* |
| 5 | **Bundled Inter font** — ship Inter via `add_fonts` at startup | §3.3/§3.6 | Nicety (not a functional gap) | App renders on the platform default font; render baselines pinned to the CI runner image | Fonts not vendored; `register_fonts` is a documented no-op | [`projects/bundled-inter-font.md`](projects/bundled-inter-font.md) |

### Detail for the two without a dedicated note

**#3 — Input-cap rejection message text (§3.3). ✅ RESOLVED (mvp-gaps Phase 1 + 2).**
An over-cap edit (formula length > 8192 chars or paren-depth > 64) is rejected at both the
`freecell-core::input_cap` validator and the worker-side re-check; the data row shows a red
danger border and keeps the user in edit mode with the cell unmodified. mvp-gaps wired the
specced inline message-popover: `DataRowEffect::ShowCapError` now renders a tooltip-style
popover below the active editor with the reason string (length vs depth), on the **data row**
(Phase 1) and the **in-cell editor** (Phase 2), auto-dismissing on the next keystroke/focus
change (`chrome/view.rs` `cap_error*` state + `cap_error_visible()`, tested via
`edit_rejected_input_cap_flags_chrome_data_row` + the `input_cap.rs` unit tests).

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

Items **#1, #2, and #3 are RESOLVED** by the `specs/projects/mvp-gaps` build (Phases 1–2 —
publication type/color + type-aware alignment + the cap-error popover). **Still open:** #4
(macOS Finder open-file — needs a gpui-capability spike before estimating) and #5 (bundled
Inter font — a nicety, not a functional gap). Neither is blocked by the other.

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

**Upgrade note:** E1/E2/E4/E5 were compensations for IronCalc import bugs. **Resolved (2026-07):**
FreeCell now builds against our fork (`scosman/ironcalc#freecell-fixes`), which fixes all five —
`open_fixups` + `open_repair` (and the `zip`/`roxmltree` prod deps) were deleted. See
`specs/projects/ironcalc-upstreaming/`.

## Engine defaults — cross-app fidelity (surfaced by the IronCalc upgrade, 2026-07)

Identified while migrating FreeCell onto the fork (`specs/projects/ironcalc-upstreaming`). Neither
blocks the upgrade; FreeCell is self-consistent (owns its view defaults). Deferred, not on the
critical path.

| Gap | Severity | Why it matters | Root cause / home |
|-----|----------|----------------|-------------------|
| **Persist sheet/workbook defaults so files open consistently in other apps** | Moderate (cosmetic cross-app; FreeCell self-consistent) | FreeCell owns its **view** defaults (default row height / col width, render font = Inter). A file it saves carries **no** sheet defaults, so opening it in Excel/Sheets uses *their* defaults — the same file looks different across apps. To be portable, FreeCell should write its chosen defaults (`<sheetFormatPr defaultRowHeight/defaultColWidth>` + the workbook default font) into the file. | **IronCalc gap → fork fix.** IronCalc has no sheet-default (`sheetFormatPr`) modelling/export (`xlsx/src/export` emits none) and no workbook-default-font setter — only per-row/col `set_row_height`/`set_column_width`. Add + upstream, then FreeCell sets defaults on new-file creation. (Font can't pixel-match Excel regardless — we bundle Inter; that's an accepted trade-off.) |
| **Render-time fallback to Inter for unavailable *explicit* fonts** | Mild | A cell with an **explicit** non-default font the OS lacks (e.g. `Calibri` off-Windows) is passed straight to GPUI, which substitutes an arbitrary system font instead of our bundled Inter. (The common case — the *workbook default* font — already renders Inter via `GRID_FONT_FAMILY`, so this is only explicit non-default fonts.) | **FreeCell-side, small.** At the grid render site, resolve the family via `text_system().all_font_names()` and fall back to `GRID_FONT_FAMILY` (Inter) when absent; keep the real name in the model for round-trip. |
| **Opened xlsx (from another app) renders wider columns than the source app** | Mild (cosmetic; values, charts, and layout logic all correct) | A file created in Excel/Numbers with **no explicit `<col width>`** opens in FreeCell using FreeCell's *wider* default column width, so the same sheet — and its cell-anchored charts — read wider than in the originating app (same column *count*, wider columns). Surfaced comparing a real chart workbook's Excel vs FreeCell render during the charts project (P4–P11, 2026-07-10). | **FreeCell-side, deferred (design TBD; NOT a charts-project fix).** For columns lacking an explicit width, adopt a default closer to Excel's (~8.43 char ≈ 64px). Open question the owner flagged: change FreeCell's default width **everywhere**, or apply an Excel-like default only to files we *open* that look Excel-originated (they omit widths). Inverse/pair of the export-direction row above (`persist sheet/workbook defaults`). |

## Data safety & robustness

| Gap | Severity | Why it matters | Sketch |
|-----|----------|----------------|--------|
| **Save a `.back` backup before the first save** ✅ **RESOLVED (mvp-gaps Phase 1)** | High (we're alpha) | The save path can lose data (IronCalc's writer silently strips anything it doesn't model; we're early and bugs are likely). A one-time backup of the original bytes means an overwrite can never be the *only* copy. | ✅ **Done.** Before the **first** save-in-place of a document opened from disk, the original bytes are copied to `<name>.xlsx.back` (write-once — never overwritten on later saves; not created by Save-As to a new path; a copy failure aborts the save with a "Couldn't create backup — file not saved." dialog). `shell/lifecycle.rs` `backup_path`/`backup_target` + the save flow, tested by `first_save_of_opened_file_writes_back_backup_once`, `backup_failure_aborts_the_save_with_a_dialog`, and the `backup_target_*` unit tests. |

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

### Render fidelity — surfaced by the render-baseline eyeball (2026-07-06)

Two rendering deviations caught when every render-test baseline was regenerated on the
bundled Inter font and eyeballed. Both are **pre-existing** (unrelated to the font change);
the baselines faithfully capture current behavior. Recorded here, **not fixed** (out of scope
for the font work).

| Deviation | Vs. Excel | Follow-up if needed |
|---|---|---|
| **A fill does not cover interior gridlines.** In a multi-cell fill block, each cell still paints its own right/bottom gridline over its fill, so faint gray gridlines cross the block interior (visible in `cell_fill_covers_gridlines`: the 2×2 yellow block shows a gray line down the B/C boundary and across the row-2/3 boundary). The case name/comment says the fill should "paint over the interior gridlines (Excel look)", but it does not. | Excel shows no interior gridlines inside a filled range — it reads as one solid block. | Skip a cell's right/bottom gridline when the neighbor across that edge shares the same fill (or draw gridlines beneath fills). Then regenerate + eyeball `cell_fill_covers_gridlines`. |
| **Full-row selection does not highlight the row-number header.** A full-row selection tints the row and draws the accent border, but the left-hand row-number header cell is **not** darkened — whereas a full-**column** selection *does* darken the column-letter header (`header_full_column_selected` vs `header_full_row_selected`). Asymmetric. | Excel highlights both the row and column headers of a full-line selection. | Apply the selected-header background to the row-number header on a full-row selection the same way the column path already does. Then regenerate + eyeball `header_full_row_selected`. |

### Charts — line render fidelity (Charts P13, 2026-07-10)

Accepted residuals from the P13 line-chart fidelity pass (axis breadth, `a:ln` styling, rotated
axis title, font/line-weight tuning). Not defects — recorded so they aren't re-litigated.

| # | Item | Vs. Excel | Root cause / current behavior | Follow-up if needed |
|---|---|---|---|---|
| C-P13-1 | **Rotated value-axis title uses the SVG system sans-serif, not the chart body font.** Observation (A) was implemented as a **true −90° rotation** (not the P6 stacked-character fallback): a `canvas` paints an inline SVG whose `<text>` is rotated `rotate(-90 …)` via `Window::paint_svg` (the one pinned-gpui painter that takes a rotation; its usvg/resvg renderer shapes `<text>` through a font DB). | Excel rotates the title in the chart's own font (Calibri). | gpui's SVG font DB resolves `sans-serif` to a **system** face (DejaVu/Liberation Sans in the render env), so the rotated title's typeface differs from the chart's Inter body text. Weight/size/rotation match; only the family differs — a sharper instance of the already-accepted "we bundle Inter, not Calibri" GAP (#5 above). No gpui bump, no new deps. | If the SVG renderer's font DB is ever fed the bundled Inter face (via the app `AssetSource`), point the SVG `font-family` at it for a consistent typeface. |
| C-P13-2 | **Line/font weights matched to Excel by *proportion*, not exact px.** Series line default = Excel's ~2.25pt (`a:ln w="28440"`), honored per-series when `a:ln@w` is present; title 18pt bold, axis titles bold, ticks/legend regular. | Excel's exact pt→px depends on the chart's on-sheet size. | Rendered at a fixed chart scale (`PT_TO_PX`), so weights read heavier/closer to Excel but are not pixel-identical. Baselines capture the tuned look. | Tie the scale to the anchored chart's pixel size if exact-scale fidelity is ever required. |
| C-P13-3 | **Non-solid `a:ln` line styles are not rendered — but now correctly DEGRADE.** The line renderer draws only a **plain solid** line (width/color/alpha). A preset/custom **dash** (`a:prstDash`/`a:custDash`) or a **compound/double** line (`a:ln@cmpd`) is not drawn as authored. | Excel draws dashed/dotted/double lines (a dashed forecast/target line is common). | Rather than silently drawing them solid, `fidelity::unsupported_line_stroke` classifies such a series **[`Degraded`]** (⚠ badge) so it is honestly flagged, not misleading (functional_spec §5). Plain joins (`a:round`, the pervasive default) and end-caps are intentionally not degrading (would false-badge every real file). No committed baseline uses these. | Render dash patterns (gpui `PathBuilder` dash) + compound lines in a later line-styling pass, then drop them from the degrade set. |
| C-P13-4 | **Minor gridlines are not rendered — but now correctly DEGRADE.** The line renderer draws only **major** gridlines; an authored `c:minorGridlines` renders without its lines. | Excel draws minor gridlines when set. | `fidelity::unsupported_minor_gridlines` classifies a line chart carrying `c:minorGridlines` as **[`Degraded`]** (honest badge). Major-gridline on/off stays Faithful (honored). No committed baseline uses minor gridlines. | Draw minor gridlines (sub-ticks off the value axis) in a later pass, then drop from the degrade set. |
| C-P13-5 | **Legend swatch ignores `a:ln` alpha (cosmetic).** The legend color chip is a solid `div` fill, so a semi-transparent series (e.g. the 40%-alpha "Light / 40%" series in `chart_line_styled.png`) shows an **opaque** swatch even though its line is faded. | Excel's legend key mirrors the series line's opacity. | Pre-existing solid-chip legend design (the chip is `bg(rgb(color))` with no alpha channel). The *line* itself renders the alpha correctly; only the tiny legend chip doesn't. Tracked, not fixed. | Apply the resolved `Hsla` (incl. alpha) to the legend chip if legend↔line opacity parity is wanted. |

### Charts — manipulate (Charts P18, 2026-07-11)

Non-blocking forward-looking note from the P18 (select/move/resize/delete) review. Not a defect — recorded so it isn't re-litigated.

| # | Item | Vs. Excel | Root cause / current behavior | Follow-up if needed |
|---|---|---|---|---|
| C-P18-1 | **Moving/resizing a LOADED chart whose drawing anchor is a `oneCellAnchor` or `absoluteAnchor` moves its position but keeps its original size (cosmetic).** The overwhelmingly-common `twoCellAnchor` (used by real files, our loader, and the authored writer) is fully rewritten; the two rarer anchor kinds carry an `<xdr:ext>` (size) instead of `<xdr:to>`. | Excel moves + resizes any anchor kind. | `chart::save::patch_drawing_xml` rewrites `<xdr:from>`/`<xdr:to>` for the target anchor; a `oneCellAnchor`/`absoluteAnchor` has no `<xdr:to>`, so its `<xdr:from>` corner is updated (position moves) but its `<xdr:ext>` is left untouched (size preserved). **No corruption** — the patched drawing stays valid and reopens; only the size edit is dropped. | Extend the P18 edit path to rewrite `<xdr:ext>` (and, for a resize, convert to `twoCellAnchor` if needed) when the target anchor is not a `twoCellAnchor`. Small once a real file needs it. |

### Charts — column & bar (Charts P22, 2026-07-11)

Non-blocking Mild note from the P22 (column & bar) review. Not a defect — recorded so it isn't re-litigated.

| # | Item | Severity | Root cause / current behavior | Follow-up if needed |
|---|---|---|---|---|
| C-P22-1 | **`BarLayout::new` and the bar write path don't clamp `gapWidth`/`overlap` to their OOXML ST ranges.** `BarLayout::new(gap_width, overlap)` stores whatever it's given, and `write::group_element` emits the stored values verbatim; only the **load** path (`load::bar_layout`) clamps (`gapWidth` 0..=500, `overlap` -100..=100) and the **renderer** clamps for geometry. | Mild — **purely theoretical today.** Every real path is safe: loaded charts are load-clamped, authored/insert charts use `BarLayout::default()` (150/0), and the render fixtures pass in-range values. gap/overlap are **not panel-editable yet**, so no path can construct an out-of-range `BarLayout` that reaches the writer. | When gap/overlap become editable via the edit panel (a later chrome-editing extension), clamp in `BarLayout::new` (or at the write arm) to `ST_GapAmount`/`ST_Overlap` before an out-of-range value can be serialized. Trivial — a `.clamp()` at construction — but unreachable until then, so deferred. |

### Charts — scatter (Charts P25, 2026-07-11)

Two non-blocking Mild notes from the P25 (scatter / XY) review — one perf gap, one test hardening.
Not defects — recorded so they aren't re-litigated.

| # | Item | Severity | Root cause / current behavior | Follow-up if needed |
|---|---|---|---|---|
| C-P25-1 | **Scatter paint is unbounded — no `downsample_for_paint` cap.** ✅ **RESOLVED (Charts P26).** `chart/scatter.rs` drew one marker quad/path **per data point** plus an N-vertex connecting `Line`, with **no** vertex cap. **Fix:** a cloud-aware cap `cap_markers_for_paint(n, MAX_PAINT_MARKERS=2048)` (uniform linspace subsample over `[0, n-1]`, identity ≤ cap so no baseline moves) now bounds the per-frame mark count for **both** scatter (`scatter.rs` — the marker loop **and** the connecting `Line` draw over the same capped subset) and bubble (`bubble.rs`). | Moderate (perf; not hit by any committed scene — every scatter/bubble scene is ≤ ~13 points, so the cap is identity there). | The line path's extrema-based `downsample_for_paint` decimates along an **index-ordered** series and doesn't transfer to an **unordered marker cloud**; the cloud cap instead uniformly sub-samples (preserving spatial extent + density, keeping ascending order so a connecting line still threads the subset in data order). | **Done.** Measured in P26's perf sweep (`chart_perf.rs` → `results/chart-perf.json`, `bubble_scatter_cloud`): mapping a 100 k-point cloud to (pixel + radius) is **p50 1.05 ms / p99 1.10 ms per frame** uncapped (a large fraction of the 8.33 ms frame budget for the map alone, before the far costlier per-mark tessellation) vs **p50 21.75 µs / p99 37.12 µs** capped to 2048 (~49× fewer marks). Even the uncapped **map alone** is **~13% of the 8.33 ms frame budget** (1.05 ms), before the far costlier per-mark quad/path tessellation — so the cap is a justified win. The remaining knob: a **size-aware** cap for bubble (keep the largest bubbles) if a huge-range bubble ever needs it — uniform sampling is honest but can drop a large bubble; no committed scene is affected. |
| C-P25-2 | **`retyped_to_scatter_chart_roundtrips` asserts only the y range survived.** The edited-path round-trip test (`worker_seam.rs`) checks the reopened scatter's `source_ranges` contains the **y** range (`$B$2:$B$3`) but not the **x** range — so scatter's defining stale-x risk (an XY chart binds **two** ranges) has no explicit test assertion. | Mild (test defense-in-depth only). | Mirrored from the single-range Column/Area/Pie retype tests (one range asserted); scatter is the first XY type, so both x and y should be asserted. The binding **provably** rebinds both — `binding.rs` resolves the domain ref as `["cat","xVal"]` and the value ref as `["val","yVal"]`, and the render + round-trip are green. | Add an x-range assertion alongside the y-range one (the x ref). Cheap **test-only** hardening — no renderer/binding change (both ranges are provably rebound). |

### Charts — bubble (Charts P26, 2026-07-11)

One non-blocking Mild note from the P26 (bubble) review — an authoring-completeness gap. Not a defect
— recorded so it isn't re-litigated.

| # | Item | Severity | Root cause / current behavior | Follow-up if needed |
|---|---|---|---|---|
| C-P26-1 | **Authored-from-range and `SetChartType→Bubble` bubbles leave the size range unbound.** A bubble created via the **range picker** (`SetChartRange`) or a **plain retype** to Bubble binds only x/y — its `c:bubbleSize` has no `c:f`, so every bubble draws at `DEFAULT_BUBBLE_RADIUS` and the chart does **not** live-update on a size-column edit. **Only the authoring path is affected:** a **loaded** or **authored-with-refs** bubble (real `c:bubbleSize` `c:f` + numCache) rebinds **all three** ranges (x, y, size) correctly — `SeriesBinding.size` reads `c:bubbleSize`, `resolve_series` re-resolves it, and the dirty test includes it (proven by `resolve_bubble_reflects_all_three_ranges_and_size_range_is_dirty` + `write_authored_bubble_reopens_as_bubble_with_size`). | Mild (authoring completeness; loaded/bound bubbles are fully three-range-bound). | `series_refs_from_block` (`chart/range.rs`) derives only x/y from the block heuristic and sets `sizes: None` — the deterministic column rule stays x/y-shaped, so a range-picked bubble has no size ref; a plain retype keeps the existing (x/y-only) refs. Currently tracked in code comments only (`range.rs` `sizes: None` comment, the `retyped_to_bubble_chart_roundtrips` test doc). | Make the range picker bubble-aware: infer a **size** column from the block for a bubble (e.g. a 3-column block → x/y/size for one series) and bind it in `SeriesRefs.sizes` (+ have the retype shell carry a size ref). Then a range-picked/retyped bubble live-updates on a size edit like the loaded path. |

### Charts — post-v1 rendering feedback (Batch 1, 2026-07-11)

One latent (non-manifesting) note surfaced while landing the Batch-1 chart rendering fixes (gridline/
axis clipping to the plot rect, solid value-axis line, marker size floor, chart outline). Not a defect
— recorded so it isn't re-litigated.

| # | Item | Severity | Root cause / current behavior | Follow-up if needed |
|---|---|---|---|---|
| C-FB1-1 | **Line data-label offset uses a fixed `DOT_SIZE/2` (3px), not the marker's actual radius.** `line.rs` `paint_data_labels` (~L557) offsets an `Above`/`Below` (etc.) data label from its point by a constant `half_marker = DOT_SIZE / 2.0` (3px). Batch-1 Fix 2 now floors a marker's diameter to `max(requested, 2× line width, 6px)`, so a **heavy-line** series' marker can be ~12–16px (radius 6–8px) — larger than the 3px the label offset assumes, so a label could kiss/overlap an enlarged marker when a heavy line **and** data labels combine on the same series. | Mild — **currently NON-MANIFESTING.** No committed fixture triggers it: every data-label scene (`chart_line_value_labels`/`_percent_labels`/`_named_labels`) uses default-width (~2.7px) lines whose markers stay 6px = `DOT_SIZE`, so the 3px offset is still exactly right and no baseline moved. The offset already ignored an explicit `marker.size` (pre-existing), so Fix 2 only widened the same latent gap. | Mirror `paint_marker`'s diameter floor in the label offset: derive the same effective marker diameter (from the series `width_px` + any `marker.size`) and offset by `effective_diameter / 2` instead of the constant `DOT_SIZE / 2`. Trivial once a heavy-line + data-labels fixture needs it. |

### Charts — post-v1 undo feedback (Batch 4, 2026-07-12)

One non-blocking ordering caveat surfaced landing the Batch-4 undo work (chart ops now ride the
unified undo/redo timeline). Not a defect — recorded so it isn't re-litigated.

| # | Item | Severity | Root cause / current behavior | Follow-up if needed |
|---|---|---|---|---|
| C-FB4-1 | **A single coalesced `process_batch` that mixes a queued `Undo`/`Redo` with forward ops can undo the wrong one of two near-simultaneous actions.** `process_batch` applies buckets in a fixed order (edits → font → chart → undo/redo → clipboard, `run.rs` ~L501/531/547/570/580). Batch 4 pulled `Undo`/`Redo` out of the in-order `edits` batch into the post-forward `undo_ops` bucket, so e.g. a coalesced `[Undo, SetCellInput]` applies the edit first (pushes a `Cell` entry) then the `Undo` pops *that just-applied edit* rather than the intended prior action; likewise `[chartOp, edit]` coalesced then Undo peels the chart before the more-recent edit. | Mild — **rare + self-correcting; never corrupts state or desyncs.** Only manifests when the worker is busy long enough to coalesce **distinct-modality** gestures (a mouse/chart op or a deliberate Ctrl+Z together with a keyboard commit) into one drain window; rapid *typing* is all `Cell` entries and stays correct regardless of order. The 1:1 IronCalc invariant always holds (no crash / phantom redo), and another undo/redo fixes the mis-pick. The bucketing was already not strictly queue-order-preserving pre-Batch-4 (`[paste, Undo]`/`[font, Undo]` also misordered); Batch 4 shifts *which* mixes are affected (it actually fixes clipboard/font-vs-undo) rather than adding a new desync class. Every real gesture arrives as its own batch — the correct path, exercised by all 12 undo tests. | Dispatch a batch in one **strict queue-order** pass (interleave undo/redo with the other buckets in arrival order) instead of the fixed per-bucket order. Larger change; deferred until a coalesced mixed-modality undo is observed to actually bite. Interim: the invariant is documented in the commit + here. |

### `mvp-gaps` UI review — accepted limitations (owner-approved 2026-07-06)

Two judgment calls from the post-Phase-8 **UI-review bug-fix round**, reviewed and accepted by
the owner as-is. Each ships as built; recorded here so neither is later mistaken for a defect.

| # | Limitation | Vs. Excel | Root cause | Current behavior | Follow-up if needed |
|---|---|---|---|---|---|
| U1 | **Open dialog shows all files, not just `*.xlsx`** | Excel's file picker filters to workbook types | The pinned gpui rev's `PathPromptOptions` (`crates/gpui/src/platform.rs`) has **no** extension/content-type field, and neither the macOS impl (`NSOpenPanel`, never calls `setAllowedContentTypes:`) nor the Linux `prompt_for_paths` exposes a filter hook — so pre-filtering is impossible without bumping the pinned gpui dep (a separate, riskier change against a pinned dependency). | **Correct + graceful fallback:** a files-only picker, then a **post-selection** magic-byte check rejects a non-`.xlsx` → `LoadError::NotXlsx` → the "Couldn't open the workbook" dialog. No crash, no wrong-file load. | Revisit if gpui is bumped to a rev whose path prompt gains a filter field; then set the filter in `open_panel_options` (`shell/window.rs`). |
| U2 | **Single-cell paste-fill uses block-uniform formula displacement, not per-cell relative fill** | Excel fills a 1×1 copy across a larger selection with **per-cell** relative-reference adjustment | The fill is one synthesized `paste_from_clipboard` call so it stays **one undo step** (IronCalc 0.7.1 has no fill-to-selection), which applies a single uniform `anchor − source` reference shift to every filled cell. Per-cell relative fill would need N×M separate engine pastes (= N×M undo entries, breaking the one-undo-step requirement) or an engine fill API that does not exist. | Pasting a 1×1 (or exact-divisor block) copy onto a larger selection fills the **whole** target in **one** undo step; **values and styles are exact** for every cell, but a **formula** gets the top-left cell's reference shift applied uniformly (not re-adjusted per cell). Over-large fills (> 100k cells) are rejected as Overflow. | Revisit if an IronCalc relative-fill API appears, or if the one-undo-step constraint is relaxed (then paste per cell). |

---

## Formatting expansion — deferred behaviors (2026-07-08)

Deferred from the **`specs/projects/formatting-expansion`** build (text formatting +
border formatting). Recorded so the follow-up isn't lost.

| # | Behavior | Spec | Severity | Current behavior | Root cause | Follow-up |
|---|----------|------|----------|------------------|------------|-----------|
| F1 | **Wrap text auto-grows row height** — a wrapped cell should expand its row to fit all lines (true Excel wrap) | `formatting-expansion/functional_spec.md` §1.2 | Moderate | **Ships as "wrap within current row height" (option 2A):** wrap sets the attribute and text wraps to multiple lines, but only lines fitting the row's current height are visible; the user resizes the row manually to reveal the rest. Content below the cell is clipped. | No content-driven variable row height in the grid: rows are default/explicit-height only, and there is no per-cell wrapped-text measurement feeding a row-height computation. Auto-grow needs text measurement over the column width + a max-over-row height pass + grid relayout (and interacts with manual row resize). | Its own project when picked up: measure wrapped line count × line height per cell, take the row max, drive an auto row height that manual resize can still override. Related survey row: "Text overflow into empty neighbors + wrap" above. |
| F2 | **Border restyle-all with no target selected (P2)** — adjusting style/color with no "which lines" target selected restyles all existing borders in the selection in place | `formatting-expansion/functional_spec.md` §2.5 | Mild | MVP: with no target selected, changing a control only updates the pen (what the next target click paints); existing borders are untouched. | Restyle-in-place needs read-modify-write of each cell's existing edges (preserve which edges exist, swap style/color) rather than the type-based paint the target path uses. | Build on the border pipeline once shipped: enumerate existing edges in the selection, re-emit each with the new pen. |
| F3 | **Dotted + dash-dot border line styles** — not offered in the line-style gallery | `formatting-expansion/functional_spec.md` §2.3 | Mild | Gallery ships thin/medium/thick solid + dashed + double. Dotted and dash-dot are absent. | **Dotted:** at IronCalc 0.7.1 `Dotted` degraded to `Thin` on `.xlsx` import → shipping it would silently lose the style on round-trip; dropped rather than degrade. **Dash-dot:** niche; skipped to keep the new render-pattern work minimal. | Dotted: verify/fix the fork's import path (per fix-upstream policy), then add the gallery entry + dot render pattern. Dash-dot: add the gallery entry + dash-dot render pattern. Each is small once the dashed/double render path exists. |

