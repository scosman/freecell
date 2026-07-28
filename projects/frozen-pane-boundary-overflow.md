# Frozen-Pane Boundary Overflow on Structural Edits (IronCalc fork fix)

**Status: Future** (found during `engine-worker-hardening` Phase 1 code review, 2026-07-28).

## The defect

IronCalc adjusts a sheet's frozen boundary inside a structural edit's own diff — the behaviour
FreeCell relies on for "insert a row above the band and the band follows, in one undo step". The
adjustment has **no upper bound**:

```rust
// base/src/actions.rs:1051 (insert_rows), and the column twin at :725
if row <= worksheet.frozen_rows {
    worksheet.frozen_rows += row_count;
}
```

Nothing checks the result against the sheet's own dimension. The obvious guard that *looks* like
it would catch this does not: `insert_rows`' range check is against the **populated** dimension —

```rust
let last_row = self.workbook.worksheet(sheet)?.dimension().max_row + row_count;
if last_row > LAST_ROW { return Err(…); }
```

The check bounds the **insert**, not the **boundary**. It permits any
`row_count <= LAST_ROW - max_row`; when the insert is at or above the band's top — the case that
grows it — the boundary then becomes `frozen + row_count`, whose largest permitted value is
`frozen + LAST_ROW - max_row`. That clears `LAST_ROW` as soon as `frozen > max_row`, i.e. from
`frozen >= 2` on an empty sheet. Two consequences, and they are different:

- **Overflow takes one insert.** Measured through the worker's command path on an empty sheet:

  | Start | Command | `frozen_rows` after |
  |---|---|---|
  | freeze 64 (legal, at the cap) | `InsertRows { row: 0, count: 1_048_575 }` | **1,048,639** — past `LAST_ROW` = 1,048,576 |
  | freeze 64 | `InsertRows { row: 0, count: 1_048_512 }` | 1,048,576 — exactly the last row, not yet over |
  | freeze 64 | `InsertRows { row: 0, count: 1_048_513 }` | 1,048,577 — the first overflowing count |
  | freeze 64 | `InsertRows { row: 0, count: 1_048_576 }` (Select-All) | 64 — **rejected** by the dimension check |
  | freeze 1 | `InsertRows { row: 0, count: 1_048_575 }` (the largest legal insert) | 1,048,576 — cannot overflow from `frozen = 1` |

  So on an empty sheet, with FreeCell's own cap of 64 frozen rows, every insert count in
  **1,048,513 ..= 1,048,575** overflows in a single gesture. (On a populated sheet the interval
  just slides down with `max_row`; it does not close.) The top end is `LAST_ROW - 1` rather than
  `LAST_ROW` because `count = LAST_ROW` — Select-All — is the one size the dimension check does
  catch: `dimension()` returns `max_row = 1` even on an empty sheet (`worksheet.rs:721-730`), so
  `1 + LAST_ROW > LAST_ROW`. That narrow fact is *all* the check buys; it does not make the
  overflow hard to reach.

- **Unbounded accumulation takes repetition,** and a sheet with no populated cells below the
  insertion point. `insert_rows` shifts populated cells down, so on a sheet with data `max_row`
  grows with each insert and the check eventually bites. On an **empty** sheet `sheet_data` stays
  empty, `max_row` stays 1, and the headroom never shrinks — so the inserts can be repeated
  forever while `frozen_rows` accumulates without limit. (Growing the band requires inserting
  at/above its top, which is above any data, so "insert below the data instead" is not an escape.)

Reproduced for the second: on an empty sheet, freeze 1 row, then three ×
`InsertRows { row: 0, count: 1_000_000 }` leaves `frozen_rows = 3_000_001` where the last row is
1,048,576. Either way the state survives save → reopen, so the writer emits a structurally invalid
`<pane ySplit>` — a count that is not a row of the sheet it describes. Both axes are affected
(`frozen_columns += column_count` at `:725`).

**Keep the two apart when reading the rest of this note.** The title, the defect statement and the
acceptance criterion are all about **overflow** (boundary > `LAST_ROW`), which is the one-gesture
case. Unboundedness is the same bug taken further, and is what makes the divergence in
`engine-worker-hardening/functional_spec.md` §F1.3 unbounded rather than merely large.

## Why it is reachable by gesture, not just by API

The insert count is not a constant the app chooses — the header menu derives it from the
**selected header run** (`grid/view.rs`, `header_menu_items`: `count = run.1 - run.0 + 1`, sent as
`insert_ev(start, count)`), exactly like the freeze count that caused B2
(`specs/projects/engine-worker-hardening/functional_spec.md` §0). This is the same
untrusted-selection pattern, one command over.

