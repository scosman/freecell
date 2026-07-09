---
status: complete
---

# Phase 2: Chart data model (`ChartSpec` envelope)

## Overview

Widen the `freecell-chart-model` crate from the PoC render seam (a static `Chart`) to the
**production, OOXML-bounded typed shape** by adding the **`ChartSpec` envelope**
(`architecture.md §3.2`). `ChartSpec` wraps the existing `Chart` with everything production
needs beyond a static picture: the retained **source** XML, the live-binding **source
ranges** (`c:f`), the in-grid **anchor**, and the chart's **origin** (loaded vs authored,
which is what carries the retained source). This is a **pure data-model addition** — the
render seam `Chart` is untouched, no parsing/rendering/saving is wired, and **nothing renders
differently yet** (the P2 exit criterion).

### Scope decisions (flagged for the reviewer)

1. **`ChartSpec` lands in `freecell-chart-model`, not `freecell-engine`.** Architecture §3.2
   calls it the "engine envelope" for its *role* (the engine produces it), but every field is
   pure data — **gpui-free and ironcalc-free** — and it is the shared shape the engine
   *produces* and the app *consumes* (anchor→pixel, origin, and the P3 fidelity accessor). It
   belongs on the stable seam both layers already depend on, keeping "no layer reaches across"
   (architecture §2). The crate stays dependency-free (pure std); the `dependency_rule` guard
   is unaffected (it only scans core/engine).

2. **Retained `source` is folded into the `Origin` enum, exposed via `source()`.** Architecture
   §3.2 lists `source` and `origin` as separate bullets but immediately states "**Authored
   charts have no source**" — i.e. source presence is *governed by* origin. Encoding that as
   `Origin::Loaded { source } | Origin::Authored` makes the invalid state (an authored chart
   with a source, or a loaded chart without one) **unrepresentable**, instead of two fields
   that must be kept in sync. A `ChartSpec::source() -> Option<&SourceXml>` accessor gives
   callers the conceptual `source` field. All four plan-named items are present:
   source (in `origin`), `source_ranges`, `anchor`, `origin`.

3. **Live-binding bookkeeping (`dirty`, `last_values`) is deferred to P9.** Architecture §3.2
   also lists these two, but they are **live-binding** state whose behavior and exact types are
   a P9 design decision (there is no binding machinery in P2, and the file cache already sitting
   in `chart.series` is the P2 fallback per functional_spec §2). The plan's P2 parenthetical
   scopes this phase to "(retained source, ranges, anchor, origin)"; adding unused speculative
   fields now would be scope-bleed the phased plan exists to avoid. Documented in the
   `ChartSpec` doc comment.

4. **No fidelity accessor / `Fidelity` enum.** `display_fidelity()` + the curated
   unsupported-feature set + 3D→2D normalization are **P3** ("Derived fidelity accessor").
   P2 only defines the shape the accessor will later read.

## Steps

