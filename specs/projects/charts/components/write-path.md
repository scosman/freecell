---
status: draft
---

# Component design: the chart write path (+ edit-panel form)

The **write path** is the save-side machinery that turns a `chart-model` value into OOXML chart +
drawing parts inside a `.xlsx`. It is the one genuinely new subsystem authoring needs
(implementation_plan P16; architecture §4.1 SAVE / §5 challenge 3, which flagged the
"template-synthesizer + edit-patcher … their own component design in the authoring phase"). This
doc is that design. The **edit panel** shares this doc only for its *form* (a right-docked window);
its concrete control set is specced with P19/P20 per `ui_design §4`.

Charts are engine-serialized **beside** IronCalc: IronCalc's writer emits a `.xlsx` from a model
that has no chart data, so the engine re-injects the chart machinery afterwards (the same
`open_fixups.rs` / `save.rs` second-pass style). P16 adds the third injection mode — synthesis.

## 1. The three save write-modes (one dispatch, by `Origin` + edited state)

A chart on save is exactly one of three cases (functional_spec §3/§6 edit-contract, architecture §5
challenge 3). The first two exist (P10, `save.rs`); the third is P16.

| Mode | Applies to | What happens | Where |
|---|---|---|---|
| **1. Byte-preserve** | a `Loaded` chart whose model equals its file cache (unedited), or any `Unsupported` retained chart | its retained `chartN.xml` (+ aux parts) is carried **byte-for-byte** | `save::reinject` carry loop |
| **2. Source-patch** | a `Loaded` chart whose model differs from its file cache (edited) | its retained `chartN.xml` is **targeted-patched** — only the changed sub-elements are spliced, everything else (incl. unmodeled DrawingML styling) stays byte-identical | `save::patch_chart_source` |
| **3. Write-from-model** | an `Authored` chart (no retained source) | its `chartN.xml` + `drawingN.xml` + rels + content-types are **synthesized** from the model | `write::write_authored_charts` (**new, P16**) |

The dispatch key is `ChartSpec::origin`: `Loaded` → mode 1 or 2 (by cache equality, already decided
in `build_live_patches`); `Authored` → mode 3. An `Authored` chart never has retained source, so it
can only be synthesized — the invariant the `Origin` enum makes unrepresentable-if-violated
(spec.rs). The three modes are **composable**: a workbook can carry loaded charts (modes 1/2, via
`reinject_live_charts`) *and* authored charts (mode 3, via `write_authored_charts`) — the app-level
orchestration that runs both over one save is P17's insert-flow concern; P16 delivers mode 3 as a
standalone, composable engine entry point that operates on already-serialized model bytes.

## 2. Write-from-model — the serializer (`serialize_chart_xml`)

The core: `serialize_chart_xml(chart: &Chart, refs: &[SeriesRefs]) -> String`. It is the inverse of
`load::parse_chart_xml`, and its correctness bar is exactly that: **`parse_chart_xml(serialize(c,
r)) == c`** for the fields the model carries. It emits classic `c:` chart XML (not `cx:`
extended), declaring the `c`/`a`/`r` namespaces on `c:chartSpace` so Excel + LibreOffice accept it.

### 2.1 Element map (model → OOXML)

```
c:chartSpace(xmlns c/a/r)
 └ c:chart
    ├ c:title            ← chart.title            (a:t rich run; omitted + c:autoTitleDeleted=1 when None)
    ├ c:plotArea
    │   ├ c:layout/
    │   ├ <group>        ← chart.kind             (see 2.2)
    │   │   └ c:ser*     ← chart.series           (see 2.3)
    │   ├ c:catAx        ← chart.cat_axis          (non-scatter; see 2.4)
    │   └ c:valAx        ← chart.val_axis
    ├ c:legend           ← chart.legend            (c:legendPos by position)
    ├ c:plotVisOnly=1
    └ c:dispBlanksAs=gap
```

### 2.2 Chart group (`chart.kind`)

- `Bar { dir, grouping }` → `c:barChart` + `c:barDir(col|bar)` + `c:grouping` + `varyColors=0` +
  series + two `c:axId`.
- `Line { grouping, smooth }` → `c:lineChart` + `c:grouping` + `varyColors=0` + series +
  `c:marker val=1` (show point markers) + `c:smooth val=1` **iff** `smooth` + two `c:axId`.
  (Our parser reads `smooth` at group level; `CT_LineChart` allows a group-level `c:smooth`, so this
  is both round-trippable and schema-valid.)
