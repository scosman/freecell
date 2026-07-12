---
status: complete
---

# Phase 16: Write path (component design + impl)

## Overview

P16 opens the authoring stage (Stage A) by building the one genuinely new subsystem the rest of
authoring hangs off: the **write path** — turning an authored `chart-model` into valid OOXML
chart + drawing parts inside a `.xlsx`. Everything before P16 only ever *read* chart XML (load) or
*byte-preserved / targeted-patched* retained source (save). Nothing yet **synthesizes** chart XML
from the typed model. P16 delivers that (`write-from-model`), and reconciles it with the existing
edited-loaded **source-patch** path so the two stay consistent (shared value-cache formatting).

Per CLAUDE.md this is **engine/serialization work only** — no grid/cell/sheet/titlebar/chart
*render* pixels move, so **no pixel-render baseline changes** and the pixel suite is out of scope.
Validation is engine unit tests + the external round-trip gate (IronCalc reopen + headless
LibreOffice), wired into the existing `charts_roundtrip_libreoffice` infrastructure.

Scope (matches implementation_plan P16):
1. **Design doc** — `components/write-path.md`: the three save write-modes, the write-from-model
   serializer contract (model→chart XML element map, drawing/anchor/rels/content-types synthesis,
   the `c:f` reference model), the edit-contract reconciliation, and the edit-panel *form* (per
   `ui_design §4`; detailed control set stays deferred to P19/P20).
2. **write-from-model** — serialize an authored `Chart` to `chartN.xml`, synthesize the
   `drawingN.xml` + rels + `[Content_Types]` overrides + worksheet `<drawing>` ref, injected into
   IronCalc's chart-less model bytes → a valid `.xlsx`.
3. **source-patch reconcile** — the edited-loaded patcher (`patch_chart_source`, P10) already
   reflows caches; P16 factors the value-cache builders so the serializer and the patcher format
   `numCache`/`strCache` **identically**, and documents the forward path to P20 chrome patches.

Exit: a model-built chart serializes to a valid `.xlsx` that reopens in Excel + LibreOffice, with
round-trip tests (our loader re-parses it; IronCalc reopens it; headless LibreOffice round-trips
it in CI).

## Steps

1. **`components/write-path.md`** — the design note (deliverable 1). Documents the write modes,
   the serializer's element-by-element mapping, the anchor→drawing synthesis, the `c:f`/`c:*Lit`
   reference rule, the edit-panel form, and the round-trip test strategy.

