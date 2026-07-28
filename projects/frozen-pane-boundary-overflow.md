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

— and inserting *empty* rows does not grow `max_row`, so an empty (or lightly populated) sheet
accepts an arbitrarily large `row_count`.

Reproduced: freeze 1 row, then three × `InsertRows { row: 0, count: 1_000_000 }` leaves
`frozen_rows = 3_000_001` on a sheet whose last row is 1,048,576. It survives save → reopen, so
the writer emits a structurally invalid `<pane ySplit="3000001">` — a count that is not a row of
the sheet it describes. Both axes are affected (`frozen_columns += column_count` at `:725`).

## Why it is reachable by gesture, not just by API

The insert count is not a constant the app chooses — the header menu derives it from the
**selected header run**, exactly like the freeze count that caused B2
(`specs/projects/engine-worker-hardening/functional_spec.md` §0). Select a large block of row
headers, choose Insert, repeat. This is the same untrusted-selection pattern, one command over.

## Where the fix belongs: the fork, not FreeCell

Per [`CLAUDE.md`](../CLAUDE.md) ("we ride our IronCalc fork — fix upstream, don't hack
FreeCell"), this is an engine defect and gets an engine fix:

- Its **own** `fix/<slug>` branch off the fork's `main` (e.g. `fix/clamp-frozen-pane-boundary`),
  with upstream-style tests, as one focused single-feature PR. It must **not** be folded into
  `fix/structural-edits-adjust-frozen-pane` (already upstreamed) or into any FreeCell phase.
- The change itself is small: clamp the adjusted boundary to the sheet dimension at all four
  adjust sites — `actions.rs:725` / `:1051` (insert, both axes) and, for symmetry, the
  `:857` / `:1131` decrements. `LAST_ROW` / `LAST_COLUMN` are already in scope there.
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
  `0..=LAST_ROW` / `0..=LAST_COLUMN`, with a test that inserts far more tracks than the sheet has.
- FreeCell: re-pin `freecell-fixes`, then extend
  `structural_edit_past_the_cap_diverges_model_from_the_clamped_cache` (`worker/run.rs`) with the
  million-row insert, and tighten `engine-worker-hardening/functional_spec.md` §F1.3's statement
  of the cost — the saved count becomes bounded-but-over-cap instead of unbounded.
