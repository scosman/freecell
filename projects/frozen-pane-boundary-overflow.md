# Frozen-Pane Boundary Overflow on Structural Edits (IronCalc fork fix)

**Status: Future** (found during `engine-worker-hardening` Phase 1 code review, 2026-07-28).

> **How to read this note.** It exists so someone can decide whether to pick this up and know
> where to start — not to specify the bug's exact reachability envelope. The §Observations below
> are **measurements taken at one pinned commit under stated conditions**, not laws; six review
> rounds on this file were each spent on a measurement that had been restated one notch broader
> than it holds. Treat §What holds as the durable part, and **re-derive the envelope from the
> source when the fix is picked up** — the arithmetic is sensitive to `LAST_ROW`, to FreeCell's
> frozen cap, and to how `dimension()` is computed, any of which may have moved.

## What holds

These are the claims this note is willing to stand behind:

1. **Two increment sites are unguarded.** `insert_rows` (`base/src/actions.rs:1051`) and
   `insert_columns` (`:725`) grow the frozen boundary with no upper bound:

   ```rust
   if row <= worksheet.frozen_rows {
       worksheet.frozen_rows += row_count;
   }
   ```

2. **The range check nearby does not cover it.** `insert_rows` checks
   `dimension().max_row + row_count > LAST_ROW`, which bounds the **insert**, not the resulting
   **boundary**. Nothing anywhere compares `frozen_rows` to the sheet dimension after the add.

3. **The result can leave the sheet, and it round-trips.** A boundary past `LAST_ROW` is written
   to xlsx verbatim and read back verbatim, so it survives save → reopen and travels to other
   applications.

4. **FreeCell itself stays bounded.** Every consumer reads the sheet cache, and
   `freecell_engine::cache::build_sheet_cache` clamps to `MAX_FROZEN_ROWS` / `MAX_FROZEN_COLS`
   (B2, `engine-worker-hardening/functional_spec.md` §F1.3). The publish loop and the grid's band
   renderer are bounded no matter what the model holds, so this is not a FreeCell hang or crash —
   which is why it is backlog and not a phase.

5. **Reaching it needs a near-empty sheet, in both variants.** The increment fires only when the
   insertion row is inside the band (`row <= frozen_rows`), and it *accumulates* only when no
   populated cell sits at or below that row — otherwise the insert shifts the data down, growing
   `max_row` and consuming the headroom the range check measures. With FreeCell's cap of 64 frozen
   rows that means a sheet with **nothing at row 64 or deeper**, for the one-insert overflow and
   the repeated accumulation alike. What differs between the two is the *gesture*, not which
   workbooks are exposed: one near-Select-All run versus repetition of ordinary ones.

6. **The fix belongs in the fork**, at those two lines, on its own branch (§Where the fix belongs).

## Observations

All measured through FreeCell's worker command path against fork commit
`cee2859dceda65ff64e52192be4ec47a259870e1`, which was the `freecell-fixes` pin in `Cargo.lock` at
the time. The pin has since moved to `c1acacbda22e98450ab36139e686ecc29ff19305`; the numbers below
were **not** re-run there, they were re-verified to still apply: every file this note cites
(`base/src/actions.rs`, `base/src/model.rs`, `xlsx/src/import/worksheets.rs`) is **byte-identical**
between the two checkouts, so both the defect and the cited line numbers carry over unchanged. Rows
axis, `LAST_ROW` = 1,048,576, FreeCell's `MAX_FROZEN_ROWS` = 64. `max_row` is `dimension().max_row`,
which is 1 on a sheet with no cells. Each row is one observation with its own conditions; nothing
here should be read as holding outside them.

| `frozen` | `max_row` | Insert | Result |
|---|---|---|---|
| 64 | 1 | `row 0, count 1_048_575` | `frozen_rows = 1_048_639` — past `LAST_ROW` |
| 64 | 1 | `row 0, count 1_048_513` | 1,048,577 — past `LAST_ROW` |
| 64 | 1 | `row 0, count 1_048_512` | 1,048,576 — exactly `LAST_ROW`, not past |
| 64 | 1 | `row 0, count 1_048_576` | rejected; `frozen_rows` stays 64 |
| 64 | 63 | `row 0, count 1_048_513` | 1,048,577 — past `LAST_ROW` |
| 64 | 63 | `row 0, count 1_048_514` | rejected; stays 64 |
| 64 | 64 | `row 0, count 1_048_512` (largest accepted) | 1,048,576 — not past |
| 64 | 100 | `row 0, count 1_048_476` (largest accepted) | 1,048,540 — not past |
| 64 | 1,000 | `row 0, count 1_047_576` (largest accepted) | 1,047,640 — not past, and **no headroom remains** for a second insert |
| 64 | 1 | `row 99, count 1_000_000` (below the band) | stays 64 — no increment |
| 1 | 1 | `row 0, count 1_048_575` (largest accepted) | 1,048,576 — not past |
| 1 | 1 | `row 0, count 1_000_000`, ×3 | 1,000,001 → 2,000,001 → **3,000,001** |
| 64 | 10 | `row 19, count 1_000_000`, ×4 | 1,000,064 → 2,000,064 → 3,000,064 → **4,000,064**, `max_row` pinned at 10 |

**Round-trip**, from the 3,000,001 state: save succeeds and the writer emits
`<pane ySplit="3000001" topLeftCell="A3000002" activePane="bottomLeft" state="frozen"/>` — the
derived `topLeftCell` is out of range too — and reopening restores the model to 3,000,001 with
FreeCell's cache clamped to 64.

