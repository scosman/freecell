---
status: draft
---

# Functional Spec: IronCalc Upstreaming

> **Direction (Option 1, adopted 2026-07-07):** Upstream the two bugs still broken on
> upstream `main` — **E2** (built-in num-fmt table) and **E5** (indexed `<indexedColors>`
> override) — as tested, PR-ready patches in our fork. The other three workarounds (E4 `xfId`,
> E1 theme, E1′ tint) are **already fixed upstream** (see `fork_audit.md`); FreeCell gets them
> by upgrading. The FreeCell-side upgrade — migrating to `main`'s new `Color` API, validating
> visually, and deleting `open_fixups`/`open_repair` — is a **separate future project**
> (`projects/ironcalc-upgrade.md`), gated on an IronCalc release that carries all five fixes.

## 1. Goal & shape

Contribute the two genuinely-still-broken IronCalc import bugs upstream, as isolated, tested
patches in `scosman/ironcalc`, PR-ready against `ironcalc/IronCalc:main` on owner sign-off.
This is a **workflow/contribution project**. Deliverables: two fix branches with upstream-style
tests, merged into an integration branch, and — on sign-off — two upstream PRs. FreeCell's own
code is **untouched** by this project.

## 2. Decisions of record

| # | Decision | Choice |
|---|---|---|
| D1 | Fork branch model | Fork `main` = clean mirror of upstream `main`. Each fix = a topic branch off `main` (`fix/e2-numfmt`, `fix/e5-indexed`). A `freecell-fixes` branch merges both (staging for the future upgrade project). |
| D2 | Scope | **E2 + E5 only.** The already-upstream fixes (E1/E4/E1′) are not re-done. The FreeCell upgrade/migration + hack deletion is **out of scope** (separate project — §4). |
| D3 | Base | Branch off upstream `main` (D6 in the audit): E2/E5 are still broken there, and upstream wants PRs against `main`. |
| D4 | Upstream flow | **PR-first**, **one PR per fix** (two independent PRs — they touch different crates and review cleanly apart). PR description carries the minimal repro. |
| D5 | No submission without sign-off | Nothing goes to `ironcalc/IronCalc` until the owner approves. Fork work is unrestricted. |
| D6 | Validation | Fork-side: each fix passes the fork's own `cargo test` + `make lint`. FreeCell-in-app visual validation of these two is **deferred to the upgrade project** (it needs the `Color`-migration to build against the fork). E2/E5 correctness is additionally corroborated by FreeCell's existing `0.7.1`-side evidence (`open_fixups` already produces the correct output on the real Numbers/mortgage files). |

## 3. The fixes

### E2 — Correct the built-in `numFmtId` table

- **Defect (fork `main`):** `base/src/number_format.rs` `DEFAULT_NUM_FMTS` maps standard
  built-in ids to garbage codes — id 39 → `"t0.00"` (line 47), id with `%` → `"t0.00 %"` (51),
  etc. `get_formatted_cell_value` then returns **`#VALUE!`** for a valid number whose cell uses
  one of those ids. (Confirmed still present on `main`.)
- **Fix:** replace the garbage entries with the correct ECMA-376 built-in codes. Scope the PR to
  the **locale-independent** numeric / currency / accounting / misc block (ids 5–8, 37–49) —
  the set FreeCell already proved correct end-to-end. The **date/time ids (14–22) are locale-
  substituted** and must be handled carefully or left out of this PR (see §5 risk) to keep it
  unambiguously correct.
- **Upstream test:** for each corrected id, assert `get_num_fmt(id, &[])` returns the ECMA-376
  code and/or that a representative value formats to the Excel string (e.g. `175000` under id 39
  → `"175,000.00 "`, not `#VALUE!`). Port the assertions from FreeCell's
  `inject_builtin_num_fmts` tests.
- **PR shape:** small, `base`-only, self-contained. Highest value (fixes a wrong-output bug).

### E5 — Honour the workbook's `<indexedColors>` override

