---
status: draft
---

# Architecture: Charts (production)

How the PoC's three layers land in FreeCell's crate structure, plus the net-new production
machinery: app integration, live binding, the compatibility parse-contract, save/reflow, and
the authoring write path. Exact engine worker/cache APIs live in the `mvp`/`mvp-gaps`
architecture + `engine_worker` component doc; this references them.

**Organization decision (1-phase vs 2-phase):** a **single `architecture.md`** now. The
v1-core design fits here; the deepest deferred piece — the **write-from-model serializer** and
the **edit-panel** — gets its own component design **when Phase 6 (authoring) is planned**
(you asked to defer that detail). Flag if you'd rather split now.

## 1. Pinned dependencies
Mirror `app/Cargo.toml` exactly — `gpui`/`gpui_platform` (zed rev `1d217ee…`) +
`gpui-component` (`a9a7341…`), `ironcalc =0.7.1`, `zip 0.6` + `roxmltree 0.19` (already engine
deps via `open_fixups.rs`), `image`/`png` for the regression harness. No new heavy deps.

## 2. Layered placement (PoC crates → app crates, by charter)

| Concern | Lands in | Why |
|---|---|---|
| **`chart-model`** (gpui-free, ironcalc-free) + **parse-outcome** types | **`freecell-core`** (or a dedicated `freecell-chart-model` crate) | pure logic; the stable seam. Recommend a dedicated sibling crate to keep it explicit + core lean — minor call. |
| **File layer** — load parser + save (byte-preserve, reflow, write-from-model) | **`freecell-engine`** | owns IronCalc, file I/O, the `open_fixups.rs` zip second-pass. |
| **Live binding** — `c:f` resolution + dirty-set | **`freecell-engine`** | only the engine sees IronCalc cells + the recompute worker. |
| **Render + interaction** — ChartLayer, badge, placeholder, selection/manipulation, edit panel, action-bar insert | **`freecell-app`** | needs gpui + gpui-component + the grid coordinate system. |

The **seam holds**: engine produces `chart-model::Chart` (now live) wrapped in `ChartSpec`;
app renders/edits; no layer reaches across.

## 3. Data model

### 3.1 `chart-model::Chart` (core seam — kept from PoC)
Unchanged shape (`ChartKind`, `Series`/`SeriesData`, `Axis`, `Legend`, `Color`), extended
only with the production fidelity fields the coverage matrix calls P1/P2 (per-point `dPt`
colors, data-label config, number-format, axis scaling) as they are implemented — additively,
so the seam stays stable (it "held across all four gates without a shape change", `SYNTHESIS §5`).

### 3.2 `ChartSpec` (engine envelope — net new)
Wraps `Chart` with everything production needs beyond a static picture:
- `source_ranges: Vec<CfRange>` — parsed `c:f` per data ref (for live binding).
- `anchor: Anchor` — `twoCellAnchor` from/to cell + EMU offsets (for in-grid placement).
- `provenance: Provenance` — original part bytes + rels (for byte-preservation save).
- `outcome: ParseOutcome` — the **compatibility parse-contract** result (below).
- `dirty: bool`, `last_values` — live-binding bookkeeping.

### 3.3 `ParseOutcome` (the compatibility parse-contract — functional_spec §5)
```
enum ParseOutcome { Supported, Degraded(Vec<DegradeReason>), Unsupported(Reason) }
```
- **Supported** → renders faithfully.
- **Degraded** → renders + the app shows the corner warning (functional_spec §5 / ui_design §2.2).
  Set by: any P2/P3 feature the parser sees but the model doesn't carry; **3D chart types**
  (parsed as their 2D equivalent — `bar3DChart→Bar`, etc.). `DegradeReason` is kept for logs,
  not shown (UI is just "⚠ May not display as intended").
- **Unsupported** → the app shows the placeholder. Set by: types with no 2D equivalent
  (surface/radar/ofPie/stock/`cx:`) and hard parse failures.
The parser is the single classifier: for each feature it *either* fills the model, *or* records
a `DegradeReason`, *or* (if essential) returns `Unsupported`.

## 4. Component breakdown & flow

```
OPEN ─ IronCalc load ─┬─ chart discovery (sheet→drawing→chart, PoC load.rs)
                      └─ parse chartN.xml → ChartSpec{Chart, ranges, anchor, provenance, outcome}
                          (lazy: on first paint of the owning sheet region, off open's crit path)

EDIT ─ IronCalc recompute ─ worker publish ─ dirty charts = (ranges ∩ changed cells)  [engine]
                                               └ re-resolve c:f → fresh Chart → publish (arc-swap + WorkerEvent)

PAINT ─ ChartLayer (app): for each on-screen ChartSpec → anchor→pixel rect → dispatch:
          Supported/Degraded → chart_element(&Chart) [+ corner badge if Degraded]
          Unsupported        → placeholder

SAVE ─ IronCalc write (chart-less) ─ splice: unedited → byte-preserve provenance
                                             edited/authored → write-from-model (serialize Chart→chartN.xml)
                                     + patch worksheet <drawing>/_rels + [Content_Types] + multi-sheet map

AUTHOR (Phase 6) ─ action-bar chart icon → type menu → insert near-empty ChartSpec (no provenance,
                    outcome=Supported) → edit panel mutates Chart/ranges → live-binds + writes-from-model
```