**Recovery**, from the same state: whole-axis deletes walk it down, 3,000,001 → 1,951,425 →
902,849 → 0. `SetFrozen` (Unfreeze included) clears it in one step and is guarded; the delete is
the only *structural* way down.

**Reached by gesture, not by API.** The insert count is not a constant the app chooses — the
header menu derives it from the selected header run (`grid/view.rs`, `header_menu_items`:
`count = run.1 - run.0 + 1`, sent as `insert_ev(start, count)`), the same untrusted-selection
pattern that caused B2 (`specs/projects/engine-worker-hardening/functional_spec.md` §0). Only the
run's **start** must sit inside the band; its extent is unconstrained.

## Who writes `frozen_rows`

Relevant because a fix has to know what else touches the field. **Only the two increments and the
xlsx reader can introduce a count that was never valid**; the rest either guard, or replay a value
the model already held.

| Writer | Guarded? |
|---|---|
| `Model::set_frozen_rows` (`base/src/model.rs:3426`) — what `UserModel::set_frozen_rows_count`, and hence FreeCell's `SetFrozen`, calls | Yes — rejects `< 0` and `>= LAST_ROW` |
| `Worksheet::set_frozen_rows` (`base/src/worksheet.rs:404`) | Yes — same two checks |
| The insert increments (`actions.rs:1051` / `:725`) | **No** — this defect |
| The xlsx reader (`xlsx/src/import/worksheets.rs:691`), `frozen_rows = get_number(pane, "ySplit")` | **No** — taken verbatim |
| Undo/redo restore, direct field writes (`user_model/undo_redo.rs:270`, `:453`) | No check, but they only replay a previously-held value — they cannot manufacture a new one |
| Undo/redo of a `SetFrozenRowsCount` diff (`undo_redo.rs:314`, `:861`) | Yes — routed through `Model::set_frozen_rows` |

So once the increments are clamped, the reader is the last path that can bring a **new**
out-of-range count into the model. An upstream reviewer will grep the field and find the undo/redo
writes; the table is here so the PR can answer that before it is asked.

## Where the fix belongs: the fork, not FreeCell

Per [`CLAUDE.md`](../CLAUDE.md) ("we ride our IronCalc fork — fix upstream, don't hack FreeCell"),
this is an engine defect and gets an engine fix on its **own** `fix/<slug>` branch off the fork's
`main` (e.g. `fix/clamp-frozen-pane-boundary`), with upstream-style tests, as one focused
single-feature PR. It must **not** be folded into `fix/structural-edits-adjust-frozen-pane`
(already upstreamed) or into any FreeCell phase.

**Clamp the two increment sites, at `LAST_ROW - 1` / `LAST_COLUMN - 1`.** Not `LAST_ROW`: both
sibling setters in the table above reject `>= LAST_ROW`, so clamping to `LAST_ROW` would let a
structural edit produce a boundary those setters define as invalid. One ceiling for every writer
is the reviewable choice, and it costs nothing.

**Leave the decrement sites (`:857`, `:1131`) alone**, and be ready to say why, because the
obvious argument is not quite right. Under their `row <= frozen` guard, with `row >= 1` and
`row_count >= 1` already validated, `deleted_in_band = min(last_deleted, frozen) - row + 1` lies
in `1..=frozen`, so the result lies in `0..=frozen - 1`: it cannot underflow and always lands
strictly below its input. That is *not* "the result is in dimension" — for an already-overflowed
count it usually still isn't, as `3,000,001 → 1,951,425` above shows. The claim that holds is
scoped: **for any count the engine can itself produce once the increments are clamped**,
`frozen <= LAST_ROW - 1` is an invariant, and a strictly-decreasing step keeps it there. A clamp
on the decrement is then dead code, and asking a reviewer to accept dead code is how a PR gets
sent back.

Worth raising alongside, as separate questions and probably separate PRs: whether the reader
should range-check `ySplit` (a file-compatibility decision, not an arithmetic one), and whether
`insert_rows` should range-check the requested shift rather than only the populated dimension.

## Acceptance

- **Fork:** an insert into or above a frozen band cannot leave the boundary outside
  `0..=LAST_ROW - 1` / `0..=LAST_COLUMN - 1`. Make the primary regression test the **single**
  insert — it is the smallest thing that reaches the increment. At the pinned commit, on a sheet
  with no cells, `frozen = 2` plus one `insert_rows(sheet, 1, LAST_ROW - 1)` overflows by exactly
  one; re-derive that arithmetic before relying on it. Two ways such a test passes without testing
  anything, both worth a comment in it: `row_count = LAST_ROW` is rejected by the dimension check
  before the increment runs, and a sheet populated at or below row `frozen` closes the window
  entirely. Add a second test repeating a mid-sized insert for the accumulation case.

  The criterion is about what the *increment* can produce. A file whose `<pane>` already carries
  an out-of-range `ySplit` still loads with one — the reader is unguarded, and out of scope here.

- **FreeCell:** re-pin `freecell-fixes`, then extend
  `structural_edit_past_the_cap_diverges_model_from_the_clamped_cache` (`worker/run.rs`) with the
  single oversized insert, and tighten `engine-worker-hardening/functional_spec.md` §F1.3 — the
  saved count becomes bounded-but-over-cap instead of unbounded, so its "How the divergence ends"
  paragraph collapses to the self-healing case.