Combined with the interval above, that makes **overflow** a two-gesture sequence with nothing
crafted in it: right-click a row header → "Freeze 64 rows" (FreeCell's own cap, so this is the
*most* it will ever grant), then select a header run of 1,048,513–1,048,575 rows — Select-All
minus a row or two — and choose "Insert". One Insert, and the boundary is past the last row of the
sheet. Select-All exactly is the only run size that bounces.

**Unboundedness** is the one that needs repetition: pick a smaller run (say 500,000 headers) on an
**empty** sheet and repeat Insert — each call passes the dimension check, and `frozen_rows`
accumulates across them without limit.

## Where the fix belongs: the fork, not FreeCell

Per [`CLAUDE.md`](../CLAUDE.md) ("we ride our IronCalc fork — fix upstream, don't hack
FreeCell"), this is an engine defect and gets an engine fix:

- Its **own** `fix/<slug>` branch off the fork's `main` (e.g. `fix/clamp-frozen-pane-boundary`),
  with upstream-style tests, as one focused single-feature PR. It must **not** be folded into
  `fix/structural-edits-adjust-frozen-pane` (already upstreamed) or into any FreeCell phase.
- The change itself is small, and it is **only the two increment sites** — `actions.rs:1051`
  (rows) and `:725` (columns): clamp the adjusted boundary to `LAST_ROW` / `LAST_COLUMN`, both
  already in scope there.

  The two **decrement** sites (`:857`, `:1131`) need nothing — but the reason has to be stated
  carefully, because the obvious form of it is not quite true. Under their `row <= frozen` guard,
  with `row >= 1` and `row_count >= 1` already validated, `deleted_in_band =
  min(last_deleted, frozen) - row + 1` lies in `1..=frozen`, so the result lies in
  `0..=frozen - 1`: the subtraction cannot underflow, and it strictly decreases the count. That is
  **not** the same as "the result is in dimension" — if `frozen` is already overflowed the result
  usually still is, which is exactly what `3,000,001 → 1,951,425` below shows.

  The claim that holds is the narrower one, and it is the one that matters here: **once the
  increment sites are clamped, `frozen <= LAST_ROW` is an invariant**, and since a decrement only
  ever lands strictly below its input, the result is `< LAST_ROW` unconditionally. A clamp on the
  decrement result is then provably a no-op. Including one "for symmetry" would ask a reviewer to
  accept dead code, which is the kind of thing that gets an upstream PR sent back.

  The decrements are worth *mentioning* in the PR only as the reason a pre-existing overflowed
  count is not permanent: a delete inside the band is the one operation that reduces it, by at
  most one sheet height per call (measured: 3,000,001 → 1,951,425 → 902,849 → 0 over three
  whole-axis deletes).
- Worth raising with it: whether `insert_rows` should range-check the **requested shift** rather
  than only the populated dimension. That is a larger behavioural question and probably a second
  PR, not a rider on this one.

## Why FreeCell does not need a workaround in the meantime

FreeCell is already bounded against this on the paths that matter:

- The frozen counts every consumer reads come from the sheet cache, and
  `freecell_engine::cache::build_sheet_cache` clamps to `MAX_FROZEN_ROWS` / `MAX_FROZEN_COLS`
  (B2, `engine-worker-hardening/functional_spec.md` §F1.3). The publish loop and the grid's band
  renderer are therefore bounded no matter what the model holds.
- A file written in this state reopens through the same clamp, so FreeCell never renders the
  overflowed band.

What is **not** covered, and is the reason this is tracked rather than dropped: the saved file
carries the invalid count out to other applications, and §F1.3's accepted model/cache divergence
becomes unbounded rather than merely over-cap. Neither is a FreeCell-side hang or crash, which is
why this is backlog and not a phase.

## Acceptance

- Fork: inserting rows/columns into or above a frozen band can never leave the boundary outside
  `0..=LAST_ROW` / `0..=LAST_COLUMN`. The primary regression test should be the **single** insert,
  because it is the simplest thing that reaches the increment: `frozen = 2` (any `frozen >= 2`
  works) plus one `insert_rows(sheet, 1, LAST_ROW - 1)` overflows by one, and the clamp is what
  stops it. Note the one size that does **not** work: `row_count = LAST_ROW` is rejected by the
  dimension check before the boundary adjustment runs, so a test written that way would pass
  without exercising the fix at all. A second test repeating a mid-sized insert covers the
  unbounded-accumulation case.
- FreeCell: re-pin `freecell-fixes`, then extend
  `structural_edit_past_the_cap_diverges_model_from_the_clamped_cache` (`worker/run.rs`) with the
  single oversized insert, and tighten `engine-worker-hardening/functional_spec.md` §F1.3 — the
  saved count becomes bounded-but-over-cap instead of unbounded, so its "How the divergence ends"
  paragraph collapses to the self-healing case.
