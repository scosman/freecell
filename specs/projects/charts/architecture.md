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

**Model-alignment decision (OOXML-shaped, bounded — not exhaustive).** The model mirrors the
`c:` structure and carries **typed Rust fields for what we render/edit** (the P1/P2 set). It
deliberately does **not** model the full DrawingML long tail (`a:spPr` fills/lines/effects/
theme, etc.) — that is effectively unbounded and we don't render it. Fidelity for everything
we don't model comes from **retaining the source XML** (§3.2) and **patching** it on edit
(§5), not from exhaustive modeling. This is the answer to "should we align to OOXML?": align
*shape and rendered fields*, preserve the rest as source — avoiding a lossy
`parse → our-model → regenerate` round-trip.

### 3.2 `ChartSpec` (engine envelope — net new)
Wraps `Chart` with everything production needs beyond a static picture:
- `source: SourceXml` — the **retained parsed chart XML** (+ its rels). The substrate for
  byte-preservation, edit-patching (§5), and the fidelity accessor (§3.3). Not just opaque
  bytes — a form we can re-serialize and targeted-patch.
- `source_ranges: Vec<CfRange>` — parsed `c:f` per data ref (for live binding).
- `anchor: Anchor` — `twoCellAnchor` from/to cell + EMU offsets (for in-grid placement).
- `origin: Loaded | Authored` — Authored charts have no `source` (synthesize on save).
- `dirty: bool`, `last_values` — live-binding bookkeeping.

### 3.3 Display fidelity — a **derived accessor**, not stored state (functional_spec §5)
There is **no parse-time `Degraded` flag to keep in sync**. The compatibility category is
*computed on demand* from the model + retained source:
```
fn display_fidelity(&self) -> Fidelity   // Faithful | Degraded | Unsupported
```
- **Unsupported** — the chart-group type has no faithful rendering (surface/radar/ofPie/stock/
  `cx:`) or the part failed to parse → placeholder.
- **Degraded** — the retained source contains **render-affecting features our renderer does not
  honor**, *or* the source's chart-group was a 3D type normalized to its 2D `ChartKind`
  (`bar3DChart→Bar`, …). → renders + the corner "⚠ May not display as intended" (ui_design §2.2).
- **Faithful** — otherwise.

"Render-affecting features we don't honor" is an **explicit, curated set** (checked against the
source), *not* "any field present" — benign fields (`c:idx`, `c:order`, layout hints) must not
trigger a false warning. The accessor **auto-clears as we add support**: once a feature becomes
rendered, it drops out of the unsupported set and the warning disappears with no separate
bookkeeping. This is the clean version of §5's three buckets — derived, self-updating.

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

SAVE ─ IronCalc write (chart-less) ─ splice: unedited     → byte-preserve retained source
                                             edited-loaded → PATCH retained source (changed fields only)
                                             authored      → synthesize source from a template
                                     + patch worksheet <drawing>/_rels + [Content_Types] + multi-sheet map

AUTHOR (Phase 6) ─ action-bar chart icon → type menu → insert Authored ChartSpec (no source) →
                    edit panel mutates Chart/ranges → live-binds → synthesize source on save
```

### 4.1 Engine — chart I/O + binding
- `discover_and_parse(path) -> Vec<ChartSpec>` (lazy per sheet).
- Live binding: build a **range→chart index**; on recompute, intersect the changed-cell set to
  get the **dirty chart set**, re-resolve their ranges from IronCalc's current values, rebuild
  their `Chart`, and publish via the **existing worker publication seam** (charts ride the same
  lock-free snapshot path as cells — not a bespoke channel).
- Save: `save_with_charts` extends PoC `save.rs` — **byte-preserve** unedited; **patch the
  retained source** for edited-loaded charts (reflow `numCache` + write back edited fields,
  keeping `c:f` and unmodeled styling); **synthesize from a template** for authored charts
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
  deferred) mutates the `Chart`/ranges; on save the source is patched (edited-loaded) or
  synthesized from a template (authored).

## 5. Technical challenges (designed here)
1. **Anchor→pixel & z-order.** Map `twoCellAnchor` (from/to cell + EMU) through the grid
   geometry cache; clip to viewport; paint above cells, below chrome. Scroll/zoom reuse the
   grid's transform. *(Front-loaded on line charts, Phase 1a.)*
2. **Live binding off the frame budget.** `c:f`→range parse once; range→chart index; dirty-set
   by intersection (no rescan); re-resolve only dirty charts; coalesce per frame. Cache =
   first-paint + fallback.
3. **Save: three write modes, source-first.** Unedited = **byte-preserve** the retained source
   (PoC-proven). Edited-loaded = **patch the retained source** — reflow `numCache`, write back
   the specific edited fields, keep `c:f` and all unmodeled styling (the fidelity win over a
   lossy regenerate; same targeted-XML pattern as `open_fixups.rs`). Authored = **synthesize
   source from a template** (no original) + drawing/anchor/rels/content-types. The
   template-synthesizer + edit-patcher are the hardest new pieces → their own component design
   in Phase 6.
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
3. Save fidelity — source-patch (edited) + template-synthesize (authored) + reflow, Excel + LibreOffice acceptance.
4. Real-file variety beyond agent-authored fixtures.
5. Bounded fidelity polish (rotated axis title, horizontal-bar order, theme/`dPt` colors).
