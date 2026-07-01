# Sub-project D — Formatting capability matrix (head-to-head)

Environment: Rust 1.94.1, 4 cores / ~15 GB RAM, no GPU/display. Date 2026-07-01.
Engines: Formualizer 0.7.0 (+ umya-spreadsheet 2.3.2), IronCalc 0.7.1.
Machine-readable source: `formualizer/capabilities.json`, `ironcalc/capabilities.json`.

Legend — **Native**: exposed by the engine's own API. **ViaUmya**: reachable only by
owning a `umya_spreadsheet` workbook alongside Formualizer. **None**: not reachable
through that engine's public API in 0.7. **Unverified**: data exists but no public API /
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

FZ = Formualizer (all style attributes are **ViaUmya** — a directly-owned umya
workbook — because Formualizer's own read path returns `style: None` and its
`to_xlsx_bytes` write path drops styles entirely). IC = IronCalc (styles are **Native**
and symmetric; the gaps are merges and conditional formatting, which have no public API).

## The decisive engine-write fact

`formualizer::Workbook::to_xlsx_bytes()` builds a **fresh** umya file from the engine's
values + formulas only. A styled `.xlsx` loaded into a Formualizer `Workbook` and saved
via `to_xlsx_bytes` comes back **with every style dropped** (probe
`values_survive_but_styles_dropped_through_to_xlsx_bytes`). The value survives; bold,
fill, and number-format do not. Formualizer therefore cannot be the persistence path for
formatting — a umya workbook (or FreeCell store serialized to xlsx via umya) must be.

## Reproduce

```sh
( cd formualizer && cargo test && cargo run --bin emit )   # writes results/formualizer/capabilities.json
( cd ironcalc    && cargo test && cargo run --bin emit )   # writes results/ironcalc/capabilities.json
```
