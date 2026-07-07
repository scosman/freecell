---
status: draft
---

# Functional Spec: IronCalc Upstreaming

> **Direction (Option 2, adopted 2026-07-07):** Upstream the two still-broken bugs — **E2**
> (num-fmt table) and **E5** (indexed `<indexedColors>` override) — in the fork, **and upgrade
> FreeCell onto upstream `main` + those fixes now**: migrate to `main`'s new `Color`-enum style
> API, **pull out the workarounds** (`open_fixups`/`open_repair`), and **prove they're redundant**
> (the three already-upstream fixes — E4 `xfId`, E1 theme, E1′ tint — plus our E2/E5 make the
> hacks unnecessary). Then, on owner sign-off, open the two upstream PRs. See `fork_audit.md`.

## 1. Goal & shape

Two things at once, because validating the second is the whole point:
1. **Contribute** E2 + E5 to `scosman/ironcalc` (tested, PR-ready).
2. **Upgrade FreeCell** to build on that fork (`main` + E2 + E5), migrate off the `0.7.1`
   `Option<String>` style-color API to `main`'s `Color` enum, **delete the import workarounds**,
   and **verify FreeCell still renders every previously-broken file correctly with the hacks
   gone** — the confidence the owner asked for ("make sure pulling out the workarounds is
   correct").

Deliverables: E2/E5 fix branches + tests in the fork; a FreeCell branch that builds on the fork,
has the hacks removed, and passes tests + a visual validation pass; and — on sign-off — two
upstream PRs.

## 2. Decisions of record

| # | Decision | Choice |
|---|---|---|
| D1 | Fork branch model | Fork `main` = clean mirror of upstream. `fix/e2-numfmt`, `fix/e5-indexed` off `main`; `freecell-fixes` merges both. FreeCell builds against `freecell-fixes`. |
| D2 | Scope | E2 + E5 (fork) **+ FreeCell upgrade**: `Color`-enum migration, delete `open_fixups`/`open_repair` (+ `roxmltree`/`zip`), tests + visual validation. |
| D3 | FreeCell → fork wiring | Cargo `[patch.crates-io]` → the fork `freecell-fixes` **git branch** (reproducible off-container). The fork crates are versioned `0.7.1`, so the patch satisfies FreeCell's `=0.7.1` pin. A local `path` patch to `/workspace/ironcalc` is fine for in-container iteration. |
| D4 | Upstream flow | **PR-first, one PR per fix** (E2, E5), against `ironcalc/IronCalc:main`, on sign-off only. |
| D5 | No submission without sign-off | Nothing to `ironcalc/IronCalc` until the owner approves. Fork + FreeCell-branch work is unrestricted. |
| D6 | Validation is central | Prove redundancy: (a) FreeCell tests green post-migration/removal; (b) `resolve_color` reproduces the **exact RGBs** `open_fixups` computed for the fixtures (assertable — our old tests carry the goldens); (c) visual pass on the real files; (d) open→save→reopen of an affected file. |
| D7 | git-`main` pin is temporary | Building on unreleased git-`main` is the validation vehicle. Moving to a **released** pin once IronCalc publishes the fixes is a slim follow-up (`projects/ironcalc-upgrade.md`). |

## 3. The fork fixes

### E2 — Correct the built-in `numFmtId` table
- **Defect (fork `main`):** `base/src/number_format.rs` `DEFAULT_NUM_FMTS` maps built-in ids to
  garbage (id 39 → `"t0.00"`), so `get_formatted_cell_value` returns `#VALUE!` for valid numbers.
- **Fix:** correct the locale-independent block (ids 5–8, 37–49) to the ECMA-376 codes (mapping
  in architecture). Leave the locale-sensitive date ids 14–22 out of this PR.
- **Test:** per corrected id, `get_num_fmt(id, &[])` == ECMA code + a value formats (no `#VALUE!`).

### E5 — Honour the workbook's `<indexedColors>` override
- **Defect (fork `main`):** `xlsx/src/import/util.rs::get_color` resolves `indexed=` via the
  hardcoded `get_indexed_color`; the `<colors><indexedColors>` override (OOXML §18.8.27) is never
  read (grep = 0 hits).
- **Fix:** parse `<indexedColors>` in `styles.rs`, thread it to `get_color`, resolve `indexed=`
  against the override when present (unchanged otherwise). Design in architecture (read-at-import;
  no new `Color` variant).
- **Test:** crafted `styles.xml` with/without the override; ported guards.

## 4. The FreeCell upgrade

### 4.1 Wire FreeCell to the fork
`[patch.crates-io]` → `freecell-fixes`. Expect the build to break on the `Color` API until 4.2–4.3.

### 4.2 Delete the redundant workarounds
- `open_repair.rs` (E4 is fixed upstream — `xfId` now optional).
- `open_fixups.rs` (E1 theme, E1′ tint fixed upstream; E5 fixed by our fork PR; E2 fixed by our
  fork PR) → delete the module and drop `roxmltree` + `zip` from `freecell-engine`.
- `apply_open_fixups`/`try_repair_and_reload` call sites in `document.rs::open`.

### 4.3 Migrate the style-color read path to `Color`
The engine now returns **unresolved** colors in `Style` (`fill.color: Color`, `font.color:
Color`, `BorderItem.color: Color`, where `Color = Rgb(String) | Theme(i32,f64) | None`).
- **Read/resolve:** replace `style.fill.fg_color.as_deref()` etc. with
  `resolve_color(&style.fill.color)` (via the worker's `UserModel`) or `color.to_rgb(&workbook.theme)`;
  treat `Color::None` as "no fill / default". Affects `cache.rs` (`render_style`, `edge_from`,
  `parse_color`) and `document.rs` (`resolve_text_color`, `default_font`, font-override path).
- **Set (unchanged):** `update_range_style(area, "fill.fg_color", hex)` still works upstream — no
  change to fill-setting code.
- **Tests/fixtures:** update assertions that read `fill.fg_color` (`document.rs`, `cache.rs`,
  `fixtures.rs`) to the resolved-`Color` form.

### 4.4 Validation (D6 — the deliverable)
- FreeCell `cargo test`/workspace checks green.
- Assert `resolve_color` yields `open_fixups`' expected RGBs for the theme + indexed fixtures.
- Visual pass (owner): open the mortgage (custom purple theme), the Numbers file (custom indexed
  palette + `xfId`-less styles), and a currency/accounting file (num-fmt) — confirm colours,
  number formats, and that the file opens at all. Open→save→reopen one affected file.

## 5. Out of scope
- **Already fixed upstream, now simply consumed:** E4 `xfId`, E1 theme, E1′ tint (no work beyond
  deleting the hacks that compensated for them).
- **Move to a released IronCalc pin** once the fixes ship in a version → slim follow-up
  `projects/ironcalc-upgrade.md` (this project deliberately pins git-`main` for validation).
- **API-visibility cluster** (Clipboard/BorderArea/Function/default-font) and **parser depth-cap**
  — deferred fast-follows.
- **Kept FreeCell-side / large features** — unchanged from the inventory.

## 6. Edge cases & risks
- **Unreleased git-`main` drift/instability.** Pin `freecell-fixes` to a recorded base SHA; re-sync
  deliberately. Accept that the shipping pin comes later (D7).
- **API drift beyond colors.** `main` may have changed other surfaces FreeCell touches; the 4.1
  compile pass is the discovery mechanism — budget for incidental fixes beyond the color path.
- **`Color::None` semantics.** Must map to "no fill"/default, not a black/`#000000` resolve — get
  this right in `cache.rs` (guard before `to_rgb`).
- **E2 date ids are locale-sensitive** — fix only 5–8/37–49; dates are a separate follow-up.
- **Save round-trip.** Some hacks (indexed borders) touched save output; validate open→save→reopen,
  not just on-screen render.
- **E5 not visually verifiable pre-fix in isolation** — but here it *is*, because FreeCell builds
  on the fork; the Numbers file’s indexed palette is a direct visual check.

## 7. Constraints
- No upstream PR without explicit owner sign-off (D5).
- FreeCell must end green (tests + visual) with the workarounds removed — that green *is* the
  proof the removal is correct.
- Fork fixes pass `cargo test` + `make lint`; synthetic fixtures only; no model identifier in fork
  commits/PRs.
- The git-`main` pin is temporary and clearly marked; the fork carries the minimum diff.