- `Area { grouping }` → `c:areaChart` + `c:grouping` + `varyColors=0` + series + two `c:axId`.
- `Pie { doughnut_hole: None }` → `c:pieChart` + `varyColors=1` + series + `c:firstSliceAng=0`
  (no axes).
- `Pie { doughnut_hole: Some(h) }` → `c:doughnutChart` + `varyColors=1` + series +
  `c:firstSliceAng=0` + `c:holeSize val=round(h·100)`.
- `Scatter` → `c:scatterChart` + `c:scatterStyle val=lineMarker` + series (xVal/yVal) + two `c:axId`
  (both `c:valAx`).

Element **order** follows the `CT_*Chart` schema sequence (grouping/varyColors before series,
`axId` last) so strict readers (Excel) accept it.

### 2.3 Series (`c:ser`) + the `c:f` reference model

Per series, in `CT_*Ser` order: `c:idx`, `c:order`, `c:tx`?, `c:spPr`?, then the data roles. The
data roles are the crux, because the model carries **cached values** but the OOXML wants a
**reference** (`c:numRef`/`c:strRef` = `c:f` formula + cache). The per-series `SeriesRefs` supplies
the `c:f` for each role:

```
SeriesRefs { name: Option<String>, categories: Option<String>, values: Option<String> }
             c:tx c:f              c:cat / c:xVal c:f          c:val / c:yVal c:f
```

Rule per role:
- **ref present** → emit `c:numRef`/`c:strRef` = `<c:f>{ref}</c:f>` + the value cache. This is the
  normal authored path: once a range is picked (P19), each role has a real `c:f` and the chart is
  fully cell-bound (live-binds + reflows like any loaded chart).
- **ref absent, data present** → emit a **literal** (`c:numLit`/`c:strLit`, or `c:v` for `c:tx`).
  Schema-valid, so a still-being-shaped ("near-empty") authored chart never produces a broken empty
  `c:f`. A literal is **not** read back by FreeCell's own parser (which reads `numCache`/`strCache`),
  which is fine: a literal-data chart has no ranges to bind. The authoring flow supplies refs the
  moment a range is set, so this state is transient.
- **no data** → omit the role.

The value cache is built by the **same** `rebuild_num_cache`/`rebuild_str_cache` helpers the
edited-loaded reflow patcher uses (§4), so an authored cache is byte-identical to a reflowed one:
non-finite values omitted (sparse blanks), `ptCount` spans the full length, whole numbers without a
decimal point. Category caches follow the parser's str-then-num preference: all-`Number` categories
→ `numCache`; any `Text` → `strCache` (numeric labels stringified).

`c:spPr` fill: `chart.series[i].color` → `<a:solidFill><a:srgbClr val=RRGGBB/></a:solidFill>`. An
authored series carries a **concrete sRGB** from FreeCell's palette (functional_spec §6.A —
authored styling is FreeCell-native, not Excel-theme-matched); a `ChartColor::Theme` is resolved to
its office-default RGB before emission, so the fill is always a round-trippable `a:srgbClr`. Marker
/ data-label / stroke emission is **out of P16 scope** (authored `Series` default them to `None`);
they are added to the serializer when their edit controls land (P20), additively.

### 2.4 Axes

Non-scatter emits one `c:catAx` (axPos `b`) + one `c:valAx` (axPos `l`); scatter emits two
`c:valAx` (x axPos `b`, y axPos `l`) — mirroring `parse_axes`. Each axis, in `CT_*Ax` order:
`c:axId`, `c:scaling`(`c:orientation minMax|maxMin`, `c:max`?, `c:min`?), `c:delete=0`, `c:axPos`,
`c:majorGridlines`? (iff `axis.major_gridlines`), `c:minorGridlines`? (iff `axis.minor_gridlines`),
`c:title`? (iff `axis.title`), `c:numFmt`? (iff `axis.number_format`), `c:crossAx`. Fixed `axId`s
(cat/x = `111111111`, val/y = `222222222`) cross-reference each other. A pie/doughnut chart emits
no axes; its model axes round-trip as `Axis::default()` (which is what `parse_axes` returns for an
absent axis).

## 3. Drawing / anchor / rels / content-types synthesis

