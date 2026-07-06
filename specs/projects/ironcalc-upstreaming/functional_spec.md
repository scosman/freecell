---
status: draft
---

# Functional Spec: IronCalc Upstreaming

## 1. Goal & shape

Fix, in our fork `scosman/ironcalc`, the IronCalc defects and API gaps that FreeCell
currently compensates for; validate each fix in the real FreeCell app built against the
fork; then, on owner sign-off, submit each upstream. Success = FreeCell carries the
**minimum** engine-compensation code (ideally `open_fixups.rs` + `open_repair.rs` deleted),
and every submitted fix is an isolated, tested, independently-mergeable patch.

This is a **workflow project**, not a feature. Its deliverables are: (a) a set of patch
branches in the fork with tests, (b) an integration branch FreeCell builds against, (c) a
FreeCell change that swaps the dep and removes the now-dead hacks, and (d) upstream PRs.

## 2. Decisions of record

Locked for this project (defaults adopted where not explicitly chosen; override on review):

| # | Decision | Choice |
|---|---|---|
| D1 | Fork branch model | Fork `main` stays a **clean mirror of upstream `main`**. Each fix is a topic branch off `main`. A dedicated integration branch **`freecell-fixes`** merges all topic branches; FreeCell builds against `freecell-fixes`. |
| D2 | Project scope | **Import correctness bugs only** — A1–A4 (E4 `xfId`, E2+E3 num-fmt table, E1 theme + tint, E5 indexed). This is exactly the set that lets us delete `open_fixups.rs` + `open_repair.rs` + `roxmltree`/`zip`. The API-visibility cluster and the parser depth-cap are **out of scope** — a fast-follow (§4). |
| D3 | Visibility packaging (deferred) | Recorded for the fast-follow: the four API-visibility fixes ship as **one PR per type** (four small independent PRs), not in this project's scope. |
| D4 | Upstream flow | **PR-first**: each PR description carries the minimal repro and doubles as the report; file a standalone issue only if a maintainer asks. |
| D5 | No submission without sign-off | Nothing is pushed to `ironcalc/IronCalc` (upstream) until the owner approves the human validation pass. Work on the fork (`scosman/ironcalc`) is unrestricted. |
| D6 | Fork base | Sync fork `main` to **upstream `main`** first, branch fixes off that (not off the older crates.io `0.7.1` cut). FreeCell validates against post-0.7.1 upstream + our fixes. |

## 3. Fixes in scope

Each fix below is one implementation phase (§ maps to implementation_plan.md). Each has:
**Defect** (what IronCalc does wrong / lacks), **Fix** (what changes in the fork), **Upstream
test** (added in the fork, synthetic — no copyrighted fixtures), and **Retires** (the FreeCell
compensation it makes dead). File citations are FreeCell-side unless prefixed `fork:`.

### Group A — `.xlsx` import correctness bugs (these delete our fix-up modules)

**A1 — Missing-`xfId` hard open failure (E4).**
- *Defect:* `fork: xlsx/src/import/styles.rs` reads `xfId` on `<cellXfs>/<xf>` as mandatory
  (`get_attribute(&xfs, "xfId")?`); OOXML §18.8.10 makes it optional. Files from Numbers /
  LibreOffice fail to open entirely (`XML Error: Missing "xfId" XML attribute`).
- *Fix:* treat `xfId` on a `cellXfs/xf` as optional (default to none / 0), matching how the
  parser already treats `cellStyleXfs`.
- *Upstream test:* a crafted styles.xml whose `cellXfs/xf` omit `xfId` loads; `cellStyleXfs`
  unaffected. (Port the crafted-zip cases from `open_repair.rs` tests.)
- *Retires:* `open_repair.rs` entirely (~370 lines).

**A2 — Wrong built-in `numFmtId` table (E2 + E3).**
- *Defect:* `fork: base/src/number_format` `DEFAULT_NUM_FMTS` maps standard built-in ids to
  garbage codes (id 39 → `"t0.00"`), so `get_formatted_cell_value` returns **`#VALUE!`** for
  valid numbers. Wrong across the numeric/currency/accounting block (5–8, 37–49) **and** the
  date/time + spacing ids (11–13, 14–22, 23–36) — the E3 residual our FreeCell fix skips.
