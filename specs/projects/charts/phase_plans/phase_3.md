---
status: complete
---

# Phase 3: Derived fidelity accessor

## Overview

Add the **display-fidelity accessor** to `freecell-chart-model`: a pure-logic, derived
classifier that answers "how faithfully can we draw this chart?" as one of
**`Faithful | Degraded | Unsupported`** (charts/functional_spec §5, architecture §3.3).

Per the architecture this is a **derived accessor, not stored state** — there is no
parse-time flag to keep in sync. The category is computed on demand from the model plus the
**retained source XML** (`ChartSpec`'s `Origin::Loaded { source }`), so it **auto-clears as we
add renderer support**: once a feature becomes rendered it drops out of the curated
"render-affecting unsupported" set and the warning disappears with no separate bookkeeping.

Three deliverables from the plan (P3):
1. `ChartSpec::display_fidelity() -> Fidelity`.
2. **3D→2D normalization** — a reusable pure mapping (`bar3DChart→barChart`, …) the P7 parser
   will use to build the 2D model, while the source retains the 3D element so fidelity reports
   `Degraded`.
3. A **curated "render-affecting unsupported" set** sourced from
   `experiments/chart-poc/ooxml-coverage-matrix.md` (the honored/OK set = Faithful; everything
   render-affecting we don't yet honor = Degraded), built to **avoid false positives** on
   benign/default fields (architecture §3.3: "benign fields must not trigger a false warning").

Scope guardrails (from the phase brief):
- **Pure logic only** — gpui-free / ironcalc-free, no rendering wiring (nothing renders
  differently yet). The accessor scans the retained source **textually** (matching the
  "engine re-parses/patches it textually" note in `spec.rs`, and avoiding a DOM dep in the
  model crate).
- Curated set is **sourced from the coverage matrix**, not invented.

## Fidelity semantics (functional_spec §5 → architecture §3.3)

Precedence, evaluated against the retained source XML:
1. **Unsupported** — chart-group type with no faithful 2D rendering:
   `surfaceChart` / `surface3DChart` / `radarChart` / `ofPieChart` / `stockChart`, or the
   `cx:` **extended** family (chartex namespace). → placeholder (ui_design §2.3).
2. **Degraded (3D→2D)** — a 3D chart-group (`bar3DChart` / `line3DChart` / `pie3DChart` /
   `area3DChart`) normalized to its 2D `ChartKind`; source retains the 3D element.
3. **Degraded (unsupported render-affecting feature)** — the curated set is present/active.
4. **Faithful** — otherwise.

**Authored** charts (no source) are `Faithful` by construction — they are built from our own
model using only features we render.

### Curated "render-affecting unsupported" set (from `experiments/chart-poc/ooxml-coverage-matrix.md`)

The renderer's honored baseline (matrix **OK** rows) = Faithful: the six 2-D classic groups,
multi-series, cat/val/xy from cache, one solid `srgbClr` series color, title, legend, axis
titles, nice numeric ticks, `barDir`, all `grouping`s, doughnut `holeSize`.

Render-affecting features **not yet honored** → Degraded, chosen so each triggers **only when
actually active** (defaults/benign forms must not flag):

| Marker (local-name, prefix-agnostic) | Detection | Matrix row / priority |
|---|---|---|
| `dPt` | presence (only written when a point is individually styled) | C `c:dPt` — P1 (pie) |
| `gradFill` | presence (anywhere in the part — a non-solid fill we don't render; asymmetric with `schemeClr` by design) | C `a:gradFill` — P3 |
| `pattFill` | presence | C `a:pattFill` — P3 |
| `min` / `max` (axis bounds) | presence (only written when scaling set) | D `c:scaling` — P2 |
| `orientation` | `val="maxMin"` (reversed; default `minMax` benign) | D `c:orientation` — P2 |
| `smooth` | `val` truthy (`1`/`true`; `0` benign) | E/F `c:smooth` — P2 |
| `numFmt` | `formatCode` present and not `General`/empty | D `c:numFmt` — P2 |
| `showVal`/`showPercent`/`showCatName`/`showSerName`/`showBubbleSize`/`showLegendKey` | any `val` truthy (all-zero `dLbls` benign) | F `c:dLbls` — P2 |
| `symbol` (marker) | `val` present and ∉ {`none`, `circle`} — the round dot is drawable (matrix §C); `none`/`circle` benign, so supported scatter series aren't badged | C `c:marker` — P2 |

**Deliberately excluded** (would false-positive on normal supported charts, per §3.3): the
axis `scaling` wrapper itself (always present), `majorGridlines`/`minorGridlines` (we draw
gridlines anyway), `varyColors` (matches our palette behavior), `gapWidth`/`overlap`/
`firstSliceAng` (Excel writes them at defaults on nearly every bar/pie), and `schemeClr`
(pervasive across chrome/text; theme-color fill fidelity is better assessed by the P7 parser
where fill context is known / lands with P6). Each entry auto-drops from the set as its
support lands (P6/P12/P13).

## Steps

1. **New module `crates/freecell-chart-model/src/fidelity.rs`.** Pure logic over an XML `&str`.
   - `pub enum Fidelity { Faithful, Degraded, Unsupported }` (`Clone,Copy,Debug,PartialEq,Eq`),
     with the §5 semantics documented, plus two predicates encoding the UI contract:
     `pub fn renders_as_chart(self) -> bool` (Faithful|Degraded) and
     `pub fn shows_compatibility_warning(self) -> bool` (Degraded only).
   - `pub fn normalize_3d_chart_group(local_name: &str) -> Option<&'static str>` — the 3D→2D
     element-name map (`bar3DChart→barChart`, `line3DChart→lineChart`, `pie3DChart→pieChart`,
     `area3DChart→areaChart`); `None` for anything else (surface/radar/… are Unsupported, not
     normalized).
   - `pub fn source_fidelity(chart_xml: &str) -> Fidelity` — the classifier, precedence above.
   - Private const sets `UNSUPPORTED_CHART_GROUPS`, `CHART_GROUPS_3D` (kept consistent with
     `normalize_3d_chart_group` via a test), and the render-affecting detectors.
   - Private textual helpers, prefix-agnostic + tag-boundary-aware so short/benign names don't
     false-match: `contains_element`, `any_opening_tag` (core scanner), `opens_tag_name`,
     `attr_value`, `is_ncname_char`, plus the value-aware detectors (`smooth_enabled`,
     `axis_reversed`, `custom_number_format`, `data_labels_shown`, `markers_shown`) and
     `is_extended_chart` (`chartex` namespace).
2. **`crates/freecell-chart-model/src/spec.rs`:** add
   `impl ChartSpec { pub fn display_fidelity(&self) -> Fidelity }` — `Loaded` → delegate to
   `source_fidelity(&source.chart_xml)`; `Authored` → `Fidelity::Faithful`. Document that it is
   derived (no stored flag) and auto-clears as support lands.
3. **`crates/freecell-chart-model/src/lib.rs`:** `mod fidelity;` and
   `pub use fidelity::{normalize_3d_chart_group, source_fidelity, Fidelity};`. Add a crate-doc
   line noting the derived fidelity accessor now lives here.
4. Run the checks below until clean.

## Tests

Pure-logic unit tests (no GPU/display). In `fidelity.rs`:
- `supported_group_sources_are_faithful` — each honored 2-D group source classifies `Faithful`
  (incl. a doughnut with `holeSize`).
- `benign_fields_do_not_degrade` — a realistic supported line chart carrying only honored +
  benign/default fields (`c:idx`/`c:order`, `scaling`+`orientation val="minMax"`,
  `numFmt formatCode="General"`, `smooth val="0"`, `marker`+`symbol val="none"`, an all-zero
  `dLbls`, `varyColors`, `majorGridlines`) stays `Faithful`.
- `unsupported_groups_are_unsupported` — each unsupported group → `Unsupported`
  (covers the exit "surface/radar⇒Unsupported"; includes `surface3DChart`).
- `extended_cx_family_is_unsupported` — a `cx:chartSpace` (chartex namespace) → `Unsupported`.
- `three_d_groups_are_degraded` — `bar3DChart`/`line3DChart`/`pie3DChart`/`area3DChart` each →
  `Degraded` (exit "3D⇒Degraded").
- `normalize_3d_chart_group_maps_each_3d_to_2d` + `normalize_3d_returns_none_for_non_3d`
  (supported + unsupported names → `None`).
- `chart_groups_3d_const_matches_normalizer` — every `CHART_GROUPS_3D` entry normalizes to
  `Some`, and nothing outside it does (consistency guard).
- `active_render_affecting_features_degrade` — one case per curated marker in its **active** form
  (`dPt`, `gradFill`, `pattFill`, explicit `min`/`max`, reversed `orientation`, `smooth val="1"`,
  non-General `numFmt`, a **conditional `numFmt` with an unescaped `>`** in its value,
  `showVal val="1"`, a **non-circle** `symbol val="diamond"`) → `Degraded`.
- `benign_feature_forms_do_not_degrade` — the default/off/drawable forms do **not** degrade
  (`smooth val="0"`, General `numFmt`, `minMax` orientation, `symbol val="none"` **and**
  `symbol val="circle"`, all-off `dLbls`) — the false-positive guard.
- `realistic_scatter_with_circle_markers_is_faithful` — a full scatter series with
  `<c:symbol val="circle"/>` stays `Faithful` (supported type must not be badged).
- `unsupported_precedes_degraded` — a source that is both a surface chart and has a degrading
  feature → `Unsupported` (precedence).
- Helper edge cases: `contains_element` is prefix-agnostic (`<c:x>` / `<x>` / `<foo:x>`),
  tag-boundary-aware (`min` not matched inside `minorGridlines`; `surfaceChart` not matched
  inside `surface3DChart`), ignores closing tags; `attr_value` does not match an attribute
  name embedded in another (`val` not matched inside `interval`); and `tag_close_offset` is
  quote-aware (a `>` inside a quoted value doesn't close the tag).
- `Fidelity` predicates: `renders_as_chart` / `shows_compatibility_warning` truth table.

In `spec.rs`:
- `loaded_spec_display_fidelity_reads_source` — a `Loaded` spec whose source is a 3D bar →
  `Degraded`; a surface source → `Unsupported`; a plain bar source → `Faithful`.
- `authored_spec_is_faithful` — an `Authored` spec (no source) → `Faithful`.

## Checks (foreground, under `timeout`)

Run the **full workspace gate** green (a scoped subset in round 1 missed a workspace-wide
`fmt` failure):
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo build --workspace`
- `cargo test --workspace`
- `RUSTDOCFLAGS="-D warnings" cargo doc -p freecell-chart-model`

No render-suite work: this phase changes no grid/cell/sheet/titlebar pixels (pure model logic).
