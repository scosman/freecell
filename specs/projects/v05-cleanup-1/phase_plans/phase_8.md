# Phase 8 — G5: `dLbls` overrides (v0.5) + chart-insert collisions (v1.0)

**Verdict: CONFIRMED (both halves). `dLbls` fixed; the insert collision confirmed and filed, as
scoped.**

## Confirmation — the `dLbls` data loss

Demonstrated by running, before writing any fix. A probe built a `c:dLbls` carrying everything the
model does not know about, edited **one** modelled field (`show_value`), ran the real
`patch_chart_source`, and reported what survived:

```
before: dLbl_survives=false  txPr_survives=false  spPr_survives=false  leader_survives=false
after:  dLbl_survives=true   txPr_survives=true   spPr_survives=true   leader_survives=true
```

So the loss was **worse than the unit described**. It named per-point overrides and label
typography; the whole-node replace also destroyed the label's `c:spPr` fill and
`c:showLeaderLines` — every child outside the model, which is the whole point of a
preserve-unknown save path.

Root cause exactly as diagnosed: `save.rs` called
`upsert_child(…, &["dLbls"], Some(chrome::dlbls_element(c, l)), …)`, and `dlbls_element` builds a
complete element from the model alone. `upsert_child` replaces the existing node's entire byte
range, so everything else went with it.

## The fix

`patch_data_labels`, following the shape `patch_series_color` already established for `c:spPr` —
which is why this was the *one* hole rather than a systemic failure: the pattern existed, `dLbls`
just did not use it.

| Existing `c:dLbls` | Action |
|---|---|
| absent | insert a whole element (nothing to preserve) — unchanged behaviour |
| self-closing `<c:dLbls/>` | replace whole — lossless, it held nothing |
| present with content | **upsert each modelled child inside it**; every other child untouched |

`chrome::dlbls_element` was refactored into `dlbls_children` — `(local name, element)` pairs in
schema order — so the whole-element builder and the in-place patcher share exactly one spelling of
each element. Two spellings that must agree is the failure mode this whole review is about.

### Details that needed deciding, not just coding

