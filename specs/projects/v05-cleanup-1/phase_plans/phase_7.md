# Phase 7 — G1: Detect multi-group (combo) charts

**Verdict: CONFIRMED, both halves — and both demonstrated by tests that fail against the old
code.**

## Confirmation

Not by reading. Each new test was run against the **pre-change** implementation (the group check
short-circuited, `is_extended_chart` restored to its bare substring) and observed to fail:

```
a_combo_chart_is_degraded ......................................... FAILED
every_combo_of_supported_groups_is_degraded ....................... FAILED
a_literal_chartex_string_in_content_is_not_an_extended_chart ...... FAILED
```

- **Combo:** an ordinary Excel bar+line plot area classified `Fidelity::Faithful`, while
  `parse_chart_xml` (`load.rs:605`) takes `.find(…)` — the **first** chart-group child — and
  discards the rest. So the line series is absent from the drawing and the chart is presented as
  exact. Exactly as the unit described.
- **`is_extended_chart`:** `xml.contains("chartex")` really can produce a wrong answer on
  realistic content. A bar chart whose series is named `"chartex rollout"` classified
  `Unsupported` — a perfectly renderable chart replaced by the placeholder. The old test only ever
  fed it a genuine `cx:` part, so the false positive was invisible.

## What changed

### `has_multiple_chart_groups` in `source_fidelity`

Placed **after** `is_unsupported_chart` and beside `has_3d_chart_group`. The ordering is
load-bearing and tested: a plot area combining `surfaceChart` with `lineChart` must stay
`Unsupported` rather than softening to `Degraded` — softening would swap an honest placeholder for
a badged drawing of a chart we cannot render.

Counting needs the **complete** OOXML group list, not the subset we parse (a `barChart` +
`surfaceChart` combo is still a combo), so `chart-model` gains `CHART_GROUP_ELEMENTS` — all
sixteen group element names. `count_opening_tags` is the counting twin of the existing
`any_opening_tag` scanner, inheriting its boundary rules, so `</c:barChart>` is not miscounted as a
second group (a detector that made that mistake would flag every single-group chart in existence —
tested).

### `is_extended_chart`

Now matches the full namespace URI `http://schemas.microsoft.com/office/drawing/2014/chartex`
rather than the seven-letter substring. Long, specific, and only ever appears in a namespace
declaration.

### Engine-side guard

`every_parsable_chart_group_is_known_to_the_combo_detector` asserts
`load::CHART_GROUP_TAGS ⊆ chart_model::CHART_GROUP_ELEMENTS` (plus the 3-D names). Without it,
adding support for a new group in the engine would leave the detector blind to combos involving it
— and a combo the detector cannot see is a chart drawn with missing series and no badge. This is
the same "two lists, nothing enforcing agreement" shape the whole review is about, so it gets an
enforcer rather than a comment.

`a_combo_parses_to_the_first_group_only_and_is_flagged` locks the parser's actual behaviour
alongside the new `Degraded` verdict — documenting the loss rather than pretending otherwise.

## Reason strings — the half the architecture approved, and the half it required

Functional spec §G1(2) asks for `Degraded` *"with a reason that names the dropped groups"*.
Architecture §7 splits that into two decisions, and only one of them is a decline:

- **Not widening `Fidelity`** — approved in advance. It is a bare enum with no reason field; adding
  one touches every classifier call site and the badge UI, and **G3 is the unit that reworks the
  classifier**. Widening it here would collide with G3 for a cosmetic gain.
- **The `tracing::warn!` in the parser** — *required*, not optional: §7 accepted the above **on
  condition** that "the parser (`load.rs`) gains a `tracing::warn!` naming the retained group and
  the dropped ones, so the information exists at runtime". It is now there
  (`parse_chart_xml`, backed by `dropped_chart_groups`). It fires only when a group is actually
  dropped — never on the ordinary single-group chart — and lands on the same channel `load.rs`
  already uses for per-chart diagnostics (lines 91/193/314/358/436; `save.rs` 215/238).

The badge's Degraded text carries no detector-sourced reason today, and §7 explicitly says the
phase does not invent that channel: recorded as the limitation it is, for G3 to close.

## `GAPS.md`

The combo row was folded into "More chart types" in a way that implied a combo gets the
"unsupported chart" placeholder, like radar or stock. It does not — that is the claim the unit
called out. It is now its own row stating what actually happens (first group only, remaining series
absent, `Degraded` after this unit) and what G1b needs, at the v2.0 tier.

