---
status: draft
---

# Architecture: IronCalc Upstreaming

> Scope = Option 1 (E2 + E5 upstreamed to the fork; FreeCell migration deferred). See
> `functional_spec.md` + `fork_audit.md`. All paths below are in `scosman/ironcalc`
> (`/workspace/ironcalc` in this container) unless prefixed `freecell:`.

## 1. Repos, crates, branches

- **Fork:** `scosman/ironcalc` — workspace: `base/` → crate `ironcalc_base`, `xlsx/` → crate
  `ironcalc` (both `0.7.1` in their manifests, but `main` is ahead of the crates.io release).
- **Branch topology (D1):**
  - `main` — clean mirror of upstream `ironcalc/IronCalc:main`. Sync before starting.
  - `fix/e2-numfmt` off `main` — the E2 fix + tests. (crate `ironcalc_base`)
  - `fix/e5-indexed` off `main` — the E5 fix + tests. (crate `ironcalc`, import path)
  - `freecell-fixes` off `main` — merges both fix branches (staging for the future upgrade
    project; nothing in *this* project builds against it).
- **Spec home:** `freecell:specs/projects/ironcalc-upstreaming/` (this repo). FreeCell code is
  untouched.

## 2. E2 — built-in num-fmt table (`base/src/number_format.rs`)

- **Where:** `const DEFAULT_NUM_FMTS: &[&str]` (line 7) — a positional table indexed by
  `numFmtId`. `get_num_fmt(id, num_fmts)` (line 64) returns a workbook-defined fmt if present,
  else `DEFAULT_NUM_FMTS[id]`. Wrong entries include index 39 = `"t0.00"` (line 47) and the
  `"t0.00 %"` neighbour (line 51).
- **Fix:** replace the garbage entries in the **locale-independent** block (ids 5–8, 37–49) with
  the ECMA-376 built-in codes. Reference mapping (from FreeCell's proven
  `STANDARD_BUILTIN_NUM_FMTS`):

  | id | code |
  |----|------|
  | 5 | `$#,##0_);($#,##0)` |
  | 6 | `$#,##0_);[Red]($#,##0)` |
  | 7 | `$#,##0.00_);($#,##0.00)` |
  | 8 | `$#,##0.00_);[Red]($#,##0.00)` |
  | 37 | `#,##0_);(#,##0)` |
  | 38 | `#,##0_);[Red](#,##0)` |
  | 39 | `#,##0.00_);(#,##0.00)` |
  | 40 | `#,##0.00_);[Red](#,##0.00)` |
  | 41–44 | accounting codes (see FreeCell `open_fixups.rs:519-528`) |
  | 45 `mm:ss` · 46 `[h]:mm:ss` · 47 `mmss.0` · 48 `##0.0E+0` · 49 `@` | |

- **Do NOT touch date/time ids 14–22** in this PR — they are locale-substituted and risk
  regressing localized output. Note them as a separate follow-up.
- **Caveat to verify:** IronCalc's `format_number` may special-case some ids (dates) *before*
  consulting this table; confirm the corrected codes actually flow through `get_formatted_cell_value`
  for the ids we change (they did in FreeCell's end-to-end check for id 39).
- **Tests:** in `number_format.rs`'s test module — for each changed id assert `get_num_fmt(id, &[])`
  equals the ECMA code, and that a representative value formats correctly (e.g. `175000` @ id 39
  → not `#VALUE!`). Follow existing test idioms in the file.

## 3. E5 — honour `<colors><indexedColors>` (`xlsx/src/import/`)

- **Current behaviour:** `util.rs::get_color(node, _theme)` (line 39) resolves an `indexed="n"`
  colour to `Color::Rgb(get_indexed_color(n))` (base's hardcoded 56-colour palette); index 64 →
  `Color::None`. The workbook's `<colors><indexedColors>` override (OOXML §18.8.27) is **never
  read** anywhere.
- **`get_color` callers (all currently pass `&Theme`):** `styles.rs` fonts (114), fills
  fg/bg (189/192), border/dxf colours (43, 416, 454, 455); `worksheets.rs` tab colour (202);
  `conditional_formatting.rs` (99, 417, 518). The override is **workbook-global**, so it must be
  threaded to every `get_color` call, exactly along the path `&Theme` already travels.
- **Chosen design — read-at-import, thread via the existing theme path (least churn):**
  1. Parse `<colors><indexedColors>` once in `styles.rs::load_styles` (it holds the
     `style_sheet` root) into a positional `Vec<String>` (`AARRGGBB`→`#RRGGBB`, drop alpha).
     Empty/absent → `None`.
  2. Replace the threaded `&Theme` with a small `&ColorContext { theme: &Theme, indexed:
     Option<&[String]> }` (or add an `indexed: Option<&[String]>` param to `get_color`). Because
     the override is workbook-global, build it at workbook-load and pass it wherever `theme` is
     passed today.
  3. In `get_color`'s `indexed` branch: if `indexed` is `Some(p)` and `n` is in range, resolve to
     `Color::Rgb(p[n].clone())`; else fall back to `get_indexed_color(n)`. Index 64 stays
     `Color::None`. `rgb=`/`theme=`/`auto=` paths unchanged.
- **Alternative — deferred `Color::Indexed(i32)` variant** (rejected): add a variant preserved
  through load and resolved late (like `Theme::resolve`). More faithful to symbolic round-trip,
  but touches `Color`'s `serde`/`bitcode` derives, export, and every `match` on `Color` across
  both crates — over-invasive for an import-fidelity fix. Revisit only if a maintainer prefers it.
- **Tests:** a crafted `styles.xml` with `<indexedColors>` resolves indexed fills/fonts/borders
  to the file's palette; a file with **no** override is unchanged (guard against regressing
  standard-palette files). Port FreeCell's `correct_indexed_colors` crafted cases + guards
  (out-of-range, index 64, `rgb=` precedence).

## 4. Test & lint conventions (fork)

- **Build/test:** `cargo test` (workspace) or `make test-rust`. Per-crate: `cd base && cargo test`
  / `cd xlsx && cargo test`.
- **Lint:** `make lint` = `cargo fmt -- --check` + `cargo clippy --all-targets --all-features -W
  clippy::unwrap_used -W clippy::expect_used -W clippy::panic -D warnings`. So **non-test** code
  must avoid `unwrap`/`expect`/`panic`; **test** modules opt out with `#![allow(clippy::unwrap_used)]`
  (as `xlsx/src/import/util.rs:1` already does). Match the file's existing style.
- **Fixtures:** synthetic inline XML / crafted zips only — no copyrighted `.xlsx`. (FreeCell's
  existing `open_fixups`/`open_repair` tests are already this shape and port directly.)
- **Commits:** follow the fork's `CONTRIBUTING.md` (branch per change, clear messages); keep MIT/
  Apache headers; do NOT include any model identifier in fork commits/PRs.

## 5. Upstream submission (on sign-off, D4/D5)

- One PR per fix against `ironcalc/IronCalc:main`, PR-first: `fix/e2-numfmt` and `fix/e5-indexed`.
- Each PR body: the defect (with a minimal repro / failing-input), the fix, and the tests added.
- Push order: fork branches first (any time); PRs only after the owner signs off.
- Track fix ↔ branch ↔ PR status in `implementation_plan.md` / a status table.

## 6. Deferred (separate project)

FreeCell's `Color`-enum migration, in-app visual validation, and deletion of
`open_fixups.rs`/`open_repair.rs` (+ `roxmltree`/`zip`) live in `freecell:projects/ironcalc-upgrade.md`
(status Future), gated on an IronCalc release carrying all five fixes.