1. **New module `app/crates/freecell-chart-model/src/spec.rs`** holding the envelope and its
   supporting types (all `#[derive(Clone, Debug, PartialEq)]`; the float-free ones also `Eq`):

   - `AnchorCell { col: u32, col_off_emu: i64, row: u32, row_off_emu: i64 }` — one corner of an
     `xdr:twoCellAnchor` (`<xdr:from>`/`<xdr:to>`): a 0-based sheet cell + intra-cell EMU
     offset. Constructors `new(col, row)` (zero offsets) and `with_offsets(col, col_off_emu,
     row, row_off_emu)`.
   - `Anchor { from: AnchorCell, to: AnchorCell }` — the chart's sheet rectangle; `new(from,
     to)`. (`Copy`, both corners are small.) Anchor→pixel mapping is the app layer's job (P8);
     this retains the raw shape.
   - `CfRange { formula: String }` — one `<c:f>` reference retained verbatim (e.g.
     `Data!$B$2:$B$5`). `new(impl Into<String>)`, `as_str()`. Structured sheet/range parsing
     is P9 (live binding).
   - `SourcePart { part_name: String, bytes: Vec<u8> }` — one retained related package part
     (the chart's `_rels`, `colorsN.xml`, `styleN.xml`, embeddings), kept as raw bytes for
     byte-for-byte save carry. `new(part_name, bytes)`.
   - `SourceXml { chart_xml: String, related_parts: Vec<SourcePart> }` — the retained chart
     part XML plus its related parts; the substrate for save byte-preservation + edit-patching
     (architecture §5) + the P3 fidelity accessor. Kept as raw text/bytes (not a borrowed DOM),
     matching the existing `open_fixups`/`save` textual second-pass. `new(chart_xml)` (empty
     related parts) + `with_related_parts(parts)` builder.
   - `Origin { Loaded { source: SourceXml }, Authored }` — where the chart came from; `Loaded`
     carries the retained source, `Authored` has none.
   - `ChartSpec { chart: Chart, source_ranges: Vec<CfRange>, anchor: Anchor, origin: Origin }`
     — the envelope, with constructors + accessors:
     - `loaded(chart, source: SourceXml, source_ranges: Vec<CfRange>, anchor) -> Self`
     - `authored(chart, anchor) -> Self` (empty `source_ranges`, `Origin::Authored`)
     - `source(&self) -> Option<&SourceXml>` (Some iff loaded)
     - `is_loaded(&self) -> bool`, `is_authored(&self) -> bool`

2. **Re-export from `src/lib.rs`.** Add `mod spec;` + `pub use spec::{Anchor, AnchorCell,
   CfRange, ChartSpec, Origin, SourcePart, SourceXml};` so callers use
   `freecell_chart_model::ChartSpec` etc. Update the crate-level doc comment to note the model
   now carries the production `ChartSpec` envelope in addition to the `Chart` render seam, and
   that it remains bounded-OOXML (not exhaustive) — additive P1/P2 render fields land with their
   phases (P6/P12/P13). Do **not** touch any existing `Chart` type or field.

3. **Green the workspace** (run in `app/`, foreground under `timeout`): `cargo fmt --all`, then
   iterate `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`,
   `cargo test --workspace` until clean.

## Tests

New unit tests in `spec.rs` (`#[cfg(test)]`), with a small `sample_chart()` helper:

- `anchor_spans_from_and_to_cells` — `Anchor::new` preserves both corners; `with_offsets`
  stores the EMU offsets; `new` defaults offsets to zero.
- `cf_range_retains_formula_text` — `CfRange::new(...).as_str()` returns the verbatim formula.
- `source_xml_holds_chart_xml_and_related_parts` — `SourceXml::new(..).with_related_parts(..)`
  keeps the chart XML and each `SourcePart` (name + bytes).
- `loaded_spec_carries_source_ranges_and_anchor` — `ChartSpec::loaded` → `source()` is `Some`
  with the given XML, `is_loaded()` true / `is_authored()` false, and `source_ranges` / `anchor`
  / `chart` are preserved.
- `authored_spec_has_no_source` — `ChartSpec::authored` → `source()` is `None`, `is_authored()`
  true, `source_ranges` empty, `origin == Origin::Authored`.
- `spec_clone_and_partial_eq` — a cloned `ChartSpec` equals the original; changing the anchor
  makes them differ (guards the derived `Clone`/`PartialEq` the worker publish path relies on).

## Render validation

**Out of scope for this phase (no pixel suite run).** This is a pure data-model / type addition
in the gpui-free `freecell-chart-model` crate; no `GridView`, cell, sheet, or titlebar pixel can
move (CLAUDE.md render-test scope), and nothing is wired into rendering. Validated by the new
unit tests + the standard `fmt`/`clippy`/`build`/`test` gate. In-grid chart rendering and its
render-test coverage begin at **P8**.
