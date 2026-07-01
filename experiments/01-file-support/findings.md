# Sub-project B — File Support (Formualizer vs IronCalc bake-off)

> Status: **complete.** Head-to-head file-I/O round-trip study for the two candidate
> engines (functional_spec §6.B, the §6 engine-bake-off note, §5.2/§5.3; architecture
> §1.1, §6). UI is settled (GPUI); this phase informs the **engine** decision
> (Sub-project G) on the file-I/O axis.
>
> Environment: Rust 1.94.1, 4 cores / ~15 GB RAM, no GPU/display. Date: 2026-07-01.
> Versions probed: **Formualizer 0.7.0** (calamine 0.35, umya-spreadsheet 2.3.2 via
> the meta-crate); **IronCalc 0.7.1** (`ironcalc` facade + `ironcalc_base` engine).
>
> **Scope boundary (coordinate with Sub-project D).** This is a **file
> structure / values / formulas / sheets** study. Deep *style* round-trip fidelity
> (which style attributes survive, how faithfully) is **Sub-project D's** job. We
> cover styles only shallowly: *does the write path carry styles at all?* — one bold
> cell + the date number-format. We do **not** enumerate style faithfulness here.

## Questions

1. Can each engine **read** and **write** modern `.xlsx`, and read/write **CSV**?
2. What survives a **load → edit → save → reload** round-trip on each engine
   (values, formulas, cached results, multiple sheets, cross-sheet refs, dates,
   number formats, booleans, shared strings)?
3. How **suitable/ergonomic** is each engine's load+save API for FreeCell?
4. What is **missing** and would need building or a fallback library?
5. What is the **recommended file-I/O design** and the next-best alternative?

## What was done

Two isolated Cargo crates under `experiments/01-file-support/`, each doing a real
`load → edit → save → reload → diff` round-trip and asserting specific reloaded cells
so every fidelity claim below is backed by a passing test (guardrail: verify, don't
assume). All inputs are produced by committed code — the shared, frozen
`datagen` generators (synthetic values + CSV) and small in-crate workbook builders —
so there are **no hand-made binary fixtures** (functional_spec §5.3).

- **`formualizer/`** — read `.xlsx` via `CalamineAdapter` + `Workbook::from_reader`;
  write via `Workbook::to_xlsx_bytes()` (umya backend); CSV read via `CsvAdapter`;
  CSV export by reading `get_value` per cell.
  - `formualizer/src/lib.rs` — documented file-I/O helpers (`write_synthetic_xlsx`,
    `build_feature_workbook`, `xlsx_bytes`, `load_xlsx`, `load_csv`,
    `workbook_to_csv`).
  - `formualizer/tests/roundtrip.rs` — 6 round-trip probes.
- **`ironcalc/`** — read `.xlsx` via `ironcalc::import::load_from_xlsx_bytes` +
  `Model::from_workbook`; write via `ironcalc::export::save_xlsx_to_writer` into an
  in-memory `Cursor`; **CSV is hand-rolled** (IronCalc ships none).
  - `ironcalc/src/lib.rs` — helpers (`write_synthetic_xlsx`, `build_feature_model`,
    `xlsx_bytes`, `load_xlsx`, `sheet_names`, `load_csv_into_model`, `model_to_csv`,
    `is_bold`) plus a minimal RFC-4180 CSV parser/escaper.
  - `ironcalc/tests/roundtrip.rs` — 6 round-trip probes.

### Reproduce

```sh
cd experiments/01-file-support/formualizer
cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test

cd ../ironcalc
cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test
```

All checks pass; **7 tests** (6 round-trip + 1 unit) for Formualizer, **8 tests** (6
round-trip + 2 unit) for IronCalc, all green.

## Results / evidence

### Read/write matrix