> **Superseded in part by the code review (2026-07-28) — see [Code-review round](#code-review-round--2026-07-28) below.** Two of the four bullets here were **wrong**: the `c:delete` rule
> (a falsy `val="0"` was left behind, producing schema-invalid output) and the optional-field rule
> (it deleted values the *model* cannot represent). The "clearing removes the whole node" decision
> was also reversed. The bullets are kept verbatim as the record of what was decided at the time.

- **Schema order.** `CT_DLbls` orders its children `dLbl*`, then `delete | (numFmt, spPr, txPr,
  dLblPos, show*, showBubbleSize, separator, showLeaderLines, leaderLines)`, then `extLst`. Each
  modelled child's insertion anchor is the **suffix of that order after its own name**
  (`dlbls_following`), so a fresh child lands somewhere Excel — a strict reader — will accept.
  `dLbl` is deliberately never in a following set: nothing we insert may land before the per-point
  overrides.
- **`c:delete`.** `<c:delete val="1"/>` means "no labels on this series". Left in place while
  turning a `show*` flag on, Excel honours the delete and the edit silently does nothing. The
  whole-node replace got this right *by accident* (it deleted the node containing it); an in-place
  patch has to do it **on purpose**. Removed when truthy, with a test. The axes' own
  `<c:delete val="0"/>` (axis visibility) must not be touched — the test asserts both survive,
  after a first version of that assertion searched the whole document and caught them.
- **Clearing an optional field.** A cleared `numFmt` / `dLblPos` / `separator` must have its
  element **removed**, not merely not-written — otherwise clearing a label's number format in the
  panel would leave the file's old code in place and be a no-op. Tested.
- **Clearing labels entirely** still removes the whole `c:dLbls`, unknown children and all. An
  empty husk reads as "labels on, with defaults" to Excel — the opposite of the request. Stated in
  a comment so it does not look like an oversight.

## Chart-insert collisions — confirmed, filed, not fixed

Confirmed at HEAD by reading the live path (`worker/run.rs`, **read only** — the parallel
engine-worker-hardening project owns that file). Its own comment documents the limitation: the save
runs `reinject_live_charts` and then `write_authored_charts` on top, and the latter has a fail-loud
precondition that the target sheet carries no `<drawing>` yet — which the re-injected loaded
drawing has just violated.

Worth recording precisely, because the review's framing was slightly broader than the truth: **two
authored charts on one sheet compose fine** (`write_two_authored_charts_on_one_sheet_share_a_drawing`).
It is specifically *loaded + authored on the same sheet* that cannot merge. And it **fails loudly**
— no silent drop, no double `<drawing>`, original untouched by the atomic save.

v1.0 per the remediation doc, so: filed as `GAPS.md` C-G5-1 with the root cause, the fail-loud
severity note, and the shape of the fix (merge into the existing drawing part rather than refuse),
including the `chart_source_path` interaction that makes it non-trivial. **Ships the `dLbls` fix
alone**, as the unit instructed.

## Code-review round — 2026-07-28

A reviewer raised four Moderate findings against the fix above. It **verified and passed** the
highest-risk items — `DLBLS_CHILD_ORDER` matches ECMA-376 `CT_DLbls` verbatim, the splice offsets
are safe under both hazardous orderings, the `dlbls_element`→`dlbls_children` refactor is
byte-identical on the authored path, and `None`-as-truthy for `c:delete` is right
(`CT_Boolean/@val` defaults `true`) — and found **two real correctness bugs** the phase's own tests
could not see. Both are of the same class the unit exists to fix: *the save silently destroys
something the file carried*.

### 1. A falsy `<c:delete val="0"/>` survived and made the output schema-invalid (real bug)

`CT_DLbls` is `dLbl*, (delete | Group_DLbls), extLst?` — `delete` and the settings group are
**mutually exclusive**. `<c:dLbls><c:delete val="0"/></c:dLbls>` is valid input on its own; the
patch turned it into `delete, showLegendKey, showVal, …`, which is not. The original comment called
`val="0"` "the benign explicit-off form" — benign **semantically**, not **structurally**.

Fixed by removing **any** `c:delete` whenever group children are written, whatever its `val` (a
`val="0"` delete is a no-op by definition, so removing it cannot change meaning). That also deleted
the truthiness predicate from the save path entirely — which resolves the reviewer's "two spellings
of one OOXML boolean in one crate" note by **subtraction** rather than by extracting a shared
helper: `load::child_bool` is now the only spelling. `child_bool` is itself wrong about the
`CT_Boolean` default (a valueless `<c:showVal/>` reads false, should be true), but fixing it changes
**loader** behaviour unrelated to this unit, so it is filed as **`GAPS.md` C-G5-2** rather than
churned in here.

### 2. The optional-field rule deleted values the MODEL cannot represent (real bug)

`DataLabelPosition::from_ooxml` returns `None` for `bestFit`/`inEnd`/`outEnd` — and **`bestFit` is
what Excel writes for pie-chart labels**. `load::read_data_labels` likewise maps
`formatCode="General"` to `None`. So "remove an optional child when the new model has `None`" fired
on values the *file* carried and the *model* simply cannot hold: toggling `show_value` on a pie
chart deleted its `<c:dLblPos val="bestFit"/>`.

Not a regression (the pre-G5 whole-node replace lost these too) — but the fix **codified** the loss
in new code, under a comment asserting it was correct, and functional_spec §G5 item 1 names
`c:numFmt` among what must survive.

The missing distinction is *"the user cleared it"* vs *"the model never carried it"*.
`collect_chrome_edits` already held `cached_series.data_labels`, so it is now threaded into
`patch_data_labels`, and the removal set is computed by **diffing `dlbls_children(cached)` against
`dlbls_children(new)`** rather than from a hardcoded `["numFmt", "dLblPos", "separator"]` list. That
kills the reviewer's `present: Vec<&str>` + `contains` note as well, and stays correct if a fourth
optional field is ever modelled. Same mechanism, one step further: an **unchanged** child is not
rewritten at all, so `c:numFmt@sourceLinked="1"` ("follow the source cell's format" — an attribute
the model does not carry) also survives an unrelated edit. Both are tested.

### 3. The tests could not detect a wrong `DLBLS_CHILD_ORDER` — the highest-risk decision was the untested one

Every case asserted `roxmltree::…parse(…).is_ok()` (well-formedness) plus a model round-trip, and
`parse_chart_xml` is **order-agnostic**. Had `DLBLS_CHILD_ORDER` put `showVal` before `dLblPos`, all
six tests would have passed and Excel would have got invalid XML. Worse, the `rich_dlbls()` fixture
was **itself schema-invalid** (`c:numFmt` after `c:spPr`/`c:txPr`), so the headline test's output was
invalid-ordered and nothing noticed.

- `rich_dlbls()` is now in schema order (and asserts it on construction).
- New `assert_dlbls_schema_order` extracts the **series'** `c:dLbls` child local-names and asserts
  they are `dLbl*` followed by either a lone `c:delete` or a **subsequence** of
  `DLBLS_CHILD_ORDER`. Asserted in **every** `dLbls` test.
- Two shapes that actually exercise **insertion** were added — the committed suite only covered
  *replacement*, since every modelled child already existed in `rich_dlbls()`: an empty-but-closed
  `<c:dLbls></c:dLbls>` (all eight children inserted at one anchor) and a `c:dLbls` holding only
  unmodelled children (`txPr` + `extLst`, so fresh children must straddle them).
- `dlbls_children_follow_the_documented_schema_order` asserts the **two** spellings of the order —
  `chrome::dlbls_children` (build order) and `save::DLBLS_CHILD_ORDER` (insert-anchor order) — agree.
  Two spellings that must agree is the failure mode this review is about; `DLBLS_FLAGS` also went
  back to private (it was `pub(super)` with no cross-module user).

**Mutation-verified:** swapping `showVal` before `dLblPos` in `DLBLS_CHILD_ORDER` now fails 4 tests
(the three insertion-shape cases + the two-spellings test) and, as expected, still passes the
replacement-only ones — which is exactly why the insertion shapes had to be added.

### 4. "Clearing removes the whole node" was silently undone by a group-level `c:dLbls`

`load.rs` resolves a **chart-group-level** `c:dLbls` as the default for every series without its
own. So on a file that has one, "turn all labels off" (the panel sends `data_labels = None` —
`worker/run.rs`'s `labels.is_shown().then_some(labels)`) deleted the series' element and the series
**re-inherited the group default**: the user's action silently did nothing (reviewer-probed: labels
returned as `show_value: true, show_category_name: true`).

**Route taken: the OOXML-correct one** — a clear now writes `<c:dLbls><c:delete val="1"/></c:dLbls>`
instead of removing the node. It is correct **with and without** a group default, it is exactly the
construct this phase had just learned to read, and it needs no new machinery. It also handles the
case the old code ignored entirely: a series with **no** `c:dLbls` of its own (pure inheritance) now
gets one **inserted**, where previously the clear was a literal no-op. Two consequences, both
recorded rather than glossed:

- the node's unknown children go with it (`CT_DLbls` forbids the settings-group ones beside a
  `delete`) — **`GAPS.md` C-G5-3**;