## Verification

- `cargo test -p freecell-chart-model --lib` — 100 passed
- `cargo test -p freecell-engine --lib chart::` — 125 passed
- `cargo fmt --all --check` clean

No render impact: this changes parse-time classification, not the chart widgets. A combo chart now
paints the existing Degraded badge it should always have had; no committed `chart_*` baseline
contains a combo (they are single-group scenes built from chart-model fixtures), so no baseline can
move.

---

## Review remediation

Code review found **no Criticals**; it verified `CHART_GROUP_ELEMENTS` is exactly the 16 elements
`CT_PlotArea` permits, that the tag-boundary logic is right (a `pieChart` inside `<c:ofPieChart>` is
correctly not counted), and that the `cx1`…`cx8` variant-namespace concern is bounded. Those were
left alone. Every finding below, including the ones disproved:

### Moderate

**1. Doc comments welded to the wrong function (`fidelity.rs`).** Confirmed — `count_opening_tags`
had been pasted between `any_opening_tag`'s doc block and `any_opening_tag`, so the predicate-taking
description sat on a function that takes no predicate, and `any_opening_tag` (referenced by name
from ten other doc comments) was undocumented. Fixed as part of finding 2: the scan description now
lives on the shared `opening_tags`, and both wrappers carry a one-line doc pointing at it.

**2. `count_opening_tags` duplicated `any_opening_tag`'s scan body.** Confirmed. Extracted
`fn opening_tags<'a>(xml: &'a str, local_name: &'a str) -> impl Iterator<Item = &'a str> + 'a`
yielding each matching tag's attribute text; `any_opening_tag` is now `.any(pred)` over it and
`count_opening_tags` is `.count()`. The boundary rules (`after_ends_name`, `opens_tag_name`, the
`from = end` advance) exist once. Behaviour-preserving, proven by the existing tests: 113 passed
before the extraction, 113 passed after, none touched.

**3. `is_extended_chart` tested free text, not a declaration.** Confirmed, and both false positives
reproduced. It now implements what architecture §7 specified: the **root element** must be a
`chartSpace` whose *own* prefix is declared as the chartex URI (`root_element` + `attr_value`).
Note the architecture's "namespace-declaration value **or** root element" reads as a disjunction,
but a declaration alone is exactly false-positive (a) — Excel writes spare `xmlns:cx` declarations
with `mc:Ignorable` on ordinary parts — so the implemented predicate is the **conjunction**, which
is the only form that rejects both cases. New tests:
`a_classic_chart_that_merely_declares_the_cx_namespace_is_not_extended`,
`the_chartex_uri_as_cached_content_is_not_an_extended_chart`,
`a_cx_part_declaring_variant_namespaces_is_still_extended` (the reviewer's cheap insurance: `cx`
and `cx1` declared side by side, root resolved through whole-attribute-name matching), and
`root_element_skips_prologue_noise`. The doc comment no longer claims the URI "appears only in a
namespace declaration"; it names both false positives and the test that pins each.

**4. The `tracing::warn!` architecture §7 required.** Confirmed — the decline rested on "no logging
today", which is false (`load.rs` warns at 91/193/314/358/436; `save.rs` at 215/238). Added, in
`parse_chart_xml`, naming the retained group and the dropped ones via the new
`dropped_chart_groups` helper (which is what the test asserts on — a warn body is not directly
observable without a subscriber). It fires only on an actual combo. The "Reason strings" section
above was rewritten to state the architecture's actual position rather than implying it
pre-approved the omission.

**5. Unguarded in-crate list agreements.** Confirmed. `classifier_group_lists_are_subsets_of_the_counting_set`
asserts `UNSUPPORTED_CHART_GROUPS` and `CHART_GROUPS_3D` are subsets of `CHART_GROUP_ELEMENTS`.
The reviewer's second point — the cross-crate guard hardcoding the four 3-D names instead of
deriving them — is fixed by moving that agreement to where it can be derived:
`no_group_normalizes_to_2d_without_being_listed_as_3d` (chart-model) asserts, over the complete
`CT_PlotArea` set, that `normalize_3d_chart_group(name).is_some() == CHART_GROUPS_3D.contains(name)`,
so a fifth 3-D mapping added to the normalizer alone goes red. The engine-side guard drops its
hardcoded copy, derives the 3-D names by filtering `CHART_GROUP_ELEMENTS` through the normalizer,
and its doc comment now says what it actually catches (a typo) instead of overselling.