- *Fix:* correct the whole `DEFAULT_NUM_FMTS` table to the ECMA-376 built-in codes (fix E2
  *and* E3 in one PR — one table, fixed once).
- *Upstream test:* per corrected id, assert `get_num_fmt` / a formatted value matches the
  ECMA-376 code / Excel output. (Extend from `inject_builtin_num_fmts` tests.)
- *Retires:* `open_fixups::inject_builtin_num_fmts` + `STANDARD_BUILTIN_NUM_FMTS`.

**A3 — Theme palette ignored + `hue_to_rgb` tint bug (E1 + E1′).**
- *Defect:* `fork: xlsx/src/import/colors.rs` `get_themed_color` resolves theme-indexed
  colours against a **hardcoded default Office palette**, never parsing the file's
  `xl/theme/theme1.xml`, and discards the theme index + tint. Separately, the tint math's
  `hue_to_rgb` keeps an un-normalised hue offset that **overflows for light tints on
  saturated hues**.
- *Fix:* parse the workbook's `theme1.xml` `clrScheme` and resolve theme colours against it
  (with the dark/light swap + §18.8.3 tint); fix `hue_to_rgb` to normalise `t` into `[0,1)`.
- *Upstream test:* a crafted workbook with a swapped/custom theme resolves fills/fonts/borders
  to the file's colours; the existing Excel-verified tint goldens still pass. (Port
  `open_fixups` theme tests + `tint_matches_excel_goldens`.)
- *Retires:* `open_fixups::correct_theme_colors` + the ported HSL/tint math.

