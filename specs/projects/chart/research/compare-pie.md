# Chart Comparison — Pie / Doughnut

Per-type Excel-vs-`gpui-component` comparison for the **pie / doughnut** family, scoped to
"at most what `gpui-component` renders." Reframing carried throughout: `gpui-component`
gives us **(A)** the `PieChart` **struct** as-is and **(B)** DIY on the `plot/` primitives
(`Pie` + `Arc`). Both levels are assessed.

Primary inputs: `research/excel-chart-data-model.md`, `research/gpui-component-charts.md`
(Wave-1 doc; pinned rev `a9a7341c35b62f27ff512371c62419342264710c`), plus direct reads of
the pinned `chart/pie_chart.rs`, `plot/shape/pie.rs`, `plot/shape/arc.rs`. Line citations
below are at that rev.

---

## 0. TL;DR

- **Single-series pie** and **single-ring doughnut** are a **strong win** at Level A
  (`PieChart` struct) — donut hole, per-slice color, leader-line labels with real
  overlap-avoidance all exist — **provided we solve the color-mapping problem** (§4). That
  problem is the crux: `gpui-component` has **no auto-palette**; unset, every slice paints
  the same color (`chart_2`) and a pie becomes an illegible monochrome disc.
- **3-D pie** → **flatten to 2-D** (Level A). Acceptable: 3-D pie carries no extra data,
  only perspective distortion.
- **Multi-ring doughnut** (multiple series = concentric rings) → **Level B DIY** (loop
  `Pie`+`Arc` rings) or **OUT**. **Pie-of-pie / bar-of-pie** (`c:ofPieChart`) → **OUT**
  (needs a secondary plot the library has no concept of).
- **Exploded slices** → Level A **approximation only** (size emphasis, not true radial
  offset); true explosion is Level B. **Rotation** (`firstSliceAng`) → **Level B only**
  (the `Pie` primitive supports it; the `PieChart` struct does not expose it).
- **No hover/tooltip** on pie (paint-only) — a **minor** loss for a display-only chart,
  partly offset by on-slice data labels. **No legend anywhere** in the module — offset by
  the same labels.

---

## 1. Excel side — the pie/doughnut family in OOXML (`c:`)

The family is four distinct chart-group elements under `c:plotArea`
(`excel-chart-data-model.md` §2a), all sharing the `c:ser` → `c:cat`/`c:val` data shape
(§4b of that doc):

| Element | Common name | Series | Key knobs |
|---|---|---|---|
| `c:pieChart` | Pie (2-D) | **single** `c:ser` typical → one **slice per category** | `c:varyColors`, `c:firstSliceAng` |
| `c:doughnutChart` | Doughnut (2-D) | **one OR many** `c:ser` → **concentric rings** | `c:holeSize` (10–90 %), `c:firstSliceAng` |
| `c:pie3DChart` | 3-D Pie | single | 3-D via `c:view3D` on `c:chart` |
| `c:ofPieChart` | Pie-of-Pie / Bar-of-Pie | single + a **secondary** plot | `c:ofPieType`(`pie`/`bar`), `c:splitType`, `c:splitPos`, `c:secondPieSize`, `c:gapWidth` |

Shared/relevant knobs (`excel-chart-data-model.md` §5):

- **`c:varyColors`** — `1` = color every point (slice) **differently** (the pie/doughnut
  default); `0` = every slice takes the single series color. Drives the auto-palette
  behavior discussed in §4.
- **`c:firstSliceAng`** — rotation of the first slice, **degrees clockwise from 12
  o'clock** (0–360).
- **`c:holeSize`** — doughnut hole as a **percent of radius** (typ. 50; range ~10–90).
- **Per-point color** — `c:ser` → `c:dPt` (carries `c:idx` + its own `c:spPr` →
  `a:solidFill` → `a:srgbClr val="RRGGBB"` **or** `a:schemeClr val="accent1…"`). Present
  only when a slice is explicitly styled (user override) or a producer bakes the auto
  colors out.