### 4.1 Engine — chart I/O + binding
- `discover_and_parse(path) -> Vec<ChartSpec>` (lazy per sheet).
- Live binding: build a **range→chart index**; on recompute, intersect the changed-cell set to
  get the **dirty chart set**, re-resolve their ranges from IronCalc's current values, rebuild
  their `Chart`, and publish via the **existing worker publication seam** (charts ride the same
  lock-free snapshot path as cells — not a bespoke channel).
- Save: `save_with_charts` extends PoC `save.rs` — byte-preserve unedited; **reflow** edited
  charts' `numCache` from current values; **write-from-model** for authored/edited charts
  (Phase 6); multi-sheet part map via `workbook.xml.rels`, **failing loudly** on a missing
  target part.

### 4.2 App — render + interaction
- **`ChartLayer`** painted after cells, before chrome overlays; anchor→pixel via the grid's
  coordinate system (row/col geometry from the all-styles resident cache), so scroll/zoom are
  free; culls off-screen; resident `Vec<RenderedChart>` repainted on the dirty set.
- **Dispatch** = PoC `chart_element(&Chart)` over `ChartKind`, extended with P1/P2 fidelity;
  `Degraded` adds the corner badge; `Unsupported` → placeholder.
- **Authoring (Phase 6):** action-bar chart-icon menu → insert; selection outline + handles on
  the layer; the right-docked **edit panel** (a chrome overlay, form-factor fixed, detail
  deferred) mutates the `Chart`/ranges and marks it authored (→ write-from-model on save).

## 5. Technical challenges (designed here)
1. **Anchor→pixel & z-order.** Map `twoCellAnchor` (from/to cell + EMU) through the grid
   geometry cache; clip to viewport; paint above cells, below chrome. Scroll/zoom reuse the
   grid's transform. *(Front-loaded on line charts, Phase 1a.)*
2. **Live binding off the frame budget.** `c:f`→range parse once; range→chart index; dirty-set
   by intersection (no rescan); re-resolve only dirty charts; coalesce per frame. Cache =
   first-paint + fallback.
3. **Save: three write modes coexisting.** Unedited = byte-preserve (PoC-proven); edited-loaded
   = reflow cache (bounded XML edit, keep `c:f`); authored/edited-structurally =
   **write-from-model** (serialize `Chart`→`chartN.xml` + synth drawing/anchor/rels/content-types).
   The write-from-model serializer is the hardest new piece → its own component design in Phase 6.
4. **Compatibility classification** (§3.3) — the parser is the sole classifier; deterministic
   feature→bucket mapping; 3D→2D reduction table.
5. **Performance** — lazy parse, off-screen cull, dirty-set recompute, large-series down-sample
   for paint (full data retained for save). p50/p99 targets measured at the checkpoint.

## 6. Error handling
- Parse failure / essential-unsupported → `Unsupported` → placeholder; **workbook open never
  breaks**; log the reason.
- Unresolvable `c:f` → fall back to cached values → else placeholder.
- Empty/non-numeric edited ranges → render valid points, blank rest, no crash.
- Multi-sheet save remap missing a target part → **fail loudly** (no silent chart drop).
- All chart errors are **per-chart, non-fatal** to the grid/app.

## 7. Testing strategy
- **Engine (headless, no GPU):** unit tests for parse, `ParseOutcome` classification (incl.
  3D→2D + placeholder types), `c:f` resolution, dirty-set intersection, save reflow, and
  write-from-model round-trip (Phase 6).
- **Render (`render-tests`):** lift the PoC capture harness (`xvfb`+lavapipe+`xrefresh`+`import`;
  provision `SYNTHESIS §4.4` container prereqs in CI); **perceptual-diff-vs-baseline** (reuse
  `round-3/C-ci-rendering` metric) with committed baseline PNGs per type/variation, incl. the
  badge + placeholder.
- **Real-file corpus** (Excel/LibreOffice-authored): load without breakage; save round-trip
  re-openable in both apps (PoC risks #10/#11).
- **Perf** (repo bench convention): first-paint, edit re-render, scroll-with-K-charts p50/p99.

## 8. Risks (carried from `SYNTHESIS §4/§5`, owned here)
1. App-integration correctness (anchor/z-order/clip/scroll) — the make-or-break, front-loaded on line charts.
2. Live-binding cost staying off the frame budget.
3. Save write-from-model + reflow fidelity, Excel + LibreOffice acceptance.
4. Real-file variety beyond agent-authored fixtures.
5. Bounded fidelity polish (rotated axis title, horizontal-bar order, theme/`dPt` colors).