**A4 — Indexed-palette override ignored (E5).**
- *Defect:* `fork: xlsx/src/import/colors.rs` `get_indexed_color` uses a hardcoded legacy
  indexed palette and never reads the workbook's `<colors><indexedColors>` override (OOXML
  §18.8.27), so `indexed="n"` fills/fonts/**borders** render the default colour n, not the
  file's redefined colour n.
- *Fix:* when the workbook supplies an `<indexedColors>` override, resolve `indexed=` colours
  against it (fills, fonts, **and borders** — closes our border residual).
- *Upstream test:* a crafted workbook with an `<indexedColors>` override resolves indexed
  colours to the file's palette; a standard-palette file (no override) is unchanged. (Port
  `open_fixups` indexed tests.)
- *Retires:* `open_fixups::correct_indexed_colors`; with A1–A4 landed, delete `open_fixups.rs`
  entirely and drop the `roxmltree` + `zip` deps from `freecell-engine`.

## 4. Out of scope (explicit)

- **API-visibility cluster — deferred to a fast-follow** (recorded here so it isn't lost;
  packaging preference: one PR per type — D3): `Clipboard` not nameable, `BorderArea` has no
  constructor, `Function` enum module is private, no workbook-default-font accessor. These
  retire our `serde_json`-for-private-types shims and the `default_font` reach-in, but delete no
  import code — lower urgency than A1–A4.
- **Parser recursion depth-cap — deferred to a fast-follow** (issue-worthy): IronCalc's parser
  has no depth limit → uncatchable SIGABRT on deep formulas. Weakest ROI (deletes nothing;
  FreeCell's `input_cap.rs` stays as defense-in-depth regardless).
- **Kept FreeCell-side by design** (legitimately consumer concerns, not IronCalc bugs): the
  date-format heuristic (`is_date_format`) and format-color-index→RGB mapping
  (`format_color_rgb`); zoom and hidden-column view state; the `catch_unwind` worker guard and
  band-driven cache rebuild.
- **Enhancement APIs** (fast-follow project if we want them): band fast-paths for range-clear /
  full-row-col style ops, `paste_csv_string` dry-run dimensions, format-code classification /
  decimal-increment API, sheet reorder, hidden-row setter, font-band style path,
  fill-to-selection, comments export (lossy-save fix), data validation, hyperlinks.
- **Large features / product decisions** (own designs, not this project): merged-cell write
  API, conditional formatting, dynamic arrays / spill.

## 5. Process contract (the workflow behaviors)

**5.1 Fork setup.** Sync fork `main` to upstream `main` (D6). Create `freecell-fixes` off
`main`. Verify the fork workspace builds + tests green *before* any fix (baseline).

**5.2 Per-fix loop (each Group-A/B/C phase).**
1. Branch `fix/<slug>` off `main`.
2. Implement the fix in the fork; add upstream-style synthetic tests.
3. Fork's own checks pass (`cargo test` for the touched crate + workspace clippy/fmt per the
   fork's `CONTRIBUTING.md`/`Makefile`).
4. Merge `fix/<slug>` into `freecell-fixes`.
5. Commit + push the fork branch(es).

**5.3 FreeCell integration (one phase, after the fixes exist).**
1. Point FreeCell at the fork via `[patch.crates-io]` → `freecell-fixes`
   (`ironcalc`/`ironcalc_base` names match, no rename needed). Path-patch to the local
   `/workspace/ironcalc` checkout is acceptable for in-container validation; the committed
   form uses the git branch so it's reproducible off this container.
2. Remove the now-dead compensations, gated per fix: `open_repair.rs` (A1), the num-fmt
   injector (A2), theme + indexed correctors → delete `open_fixups.rs` and drop
   `roxmltree`/`zip` (A3+A4).
3. FreeCell builds, all tests pass, render/behavior unchanged (the fixes reproduce what the
   hacks did).

**5.4 Human validation pass (single, owner-run).** With FreeCell on the fork and hacks
removed, the owner confirms: the previously-broken files open and render correctly (theme
colours, indexed colours, number formats, the Numbers `xfId` file), and nothing regressed with
the compensation code gone. This is the **gate** before any upstream submission.

**5.5 Upstream submission (on sign-off only).** For each fix (bundled per D3), open a PR from
the topic branch against `ironcalc/IronCalc:main`, PR-first (D4), each with its minimal repro +
tests. Track fix ↔ branch ↔ PR ↔ FreeCell-code-deleted in a status table.

## 6. Edge cases, risks, failure modes

- **A fix is rejected or stalls upstream.** The fork is a bridge: FreeCell keeps building
  against `freecell-fixes`; if upstream declines a fix, we either keep carrying it in the fork
  or restore the corresponding FreeCell hack. Because each patch is isolated (one branch, one
  commit range), any single one can be dropped without disturbing the others.
- **Fork `main` drifts from upstream.** Re-sync `main` and rebase the topic branches +
  `freecell-fixes` before submitting, so each PR applies cleanly.
- **Post-0.7.1 upstream changes break FreeCell.** Building against upstream `main` (D6) may
  pull unrelated engine changes. If that surfaces breakage unrelated to our fixes, fall back to
  branching the fixes off the `0.7.1` tag instead (documented fallback, not the default).
- **`[patch.crates-io]` + exact-version pin.** FreeCell pins `=0.7.1`; a patch source whose
  crate version differs can fail to satisfy `=0.7.1`. Keep the fork crate versions compatible
  with the pin (or relax the FreeCell pin to match) — resolve during 5.3.
- **Test fixtures.** All upstream tests use **synthetic crafted zips / inline XML** (as our
  existing `open_fixups`/`open_repair` tests already do) — no copyrighted `.xlsx` files travel
  into the fork or upstream.
- **Removing a hack changes save output.** Some fix-ups only affected in-memory display; a few
  (e.g. indexed borders) also affect the save round-trip. Validation (5.4) must eyeball an
  open→save→reopen of an affected file, not just the on-screen render.

## 7. Constraints

- No upstream PR without explicit owner sign-off (D5).
- FreeCell stays green throughout; the dep-swap + hack-removal lands only after it builds,
  tests, and validates clean.
- The fork carries the minimum diff; each patch is independently submittable and droppable.
- License is MIT/Apache-2.0 both sides — no license friction; keep our commits under the
  fork's existing license headers/conventions.
