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
`row_count <= LAST_ROW - max_row`; when the insert lands **within or above** the band — the guard
is `row <= frozen_rows`, so anywhere in 1-based rows `1..=frozen`, which is the case that grows it
— the boundary becomes `frozen + row_count`, whose largest permitted value is therefore
`frozen + LAST_ROW - max_row`. So the counts that overflow in a single insert are exactly

```
[ LAST_ROW - frozen + 1 , LAST_ROW - max_row ]
```

and that interval is **non-empty iff `frozen > max_row`**. Note the shape: the lower bound is
fixed by `frozen` alone and does not move, while the upper bound falls as the sheet fills. The
interval therefore *narrows from the top* as `max_row` grows and **closes** at `max_row >= frozen`
— it does not slide.

Two consequences follow, with different preconditions:

- **Overflow takes one insert — on a sheet with nothing at or below row `frozen`.** Measured
  through the worker's command path, `frozen = 64` (FreeCell's cap), `LAST_ROW` = 1,048,576:

  | Sheet's `max_row` | Largest legal count | Command | `frozen_rows` after |
  |---|---|---|---|
  | 1 (empty) | 1,048,575 | `InsertRows { row: 0, count: 1_048_575 }` | **1,048,639** — over |
  | 1 (empty) | — | `count: 1_048_513` (interval's lower bound) | **1,048,577** — first overflowing count |
  | 1 (empty) | — | `count: 1_048_512` (one below it) | 1,048,576 — exactly the last row, not over |
  | 1 (empty) | — | `count: 1_048_576` (Select-All) | 64 — **rejected** |
  | 63 | 1,048,513 | `count: 1_048_513` | **1,048,577** — over; interval is this single count |
  | 63 | — | `count: 1_048_514` | 64 — **rejected** |
  | 64 | 1,048,512 | `count: 1_048_512` (the largest legal) | 1,048,576 — exactly the last row, **not over** |
  | 100 | 1,048,476 | `count: 1_048_476` (the largest legal) | 1,048,540 — not over |
  | 1,000 | 1,047,576 | `count: 1_047_576` (the largest legal) | 1,047,640 — not over |

  So with the 64-row cap the one-gesture window is **1,048,513 ..= 1,048,575** on an empty sheet,
  shrinks to the single count 1,048,513 once the sheet reaches row 63, and **closes entirely at
  row 64**. A workbook holding any value at row 64 or deeper cannot be overflowed in one gesture
  at all — which is most real documents, and is the honest severity statement. (A larger `frozen`
  widens the window again, and a crafted `<pane>` can supply one: the sheet-cache clamp bounds what
  FreeCell *renders*, not what the increment reads, which is the model's own `frozen_rows`. That
  path needs a hostile file, so it is not the ordinary-gesture case this note is scoped to.)

  The `frozen = 1` case is the floor: its largest legal insert lands on exactly `LAST_ROW`, so a
  single-row freeze can never overflow. And `count = LAST_ROW` (Select-All) is the one size the
  dimension check catches on an empty sheet — `dimension()` returns `max_row = 1` even when empty
  (`worksheet.rs:721-730`), so `1 + LAST_ROW > LAST_ROW`.

- **Unbounded accumulation takes repetition, and does *not* require an empty sheet.** What it
  requires is no populated cell **at or below the insertion row**: `insert_rows` shifts only cells
  at `r >= row`, so data strictly above the insertion point never moves and `max_row` never grows,
  leaving the headroom fixed forever. Measured: a value at row 10, freeze 64, then repeated
  `InsertRows { row: 19, count: 1_000_000 }` (1-based row 20, still inside the band) gives
  `frozen_rows` 1,000,064 → 2,000,064 → 3,000,064 → 4,000,064 with `dimension().max_row` pinned at
  10 throughout. An empty sheet is just the easiest instance of that condition, not the condition.

Reproduced for the second, and checked end to end: on an empty sheet, freeze 1 row, then three ×
`InsertRows { row: 0, count: 1_000_000 }` leaves `frozen_rows = 3_000_001` where the last row is
1,048,576. Saving succeeds and the writer emits the count verbatim —
`<pane ySplit="3000001" topLeftCell="A3000002" activePane="bottomLeft" state="frozen"/>`, so the
derived `topLeftCell` is out of range too — and reopening restores the model to 3,000,001 (the
cache clamps to 64). Both axes are affected (`frozen_columns += column_count` at `:725`).

**Keep the two apart when reading the rest of this note.** The title, the defect statement and the
acceptance criterion are all about **overflow** (boundary > `LAST_ROW`), which is the one-insert
case on a near-empty sheet. Unboundedness is the same bug taken further; it needs repetition but
tolerates a populated sheet, and it is what makes the divergence in
`engine-worker-hardening/functional_spec.md` §F1.3 unbounded rather than merely large.

## How it is reached: by gesture, not by the setter API

Three paths write `frozen_rows`, and they are not equally guarded:

| Path | Range-checked? |
|---|---|
| `Model::set_frozen_rows` (`base/src/model.rs:3426-3439`) — what `UserModel::set_frozen_rows_count`, and hence FreeCell's `SetFrozen`, calls | **Yes** — rejects `< 0` and `>= LAST_ROW` |
| The insert increment (`actions.rs:1051` / `:725`) | **No** — this defect |
| The xlsx reader (`xlsx/src/import/worksheets.rs:691`) — `frozen_rows = get_number(pane, "ySplit")` | **No** — taken verbatim |

So the direct setter is *not* a way in; the section heading is about the two that are. The insert
count is not a constant the app chooses — the header menu derives it from the **selected header
run** (`grid/view.rs`, `header_menu_items`: `count = run.1 - run.0 + 1`, sent as
`insert_ev(start, count)`), exactly like the freeze count that caused B2
(`specs/projects/engine-worker-hardening/functional_spec.md` §0). Same untrusted-selection
pattern, one command over.

Combined with the interval above, **overflow** is a two-gesture sequence with nothing crafted in
it *provided the sheet is near-empty*: on a sheet with nothing at or below row 64, right-click a
row header → "Freeze 64 rows" (FreeCell's cap, the most it will ever grant), then select a header
run of 1,048,513–1,048,575 rows — Select-All minus a row or two — and choose "Insert". One Insert
and the boundary is past the last row. On that sheet Select-All exactly is the only run size that
bounces; on a sheet with data at row 64 or deeper, *every* run size bounces or lands in range.

**Unboundedness** is the one that needs repetition, and it survives a populated sheet. Only the
run's **start** has to sit inside the band — the increment's guard is `row <= frozen_rows`, which
reads the insertion row, not the run's extent — so a run beginning below the data but above the
band boundary, and running as far down the sheet as you like, qualifies. Repeat Insert: each call
passes the dimension check, `max_row` never moves, and `frozen_rows` accumulates without limit.

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

  The claim that holds is the narrower one, and it needs its scope said out loud: **for any count
  this engine can itself produce once the increments are clamped**, `frozen <= LAST_ROW` is an
  invariant, and since a decrement always lands strictly below its input the result stays in
  dimension. A clamp on the decrement result is then dead code, and including one "for symmetry"
  would ask a reviewer to accept dead code — the kind of thing that gets an upstream PR sent back.

  Not *unconditionally*, though, and the difference is load-bearing: the **xlsx reader** takes
  `ySplit` verbatim (`import/worksheets.rs:691`, no range check), so a workbook written by a
  pre-fix build reopens with `frozen > LAST_ROW` even after the increments are clamped, and a
  delete on it lands at 1,951,425 — still out of dimension. Any upstream reviewer will land on
  exactly this, so the PR should name it: `Model::set_frozen_rows` guards `>= LAST_ROW`, the
  increments will guard after this fix, and the reader is then the last unguarded entry point.
  Whether it should range-check (clamp, or reject the file) is a separate question and a separate
  PR — it is a file-compatibility decision, not an arithmetic one.

  The decrements are worth *mentioning* in the PR only as the reason a pre-existing overflowed
  count is not permanent: a delete inside the band reduces it by at most one sheet height per call
  (measured: 3,000,001 → 1,951,425 → 902,849 → 0 over three whole-axis deletes). It is not the
  *only* way down — `SetFrozen` to any legal count, Unfreeze included, clears it in one step, and
  that path is guarded — but it is the only *structural* one.
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
carries the invalid count (and an out-of-range `topLeftCell` derived from it) out to other
applications, and §F1.3's accepted model/cache divergence becomes unbounded rather than merely
over-cap. Neither of those two consequences is a FreeCell-side hang or crash — the cache clamp
bounds every loop regardless — which is why this is backlog and not a phase.

## Acceptance

- Fork: an insert into or above a frozen band can never leave the boundary outside `0..=LAST_ROW`
  / `0..=LAST_COLUMN`. The primary regression test should be the **single** insert, because it is
  the simplest thing that reaches the increment: on an **empty** sheet, `frozen = 2` (any
  `frozen >= 2` works there) plus one `insert_rows(sheet, 1, LAST_ROW - 1)` overflows by exactly
  one, and the clamp is what stops it. Two ways such a test can pass without testing anything, both
  worth a comment in it: `row_count = LAST_ROW` is rejected by the dimension check before the
  boundary adjustment runs, and any sheet populated at or below row `frozen` closes the overflow
  window entirely. A second test repeating a mid-sized insert covers the unbounded-accumulation
  case.

  Note what this criterion does **not** claim: it is about what the *increment* can produce. A file
  whose `<pane>` already carries an out-of-range `ySplit` still loads with one, because the reader
  is unguarded (above).
- FreeCell: re-pin `freecell-fixes`, then extend
  `structural_edit_past_the_cap_diverges_model_from_the_clamped_cache` (`worker/run.rs`) with the
  single oversized insert, and tighten `engine-worker-hardening/functional_spec.md` §F1.3 — the
  saved count becomes bounded-but-over-cap instead of unbounded, so its "How the divergence ends"
  paragraph collapses to the self-healing case.