| Capability | Formualizer 0.7.0 | IronCalc 0.7.1 |
|---|---|---|
| `.xlsx` **read** | Yes — `calamine` backend | Yes — **native** importer |
| `.xlsx` **write** | Yes — `to_xlsx_bytes()` (umya backend) | Yes — **native** exporter (styled) |
| In-memory bytes (no temp file) | Yes (`open_bytes` / `to_xlsx_bytes`) | Yes (`load_from_xlsx_bytes` / `save_xlsx_to_writer` + `Cursor`) |
| `.xlsx` write refuses overwrite? | N/A (returns bytes) | `save_to_xlsx` (path form) **refuses to overwrite**; use the writer form |
| CSV **read** | Yes — `CsvAdapter` | **No built-in** — hand-rolled bridge required |
| CSV **write** | Not first-class (export via `get_value`) | **No built-in** — hand-rolled bridge required |
| `.ods` read | Yes (calamine) — not exercised here | No |
| Internal binary format | — | `.ic` (bitcode) via `save_to_icalc`/`load_from_icalc` |

### Round-trip fidelity — what survives `load → edit → save → reload`

Backed by the `tests/roundtrip.rs` assertions in each crate.

| Element | Formualizer (umya write / calamine read) | IronCalc (native write / native read) |
|---|---|---|
| **Number literals** | Survive as values | Survive as values |
| **Text literals** | Survive as values | Survive as values |
| **Int vs float** | `Int(1)` reads back as **`Number(1.0)`** (calamine normalizes to f64) | Numbers are f64 throughout (`CellValue::Number`) |
| **Booleans** | Survive as `Boolean(true)` | Survive as `Boolean(true)` |
| **Formula text** | Survives (`get_formula` → `"=A1+A2"`) | Survives (`get_cell_formula` → `"=A1+A2"`) |
| **Cached formula result** | **DROPPED** — after reload `get_value` is `None`; needs `prepare_graph_all()` + eval to restore | Not surfaced as a stored cache, but a single `evaluate()` after reload yields the value (full recompute) |
| **Recompute after reload** | Correct after `prepare_graph_all()` + `evaluate_cell` (→ 3.5) | Correct after `evaluate()` (→ 3.5) |
| **Multiple sheets** | Survive (`sheet_names()` has Sheet1+Sheet2) | Survive (`get_worksheets_properties()` has Sheet1+Sheet2) |
| **Cross-sheet formula** | Survives as text; recomputes (`=Sheet1!A1*10` → 10) | Survives; recomputes (→ 10) |
| **Dates (serial value)** | Survives as the numeric serial (45658) | Survives as the numeric serial (45658) |
| **Date number format** | **Not surfaced on read** (calamine `styles=false`) — value only | **Survives**: `get_formatted_cell_value` renders `"2025-01-01"` after reload |
| **Shared strings** | Handled transparently (text values survive) | Handled transparently (text values survive) |
| **Styles (shallow: 1 bold cell)** | **Not surfaced on the read path** (Sub-project A finding; not readable via `CellData` in 0.7.0) | **Survives** the native writer: `get_style_for_cell(..).font.b == true` after reload |

**File sizes (evidence the writers produce real, small OOXML):** feature workbook →
Formualizer 6,121 bytes / IronCalc 4,407 bytes; a 100×20 synthetic sheet →
Formualizer 20,317 bytes / IronCalc 18,535 bytes. Both are valid ZIP/OOXML (verified
`PK` magic + successful reload).

### Headline structural differences

- **Formualizer drops cached formula values on write** (umya path): the reloaded
  file carries formula *text* only, so opening a workbook requires a graph rebuild +
  recompute before dependent cells show values. IronCalc's importer yields a model
  that produces correct values after a single `evaluate()`. Both need a recompute on
  open; the difference is that Formualizer's `get_value` is `None` until you do so,
  whereas IronCalc's evaluate is a **full-workbook** recompute (no incremental path —
  the Sub-project C perf lens; heavier on huge files).
