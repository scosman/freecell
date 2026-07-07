# Manual Verification — IronCalc Hack Removal (E1–E5)

**Purpose.** Confirm, hands-on, that the five IronCalc import fixes work **with FreeCell's local
workarounds removed**, and that colours / number formats / values did **not** regress.

**Branch under test:** `claude/ironcalc-workarounds-oss-rlt0i1`. It builds FreeCell against our fork
(`scosman/ironcalc#freecell-fixes` = upstream `main` + our E2/E5 fixes) via a `[patch.crates-io]`
already wired in `app/Cargo.toml`, and it has `open_fixups.rs` / `open_repair.rs` **deleted**.

---

## 0. Read this first — the one expected difference (NOT a regression)

Building against unreleased upstream `main` also pulls an **unrelated font/geometry refresh**:
default font is now **12 pt Inter** (was 13 pt Calibri) and default **row height / column width**
changed. So:

- **The app will look a little different** (Inter font, slightly tighter rows). That is expected.
- **`cargo test --workspace` shows ~21 failures**, all in `freecell-engine`'s cache↔engine
  **geometry-agreement** + `default_font` unit tests (see §5). They assert *row-height / font-size
  defaults*, **not** colours or values. They are the known cost of the git-`main` upgrade, tracked
  separately (`implementation_plan.md` Phase-3 finding). **Do not treat them as fix regressions.**

A **real** regression = a wrong colour, a `#VALUE!`, a file that won't open, or a wrong cell value.

---

## 1. Build

```sh
cd app
cargo build --workspace         # first build compiles IronCalc from the fork (a few min)
```

macOS is best for the visual pass (primary target; Finder-open still CLI-only, so pass the path).

---

## 2. Automated quick-check (do this first — it front-loads most of the confidence)

**a) The two fix-relevant integration suites must be GREEN** (they open the real fixture and
round-trip styles/values — this is the strongest automated signal):

```sh
cd app
cargo test -p freecell-engine --test open_numbers_fixture   # E4: Numbers file opens, values correct
cargo test -p freecell-engine --test roundtrip              # values + Color fills/fonts survive save→reopen
```

Expected: `open_numbers_fixture` 1/1 pass, `roundtrip` 19/19 pass. (Confirmed green on this branch.)
`open_numbers_fixture` passing **is** E4 verified: `numbers_table.xlsx` opens even though
`open_repair` is gone.

**b) The fork's own engine tests are green** (proves E2/E5 at the engine level):

```sh
# in your fork checkout (scosman/ironcalc, branch freecell-fixes)
cargo test -p ironcalc_base    # 2107 pass (incl. the E2 num-fmt table tests)
cargo test -p ironcalc         # 213+ pass (incl. the E5 <indexedColors> tests)
```

**c) Full workspace** — `cargo test --workspace` — expect ONLY the ~21 geometry/default failures in
§5. **Any colour / value / open / format test failure outside that list is a real regression → stop
and report it.**

---

## 3. The five fixes — open & eyeball

Run `cargo run -p freecell-app -- <path>` from `app/`.

### E4 — the Numbers file opens at all  *(was: fails to open)*
- **File:** `app/crates/freecell-engine/tests/fixtures/numbers_table.xlsx`
- **Do:** `cargo run -p freecell-app -- crates/freecell-engine/tests/fixtures/numbers_table.xlsx`
- **PASS:** the window opens and shows `Table 1` (A1), `ASDF` (A3), a `Test 1` header (B2), and
  column B counting powers of two down the rows (1, 2, 4, 8, … 16384).
- **Old bug:** a "Couldn't open the workbook" dialog — `XML Error: Missing "xfId"`.