- **Exploded slice** — `c:dPt` → `c:explosion` (percent of radius the slice is pulled
  radially **outward** from center).
- **Data labels** — `c:dLbls` with `c:showCatName` / `c:showVal` / `c:showPercent` /
  `c:showLegendKey`, optional leader lines; per-chart, per-series, or per-`c:dPt`.

**Data model note:** a pie's slices come from **one series' `c:cat` (labels) + `c:val`
(numbers)** read from `numCache`/`strCache` or resolved from the `c:f` range
(`excel-chart-data-model.md` §4b–4c; `ironcalc-chart-exposure.md` §4 — we roll our own
extractor, IronCalc exposes no chart data). A **multi-ring doughnut** is simply **multiple
`c:ser` siblings** each supplying a ring (`excel-chart-data-model.md` §4d).

**Prevalence (design guidance):** single-series pie and single-ring doughnut are the
common cases; 3-D pie is common-ish in older decks; multi-ring doughnut, `ofPie`, and
exploded slices are comparatively rare.

---

## 2. gpui side — `PieChart` struct vs `Pie`/`Arc` primitives

### Level A — the `PieChart<T>` struct (`chart/pie_chart.rs`)

Verbatim capability (`gpui-component-charts.md` §2 PieChart; confirmed in source):

```rust
PieChart::new(data)                       // Vec<T>, generic item type
  .value(|&T| -> f32)                      // NOTE f32 (not f64)          pie_chart.rs:109
  .inner_radius(f32) | .inner_radius_fn(|&ArcData<T>| -> f32)   // donut hole (PIXELS)
  .outer_radius(f32) | .outer_radius_fn(|&ArcData<T>| -> f32)   // per-slice radius (PIXELS)
  .pad_angle(f32)                          // gap between slices
  .color(|&T| -> impl Into<Hsla>)          // per-slice color               pie_chart.rs:115
  .label(|&T| -> SharedString)             // leader-line label per slice   pie_chart.rs:127
  .label_line_color(..) .label_color(..) .label_gap(..)
```

- **Single ring only.** `paint` builds one `Pie::<T>::new()` over the one `Vec<T>` and
  draws each arc (`pie_chart.rs:168-187`).
- **Donut: yes, well-supported.** `inner_radius`/`outer_radius` (+ `*_fn` per-slice
  variants), `pad_angle` for slice gaps. Outer radius auto-defaults to `0.4 * height`
  when unset (`pie_chart.rs:158-162`).
- **Color: per-slice via `.color()` closure** (`pie_chart.rs:115-121`). **Crux —** if
  `.color()` is unset, every slice paints `cx.theme().chart_2` (`pie_chart.rs:180`).
  **There is NO auto-palette**; we must supply the closure or the pie is monochrome.
- **Labels: yes, high quality.** Leader-line + text outside the ring with a genuine
  overlap-avoidance layout pass (`spread_labels`, `pie_chart.rs:210-329`); slices < 0.5°
  are skipped (`pie_chart.rs:214`). Takes **one `SharedString` per slice** — we compose
  cat/value/percent into it ourselves.
- **No rotation setter, no explosion, no hover.** The struct hard-codes
  `Pie::<T>::new()` at the default start angle and never offsets a slice's center. The
  `Plot` impl is **paint-only** (no `id`/`tooltip_state`/`tooltip`, `pie_chart.rs:152`) —
  **no interactivity** (`gpui-component-charts.md` §2 PieChart, §3 tooltip row).

### Level B — the `Pie` + `Arc` primitives (`plot/shape/pie.rs`, `plot/shape/arc.rs`)

These are d3-shape ports and are **more capable than the struct exposes**:

- **`Pie<T>`** computes slice angles: `.value()`, `.pad_angle()`, and — key — **`.start_angle()`
  / `.end_angle()`** (`pie.rs:41-50`). `Pie::arcs(&data)` walks values, **drops any value
  ≤ 0**, and lays out `ArcData{start_angle,end_angle,pad_angle,value,index}` sweeping from
  `start_angle` (`pie.rs:59-96`). So **rotation is achievable at Level B** by feeding
  `firstSliceAng` (deg→rad) into `.start_angle()` — even though `PieChart` never exposes it.