- **Defect (fork `main`):** `xlsx/src/import/util.rs::get_color` resolves an `indexed="n"`
  colour via the hardcoded `ironcalc_base::colors::get_indexed_color` (the legacy 56-colour
  palette) and **never reads the workbook's `<colors><indexedColors>` override** (OOXML
  §18.8.27) — grep of `base/src` + `xlsx/src` finds no reader. So a file that redefines palette
  entries (as Numbers exports do) renders `indexed=` fills/fonts/borders with Excel's default
  colour n, not the file's colour n.
- **Fix (design in architecture):** parse `<colors><indexedColors>` in the styles importer into
  a positional palette and, **when present**, resolve `indexed=` colours against it instead of
  the hardcoded table; absent an override, behaviour is unchanged. Preferred approach:
  read-at-import (thread the override palette into `get_color`), matching how `main` already
  resolves `indexed=`/`rgb=` to `Color::Rgb` immediately — no new `Color` variant. (Alternative:
  a deferred `Color::Indexed(i32)` variant resolved like `Theme`; rejected as over-invasive —
  see architecture.)
- **Upstream test:** a crafted `styles.xml` with an `<indexedColors>` override resolves indexed
  fills/fonts/borders to the file's palette; a file with no override is byte-for-byte unchanged.
  Port the crafted cases from FreeCell's `correct_indexed_colors` tests.
- **PR shape:** touches `xlsx` import (+ possibly a `get_color` signature change); larger than
  E2 and must be designed against `main`'s `Color` model.

## 4. Out of scope (explicit)

- **Already fixed upstream — nothing to do** (arrive via a future FreeCell upgrade): E4 `xfId`
  optionality, E1 file-theme resolution, E1′ `hue_to_rgb` tint normalisation. (`fork_audit.md`.)
- **FreeCell upgrade / `Color`-enum migration / hack deletion — separate future project**
  (`projects/ironcalc-upgrade.md`): migrate FreeCell off the `0.7.1` `Option<String>` style API
  to `main`'s `Color` enum, delete `open_fixups.rs` + `open_repair.rs`, drop `roxmltree`/`zip`,
  and do the in-app visual validation. Gated on an IronCalc **release** containing all five
  fixes (so we pin a stable version, not git-`main`).
- **API-visibility cluster** (Clipboard/BorderArea/Function/default-font) and the **parser
  depth-cap** — deferred fast-follows (recorded in the earlier scope notes); not this project.
- **Kept FreeCell-side by design / large features** — unchanged from the original inventory.

## 5. Edge cases & risks

- **E2 date/time ids are locale-sensitive.** IronCalc may substitute locale-specific date codes
  elsewhere; blindly overwriting ids 14–22 in the default table could regress localized output.
  Mitigation: the PR fixes only the locale-independent block (5–8, 37–49); dates are a separate,
  carefully-scoped follow-up if wanted. Verify against IronCalc's existing formatter tests.
- **E5 must fit `main`'s `Color` model.** The fix is designed against the current enum, not
  `0.7.1`. Risk: a `get_color` signature change ripples to its callers (fills/fonts/borders in
  `styles.rs`). Keep the change local; the styles importer already has the styleSheet root to
  parse `<indexedColors>` once and pass down.
- **No in-app visual validation this project.** Because FreeCell can't build against `main`
  (Color API), E2/E5 are validated by fork tests + FreeCell's existing `0.7.1` evidence, not a
  live render. The live-render confirmation happens in the upgrade project. Acceptable: both
  fixes have deterministic, unit-testable outputs.
- **Fork `main` drift from upstream.** Re-sync `main` and rebase the fix branches before opening
  PRs so each applies cleanly.
- **Fork clippy is strict** (`make lint`: `-D warnings` with `unwrap_used`/`expect_used`/`panic`
  as warns). Match the fork's own conventions — test modules use `#![allow(clippy::unwrap_used)]`
  (e.g. `xlsx/src/import/util.rs:1`); non-test code must avoid `unwrap`/`expect`/`panic`.

## 6. Constraints

- No upstream PR without explicit owner sign-off (D5).
- FreeCell code is untouched by this project (the migration is a separate project).
- The fork carries the minimum diff; each fix is an independent, droppable branch/PR.
- Fixes must pass the fork's `cargo test` + `make lint`, and follow its commit/style conventions
  (MIT/Apache headers, existing test idioms). Tests use synthetic inline XML — no copyrighted
  `.xlsx` files.
