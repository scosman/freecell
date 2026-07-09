---
status: draft
---

# Architecture: Charts (production)

How the PoC's three layers land in FreeCell's real crate structure, plus the net-new
production machinery (app integration, live binding, perf, save-reflow). Exact engine
worker/cache APIs are in the existing `mvp`/`mvp-gaps` architecture + the `engine_worker`
component doc; this references them rather than restating.

## 1. Pinned dependencies
Mirror `app/Cargo.toml` exactly — the known-good pair `gpui`/`gpui_platform` (zed rev
`1d217ee…`) + `gpui-component` (`a9a7341…`), `ironcalc =0.7.1`, `zip 0.6` + `roxmltree 0.19`
(already engine deps via `open_fixups.rs`), `image`/`png` for the regression harness. No new
heavy deps; the PoC proved the set.

## 2. Layered placement (map PoC crates → app crates by charter)

The PoC's three concerns map cleanly onto the existing crate boundaries (which are defined
by *what they may depend on*):

| Concern (PoC crate) | Lands in | Why |
|---|---|---|
| **`chart-model`** (gpui-free, ironcalc-free data model) | **`freecell-core`** (or a sibling `freecell-chart-model` crate) | core is exactly "pure logic types, no GPU, no engine." The model is the seam; keep it dependency-light and shared. |
| **File layer** — load parser (`load.rs`/`xlsx.rs`) + save re-injection (`save.rs`) | **`freecell-engine`** | engine already owns IronCalc, file I/O, and the `open_fixups.rs` zip second-pass. Chart parse/save is the same shape of code, next to it. |
| **Live binding** — resolve `c:f` → current cell values; dirty-set on edit | **`freecell-engine`** | only the engine sees IronCalc's cells + the recompute/worker. Charts become part of what the worker resolves and publishes. |
| **Render layer** — chrome, palette, ticks, stacking, per-type renderers | **`freecell-app`** | needs gpui + gpui-component + the grid coordinate system. Lift the PoC render modules here (drop the standalone capture bins). |

The **seam stays**: engine produces `chart-model::Chart` values (now with *live* values);
app renders them. No layer reaches across.

## 3. Data flow

```
open .xlsx ─┬─ IronCalc load (cells, styles)                    [engine, existing]
            └─ chart discovery: sheet→drawing→chart chain        [engine, PoC load.rs]
                 → parse chartN.xml → ChartSpec {model + c:f ranges + anchor}
                     (first paint uses numCache; ranges kept for live binding)

edit cell ── IronCalc recompute ── worker publishes ── dirty charts (ranges ∩ edit) [engine]
                                                          │
                                          re-resolve c:f → fresh values → Chart
                                                          │
grid paint ── ChartLayer over cells ── chart_element(&Chart) at anchor rect        [app]
                 (only on-screen, non-empty charts painted)

save .xlsx ── IronCalc write (chart-less) ── re-injection splice:                  [engine]
                 unedited charts: byte-preserve parts
                 edited charts:   refresh numCache from current cells, re-emit part
                 + patch worksheet <drawing>/_rels + [Content_Types] + multi-sheet map
```

## 4. Key components

### 4.1 `ChartSpec` — the resident, live-capable chart
Wraps `chart-model::Chart` with what production needs beyond the PoC's static model:
- the **`c:f` source ranges** per series (parsed but unused in the PoC) — for live binding;
- the **anchor** (from/to cell + EMU offsets) — for in-grid positioning;
- the **raw part bytes / provenance** — for byte-preservation save of unedited charts;
- a **dirty flag** + last-resolved values.
`chart-model::Chart` itself stays the pure seam; `ChartSpec` is the engine-side envelope.

### 4.2 Live binding (engine)
- Parse each data ref's `c:f` into a resolved range (reuse IronCalc's range parsing).
- Maintain a **range→chart index** so an edit's changed-cell set yields the **dirty chart
  set** by intersection (cheap; no full rescan).
- On recompute, re-read the dirty charts' ranges from IronCalc's current values, rebuild
  their `chart-model::Chart`, and publish via the **existing worker publication seam**
  (arc-swap snapshot + `WorkerEvent`), so charts ride the same lock-free UI update path as
  cell values — not a bespoke channel.
- Cache is first-paint + fallback only.

### 4.3 Render integration (app)
- A **`ChartLayer`** painted after cells, before selection chrome (z-order), clipped to the
  grid viewport.
- **Anchor → pixels** uses the grid's existing coordinate system (row/col geometry from the
  all-styles resident cache), so charts scroll/zoom for free.
- Only **on-screen, non-empty** charts paint; a resident `Vec<RenderedChart>` keyed by
  (sheet, chart idx) is repainted on the dirty set. Off-screen = skipped.
- Per-type dispatch is the PoC's `chart_element(&Chart)` over `ChartKind`, lifted verbatim,
  extended with the fidelity features (dPt colors, data labels, number formats, axis
  scaling, rotated axis title).

### 4.4 Save / reflow (engine)
- Extend PoC `save.rs`: multi-sheet worksheet→part mapping via `workbook.xml.rels`; carry
  `styleN`/`colorsN`; **fail loudly** if a targeted worksheet part is absent (PoC risk #8).
- **Edit-reflow:** for charts whose source cells changed since load, rewrite the chart part's
  `numCache`/`strCache` from current values (bounded XML edit, not a full writer); untouched
  charts byte-preserve. Keeps `c:f` intact.

## 5. Performance strategy (north-star constraint)
- **Lazy parse** charts on first paint of their sheet region, off the open critical path.
- **Off-screen = free** (culled before paint/recompute).
- **Dirty-set recompute** — only charts whose ranges intersect an edit; coalesced per frame;
  the scroll path never rebuilds charts.
- **Large-series cap/down-sample** for paint (full data retained for save).
- Explicit **p50/p99** targets measured at the checkpoint (foreground `timeout`, forced +
  asserted op, environment-stamped — repo bench convention): first-paint, edit re-render,
  scroll frame time with K charts.

## 6. Testing & regression
- Lift the PoC capture harness into a **chart render-test suite** under `render-tests`
  (gpui window under `xvfb-run`+lavapipe+`xrefresh`+`import`; the container prereqs in
  `SYNTHESIS §4.4` must be provisioned in CI).
- **Perceptual-diff-vs-baseline** for stability (reuse `round-3/C-ci-rendering` metric —
  per-channel tolerance + fail-fraction), committed baseline PNGs per type/variation.
- **Real-file corpus** of Excel/LibreOffice-authored `.xlsx` (PoC risks #10/#11): load
  without breakage + save round-trip re-openable in both apps.
- Engine-side unit tests for parse, range resolution, dirty-set, and save reflow (headless,
  no GPU — matching `freecell-engine`'s headless CI charter).

## 7. Risks (carried from `SYNTHESIS §4/§5`, owned here)
1. **App-integration correctness** — anchor mapping, z-order, clipping, scroll/zoom sync (net-new; the make-or-break of this project, front-loaded on line charts).
2. **Live-binding cost** — range-intersection + re-resolve must stay off the frame budget.
3. **Save reflow fidelity** — refreshing caches without corrupting the part; Excel+LibreOffice acceptance.
4. **Real-file variety** — namespaces/styling the agent-authored PoC fixtures never showed.
5. **Bounded fidelity polish** — rotated axis title, horizontal-bar order, theme colors, dPt.
