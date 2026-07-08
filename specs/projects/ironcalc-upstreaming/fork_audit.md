---
status: complete
---

# Fork audit — upstream `main` vs. our five workarounds

Audited `scosman/ironcalc` `main` (= upstream `main`, cloned 2026-07-06 at `29daa42`) against
the five IronCalc bugs FreeCell compensates for. **`main` is well ahead of the crates.io
`0.7.1` we pin** — several bugs are already fixed there, and the style-color API changed.

## Per-bug status

| Bug (our hack) | `0.7.1` (pinned, hacked) | fork `main` | Evidence | PR needed? |
|---|---|---|---|---|
| **E4** `xfId` hard open-fail (`open_repair.rs`) | broken (mandatory `xfId`) | ✅ **fixed** | `xlsx/src/import/styles.rs:295-301` reads `xfId` optionally, defaults 0; comment cites the OOXML optionality | No |
| **E1** theme palette ignored (`open_fixups::correct_theme_colors`) | broken (hardcoded Office palette, index+tint discarded) | ✅ **fixed** | Import parses the file's `theme1.xml` (`xlsx/src/import/theme.rs`) into a `Theme`; `get_color` returns `Color::Theme(idx, tint)` (`xlsx/src/import/util.rs:58-64`); resolved by `Theme::resolve(idx, tint)` with dk/lt swap + tint (`base/src/types.rs:830`) | No |
| **E1′** `hue_to_rgb` tint overflow | broken | ✅ **fixed** | `base/src/colors.rs:61-79` normalises the hue offset into `[0,1)` (our correction) | No |
| **E2** wrong built-in num-fmt table (`open_fixups::inject_builtin_num_fmts`) | broken (`#VALUE!` on valid numbers) | ❌ **still broken** | `base/src/number_format.rs:7` `DEFAULT_NUM_FMTS` still has `"t0.00"` (line 47), `"t0.00 %"` (51) | **Yes** |
| **E5** indexed `<indexedColors>` override (`open_fixups::correct_indexed_colors`) | broken (hardcoded legacy palette) | ❌ **still broken** | `get_color` resolves `indexed=` via hardcoded `get_indexed_color` (`xlsx/src/import/util.rs:56`, `base/src/colors.rs:131`); **no `<indexedColors>` reader anywhere** (grep of `base/src` + `xlsx/src` = 0 hits) | **Yes** |

## The API break that blocks building FreeCell against `main`

`main` fixed the theme bug by **replacing the style-color API**:

- `0.7.1`: `Fill { pattern_type: String, fg_color: Option<String>, bg_color: Option<String> }`
  — hex strings, which `open_fixups`/`cache`/style-reading read and write.
- `main`: `Fill { color: Color }` (`base/src/types.rs:568-572`), where `Color` is an enum
  (`Rgb(String) | Theme(i32, f64) | None | …`, `types.rs:17`). `Font.color`, border colors,
  etc. are all `Color`. `pattern_type` / `fg_color` / `bg_color` are **gone**.

FreeCell references the old fields throughout → **it cannot compile against `main`** without a
migration to the `Color` enum. The "build FreeCell against the fork to validate visually"
loop is blocked until that migration lands.

## Implication for the project

- Only **E2** (num-fmt table) and **E5** (indexed override) still need upstreaming; the other
  three are already fixed on `main` and arrive via an **upgrade**, not a patch.
- E2 is an isolated `base`-only table fix (no migration to validate). E5's fix must be designed
  against `main`'s new `Color` model (candidate: a `Color::Indexed(i32)` variant + a
  workbook-level indexed palette, resolved like `Theme`).
- The big FreeCell cleanup (delete `open_fixups`/`open_repair`, drop `roxmltree`/`zip`) comes
  from **upgrading to a release that contains all five fixes**, gated on the `Color`-enum
  migration — not from patching the fork.

Direction decision (which fixes to PR, and when to do the FreeCell upgrade/migration) is
pending owner input; the functional spec will be rewritten to match once chosen.
