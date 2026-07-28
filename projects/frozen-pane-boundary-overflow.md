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

Two things follow from that, and they are different:

- **A single whole-axis insert is always rejected.** `dimension()` returns `max_row = 1` even for
  an empty sheet (`worksheet.rs:721-730`), so `InsertRows { count: LAST_ROW }` fails everywhere.
  The overflow is not reachable in one gesture.
- **Repeated inserts are unbounded on a sheet with no populated cells below the insertion
  point.** `insert_rows` shifts populated cells down, so on a sheet with data `max_row` grows with
  each insert and the check eventually bites. On an **empty** sheet `sheet_data` stays empty,
  `max_row` stays 1, and the headroom never shrinks — so the inserts can be repeated forever while
  `frozen_rows` accumulates. (Growing the band requires inserting at/above its top, which is above
  any data, so "insert below the data instead" is not an escape.)

Reproduced: on an empty sheet, freeze 1 row, then three × `InsertRows { row: 0, count: 1_000_000 }`
leaves `frozen_rows = 3_000_001` where the last row is 1,048,576. It survives save → reopen, so
the writer emits a structurally invalid `<pane ySplit="3000001">` — a count that is not a row of
the sheet it describes. Both axes are affected (`frozen_columns += column_count` at `:725`).

## Why it is reachable by gesture, not just by API

The insert count is not a constant the app chooses — the header menu derives it from the
**selected header run** (`grid/view.rs`, `header_menu_items`: `count = run.1 - run.0 + 1`),
exactly like the freeze count that caused B2
(`specs/projects/engine-worker-hardening/functional_spec.md` §0). This is the same
untrusted-selection pattern, one command over.

It is not a *one-gesture* exploit, though, and the note should not imply otherwise: Select-All →
Insert asks for 1,048,576 rows and the dimension check rejects it. What works is selecting a large
sub-run (say 500,000 headers) on an **empty** sheet and repeating Insert — each call passes the
check, and `frozen_rows` accumulates across them. So: a handful of ordinary gestures on a blank
sheet, not one.

## Where the fix belongs: the fork, not FreeCell

Per [`CLAUDE.md`](../CLAUDE.md) ("we ride our IronCalc fork — fix upstream, don't hack
FreeCell"), this is an engine defect and gets an engine fix:

- Its **own** `fix/<slug>` branch off the fork's `main` (e.g. `fix/clamp-frozen-pane-boundary`),
  with upstream-style tests, as one focused single-feature PR. It must **not** be folded into
  `fix/structural-edits-adjust-frozen-pane` (already upstreamed) or into any FreeCell phase.
- The change itself is small, and it is **only the two increment sites** — `actions.rs:1051`
  (rows) and `:725` (columns): clamp the adjusted boundary to `LAST_ROW` / `LAST_COLUMN`, both
  already in scope there.

  The two **decrement** sites (`:857`, `:1131`) need nothing. Under their `row <= frozen` guard,
  with `row >= 1` and `row_count >= 1` already validated, `deleted_in_band =
  min(last_deleted, frozen) - row + 1` is provably in `1..=frozen`, so the subtraction can neither
  underflow nor leave the result outside the dimension. Including them "for symmetry" would ask a
  reviewer to accept a no-op, which is exactly the kind of thing that gets an upstream PR sent
  back. They are worth *mentioning* in the PR only as the reason an overflowed count is not
  permanent: a delete inside the band is the one operation that reduces it, at up to `LAST_ROW`
  per call (measured: 3,000,001 → 1,951,425 → 902,849 → 0 over three whole-axis deletes).
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
  `0..=LAST_ROW` / `0..=LAST_COLUMN`. The test has to be the **repeated** insert on an empty sheet
  (a single oversized insert is rejected by the dimension check before it reaches the boundary
  adjustment, so it would pass for the wrong reason).
- FreeCell: re-pin `freecell-fixes`, then extend
  `structural_edit_past_the_cap_diverges_model_from_the_clamped_cache` (`worker/run.rs`) with the
  repeated million-row insert, and tighten `engine-worker-hardening/functional_spec.md` §F1.3 —
  the saved count becomes bounded-but-over-cap instead of unbounded, so its "How the divergence
  ends" paragraph collapses to the self-healing case.
