---
status: draft
---

# Phase 3: Sub-project B — File Support (Formualizer vs IronCalc bake-off)

## Overview

Sub-project B answers, head-to-head for **both** candidate engines: *can we
load/edit/save modern `.xlsx` and CSV, and what survives a
load → edit → save → reload round-trip?* (functional_spec §6.B, architecture §6,
§1.1 engine bake-off). It is a **file structure / values / formulas / sheets**
study — deep *style* round-trip fidelity is Sub-project D's job, so here we probe
styles only enough to state "the write path carries styles / it does not," without
duplicating D's formatting deep-dive.

Two isolated crates under `experiments/01-file-support/`:

- `formualizer/` — read via **calamine** (`CalamineAdapter`), write via
  `Workbook::to_xlsx_bytes()` (**umya** backend); CSV via `CsvAdapter`.
- `ironcalc/` — native xlsx read (`load_from_xlsx_bytes` → `Model::from_workbook`)
  + native write (`save_xlsx_to_writer` into an in-memory `Cursor`); CSV via a thin
  reader/writer over `set_user_input` / `get_formatted_cell_value` (IronCalc has no
  built-in CSV, which is itself a finding).

Both crates generate all inputs from committed code (shared `datagen` for CSV +
synthetic values; a small in-crate `.xlsx` writer built from the engine itself — no
hand-made binary fixtures, per functional_spec §5.3). Every round-trip claim is
asserted in a test that **reloads and checks specific cells**, so the fidelity table
in `findings.md` is backed by passing tests (guardrail: verify, don't assume).

Then `experiments/01-file-support/findings.md` is rewritten (§5.2 headings) as a
Formualizer-vs-IronCalc comparison: what each reads/writes, a concrete per-engine
"what survives" table, missing/needed features, API ergonomics for load+save in
FreeCell, and a recommended file-I/O design + next-best.

### Grounding facts already established (do not re-derive)

- Formualizer 0.7.0 (`00-stack-decision/smoke`): `to_xlsx_bytes` writes
  values+formulas; **formula cached results are dropped** (calamine reads
  `value=None, formula=Some(..)`); re-eval after `prepare_graph_all()` restores
  them. Styles are **not** surfaced on the calamine read path (`styles=false`,
  `CellData.style=None`) — D owns styles.
- IronCalc 0.7.1 (`02-.../ironcalc/tests/smoke.rs`): `evaluate()` is full-workbook
  (no incremental); styles **are** present on read (`get_style_for_cell`); storage
  is `HashMap<i32,HashMap<i32,Cell>>`. File I/O lives in the `ironcalc` crate
  (`import::load_from_xlsx_bytes`, `export::save_xlsx_to_writer`).

### Verified API pointers for this phase (read from the registry source)

- IronCalc read: `ironcalc::import::load_from_xlsx_bytes(&[u8], name, locale, tz)
  -> Result<Workbook>`, then `ironcalc_base::Model::from_workbook(workbook, lang)
  -> Result<Model>`. (`load_from_xlsx` is path-based.)
- IronCalc write: `ironcalc::export::save_xlsx_to_writer<W: Write + Seek>(&Model, W)
  -> Result<W>` — pass a `std::io::Cursor<Vec<u8>>` for an in-memory round-trip.
  (`save_to_xlsx` refuses to overwrite / needs a path; the writer form is what we
  use.)
- IronCalc sheets: `Model::new_sheet() -> (String, u32)`,
  `Model::add_sheet(&str)`, `Model::get_worksheets_properties() -> Vec<SheetProperties{name,..}>`.
- IronCalc cells/format: `set_user_input(sheet,row,col,String)`,
  `get_cell_value_by_index`, `get_cell_formula`, `get_formatted_cell_value`,
  `get_style_for_cell` / `set_cell_style(sheet,row,col,&Style)`, `Style.num_fmt:
  String`, `set_column_width`/`get_column_width`, `set_row_height`/`get_row_height`.
- Formualizer file path (from `smoke/src/lib.rs`): `CalamineAdapter::open_bytes`,
  `Workbook::from_reader(adapter, LoadStrategy::EagerAll, WorkbookConfig::ephemeral())`,
  `Workbook::to_xlsx_bytes()`, `CsvAdapter::open_bytes`. `add_sheet`, `set_value`,
  `set_formula`, `get_value`, `get_formula`, `sheet_names`, `sheet_dimensions`,
  `prepare_graph_all`, `evaluate_cell`/`evaluate_all`.

## Steps

### 1. `experiments/01-file-support/formualizer/` crate

`Cargo.toml` (isolated crate, mirrors the smoke crate's feature set):

```toml
[package]
name = "formualizer_file"
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"
publish = false

[dependencies]
datagen = { path = "../../shared/datagen" }
anyhow = "1"

[dependencies.formualizer]
version = "0.7"
default-features = false
features = ["eval", "parse", "workbook", "calamine", "csv", "umya", "json", "system-clock"]
```

Add `.gitignore` with `/target` (matches the smoke crate).

`src/lib.rs` — thin, documented helpers wrapping the file-I/O surface, so tests read
cleanly and the module docs double as the captured file-I/O API. Functions:

- `write_synthetic_xlsx(seed, rows, cols) -> anyhow::Result<Vec<u8>>` — build a
  `Workbook` from a `datagen::SyntheticSheet`: numbers via `set_value(Number)`,
  text via `set_value(Text)`, skipping `Empty`; then `to_xlsx_bytes()`. This is the
  committed `.xlsx` generator (no binary fixture).
- `build_feature_workbook() -> Workbook` — a small, hand-composed workbook covering
  the edge cases: literals (int/float/text/bool), a **formula** (`=A1+A2`), a
  **second sheet** (`add_sheet("Sheet2")` with its own values + a **cross-sheet
  formula** `=Sheet1!A1*10`), a **date-serial** cell (Excel serial number, e.g.
  `45658` for 2025-01-01) — documenting that dates are numbers unless a number
  format is applied (styles/number-formats are D's depth, but the value must
  survive). Call `prepare_graph_all()` + `evaluate_all()` before writing.
- `xlsx_bytes(&Workbook) -> Result<Vec<u8>>` (wraps `to_xlsx_bytes`).
- `load_xlsx(&[u8]) -> Result<Workbook>` (calamine adapter + `from_reader`).
- `load_csv(&str) -> Result<Workbook>` (`CsvAdapter`).
- `workbook_to_csv(&Workbook, sheet, rows, cols) -> String` — export path: read
  `get_value` per cell and format as CSV (Formualizer has a CSV *read* adapter but
  we exercise export via values; note this in docs).

### 2. `experiments/01-file-support/ironcalc/` crate

`Cargo.toml`:

```toml
[package]
name = "ironcalc_file"
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"
publish = false

[dependencies]
datagen = { path = "../../shared/datagen" }
anyhow = "1"
ironcalc = "0.7"
ironcalc_base = "0.7"
```

Add `.gitignore` with `/target`.

`src/lib.rs` helpers mirroring the Formualizer crate so tests are symmetric:

- `write_synthetic_xlsx(seed, rows, cols) -> Result<Vec<u8>>` — `Model::new_empty`,
  `set_user_input` per non-empty synthetic cell, then `save_xlsx_to_writer(&model,
  Cursor::new(Vec::new()))?.into_inner()`.
- `build_feature_model() -> Result<Model<'static>>` — same edge cases: literals,
  `=A1+A2`, `new_sheet()`/`add_sheet("Sheet2")` with a **cross-sheet formula**
  `=Sheet1!A1*10`, a **date-serial** value plus a `num_fmt` date format set via
  `set_cell_style` (so we can report IronCalc formats a serial as a date string via
  `get_formatted_cell_value`). `model.evaluate()` before saving.
- `xlsx_bytes(&Model) -> Result<Vec<u8>>` (wraps `save_xlsx_to_writer` + a `Cursor`).
- `load_xlsx(&[u8]) -> Result<Model<'static>>` — `load_from_xlsx_bytes(bytes,
  "roundtrip", "en", "UTC")` then `Model::from_workbook(wb, "en")`. (Leaks/owns
  `'static` strings the same way `ironcalc_bench` does: pass string literals.)
- `model_to_csv(&Model, sheet, rows, cols) -> String` and
  `load_csv_into_model(&str) -> Result<Model<'static>>` — a **hand-rolled** CSV
  bridge (split on `\n`/`,`, minimal RFC-4180 unquoting) that feeds
  `set_user_input`, because IronCalc ships **no** CSV support. Reuse
  `datagen::csv_string` to produce the input; the finding is that CSV is DIY on
  IronCalc.

### 3. `findings.md` (rewrite to §5.2, head-to-head)

Sections: **Questions**, **What was done** (crate pointers + reproduce commands),
**Results / evidence** (the per-engine "what survives" table + API/ergonomics
notes), **Conclusion**, **Recommended design + next-best alternative**, **Risks /
open questions**. Explicitly scope out deep style fidelity (Sub-project D) and note
the coordination boundary. Include a one-command reproduce block per crate.

## Tests

Symmetric integration tests in each crate's `tests/roundtrip.rs`, each reloading and
asserting specific cells (guardrail: claims backed by passing tests). Kept small
(low row/col counts) so they run fast in-container — no large-file perf work here
(that is Sub-project C).

**Formualizer (`formualizer/tests/roundtrip.rs`):**
- `xlsx_literals_survive` — synthetic `.xlsx` → reload → a known number and a known
  text cell read back equal (values survive).
- `xlsx_formula_survives_as_formula_not_cached` — build_feature workbook → save →
  reload: `get_value(A3)` is `None`, `get_formula(A3)` == `=A1+A2`; after
  `prepare_graph_all()` + `evaluate_cell` it recomputes to 3 (locks the
  cached-results-dropped finding).
- `xlsx_multiple_sheets_survive` — Sheet2 exists after reload
  (`sheet_names()` contains it) and its cross-sheet formula reloads as formula text.
- `xlsx_date_serial_survives_as_number` — the date-serial value reloads as the same
  number (documents dates = serials; formatting is D's job).
- `csv_import_reads_values` — `datagen` CSV → `CsvAdapter` → non-empty cells match.
- `csv_export_roundtrips_values` — workbook → `workbook_to_csv` → parse back →
  values match (export path).

**IronCalc (`ironcalc/tests/roundtrip.rs`):**
- `xlsx_literals_survive` — synthetic `.xlsx` → reload → number + text cells equal.
- `xlsx_formula_survives_and_recomputes` — feature model → save → reload:
  `get_cell_formula(A3)` == `=A1+A2`; after `evaluate()`, `get_cell_value_by_index`
  == 3 (does IronCalc keep the cached value too? assert what it actually does and
  record it — likely keeps both formula and a computed value on reload).
- `xlsx_multiple_sheets_survive` — `get_worksheets_properties()` contains Sheet2
  after reload; cross-sheet formula recomputes.
- `xlsx_date_number_format_survives` — the date-serial value survives and
  `get_formatted_cell_value` still renders it as a date (number format survives —
  a concrete IronCalc advantage to record; deep style fidelity stays with D).
- `xlsx_styles_survive_shallow` — set one bold cell, save, reload, assert
  `get_style_for_cell(..).font.b` is still true (shallow style-survival probe only;
  depth is D). This documents that IronCalc's native writer carries styles.
- `csv_bridge_roundtrips_values` — `datagen` CSV → `load_csv_into_model` →
  `model_to_csv` → values match (DIY CSV works but is ours to build).

Plus each crate keeps its helper unit tests (e.g. CSV field parsing) where useful.

## Checks (per crate, run in each crate dir)

`cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo build`, `cargo test`. `target/` is gitignored. Git ops path-scoped to
`experiments/01-file-support/`; no commit (manager commits).
</parameter>
</invoke>
