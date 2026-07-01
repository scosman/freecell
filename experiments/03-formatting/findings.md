# Sub-project D — Formatting: Research & Pre-validation

> Status: **complete** — Phase 4. Two-engine bake-off (Formualizer vs IronCalc) of
> formatting/metadata exposure, per functional_spec §6.D and architecture §6 / §1.1.
> Runnable probes: `formualizer/` and `ironcalc/` (each `cargo test`); recorded matrices
> in `results/`. Environment: Rust 1.94.1, 4 cores / ~15 GB RAM, no GPU/display,
> 2026-07-01. Versions probed: Formualizer 0.7.0 (+ umya-spreadsheet 2.3.2),
> IronCalc 0.7.1.
>
> Per the human's note, the exhaustive **style-roundtrip-fidelity matrix is deferred to
> Round 2**. This phase delivers the capability probe + the FreeCell formatting-model
> design, not an exhaustive fidelity sweep.

## Questions

1. What formatting/metadata does each engine (and its XLSX layer) expose — row/col
   sizes, bold/italic, font size, fills, borders, number formats, merges, conditional
   formatting — on **read**, on **write**, and across a **load → edit → save → reload
   round-trip**?
2. Does the engine offer format/metadata storage on the same backend, or must FreeCell
   build its own formatting model?
3. Is load → edit formatting → save easy or hard, per engine?
4. What FreeCell formatting model does this imply — **native (via engine / umya)** vs a
   **side-table keyed by cell** vs a **custom Arrow-backed store** — and what is the
   next-best alternative?

## What was done

Two isolated probe crates, each mapping datagen's neutral `CellFormat` vocabulary (bold,
italic, fill, alignment) plus a few Excel attributes (font size, number format, row/col
size, merges) onto the engine, with **passing tests** as the evidence (verify, don't
assume). Fixtures are generated from committed code (no hand-made binaries, §5.3).

- **`formualizer/`** — `formualizer` 0.7 (`eval,parse,workbook,calamine,umya`) **plus a
  direct `umya-spreadsheet` 2.3.2 dependency**. `src/lib.rs` builds a styled `.xlsx` from
  committed umya code, loads it into both a Formualizer `Workbook` (calc path) and a
  directly-owned umya `Spreadsheet` (style path), and reads formatting via umya.
  `tests/probe.rs` (4 tests): the `CellData.style == None` read gap, the
  `to_xlsx_bytes` style-drop, umya-direct readability of every attribute, and umya-direct
  edit round-trip.
- **`ironcalc/`** — `ironcalc` / `ironcalc_base` 0.7.1. `src/lib.rs` builds a styled
  `Model` via `get_style_for_cell` + `set_cell_style`, round-trips it through real
  `.xlsx` bytes (`save_xlsx_to_writer` → `load_from_xlsx_bytes` → `Model::from_workbook`).
  `tests/probe.rs` (5 tests): native read/write, row/col sizing, style survival through
  an xlsx round-trip, an after-load edit surviving a second round-trip, and the
  documented merges/conditional-formatting gaps.
- Each crate's `emit` binary writes an env-stamped capability matrix to
  `results/<engine>/capabilities.json`; `results/summary.md` is the merged head-to-head.

Reproduce:

```sh
( cd formualizer && cargo test && cargo run --bin emit )
( cd ironcalc    && cargo test && cargo run --bin emit )
```

## Results / evidence

### Capability matrix — attribute × {read / write / round-trip} × {engine}

**Native** = engine's own API; **ViaUmya** = only via a directly-owned umya workbook
alongside Formualizer; **None** = no public API in 0.7; **Unverified** = data exists,
fidelity not proven here.

| Attribute | FZ read | FZ write | FZ round-trip | IC read | IC write | IC round-trip |
|---|---|---|---|---|---|---|
| bold | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| italic | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| font_size | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| fill_color | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| number_format | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| borders | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| alignment | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| row_height | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| col_width | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| merges | ViaUmya | ViaUmya | ViaUmya | None | None | None |
| conditional_formatting | ViaUmya | ViaUmya | Unverified | None | None | None |

### Formualizer — the calc `Workbook` is a values/formulas pipe with no style path