- the round-trip is asymmetric by design: a cleared `None` re-opens as `Some(all-off)`, since the
  loader does not read `c:delete`. `is_shown()` is false either way, so nothing renders differently.

The doc comment no longer claims the old behaviour "is the right trade and is not an oversight".

### Spec sections this contradicts (dated annotation, 2026-07-28)

`architecture.md` §8 describes behaviour the review reversed. It is **not** rewritten — the section
is `status: complete`, so the original text stays and each contradiction carries a dated
supersession note beside it. **Applied 2026-07-28** (second review round: deferring them to "when
the spec is next touched" left nothing forcing it, and §8 is this unit's own doc). The three notes
say:

- **`architecture.md` §8 "G5 — `dLbls` per-point overrides" → "Setting to `None` (clearing
  labels)"** — says *"clearing still removes the node, and this is stated in a comment so it does
  not read as an oversight."* **Superseded 2026-07-28:** removing the node lets a series with a
  chart-**group**-level `c:dLbls` re-inherit that default, so the clear silently does nothing. The
  save now writes `<c:dLbls><c:delete val="1"/></c:dLbls>`. (The section's premise — that a `c:dLbls`
  *husk* would read as "labels on, with defaults" — is still right; `c:delete` is not a husk.)
- **`architecture.md` §8 → "`c:delete`"** — says the patcher *"removes a **truthy** `c:delete` when
  it sets any `show*` flag on."* **Superseded 2026-07-28:** it removes **any** `c:delete` whenever
  it writes group children. `CT_DLbls` is `dLbl*, (delete | Group_DLbls), extLst?`, so a leftover
  falsy `<c:delete val="0"/>` beside the `show*` flags is schema-invalid, and a `val="0"` delete is
  a no-op so removing it is free.