`serialize_chart_xml` produces just `chartN.xml`. `write_authored_charts(model_bytes, authored)`
assembles the rest into IronCalc's chart-less zip:

- **Drawing part** `xl/drawings/drawingN.xml` — a `xdr:wsDr` with one `xdr:twoCellAnchor` graphic
  frame per authored chart on that sheet, `<xdr:from>`/`<xdr:to>` from the `Anchor` (EMU offsets
  included), each frame's `<c:chart r:id=…>` pointing at its chart part. One drawing per host sheet
  (a worksheet has at most one `<drawing>`), carrying all that sheet's authored anchors.
- **Drawing `_rels`** — one `<Relationship Type=…/chart Target=../charts/chartN.xml>` per chart.
- **Worksheet patch** — `<drawing r:id=…/>` injected before `</worksheet>` (reusing `patch_worksheet`
  + `ensure_r_namespace`), and the worksheet `_rels` gets the drawing relationship (reusing
  `build_sheet_rels`, merging any IronCalc-emitted rels).
- **`[Content_Types]`** — `<Override>` for each new chart part
  (`…drawingml.chart+xml`) + drawing part (`…drawing+xml`).

The host worksheet is resolved by **name** through `model_bytes`' `workbook.xml(.rels)` map (the
same `name_to_part_map` the save part-map uses). **Fail-loud** preconditions (architecture §6 — no
silent corruption): an unknown sheet name, or a target worksheet that **already** carries a
`<drawing>` (authoring onto a sheet that already has charts requires merging into the existing
drawing — deferred to P17), is a hard error, not a silent drop.

## 4. Reconciliation with the source-patch path

Modes 2 and 3 must format cache values **identically**, or an authored chart later edited + reflowed
would rewrite its own caches spuriously (breaking the mode-2 "byte-identical when unchanged"
invariant). P16 factors the value-cache builders (`rebuild_num_cache`, `rebuild_str_cache`,
`fmt_cache_num`, `escape_xml`) to `pub(super)` and has **both** the serializer and the patcher call
them. A test (`patch_and_serialize_share_cache_format`) pins the invariant.

The edited-loaded patcher itself (`patch_chart_source`) is unchanged by P16: it still reflows only
`numCache`/`strCache` (the P10 scope). The **edit contract** (functional_spec §6 — patching a loaded
chart preserves unmodeled styling) is a mode-2 property; extending the patcher to also rewrite
edited chrome fields (title / legend / axis titles / series colors / data-label toggles) is **P20**,
and will splice those specific sub-elements the same targeted-XML way, leaving everything else
byte-stable. P16 only documents that forward seam; it does not build the chrome patcher.

## 5. Edit panel — form only (detail deferred to P19/P20)

Per `ui_design §4`: the chart Edit panel is a **right-docked floating window** (a chrome overlay in
`freecell-app`, not a popover on the chart), opened on insert (P17) or when a chart is selected
(P18). It mutates the authored `ChartSpec`'s `Chart`/ranges in place; the mutated model feeds §2's
serializer on save (authored) or §4's patcher (loaded). Its concrete control set — chart **type**
+ data **range** (P19), then **title / legend / axis titles / series colors / data-label toggles**
(P20) — is specced with those phases. P16 fixes only that the panel edits *the same model* the
write path consumes, so there is no second serialization contract.

## 6. Testing strategy

- **Unit (engine, headless)**: serializer round-trips through `parse_chart_xml` across all six
  kinds; `c:f`s re-parse via `parse_cf_ranges`; well-formed XML; literal fallback for a ref-less
  role; drawing-anchor round-trip; the shared-cache-format invariant.
- **Package (engine, headless)**: `write_authored_charts` into a real IronCalc workbook →
  IronCalc reopens + `discover_and_parse` re-reads it as a Loaded chart with the authored values;
  multi-sheet; fail-loud preconditions.
- **External round-trip (CI-gated)**: an authored line-chart fixture (real data cells) survives
  headless LibreOffice `--convert-to xlsx` and re-parses as a line chart — wired into the existing
  `charts_roundtrip_libreoffice` test + `roundtrip.yml` gate (the same policy as the P15
  byte-preserve external round-trip). This is the "reopens in Excel + LibreOffice" exit proof
  (Excel can't run in CI; LibreOffice is the external stand-in, as established in P15).
