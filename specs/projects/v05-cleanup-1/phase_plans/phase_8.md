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