- **Read gap (locked).** Every Formualizer read path emits `CellData { style: None }`.
  The calamine backend advertises `capabilities().styles == false` **and** returns
  `style: None` for a cell that is bold in the source file; the umya backend's
  `read_cell` also hard-codes `style: None` (Sub-project A). Probe
  `celldata_style_is_none_on_read` reads a known-bold `A1` through the calamine backend
  and asserts value present, style `None`.
- **Write gap (locked).** `Workbook::to_xlsx_bytes()` constructs a **fresh**
  `UmyaAdapter::new_empty()` and writes only the engine's values + formulas into it. Probe
  `values_survive_but_styles_dropped_through_to_xlsx_bytes` loads a styled file, saves via
  the engine, reloads, and finds the numeric value present (`"12.5"`) but bold, fill, and
  number-format all gone. **Formualizer cannot persist formatting.**
- **No internal bridge.** Formualizer's own `UmyaAdapter` wraps a private
  `umya_spreadsheet::Spreadsheet` with **no accessor** (`into_inner`/`workbook()` do not
  exist), and `Workbook` does not retain the adapter after `from_reader`. There is no way
  to reach a styled umya workbook *through* Formualizer.
- **The umya-direct path works fully.** Owning a `umya_spreadsheet::Spreadsheet`
  ourselves, every attribute is readable (probe `styles_readable_via_umya_directly`:
  bold, 16pt, `FFFFFF00` fill, `0.00` number format, row-1 height 40.0, col-A width 30.0,
  `C3:D3` merge) and format edits survive a save+reload (probe
  `umya_style_edit_survives_roundtrip`).

### IronCalc — styles are native and symmetric

- **Read/write (native).** `get_style_for_cell` / `set_cell_style` expose a full `Style`
  (`font.{b,i,u,sz,color,name}`, `fill.{pattern_type,fg_color,bg_color}`, `border`,
  `num_fmt`, `alignment`). Probe `styles_read_and_write_natively` sets bold+16pt+
  `#FFFF00` fill+`0.00` on `A1` and reads back an exact match. Row/col sizes are
  first-class (`set/get_row_height`, `set/get_column_width`; probe
  `row_col_sizes_settable`).
- **Round-trip (native).** Styles cross a real `.xlsx` boundary. Probe
  `styles_survive_xlsx_roundtrip`: styled model → `save_xlsx_to_writer` (in-memory
  cursor) → `load_from_xlsx_bytes` → `Model::from_workbook` → bold/italic/font-size/fill/
  number-format and row height all survive. Probe `style_edit_survives_second_roundtrip`
  confirms a post-load edit (bold off, cyan fill) also persists.
- **Gaps.** IronCalc 0.7 exposes **no public merged-cells API** (the internal
  `Worksheet.merge_cells` field has no getter/setter) and **no conditional-formatting
  API**. Recorded as `None` (probe
  `merges_and_conditional_formatting_absent_from_public_api` documents the absence; those
  methods do not compile). `set_cell_style_by_name`/`set_sheet_style` exist (named
  styles) but no general theme read API — `Unverified`, not probed.

## Conclusion

- **Formualizer surfaces no formatting itself, in either direction.** On read every cell
  is `style: None`; on write `to_xlsx_bytes` drops all styles. To read *or* preserve
  formatting with Formualizer, FreeCell must own a **`umya_spreadsheet` workbook
  directly** as the style source of truth (a direct crate dependency — Formualizer's own
  umya adapter is not reachable). The umya-direct path does expose and round-trip every
  attribute FreeCell needs (bold/italic/size/fill/number-format/borders/alignment/row-
  height/col-width/merges; conditional formatting readable, write fidelity unverified).
  **Load → edit → save is workable but is entirely on FreeCell's side of the seam, not
  the engine's.**
- **IronCalc surfaces formatting natively and symmetrically**, and it survives a real
  `.xlsx` round-trip. Load → edit → save is **easy**: it is just the engine's own API.
  The only gaps are merged cells and conditional formatting, which have no public API in
  0.7 and would need a FreeCell side-store or an upstream contribution.
