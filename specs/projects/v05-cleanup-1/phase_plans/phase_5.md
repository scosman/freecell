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

---

# Review remediation (2026-08-07)

Code review found **no Criticals** and verified the detector is live (three independent
perturbations each made it red), that all five baselines are exactly accurate, that there is no
exclusion list on `dropped`, and that the green-not-red judgement call is required in terms by
`functional_spec.md` §C1 and `architecture.md` §5. None of that was churned. The suite is now
**10 tests**, all green.

## Moderate

### M1 — "nothing dropped" was glossed as "fully covered by IronCalc's model" (false)

**Accepted, and the numbers re-derived independently rather than copied.** Part-level parity does
not imply content parity. A throwaway probe round-tripped each fixture through the same app save
path and diffed *inside* every surviving part (byte length + element-name census). Measured
2026-08-07:

| fixture | `xl/worksheets/sheet1.xml` | lost inside the part |
|---|---|---|
| `dates.xlsx` | 1012 → 1003 | (none — inline `<is>`/`<t>` moved to `sharedStrings`) |
| `FONTS.xlsx` | 5364 → 5350 | `pageSetup`, `pageMargins`, `headerFooter`, `oddFooter`, `pane`, `sheetFormatPr` |
| `numbers_table.xlsx` | 6866 → 6963 | same six |
| `libreoffice_custom_height_wrap.xlsx` | 10186 → **6665** | `pageSetup`, `printOptions`, `pageMargins`, `headerFooter`, `oddHeader`/`oddFooter`, `sheetPr`, `pageSetUpPr`, `sheetFormatPr`; `workbook.xml` 1102 → 560, `extLst` gone |
| `personal_monthly_budget.xlsx` | 1869 → 1274 | `pageSetup`, `pageMargins`, `sheetPr`, `tabColor`, `sheetFormatPr`; `workbook.xml` 2227 → 448, `extLst` gone |

**Two deltas from the review's table, both reported as asked:**
- `libreoffice_custom_height_wrap.xlsx`'s `sheet1.xml` measures **6665** bytes here, not 6635.
  Directionally identical (~35% lost); the exact figure differs.
- The per-part loss is **broader than `pageSetup` + `pageMargins`**. Every affected sheet also
  loses `headerFooter`/`oddFooter`, `sheetFormatPr` and `sheetPr`, `numbers_table`/`FONTS` also
  lose their frozen `<pane>`, and `personal_monthly_budget`'s `sheet2.xml` loses **all twelve
  `<tablePart>` references** plus `ignoredErrors`. Beyond worksheets: `xl/styles.xml` loses
  `indexedColors` / `tableStyles` / `protection`, `docProps/app.xml` loses `Company` / `Manager`
  / `Template`, and **`xl/theme/theme1.xml` is replaced wholesale** with IronCalc's default
  (`FONTS.xlsx`: 21716 → 8716 bytes). So the review's point holds *more* strongly than stated.

Fixed in three places: the `part_inventory.rs` module doc gained a **SCOPE** block listing the
above; the three "nothing dropped" baselines now say **"no whole parts dropped"** with the
content caveat named per fixture; and the `GAPS.md` C1 box gained a "what *measured* does and
does not cover" paragraph plus the sentence that every number in it is a **floor**. The headline
was re-titled "MEASURED **at the package level**".

### M2 — "dropped by every save, on every fixture measured" was wrong

**Accepted.** Only 3 of 6 fixtures contain `docProps/custom.xml` at all
(`personal_monthly_budget`, `libreoffice_custom_height_wrap`, `charts/excel_line_chart_workbook`);
`dates`, `numbers_table` and `FONTS` do not — verified by `unzip -l`. Both the `CUSTOM_PROPS` doc
comment and the `GAPS.md` box now say "on every fixture **that has one** — 3 of the 6 measured",
and name which three. That removes the self-contradiction with "three fixtures lose nothing".

### M3 — the edit variant never asserted the edit happened

**Accepted.** `save_through_worker` now **forces and asserts** the measured op, per CLAUDE.md:

- it keeps the `Saved { ops_seen }` payload and asserts `ops_seen == u64::from(edit)` — exactly
  one committed op on the edited run, exactly zero on the clean one (which also guards the clean
  run against a stray edit);
- on the edited run it then scans every part of the saved package for the probe string and fails
  if it is absent.

Also moved the edit **off sheet 0**: on `personal_monthly_budget.xlsx` sheet 0 is the "START"
cover sheet and all twelve tables live on sheet 1, so the edit now targets `sheets[1]` (with an
explicit panic if the fixture has no second sheet).

### M4 — blind to `[Content_Types].xml` override loss

**Accepted — this was the highest-value finding, and it was a real hole.** Added
`content_type_decls()`: parses `[Content_Types].xml` with `roxmltree` and returns the set of
`Default .<ext> -> <ct>` / `Override <part> -> <ct>` declarations. `Inventory` now carries
`ct_dropped` / `ct_added` alongside `dropped` / `added`, every fixture test asserts a **committed
CT baseline in both directions**, and the content *type* is compared, not just the part name, so
a silently retyped part is caught too.

Baselines committed (all measured, none guessed). Notable entries:

- **`CHART_PLAIN_CT_DROPPED` vs `CHART_APP_CT_DROPPED`** — the plain path loses the four
  `Override /xl/charts/chartN.xml` and the drawing's overrides; the app path loses none. This is
  the direct measurement of `chart/save.rs`'s `merge_content_types` splice.
