# SP4 — style-API coverage (IronCalc 0.7.1, assertion-backed)

Each row is proven by an executed assertion in `src/bin/probe.rs` (run `cargo run --release --bin probe`).

| capability | supported | public API | evidence |
|------------|-----------|------------|----------|
| per_cell | YES | `Model::set_cell_style / Model::get_style_for_cell` | set a non-default Style on one cell; get_style_for_cell reads back bold + num_fmt + fill |
| row_band | YES | `Model::set_row_style / Model::get_row_style; resolved by get_style_for_cell` | set a row band; an untouched cell far along that row (col 250) resolves the band style; adjacent row does not |
| column_band | YES | `Model::set_column_style / Model::get_column_style; resolved by get_style_for_cell` | set a column band; an untouched cell far down that column (row 9000) resolves the band style; adjacent column does not |
| empty_cell | YES | `get_style_for_cell over a valueless cell under a band OR with a direct set_cell_style` | a cell with get_cell_value == empty still resolves bold via both a row band and a direct per-cell style |
| precedence | YES | `get_cell_style_index resolution order, surfaced by get_style_for_cell` | with a column band, a row band over it, and a per-cell style, each cell resolves to the expected winner (cell/row/column/default) |

## Verdict

- per-cell styles: **YES**
- row + column band styles: **YES**
- empty-cell styling: **YES**

**Overview §2 formatting decision STANDS.** IronCalc's public API natively exposes per-cell, row-band, column-band, and empty-cell styling with a deterministic cell>row>column>default resolution — so "native styles as the source of truth" holds for these attributes; **SP4 does not force a side-store.**

(Known gaps carried from overview §2 / SP5, NOT SP4 regressions: no public merged-cells API, no conditional-formatting API — a side-store remains needed for *those two features only*.)