- **`architecture.md` §8 → "Tests"** item 6 (*"clearing labels removes the node"*) changes to
  *"clearing labels writes an explicit `c:delete`, overriding any group-level default"*, and the
  test list gains the schema-order assertion + the two insertion shapes (§3 above).
- **`functional_spec.md` §G5** needs no change — item 1 explicitly names `c:numFmt` among what must
  survive, which is precisely what finding 2 restores.

### Tests added / changed

`patch_data_labels_keeps_values_the_model_cannot_represent` (bestFit + `General`),
`patch_data_labels_keeps_source_linked_on_an_unchanged_numfmt`,
`patch_data_labels_into_an_empty_closed_dlbls`,
`patch_data_labels_into_a_dlbls_of_only_unmodelled_children`,
`patch_data_labels_clearing_overrides_a_group_level_default`,
`dlbls_children_follow_the_documented_schema_order`;
`patch_data_labels_removes_a_delete_when_turning_labels_on` now loops `val="1"`/`val="0"`;
`patch_data_labels_clearing_removes_the_element` → `…_clearing_writes_an_explicit_delete`.

### Review verification (2026-07-28)

- `cargo test -p freecell-engine --lib chart::` — **137 passed**
- `cargo test -p freecell-engine --lib` — 404 passed; `--test worker_seam` — 58 passed
- `cargo clippy -p freecell-engine --all-targets` — clean; `cargo fmt --all --check` — clean
- Files touched: `chart/save.rs`, `chart/chrome.rs`, `GAPS.md`, this file. No render impact (chart
  **save**, not the chart widgets).

## Second review round — 2026-07-28

The re-review cleared nearly everything and left one finding standing plus a new Moderate.

### 1. `assert_dlbls_schema_order` had no external anchor (the finding that stayed open)

The order assertion added in round 1 validates patched output against **`DLBLS_CHILD_ORDER`
itself** — the very constant it is supposed to be testing. So a wrong constant is *self-consistent*
and invisible. The reviewer demonstrated it: swap the adjacent `separator` / `showLeaderLines` (a
genuinely invalid `CT_DLbls` order, and a realistic typo — only the first of the two is modelled)
and **all 137 tests pass**, while an upsert into a `c:dLbls` that carries `showLeaderLines` emits
`… showPercent, showLeaderLines, separator` — exactly the invalid XML the assertion exists to
catch. Round 1's mutation check (`showVal` before `dLblPos`) only failed because two fixtures
happen to spell the true order as string literals; it proved less than it looked.

Fixed by pinning the constant to a **hand-written literal transcribed from the schema**, with the
citation, in `dlbls_child_order_matches_ct_dlbls`. That is the one assertion no other test can
derive from the code under test.