- **IronCalc carries styles + number formats through its native writer** (bold, date
  format survive); **Formualizer's read path does not surface styles at all** in
  0.7.0 (calamine `styles=false`, `CellData.style=None`). Formatting on the
  Formualizer side must be read from the underlying `umya_spreadsheet` workbook
  directly or held in a FreeCell-side store — this is **Sub-project D's** deep-dive;
  we only confirm the direction here.
- **CSV is first-class on Formualizer, absent on IronCalc.** Formualizer imports CSV
  as a single sheet via `CsvAdapter`. IronCalc has no CSV; our ~40-line RFC-4180
  bridge over `set_user_input`/`get_formatted_cell_value` round-trips values
  correctly, but FreeCell would own that code on the IronCalc path.

### API suitability / ergonomics for load+save in FreeCell

**Formualizer.**
- Read: `CalamineAdapter::open_bytes(Vec<u8>)` then `Workbook::from_reader(adapter,
  LoadStrategy::EagerAll, WorkbookConfig::ephemeral())`. `LoadStrategy` (incl.
  `LazyRange`) is a real knob for big-file streaming — a plus for the huge-sheet
  thesis, though not exercised here.
- Write: `wb.to_xlsx_bytes() -> Result<Vec<u8>>` — dead simple, returns bytes; no
  temp-file dance.
- CSV: symmetric adapter for read.
- Rough edge: `to_xlsx_bytes` errors and the reader trait need trait imports in
  scope; formula results aren't materialized in the file (recompute-on-open needed).

**IronCalc.**
- Read: `import::load_from_xlsx_bytes(&[u8], name, locale, tz) -> Workbook`, then
  `Model::from_workbook(workbook, language)` — **two steps** and four
  locale/tz/language string args to thread through consistently (a mismatch across
  build/reload could shift formatting).
- Write: `export::save_xlsx_to_writer(&Model, W: Write+Seek) -> Result<W>` — pass a
  `Cursor<Vec<u8>>` and `into_inner()` for bytes. The convenience `save_to_xlsx`
  takes a **path and refuses to overwrite an existing file**, so it's unsuitable for
  a normal "Save" — the writer form is the one to use.
- No CSV, no `Vec<u8>` "save to bytes" convenience (must build the `Cursor`), and
  `evaluate()` is whole-workbook.
- Plus: styles/number-formats are readable and writable through the same `Model` API
  (`get_style_for_cell`/`set_cell_style`), so formatting persistence is built in.

## Conclusion

**Both engines can load and save modern `.xlsx` in-memory, and both round-trip
values, formulas, multiple sheets, cross-sheet references, dates-as-serials, and
booleans** — all asserted by passing tests. The decisive file-I/O differences:

- **Formualizer** has **first-class CSV** and a dead-simple `to_xlsx_bytes()`/
  `LoadStrategy` API, but its write path **drops cached formula results** (formula
  text only) and its read path **does not surface styles/number-formats** in 0.7.0
  (calamine `styles=false`). Formatting fidelity is bounded by reading umya directly
  (Sub-project D).
- **IronCalc** has a **native, styled** reader/writer that **preserves styles and
  number formats** (bold + date format survived) and yields an evaluatable model on
  reload, but it has **no CSV** (DIY bridge required), a slightly clunkier
  two-step + four-arg load API, an overwrite-refusing path-based save convenience,
  and a **full-recompute-only** evaluate (the Sub-project C perf caveat) that makes
  open-time recompute costlier on huge files.

We could **not** determine deep style faithfulness here because that is deliberately
Sub-project D's scope (we confirmed only *that* IronCalc carries styles and that
Formualizer does not surface them on read). We also did not stress a 100 MB+ file in
this phase — file-load time/peak-memory at scale is a §5.4 discovery metric owned by
Sub-project C's memory/load benchmarks; here the focus is fidelity correctness.