- **`Arc`** draws one slice path with `inner_radius`/`outer_radius`, centered on the
  **passed `bounds`** center (`arc.rs:91-92`). Angle convention subtracts `π/2`, i.e.
  **angle 0 = 12 o'clock, increasing clockwise** — the **same convention as Excel's
  `firstSliceAng`** (`arc.rs:62,77`). Handles the full-circle single-slice edge case
  (`arc.rs:80-86`).
- **No center-offset param on `Arc`.** Because the center is derived from the `bounds`
  argument, **true slice explosion is DIY**: paint an individual arc with a `bounds` whose
  center is shifted along the slice's bisector by `explosion% * radius`. Feasible but
  hand-rolled.
- **Multi-ring is DIY:** run `Pie::arcs` per series and paint each series' arcs into a
  distinct `[inner,outer]` radius band (concentric). Straightforward on the primitives, but
  you **lose the struct's built-in leader-line labeling** (which is single-ring only) and
  must write your own custom `Plot`.

---

## 3. Mapping table

Level A = `PieChart` struct as-is; Level B = DIY on `Pie`/`Arc` primitives in a custom
`Plot`; Approximate = renders but not faithful; OUT = not renderable within
"at most what gpui-component renders."

| Excel feature | Level | Render note |
|---|---|---|
| **Pie (single series)** — `c:pieChart` | **Level A** ✅ | One slice per category via `.value()`; **must** supply `.color()` (§4) or all slices are `chart_2`. Strongest case. |
| **Doughnut single-ring** — `c:doughnutChart`, 1 series | **Level A** ✅ | Set `inner_radius = holeSize% × outer_radius`; `pad_angle` for gaps. Same color story as pie. |
| **Doughnut multi-ring** — `c:doughnutChart`, N series | **Level B** ⚠ / OUT | Loop N rings, each its own `Pie::arcs` in a concentric `[inner,outer]` band. No struct support; loses built-in labels → custom `Plot`. Rare; reasonable to defer/OUT for MVP. |
| **3-D pie** — `c:pie3DChart` | **OUT → flatten to Level A** ✅ | No 3-D anywhere in the library (`gpui-component-charts.md` §3 "3D: None"). Render as flat 2-D pie; loses only perspective, not data. Usually acceptable. |
| **Pie-of-pie / bar-of-pie** — `c:ofPieChart` | **OUT** ❌ | Needs a *secondary* plot (a second pie or a bar) linked by a split rule — no concept in the library. Best degrade = draw the primary pie only, dropping the "of-pie" split (semantics lost). Rare. |
| **Exploded slices** — `c:dPt`→`c:explosion` | **Approximate (Level A)** / **Level B** ⚠ | Level A: fake emphasis via `.outer_radius_fn()` (bigger slice) — **not** true radial offset (same center). True explosion = Level B: paint that slice's `Arc` with a bounds center shifted along its bisector. |
| **Rotation** — `c:firstSliceAng` | **Level B** ⚠ | `Pie::start_angle(deg→rad)` — matching clockwise-from-top convention (`arc.rs:62`). **Not** on the `PieChart` struct, so Level A ignores rotation (slices start at 12 o'clock). |
| **Per-slice color** — `c:dPt`→`a:solidFill` | **Level A** ✅ | `.color()` closure keyed on slice. This is *how* we solve the pie color problem (§4), not a limitation. |
| **Data labels** — `c:dLbls` (cat/val/%) | **Level A** ✅ | `.label()` composes cat/value/percent into one string; real leader lines + de-overlap. |
| **Legend** | **OUT** ❌ | No legend in the module at all (`gpui-component-charts.md` §3). Compensate with on-slice labels. |
| **Hover tooltip** | **OUT** ❌ | Pie `Plot` is paint-only (`pie_chart.rs:152`). Minor for display-only; labels compensate. |

**Unit/convention conversions we own (all trivial):** `holeSize` percent → `inner_radius`
pixels (`inner = holeSize/100 × outer_radius`); `firstSliceAng` degrees → radians for
`Pie::start_angle`; `explosion` percent → pixel offset along the slice bisector.

**Fidelity nit:** `Pie::arcs` silently **drops values ≤ 0** (`pie.rs:64-70`). Excel plots a
pie slice for the *magnitude* of a value; a zero yields no slice either way, but a negative
value that Excel would still show (as its absolute size) disappears in gpui. Rare in pies;
note it.

---

## 4. Owner's asks

### Multiple datasets
Pie is **inherently single-series** — one `c:ser`, one ring, one slice per category. The
"multiple datasets" case in this family is the **multi-ring doughnut** (multiple `c:ser` =
concentric rings). `PieChart` renders **one ring only**, so multi-dataset pie/doughnut is
**Level B DIY (loop rings) or OUT** (§2, §3). For a display MVP the pragmatic call is:
support single-series pie and **single-ring** doughnut on the struct; treat multi-ring as a
later DIY item or a documented gap.

### Coloring — THE CRUX for pie
Two facts collide: (1) Excel **auto-colors** pie slices from a theme palette (default
`c:varyColors=1`), and (2) `gpui-component` has **no auto-palette** — an unset `.color()`
paints **every** slice `chart_2` (`pie_chart.rs:180`), i.e. a monochrome disc that is
useless as a pie. **We must synthesize a per-slice color for every slice.** Excel's slice
color resolves in three tiers:

1. **Explicit per-point override** — `c:dPt`→`c:spPr`→`a:solidFill` (`a:srgbClr` literal or
   `a:schemeClr` accent name). Present only when a slice was manually styled (or a producer
   baked colors out). **Cheap to honor.**
2. **Chart color style** — `xl/charts/colorsN.xml` (`cs:colorStyle`): an ordered cycle of
   `a:schemeClr` accent refs **plus variation transforms** (lum/tint) applied when there are
   more slices than base colors. This is where **modern Excel's auto pie colors actually
   live** — a **separate part** from `chartN.xml`, and it is **usually absent from the chart
   XML itself**.
3. **Theme** — `xl/theme/theme1.xml` `a:clrScheme` resolves `accent1…6` to concrete RGB.
   **FreeCell already parses `theme1.xml`** in `open_fixups.rs`
   (`ironcalc-chart-exposure.md` §4), so the accent palette is already in hand.

**Assess the two mapping strategies the brief names:**

- **(A) Read colors from the file.** Honor `c:dPt` `a:solidFill` where present (easy for
  `srgbClr`; `schemeClr` → resolve via the already-parsed theme). For **auto** slices,
  either (a) also parse `colorsN.xml` and replicate Office's accent-cycle + variation
  algorithm (**highest fidelity, moderate effort** — the variation/tint math is the fiddly,
  under-documented tail), or (b) **cycle the theme accent palette (`accent1…6`) directly** —
  which *is* the default color style for the common "Colorful" pie — applying a deterministic
  tint/shade for slice #7+. Strategy (A/b) reproduces the **default Excel pie look closely**
  at low cost because FreeCell already has the accents; only the exact tints for >6 slices
  and non-default color styles are approximate.
- **(B) Synthesize from gpui `chart_1..chart_5`.** Ignore the file, cycle the gpui theme's
  **5** categorical colors by slice index. Zero OOXML color parsing and on-brand with the
  rest of FreeCell's UI, **but**: only 5 base colors (visible repetition past 5 slices
  unless we generate shades), and it **will not match** the author's chosen/Excel colors.
  Acceptable as a last-resort fallback, wrong as the primary path for a file that specifies
  colors.