- **What we could not determine here (and why):** (a) exhaustive per-attribute *fidelity*
  across a round-trip (exact colour/format-code/border-style preservation for the long
  tail) — deferred to Round 2 by the human, so we probed representative attributes only;
  (b) conditional-formatting write fidelity through umya (`Unverified` — umya exposes the
  collection but we did not prove a mutate-and-persist cycle); (c) IronCalc theme/named-
  style read fidelity (no general read API, not probed).

## Recommended design + next-best alternative

FreeCell's formatting model should be a **FreeCell-owned side-table keyed by cell +
column/row-band records** — a small, engine-neutral `FormatStore` that is the app's
source of truth for style — **regardless of which engine wins**. Rationale:

- It is **mandatory** if the engine is Formualizer (the engine surfaces nothing), and it
  is **still the right call** if the engine is IronCalc, because it (1) keeps the
  formatting model identical across an engine swap (the bake-off has not been decided),
  (2) decouples FreeCell's style vocabulary from a 0.x engine's `Style` churn, and (3)
  lets the GPUI datamodel provider read styles directly from FreeCell memory without a
  per-cell engine round-trip on the render hot path (Sub-project E reads
  values + format per visible cell).
- **Persistence** rides on **umya-spreadsheet** on save (write the store into a umya
  workbook and serialize), and on **load** reads styles from umya (Formualizer path) or
  from the engine's native `Style` (IronCalc path) into the same store. So the store is
  engine-independent; only its *populate-on-load* and *flush-on-save* adapters differ.
- **Shape.** A cell format is small and highly repetitive (10–20% of cells highlighted,
  scattered bold/italic — datagen's own model), so store **interned `StyleId -> Style`**
  plus a **sparse `(row,col) -> StyleId`** map, with separate `row_height` / `col_width`
  band maps and a `merges` list. This is cache-friendly, dedupes aggressively, and maps
  cleanly onto both engines' concepts.

**Next-best alternative — native (engine-resident) styles, IronCalc only.** If the
engine bake-off (Sub-project G) picks IronCalc, FreeCell *could* skip the side-table for
the attributes IronCalc models and read/write styles straight from the `Model` (with a
small side-store only for merges + conditional formatting). Cost: it couples FreeCell's
formatting to IronCalc's `Style` and its 0.x API, adds a per-cell engine call on the
render path, and would have to be re-architected if the engine ever changes. Viable but
lower-leverage than the engine-neutral store.

**Explicitly not recommended — a custom Arrow-backed formatting store.** Formualizer's
Arrow columnar store is not a public read/write API for styles (Sub-project A), and
formatting is sparse + interned rather than dense-columnar, so an Arrow lane per attribute
buys little over the interned side-table and adds real complexity. Revisit only if a
profiled hot path demands columnar style scans (not evidenced in Phase 1).

### load → edit → save verdict per engine

- **Formualizer:** **hard on the engine, workable off it.** The engine neither reads nor
  writes styles; FreeCell must run a parallel umya workbook (load styles from it, flush
  the store back through it on save). Extra moving part, but proven to round-trip.
- **IronCalc:** **easy.** Styles live in the engine and survive a native `.xlsx` round-
  trip; the only add-on is a side-store for merges + conditional formatting.

## Risks / open questions

- **Deferred fidelity (Round 2).** We probed representative attributes; exact
  preservation of the long tail (all border styles, theme/indexed colours, every
  number-format code, rich text) across round-trips is unproven. Owned by the Round-2
  style-roundtrip-fidelity experiment.
- **Formualizer double-load / sync cost.** The umya-direct model means FreeCell holds the
  sheet twice (Arrow engine + umya for styles) and must keep row/col identity in sync
  across edits (insert/delete row must shift both). Memory + sync overhead not yet
  measured; flag for the engine decision and Round 2.
- **umya conditional-formatting write fidelity** is `Unverified` (readable collection; no
  proven mutate-and-persist cycle). IronCalc has **no** conditional-formatting or merged-
  cells API at all — both engines need a FreeCell side-store or upstream work for these.
- **Both crates are 0.x** — `Style` (IronCalc) and the umya style structs may shift; the
  engine-neutral `FormatStore` is the mitigation (it isolates FreeCell from that churn).
- **`to_xlsx_bytes` dropping styles** also means the file-support sub-project (B) must not
  rely on the engine for style-preserving save; the umya save path is required. Cross-
  reference for Sub-project B / G.
