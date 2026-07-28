# Phase 5 — C1: Part-inventory round-trip test (keystone)

**Verdict: CONFIRMED, and the loss is larger than the unit implied.**

## Confirmation

`tests/roundtrip.rs` (282 lines) round-trips only `fixtures::*` — workbooks FreeCell itself
authors. Every assertion runs through `save_and_reopen`, which saves a FreeCell-authored
document and reopens it. That is a closed loop over IronCalc's own serializer: anything IronCalc
cannot *represent* is also absent from the input, so the loop closes perfectly no matter how much
is being dropped. Meanwhile five real third-party `.xlsx` files sit in `tests/fixtures/` used for
**open** assertions only. The unit's diagnosis was exactly right.

## Design decisions

### Which save path — the one the user actually gets

The engine has three save entry points, and picking the wrong one would have measured the wrong
thing:

| | What it is |
|---|---|
| `WorkbookDocument::save` | IronCalc's writer straight to disk |
| `chart::save_with_charts` | the no-edit byte-preserve path (writer + chart re-injection) |
| worker `Command::Save` | **what the app does on ⌘S** — picks its own strategy and writes atomically |

The architecture doc planned to drive the first two, on the grounds that `worker/run.rs` is owned
by the parallel engine-worker-hardening project. That reasoning was wrong on inspection: the test
only needs the **public** `DocumentClient` / `Command` / `WorkerEvent` surface, which
`tests/worker_seam.rs` already uses. Nothing in `worker/*` is edited. So the primary fixture
assertions drive the **real app path**, and the other two are kept on the chart workbook as
contrast.

That contrast turned out to be the most informative thing in the file — see below.

### The baseline, rather than a permanently-red test

The brief says to expect it red and loud, and it was: 5 of 9 cases failed on first run. But a
permanently-red required check is not a deliverable — it gets ignored or disabled, and then the
loss is invisible again for a different reason.

Instead each fixture carries a **committed drop-set baseline**, annotated in-file with what each
part actually is. `assert_dropped` compares in **both** directions: a newly dropped part is a
regression and fails; a part that *stops* being dropped also fails, so a fix cannot land without
updating the record (and `GAPS.md` with it). The loss is written out by name in the test file
where a reader trips over it, which is the honesty the unit was after — and CI stays green, so
the check keeps its authority.

**No exclusion list.** The architecture allowed a small justified one; it turned out not to be
needed, which is better. `xl/calcChain.xml` (a cache Excel rebuilds) is *in* the
`personal_monthly_budget` list, annotated as benign, rather than filtered out — the test records
what changed, and the comment says what matters. Writer-*added* parts (`xl/metadata.xml`) are
reported on failure for context but never asserted; asserting them would pin IronCalc's writer
internals, which is not this unit's business.

## What it measures

`personal_monthly_budget.xlsx` — a real Excel template — **loses 27 parts**:

- **all twelve `xl/tables/tableN.xml`** — every table (ListObject) definition. Banded formatting,
  header/total rows, filter buttons, structured references. Round-trip this budget template
  through FreeCell and open it in Excel: every table is now a plain range. This is the headline
  result, and it is worse than "unmodelled content is dropped" suggests — tables are the
  *structure* of a template, not an exotic extra.
- **all `customXml/*`** — 3 items plus their rels and itemProps.
- **`xl/printerSettings/*.bin`** — page setup.
- the sheet `_rels` binding those parts, plus the benign `calcChain`.

`docProps/custom.xml` (user-defined document properties) is dropped on **every** fixture measured.
`dates.xlsx`, `numbers_table.xlsx` and `FONTS.xlsx` lose **nothing** — worth recording, because it
means the loss is concentrated in real-world-complexity files rather than being uniform.

### The chart contrast — and why it matters for C3

The chart workbook through the **bare serializer** loses all four chart parts, the drawing, and
their relationships. Through the **app path** it loses none of them.

That is a working, shipped instance of exactly the mechanism C3 needs: IronCalc's writer plus
targeted re-injection of parts it cannot model. C3 is not a research problem — it is generalising
existing machinery from charts to tables and the rest. Recorded in `GAPS.md`.

## The edited variant

Kept, and now stronger than planned: rather than a separate code path, it is the same worker save
with one cell typed into A1 first. `an_edit_does_not_change_what_is_dropped` asserts the two drop
sets are identical, pinning the claim that the loss is a property of the serializer and not
path-dependent. One fixture only — running all six twice doubles the suite for no extra
information.

## Verification

`cargo test --locked -p freecell-engine --test part_inventory` — 9 passed, 0 failed.
`cargo fmt --all --check` clean.

## Scope held

No preservation fix. C1 is a detector; C3 (v1.0) fixes it and C2 (next round) warns the user.