Net for the file-I/O axis: **IronCalc is the stronger file-fidelity story
out-of-the-box** (styles + number formats survive, native writer), while
**Formualizer is stronger on CSV and API simplicity** but needs a
formatting-read path and a recompute-on-open. Neither is a blocker; both are viable.
This is one input to the engine decision (Sub-project G), to be weighed against
Formualizer's Arrow huge-sheet perf fit vs IronCalc's `HashMap` storage.

## Recommended design + next-best alternative

**Recommended file-I/O design (engine-agnostic, works for either winner):**

1. **`.xlsx` read/write through the chosen engine's native/first-party path**
   (Formualizer: calamine + `to_xlsx_bytes`; IronCalc: `load_from_xlsx_bytes` +
   `save_xlsx_to_writer`), always on **in-memory bytes** so FreeCell owns temp-file
   and atomic-save policy.
2. **Recompute on open.** Both engines require a post-load evaluate to materialize
   formula results; make "open → build/prepare graph → evaluate → render" the load
   pipeline. On Formualizer this is mandatory (values are `None` otherwise); on
   IronCalc budget for its full-workbook `evaluate()` cost on large files.
3. **A FreeCell-side formatting store** keyed by cell, populated on load and written
   on save, is the safe design **regardless of engine** — it removes the dependency
   on whether the engine surfaces styles on read (Formualizer does not in 0.7.0) and
   gives FreeCell one formatting model across engines. Sub-project D designs this;
   this phase's finding is that it is *needed on Formualizer* and *optional but
   still advisable on IronCalc* (to normalize the model and cover gaps).
4. **CSV:** use the engine's importer where present (Formualizer `CsvAdapter`);
   otherwise keep the ~40-line RFC-4180 bridge demonstrated here (IronCalc). A
   FreeCell-owned CSV layer is cheap and makes CSV behavior identical across engines
   — recommended either way for consistent quoting/encoding control.

**Next-best alternative (engine-independent file stack):** if the chosen engine's
file fidelity proves insufficient (e.g. Formualizer's style-read gap plus a need for
richer OOXML features), **decouple file I/O from the engine**: read with **calamine**
and/or **umya-spreadsheet** directly (umya preserves styles for round-trip), write
with **rust_xlsxwriter** or **umya**, and feed values/formulas into the engine purely
for calculation. This is the "own the file layer" path flagged in Sub-project A; it
costs more code but caps file-fidelity risk on either engine and lets FreeCell pick
the best-of-breed reader/writer independent of the calc engine.

## Risks / open questions

- **Formualizer drops cached formula results on save** → mandatory recompute-on-open;
  matters for open-time perf on large workbooks (owned jointly with Sub-project C).
- **Formualizer does not surface styles/number-formats on read** (0.7.0) → FreeCell
  needs a umya-direct read path and/or its own formatting store. **Owned by
  Sub-project D** (this phase confirmed the direction, not the depth).
- **IronCalc `evaluate()` is full-workbook** (no incremental) → open-time recompute
  and any edit recompute are O(all cells); a real cost on Excel-max sheets.
  **Owned by Sub-project C.**
- **IronCalc has no CSV** → FreeCell owns a CSV bridge on that path (small, done here).
- **IronCalc load API friction:** two-step load + four locale/tz/language args must be
  threaded consistently across save/reload or formatting/parse behavior can shift;
  `save_to_xlsx` refuses overwrites (use the writer form).
- **No 100 MB+ file / large-workbook load-time + peak-memory measurement** here
  (fidelity-only phase). That §5.4 discovery metric is owned by Sub-project C.
- **Deep style/merge/conditional-formatting fidelity** (IronCalc reportedly supports
  merges + conditional formatting on its native path) is **out of scope here** and
  is Sub-project D's deep-dive — not duplicated in this phase.
- **Int→float normalization** through calamine (`Int(1)` → `Number(1.0)`) is benign
  for a spreadsheet (Excel numbers are f64) but noted so nothing downstream assumes
  an integer literal type survives.
