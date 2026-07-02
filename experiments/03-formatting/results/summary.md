# Sub-project D — Formatting capability matrix (head-to-head)

Environment: Rust 1.94.1, 4 cores / ~15 GB RAM, no GPU/display. Date 2026-07-01.
Engines: Formualizer 0.7.0 (+ umya-spreadsheet 2.3.2), IronCalc 0.7.1.
Machine-readable source: `formualizer/capabilities.json`, `ironcalc/capabilities.json`.

Legend — support: **Native** (engine's own API); **ViaUmya** (only via a
directly-owned umya workbook alongside Formualizer); **Unverified** (API exists, this
axis not exercised / fidelity unproven); **None** (no public API in 0.7).

Provenance — **[P] ProbeBacked**: a passing test in that engine's `tests/probe.rs` reads
this attribute back (named in the row note). **[I] Inferred**: reasoned from the API
surface, no executed assertion — do NOT over-trust these rows; the exhaustive fidelity
sweep is deferred to Round 2. Every cell inherits its row's provenance marker.

| Attribute | Prov | FZ read | FZ write | FZ round-trip | IC read | IC write | IC round-trip |
|---|---|---|---|---|---|---|---|
| bold | FZ [P] / IC [P] | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| italic | FZ [P] / IC [P] | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| font_size | FZ [P] / IC [P] | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| fill_color | FZ [P] / IC [P] | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| number_format | FZ [P] / IC [P] | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| borders | FZ **[I]** / IC [P] | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| alignment | IC [P] only* | — | — | — | Native | Native | Native |
| row_height | FZ [P] / IC [P] | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| col_width | FZ [P] / IC [P] | ViaUmya | ViaUmya | ViaUmya | Native | Native | Native |
| merges | FZ [P] / IC [P] | ViaUmya | ViaUmya | ViaUmya | None | None | None |
| conditional_formatting | FZ **[I]** / IC [P] | ViaUmya | ViaUmya | Unverified | None | None | None |
| themes / named styles | IC **[I]** | — | — | — | Unverified | Native | Unverified |

\* **alignment** is not in the Formualizer matrix: the FZ probe has no alignment read
helper or assertion, so there is nothing to report for Formualizer (do not infer it).
IronCalc alignment is probe-backed (right-align set + read + round-tripped).

Provenance detail:
- **FZ probe-backed** (round-trip re-read by `umya_style_edit_survives_roundtrip`, which
  now re-reads the full A1 attribute set + row/col size + merge *after* save→reload):
  bold, italic, font_size, fill_color, number_format, row_height, col_width, merges. The
  `to_xlsx_bytes` style-drop is probe-backed by
  `values_survive_but_styles_dropped_through_to_xlsx_bytes`.
- **FZ inferred (not probed)**: borders (umya per-side API exists, unexercised);
  conditional_formatting (umya collection exists; read plausible, write-back fidelity
  unverified → round-trip = Unverified).
- **IC probe-backed**: bold/italic/font_size/fill_color/number_format (round-trip re-read
  by `styles_survive_xlsx_roundtrip`), row_height/col_width (`row_col_sizes_settable` +
  round-trip), borders + alignment (representative thin-left-border + right-align set,
  read, and round-tripped by `borders_and_alignment_read_and_survive_roundtrip`), and the
  merges/conditional_formatting = None absences (documented by
  `merges_and_conditional_formatting_absent_from_public_api`).
- **IC inferred (not probed)**: themes / named styles (`set_cell_style_by_name` /
  `set_sheet_style` exist; no general theme read API).

## The two decisive rows (unchanged, airtight, probe-backed)

1. **Formualizer surfaces no styles on read** and **`to_xlsx_bytes` drops all styles on
   write** — a styled `.xlsx` loaded into a Formualizer `Workbook` and saved via the
   engine comes back value-only (probes `celldata_style_is_none_on_read`,
   `values_survive_but_styles_dropped_through_to_xlsx_bytes`).
2. **IronCalc reads/writes styles natively and they survive a real `.xlsx` round-trip**
   (probe `styles_survive_xlsx_roundtrip`).

## Reproduce

```sh
( cd formualizer && cargo test && cargo run --bin emit )   # writes results/formualizer/capabilities.json
( cd ironcalc    && cargo test && cargo run --bin emit )   # writes results/ironcalc/capabilities.json
```