- **`NUMBERS_MEDIA_DEFAULTS`** — nine `<Default>` media declarations Numbers' exporter writes for
  extensions the package does not contain. Benign; recorded because the *mechanism* that loses
  them (the writer rebuilds `[Content_Types].xml` from its own model) is the one that loses the
  declarations that matter.
- **LibreOffice's six redundant `.rels` `Override`s** — LibreOffice types every `.rels` part
  twice (once via `Default Extension="rels"`, once by name); IronCalc keeps the `Default` and
  drops the redundant `Override`, so nothing becomes untyped. Recorded and annotated as benign.

Incidental finding: `libreoffice_custom_height_wrap.xlsx` also carries **five charts and four
drawings**, which the app path re-injects — so it exercises the same machinery as the chart
fixture. Noted in its baseline comment.

## Mild

| Finding | Resolution |
|---|---|
| `wait_for` matched only `Saved`, so a `SaveFailed` burned the 30 s timeout and blamed the wrong thing | **Fixed.** Matches both, then `panic!`s immediately with the `SaveError` payload. |
| "**exactly** what the shipped app does on ⌘S" overstated | **Fixed.** Now "the same engine path", with the two differences named: the app writes a one-time `.back` copy first (`shell/window.rs:981`) and normally saves in place rather than to a fresh path — neither can move the inventory, and saving to a fresh path is what makes the diff possible. |
| The chart contrast was only implicit across three constants | **Fixed.** New `chart_reinjection_strictly_reduces_the_loss` asserts `CHART_PLAIN_* ⊇ CHART_APP_*` for **both** the part and the CT baselines, and that the four chart parts / the drawing / their overrides are specifically in the delta. It re-measures nothing (each constant is pinned by its own test); it stops a careless simultaneous edit from erasing the finding while staying green. |
| `assert!(out.exists())` redundant with the `WorkbookDocument::open` two lines later | **Fixed.** Removed; the reopen assertion's comment now says it subsumes the existence check. |
| `TIMEOUT` + `wait_for` duplicate `tests/worker_seam.rs:20-42` | **Noted, not extracted.** Two integration binaries cannot share without a `tests/common/mod.rs`, which fourteen duplicated lines do not earn. The doc comment now says so and adds: **a third copy should trigger the extraction.** |
| `architecture.md` §5 stale on the save path | **Fixed as a dated correction note**, not a rewrite (matching the §2 convention). The note covers both the save-path supersession *and* the newly-added content-type dimension, and restates the package-level scope limit. |
| Nothing couples the baselines to `GAPS.md` | **Declined as unfixable cheaply, and stated explicitly instead.** A mechanical coupling would mean parsing prose out of `GAPS.md` or generating that box from the test — both worse than the problem. The `GAPS.md` box now ends with: the non-regression property is enforced by CI; **the currency of the prose is not** — re-derive before relying on it. |

## Mutation proofs

Every behaviour change was proven to be able to fail. Each mutation was applied, run, and
reverted.

| # | Mutation | Result |
|---|---|---|
| A | `chart/save.rs`: write IronCalc's `[Content_Types].xml` unchanged instead of `merge_content_types(…)` — i.e. carry every chart part across but lose its `<Override>` | **RED**, 3 tests: `chart_workbook_part_inventory_app`, `…_preserved`, `libreoffice_part_inventory`. Critically, **all three failed in `assert_ct_dropped`, and every `assert_dropped` (parts) passed** — the pre-remediation suite would have been fully green on a package Excel would refuse to open. This is the exact blind spot M4 describes, demonstrated. |
| B | Delete the `Command::SetCellInput` send in the `edit` branch (the edit silently becomes a no-op) | **RED**: `an_edit_does_not_change_what_is_dropped`, on `ops_seen == 0, expected 1`. |
| B2 | Keep the edit but type a different string, so the op commits and the probe never reaches the file | **RED**: same test, `the edit never reached the saved package: "c1-probe" is in none of its parts`. Both halves of the new edit assertion are independently live. |
| C | Delete `xl/charts/chart1.xml` from `CHART_PLAIN_DROPPED` (simulating a careless baseline edit that erases the C3 evidence) | **RED**: `chart_reinjection_strictly_reduces_the_loss`, naming the erased part. |
| D | Point `Command::Save` at an uncreatable path | **RED in 0.22 s** with `the worker save failed: Io("couldn't create a temporary file …")`. Before the fix this would have hung for the full 30 s `TIMEOUT` and then reported "worker emits Saved". |

## Verification

- `cargo test -p freecell-engine --test part_inventory` — **10 passed**, 0 failed.
- `cargo test -p freecell-engine --lib` — green.
- `cargo clippy -p freecell-engine --all-targets -- -D warnings` — clean.
- `cargo fmt --all --check` — clean.
- Probe files (`tests/zz_content_probe.rs`, `tests/zz_ct_probe.rs`) deleted.
- `charts_roundtrip_libreoffice` fails in this container because headless `soffice` cannot load
  any file — pre-existing and environmental, not from this work.

## Scope held (remediation)

Still no preservation fix — C1 remains a detector. `chrome/view.rs` and `worker/*` untouched
(`worker/run.rs` and `worker/protocol.rs` read only). The `chart/save.rs` edit in mutation A was
temporary and reverted (`git checkout`); the working tree contains no engine-source change.