### Mild

- **`has_multiple_chart_groups` doc vs. scan scope** — *doc fixed, scan deliberately not bounded.*
  Bounding it to the `c:plotArea` span would make the detector fail **closed** on every fragment
  that carries a group without the surrounding plot area — which is every group-level unit test in
  the file, and any patched-source fragment. A chart-group element is only legal inside
  `c:plotArea` anyway, so the bound buys nothing against well-formed input. The doc now says "in the
  part", states the scope decision, and points at the blind-spot list.
- **Flat-scanner blind spots named** — `opening_tags`' doc now lists XML comments, CDATA and
  `mc:AlternateContent` Choice/Fallback as shapes that can over-count, and records the reviewer's
  point that the *realistic* vector (an escaped `&lt;c:lineChart&gt;` in a cached series name) is
  safe because XML forbids a raw `<` in character data and attribute values.
- **An empty chart-group element over-counts** — not cheaply fixable in a flat scan (it needs the
  group's children, i.e. nesting). Recorded as a "Known over-count" paragraph on the detector, with
  the reasoning that a too-honest badge on a degenerate part Excel never writes is the right side to
  err on.
- **`a_three_d_combo_is_degraded` was inert** — confirmed by mutation (below). Renamed
  `a_three_d_combo_trips_both_detectors_and_is_degraded` and now asserts `has_multiple_chart_groups`
  and `has_3d_chart_group` directly, so deleting the combo counter turns it red.
- **`load.rs` `.all(…)` passes on zero series** — confirmed by mutation. Paired with
  `assert_eq!(chart.series.len(), 1)` plus a name assertion on the retained series.
- **`GAPS.md` combo row** — "first" corrected to "first **recognized**" (`radarChart` + `barChart`
  keeps the second, since `is_chart_group` skips the radar); the five-sentence narrative moved out
  of the **Logged?** column into *Readiness / notes*, leaving that column a one-token status like
  every other row.
- **Const placement** — `CHART_GROUP_ELEMENTS` and `NS_CHARTEX` moved up into the top const block
  with the other lists; `NS_CHARTEX` no longer sits after its only user.

### Mutations run (each new/changed behaviour proven red)

| Mutation | Expected | Result |
|---|---|---|
| A — `is_extended_chart` restored to `xml.contains(NS_CHARTEX)` | the two false-positive tests fail | **RED**: `a_classic_chart_that_merely_declares_the_cx_namespace_is_not_extended`, `the_chartex_uri_as_cached_content_is_not_an_extended_chart`, `root_element_skips_prologue_noise` (3 failed / 116 passed) |
| B — `has_multiple_chart_groups` → `false` | the 3-D combo test must now go red (it did not before) | **RED**, and it fails at the *counter* assertion (`fidelity.rs:1003`), reaching neither the 3-D nor the `source_fidelity` assertion — i.e. the old form would still have passed |
| C — `"pyramidChart"` added to `UNSUPPORTED_CHART_GROUPS` only | subset guard fails | **RED**: `classifier_group_lists_are_subsets_of_the_counting_set` |
| D — a fifth arm `"surface3DChart" => Some("surfaceChart")` in the normalizer | normalizer/const agreement fails | **RED**: `no_group_normalizes_to_2d_without_being_listed_as_3d` |
| E — `dropped_chart_groups` → `Vec::new()` | the warn's payload is unobserved | **RED**: `a_combo_parses_to_the_first_group_only_and_is_flagged`, `dropped_chart_groups_names_the_losses_and_is_empty_otherwise` |
| F — series filter renamed so `parse_chart_xml` returns **zero** series | the old `.all(…)` would pass; the new count assertion must not | **RED** at `assert_eq!(chart.series.len(), 1)` (`left: 0, right: 1`) |
| (2) — the `opening_tags` extraction | behaviour-preserving | **GREEN unchanged**: 113 → 113 passing, no test edited |

### Re-verification

- `cargo test -p freecell-chart-model --lib` — 119 passed
- `cargo test -p freecell-engine --lib chart::` — 139 passed
- `cargo test -p freecell-engine --test charts_corpus` — 8 passed
- `cargo clippy -p freecell-chart-model -p freecell-engine --all-targets -- -D warnings` — clean
- `cargo fmt --all --check` — clean

Render scope unchanged: still parse-time classification only, no chart widget touched.