**Mutation-verified:** swapping `separator`/`showLeaderLines` in `DLBLS_CHILD_ORDER` now fails
`dlbls_child_order_matches_ct_dlbls` — and, as the reviewer said, *only* that test (137 pass, 1
fails), which is precisely why the anchor had to exist. Restored, 138 pass. Re-running round 1's
`showVal`-before-`dLblPos` mutation against the reworked helper fails 13 tests.

Two weaknesses in the same helper, fixed with it:

- it inspected only the **first** series and silently returned when that series had no `c:dLbls`,
  so a case editing a later series went unchecked. It now walks **every** series
  (`dlbls_child_names_by_series`) and asserts at least one carries a `c:dLbls`, so a vacuous call
  fails instead of passing. `series_dlbls_child_names` remains as a thin first-series view for the
  two cases that assert on that series specifically.
- under a mutated constant its failure message claimed *"Excel would reject this part"* about
  output that was correctly ordered — schema authority the assertion does not have. It now says
  what it checks: **agreement with `DLBLS_CHILD_ORDER`**, naming the test that anchors the
  constant to ECMA-376.

### 2. `architecture.md` §8 still stated the reversed behaviour (new Moderate)

Round 1 recorded the three contradictions but deferred writing them "when the spec is next
touched" — which nothing forced, and the scoping reason did not hold (§8 is this unit's own doc,
and the annotation is a smaller diff than the section that recorded the intent). **Applied**: the
three dated supersession notes now sit in `architecture.md` §8 beside the text they supersede,
annotating rather than rewriting.

### 3. `GAPS.md` sharpened (two Mild)

- **C-G5-2** gained the save-side half. The unchanged-child skip introduced in this phase
  **persists** the `CT_Boolean` misread: before, every modelled child was regenerated on each save,
  so a `<c:showVal/>` file converged to `val="0"`; now an unrelated toggle leaves it byte-identical
  and the file keeps showing values in Excel while FreeCell shows them off. A read-only bug became
  a persisted file/UI divergence — the real argument for fixing `child_bool`.
- **C-G5-3** understated twice. Its trigger is not only a deliberate turn-off: the worker sends
  `None` for *any* all-off toggle set and the call is gated only on `series.data_labels !=
  cached_series.data_labels`, so an **already**-all-off `c:dLbls` carrying a per-point `c:dLbl`,
  `c:txPr` and `c:extLst` is replaced by a bare `c:delete` on a semantically **no-op** edit. And
  "styling that no longer renders anything" was wrong — a per-point `c:dLbl` can carry
  `c:tx`/`c:rich`, i.e. **user-authored label text**, so this can destroy content. The cheap guard
  the reviewer floated (skip the clear when `cached.is_shown()` is already false) is now recorded
  **with a decision**: not taken here, because it drops today's normalization of an already-off
  `c:dLbls` to the canonical `c:delete` (re-premising a committed test) and makes the C-G5-2
  misread load-bearing on the **save** path. Order: fix `child_bool`, then add the guard.

### Second-round verification (2026-07-28)

- `cargo test -p freecell-engine --lib chart::` — **138 passed** (137 + the new schema anchor)
- `cargo clippy --locked -p freecell-engine --all-targets -- -D warnings` — clean;
  `cargo fmt --all --check` — clean
- Files touched: `chart/save.rs` (tests only — no production behaviour change), `GAPS.md`,
  `architecture.md`, this file. No render impact.

## Verification

- `cargo test -p freecell-engine --lib chart::` — 131 passed
- `cargo test -p freecell-engine` — 398 lib + 8 integration passed

**Two pre-existing failures, not caused by this work:** `charts_roundtrip_libreoffice`'s two cases
fail in this container because headless `soffice` produces no converted file. Verified by
`git stash`-ing every change in this phase and re-running — **identical failures**. That suite is
gated to the dedicated `roundtrip` workflow, not `checks`, and one of the two failing cases
exercises the authored-write path this phase never touched.

- `cargo fmt --all --check` clean

No render impact: this is chart **save** (byte-splicing on the way out), not the chart widgets.