**Recommended (fidelity-for-effort):** a **hybrid** — honor explicit `c:dPt` solidFills
(respects user intent, cheap), and for the remaining auto slices **cycle the Excel theme
accent palette** (already parsed) with a deterministic tint for large slice counts. Honor
`c:varyColors=0` by painting all slices the single series color (which happens to be gpui's
own default behavior). Full `colorsN.xml` parsing is a later fidelity upgrade, not an MVP
blocker. Note this color-mapping work is **required, not optional polish** — without it the
pie does not read as a pie.

### 3-D
**OUT — flatten to 2-D** (`c:pie3DChart` → `PieChart`). The library has no 3-D of any kind
(`gpui-component-charts.md` §3). A 3-D pie encodes no data the 2-D pie lacks — only tilt and
depth — so flattening is **usually acceptable** and arguably *more* readable (3-D pie
perspective distorts slice-area perception). Doughnut has no 3-D variant in OOXML, so nothing
to flatten there.

### Interactivity
Pie is **paint-only — no hover/tooltip** (`pie_chart.rs:152`; `gpui-component-charts.md` §2).
A viewer cannot hover a slice to reveal its exact value/percent. For a **display-only**
chart this is a **minor loss**, and it is largely mitigated by baking category + value +
percent into the on-slice `.label()` text (which also stands in for the missing legend).