2. **New module `app/crates/freecell-engine/src/chart/write.rs`** (deliverable 2), wired into
   `chart/mod.rs` (`pub mod write;` + re-exports). Public surface:
   - `pub struct SeriesRefs { name: Option<String>, categories: Option<String>, values: Option<String> }`
     — the `c:f` formulas for one series' roles (`categories` = `c:cat`/`c:xVal` domain,
     `values` = `c:val`/`c:yVal`). A `None` role with data is emitted as a literal
     (`c:numLit`/`c:strLit`/`c:v`) so the XML is always schema-valid even before a range is set.
   - `pub fn serialize_chart_xml(chart: &Chart, refs: &[SeriesRefs]) -> String` — the
     **write-from-model core**: `c:chartSpace`→`c:chart`→(title, `plotArea`(layout, chart-group,
     axes), legend, `plotVisOnly`, `dispBlanksAs`). Round-trips through `parse_chart_xml`.
   - `pub fn synthesize_drawing_xml(anchors: &[(Anchor, &str)]) -> String` — a `xdr:wsDr` with one
     `twoCellAnchor` graphic frame per chart (`<c:chart r:id=…>`), from the anchor(s).
   - `pub struct AuthoredChart { sheet_name, chart_part, chart, anchor, refs }`.
   - `pub fn write_authored_charts(model_bytes: &[u8], authored: &[AuthoredChart]) -> Result<(Vec<u8>, AuthoredWriteReport)>`
     — groups authored charts by host sheet, synthesizes one drawing per sheet, injects chart
     parts + drawings + their rels + content-type overrides, and patches each target worksheet's
     `<drawing>`. **Fails loudly** on an unknown sheet name or a sheet that already carries a
     `<drawing>` (merging authored charts onto a sheet that already has charts is P17's concern).
     Returns a dedicated `AuthoredWriteReport { charts_authored, patched_sheets, synthesized_parts }`
     — a distinct shape from `save::SaveReport` so a combined save (loaded re-inject + authored
     write) never conflates written-from-scratch charts with byte-preserved ones.

3. **Reconcile the source-patch path** (deliverable 3): in `save.rs`, widen the value-cache
   builders (`rebuild_num_cache`, `rebuild_str_cache`, `fmt_cache_num`, `escape_xml`) and the
   package helpers (`patch_worksheet`, `ensure_r_namespace`, `build_sheet_rels`, `relative_part`,
   `write_part`, `read_named_bytes`, `read_named_string`, `name_to_part_map`) to `pub(super)` so
   `write.rs` reuses them. The serializer's `numRef`/`strRef` caches are built by the **same**
   `rebuild_*` helpers the reflow patcher uses → an authored cache is byte-identical to a reflowed
   one. No behavior change to the existing byte-preserve / patch paths.

4. **Authored fixture** — add `pub fn write_authored_line_fixture(path)` to `authoring.rs`: build a
   data-only workbook via `WorkbookDocument` (IronCalc), set category/value cells, serialize an
   authored line `Chart` referencing those cells through the write path, write the `.xlsx`. This is
   the authored artifact the external LibreOffice gate consumes (real cells so LibreOffice keeps the
   data on re-read).

5. **External round-trip** — add `libreoffice_reopens_freecell_authored_line_chart` to the existing
   `tests/charts_roundtrip_libreoffice.rs` (same gate policy / CI job as the P15 test): the
   authored fixture → headless LibreOffice `--convert-to xlsx` → the line chart part survives + our
   loader re-parses it as a line chart.

## Tests

- **`serialize_roundtrips_through_our_parser`** — for line / column / bar / area / pie / doughnut /
  scatter: `parse_chart_xml(serialize_chart_xml(chart, refs)) == chart` (core fields: title, kind,
  series name+data+color, axis titles+gridlines+scaling+numFmt, legend).
- **`serialize_emits_cf_ranges`** — the serialized `c:f`s are read back by `parse_cf_ranges` in
  order (so a saved authored chart re-parses with live-binding ranges).
- **`serialize_is_well_formed_xml`** — roxmltree parses the output; declares the `c`/`a`/`r`
  namespaces.
- **`serialize_no_ref_role_emits_literal`** — a series role with data but no ref emits a
  `c:numLit`/`c:strLit` (still valid XML), not a broken empty `c:f`.
- **`synthesize_drawing_roundtrips_anchor`** — the synthesized drawing's anchor re-parses (via the
  load path's `parse_anchor`/`discover`) to the input `Anchor`.
- **`write_authored_chart_reopens_and_reparses`** — author a line chart into a `new_empty`
  IronCalc workbook with real data cells; the output: IronCalc reopens it, `discover_and_parse`
  returns it as a **Loaded** line chart with the authored values, the worksheet carries a
  `<drawing>`, and `[Content_Types]` declares the chart + drawing parts.
- **`write_authored_charts_two_sheets`** — two authored charts on two sheets both survive + reopen.
- **`write_authored_fails_loudly_on_unknown_sheet`** / **`…_on_sheet_with_existing_drawing`** — the
  fail-loud preconditions.
- **`patch_and_serialize_share_cache_format`** — an authored value cache equals the cache a reflow
  patch would emit for the same values (the reconciliation invariant).
- **`libreoffice_reopens_freecell_authored_line_chart`** (external, gated) — the authored fixture
  survives a headless-LibreOffice read+rewrite and re-parses as a line chart.
