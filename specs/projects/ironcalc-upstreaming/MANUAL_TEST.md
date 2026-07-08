# Manual Verification — IronCalc Hack Removal (E1–E5)

**Purpose.** Confirm, hands-on, that the five IronCalc import fixes work **with FreeCell's local
workarounds removed**, and that colours / number formats / values did **not** regress.

**Branch under test:** `claude/ironcalc-workarounds-oss-rlt0i1`. It builds FreeCell against our fork
(`scosman/ironcalc#freecell-fixes` = upstream `main` + our E2/E5 fixes) via a `[patch.crates-io]`
already wired in `app/Cargo.toml`, and it has `open_fixups.rs` / `open_repair.rs` **deleted**.

---

## 0. Read this first — the one expected difference (NOT a regression)

Building against unreleased upstream `main` also pulls an **unrelated font/geometry refresh**:
IronCalc's default font is now **12 pt Inter** (was 13 pt Calibri) and its default **row height /
column width** changed. FreeCell keeps its **own** render defaults (24 px rows, 100 px cols, 13 px
cell font) — those are FreeCell's, not IronCalc's to dictate — so the only reconciliation was
recalibrating the two unit-conversion **reference** constants (`IRONCALC_DEFAULT_ROW_HEIGHT_PX` 28→25,
`IRONCALC_DEFAULT_COL_WIDTH_PX` 125→90) and the `default_font` expectation. That is **done** on this
branch. So:

- **The engine test suite is fully green** — `cargo test --workspace` passes (no more geometry/
  `default_font` failures). *(This was ~21 failures before the reconciliation; if you're on an older
  checkout you may still see them — pull the latest branch.)*
- **The app looks the same** as before the upgrade: default cells still render bundled **Inter** at
  FreeCell's own metrics; the engine-default-font change only feeds the cache's "is this the default?"
  detection, not what's painted.

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

**c) Full workspace** — `cargo test --workspace` — expect **all green** (the geometry/`default_font`
reconciliation landed on this branch). **Any failure is a real regression → stop and report it.**

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

### E3 — date/time built-ins render  *(was: residual mis-map)* — **covered by an automated test**
- **No manual step needed** — a date format is a deterministic string, so it's asserted, not eyeballed:
  ```sh
  cd app && cargo test -p freecell-engine --test dates_fixture   # opens tests/fixtures/dates.xlsx
  ```
  It opens a crafted workbook referencing the built-in date/time ids **14–22 undefined** (the exact
  E3 case) and asserts each renders as a date/time, not a raw serial. Verified rendering:
  `id14 → 01-01-21`, `id15 → 1-Jan-21`, `id16 → 1-Jan`, `id17 → Jan-21`, `id22 → 1/1/22 12:00`,
  `id20 → 12:00`, `id21 → 12:00:00`, `id18 → 12:00 PM`.
- *Optional* visual spot-check: open `crates/freecell-engine/tests/fixtures/dates.xlsx` in the app.
- *Note:* E2's fix rebuilt the **whole** built-in table, so the old E3 residual is covered too. (One
  known engine gap: id 47 `mmss.0` still won't render — separate upstream formatter issue, out of
  scope.)

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

## 5. Geometry/font reconciliation — DONE (historical note)

Earlier in this branch, before the reconciliation, `cargo test --workspace` failed ~21
`freecell-engine` tests — **all** cache↔engine **geometry agreement** / **default_font** assertions
(messages like `row 0 height mismatch: cache=24 engine=21.43` or `left: 12 right: 13`), never a wrong
colour or value. That drift is now reconciled: the two unit-conversion reference constants track the
fork's defaults (25/90) and `default_font` expects 12 pt Inter. **All those tests now pass** — there
is no expected-failing list anymore. **Render baselines were verified (by code analysis) NOT to move**:
every `render-tests` scene spawns a `NewWorkbook` and injects custom col/row geometry directly as
device px (`cache.set_col_width`), bypassing the `col_px`/`row_px` conversion; default cells render at
the fixed `CELL_FONT_PX = 13` app constant; and the only explicit case font is 24 pt (≠ 12/13). So no
baseline regeneration was needed.

---

## 6. Sign-off checklist

- [ ] E4 — `numbers_table.xlsx` opens (app + `--test open_numbers_fixture`)
- [ ] E5 — its label/header bands are grey; TOTALs gold/red-orange (not blue/yellow/magenta)
- [ ] E1 — a custom-theme file renders the file's theme colours (purple header, etc.)
- [ ] E2 — built-in currency/accounting cells show numbers, not `#VALUE!`
- [ ] E3 — built-in date/time cells render as dates
- [ ] Regression §4 — explicit-rgb, standard-palette, values/formulas, round-trip, new-book all OK
- [ ] `cargo test --workspace` is fully green (the §5 geometry/default reconciliation landed)

If every box holds, removing the workarounds is confirmed correct, and E2/E5 are safe to PR upstream.
```
```