### E5 — the Numbers custom indexed palette renders the file's colours  *(was: Excel defaults)*
- **File:** the same `numbers_table.xlsx`. Look at the **fills**:
  | Where | Expect (file's palette) | Old bug (default palette) |
  |---|---|---|
  | Column A label band | light grey `#DBDBDB` | bright **blue** `#0000FF` |
  | Header band | light grey `#BDC0BF` | **white** `#FFFFFF` |
  | TOTAL cells | gold `#FFD931` / red-orange `#FE634D` | **yellow** `#FFFF00` / **magenta** `#FF00FF` |
- **PASS:** greys + gold/red-orange, **not** blue/yellow/magenta.
- **Border sub-case** (`fixtures/FONTS.xlsx`, indexed border colours): if your build paints cell
  borders, the edges should be the file's grey, not red/green. If the grid doesn't render borders
  yet, verify instead by save→reopen (the border colour must survive) or skip — lowest priority.

### E1 — a custom theme resolves correctly  *(was: wrong theme colours)*
- **File:** a workbook with a **custom theme** — e.g. your mortgage calculator with the purple
  "Custom 8" theme (the file that first surfaced this). *Not committed (bring your own.)*
- **PASS:** theme-coloured fills/fonts match Excel/Numbers — the purple header is **purple**, white
  label cells are **white**, lavender bands are **lavender**.
- **Old bug:** purple→**navy**, white→**solid black**, lavender→**blue-grey**.
- *Quick way to make one:* in Excel, Page Layout → Themes → pick/customize a non-Office theme, fill
  a few cells with theme colours (+ a tint), save as `.xlsx`.

### E2 — built-in number formats render  *(was: `#VALUE!`)*
- **File:** a workbook whose cells use **built-in** currency/accounting/number formats (ids 5–8,
  37–44) — e.g. the mortgage file's loan / payment / total cells, or a new file where you apply
  Excel's Accounting/Currency/comma styles and **don't** hand-edit the format code.
- **PASS:** those cells show formatted numbers (e.g. `175,000.00`, `$1,234`, `(1,234.00)`), **not**
  `#VALUE!`.
- **Old bug:** every such cell showed `#VALUE!` even though the value was fine.

### E3 — date/time built-ins render  *(was: residual mis-map)*
- **File:** a workbook with **date/time** cells using built-in date formats (ids 14–22) — e.g. cells
  formatted `Short Date`, `Long Date`, `Time` in Excel.
- **PASS:** they render as dates/times (`1/1/2021`, `13:45`), not raw serials (`44197`) or garbage.
- *Note:* E2's fix rebuilt the **whole** built-in table, so the old E3 date/time residual is covered
  too. (One known engine gap: format id 47 `mmss.0` still won't render — separate upstream formatter
  issue, out of scope.)

---

## 4. Regression checks (did anything colour/format/value break?)

Open a few *ordinary* files and confirm the engine still behaves:

- [ ] **Explicit RGB colours** — a file with hand-picked fill/font colours → renders **exactly** as
  authored (the fix only touches `indexed=`/`theme=`; explicit `rgb=` must be untouched).
- [ ] **Standard-palette file** (no `<indexedColors>` override) — indexed colours render the Excel
  defaults, unchanged (E5 only activates when the file supplies an override).
- [ ] **Values / formulas** — numbers, text, booleans, and formulas all show correct results
  (type a `=SUM(...)`, confirm it evaluates).
- [ ] **Round-trip** — open a styled file, Save, reopen → colours, number formats, and values all
  survive (this is what `--test roundtrip` checks automatically; spot-check one file by hand too).
- [ ] **New empty workbook** — launches, edit a cell, Save, reopen cleanly.

If all of these look right, the colour/format/value surface is **not** regressed.

---

## 5. Known-failing tests (expected — the git-`main` geometry/font drift, not the fixes)

`cargo test --workspace` will fail these ~21, **all** in `freecell-engine`. They assert the
cache↔engine **geometry agreement** or the **default font**, which changed in `main` (row/col
defaults; 12 pt Inter vs 13 pt Calibri). Spot-check a couple of messages — they say things like
`row 0 height mismatch: cache=24 engine=21.43` or `left: 12 right: 13`, i.e. **geometry/font, never a
wrong colour or value**:

- `cache::tests::` — `build_matches_engine_empty`, `build_matches_engine_band_only`,
  `build_matches_engine_styled_fixture`, `mirror_set_style_each_attr_agrees`,
  `negative_control_skipping_a_mirror_diverges`
- `document::tests::default_font_reads_workbook_default`
- `worker::run::tests::` — the `*_agrees` / `*_band*` / `set_font_*` / `set_rows_height*` /
  `set_borders_*` / `set_style_path_*` / `sheet_switch_builds_cache_on_activation` /
  `load_builds_active_sheet_cache` / `style_edit_mirrors_cache_*` / `undo_redo_agreement_walk` /
  `multiline_input_mirrors_row_height_and_agrees` group

These get fixed in the separate engine-upgrade work (reconcile the geometry/font defaults + refresh
render baselines). They are **out of scope** for verifying E1–E5.

---

## 6. Sign-off checklist

- [ ] E4 — `numbers_table.xlsx` opens (app + `--test open_numbers_fixture`)
- [ ] E5 — its label/header bands are grey; TOTALs gold/red-orange (not blue/yellow/magenta)
- [ ] E1 — a custom-theme file renders the file's theme colours (purple header, etc.)
- [ ] E2 — built-in currency/accounting cells show numbers, not `#VALUE!`
- [ ] E3 — built-in date/time cells render as dates
- [ ] Regression §4 — explicit-rgb, standard-palette, values/formulas, round-trip, new-book all OK
- [ ] Only the §5 geometry/default tests fail; nothing colour/value/open/format outside that list

If every box holds, removing the workarounds is confirmed correct, and E2/E5 are safe to PR upstream.
```
```
