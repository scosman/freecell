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

## Reason strings — deliberately not done

The functional spec asked for a reason naming the dropped groups. `Fidelity` is a bare enum with no
reason field; adding one touches every classifier call site and the badge UI, and **G3 is the unit
that reworks the classifier**. Widening the type here would collide with it for a cosmetic gain, so
G1 stays a detection-only unit — as scoped. The architecture doc anticipated this and made the call
in advance.

I also did not add the `tracing::warn!` the architecture sketched: `parse_chart_xml` is a pure
parse function with no logging today, and a warn there fires once per chart load with no consumer.
The information the user needs is the badge, which they now get.

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