---

## 5. Verdict

**Render pie/doughnut WELL, with one required piece of work and a few bounded gaps.**

- **Single-series pie & single-ring doughnut = strong win** at Level A: donut hole, per-slice
  color, `pad_angle` gaps, and genuinely polished leader-line labels with overlap-avoidance
  all exist in the `PieChart` struct — *conditional on us supplying the per-slice colors*,
  because gpui has **no auto-palette** (unset → monochrome). The **color-mapping story is the
  crux and is load-bearing**, not polish; the hybrid "honor `c:dPt`, else cycle the
  already-parsed Excel theme accents" path gets us close to Excel's default look at low cost.
- **3-D pie → flatten to 2-D:** acceptable (no data lost).
- **Multi-ring doughnut → Level B DIY (loop `Pie`/`Arc` rings) or OUT**, and **pie-of-pie /
  bar-of-pie → OUT** (no secondary-plot concept). Both are rare; reasonable MVP gaps.
- **Exploded slices → approximate** (Level A size-emphasis) or true-offset via Level B;
  **rotation (`firstSliceAng`) → Level B only** (the `Pie` primitive supports `start_angle`;
  the struct does not).
- **No hover tooltip and no legend** → minor losses for a static display chart, mitigated by
  on-slice data labels.

Bottom line: the two common cases (single pie, single-ring doughnut) render **well** once the
color mapping is built; 3-D flattens acceptably; the remaining family members (multi-ring,
ofPie, true explosion, rotation) are **bounded, mostly-rare gaps** that are either DIY on the
primitives or out of scope.

---

### Sources
- `research/gpui-component-charts.md` (Wave-1, rev `a9a7341…`): §2 PieChart, §3 cross-cutting
  (no legend, no 3-D, tooltip matrix), §4 capability matrix.
- `research/excel-chart-data-model.md`: §2a (pie/doughnut/pie3D/ofPie elements), §4b–4d
  (`c:ser`/`c:cat`/`c:val`, multi-series), §5 (varyColors, firstSliceAng, holeSize, dPt,
  explosion, dLbls).
- `research/ironcalc-chart-exposure.md`: §4 (we roll our own extractor; `theme1.xml` already
  parsed in `open_fixups.rs`).
- Pinned source (rev `a9a7341c35b62f27ff512371c62419342264710c`): `chart/pie_chart.rs`
  (`.value` f32 :109, `.color` :115, default `chart_2` :180, paint-only `Plot` :152,
  `spread_labels` :297-329); `plot/shape/pie.rs` (`start_angle`/`end_angle` :41-50, `arcs`
  drops ≤0 :59-96); `plot/shape/arc.rs` (center from bounds :91-92, clockwise-from-top
  convention :62/:77, no offset param).
</content>
</invoke>
