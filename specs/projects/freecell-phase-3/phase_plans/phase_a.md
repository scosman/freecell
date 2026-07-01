---
status: complete
---

# Phase A: Style/geometry cache sync + structural editing (the crux)

## Overview

Phase A closes the highest-stakes pre-build unknown: does the just-adopted
**always-resident style+geometry cache** stay provably in lockstep with IronCalc across
the one operation that shifts everything downstream — **insert/delete row/column** — and
its **undo/redo**? And does the app build on the low-level `Model` or the interactive
`UserModel`, and if the latter, does the **SP1 worker seam** (worker owns a `Send`
model) still hold?

Deliverable: `experiments/round-3/A-cache-sync/` with (1) a `UserModel` API + `Send`
probe, (2) a structural-edit correctness harness (references + band styles + sizes shift
correctly, `.xlsx` round-trip), (3) undo/redo coverage across value/style/structural
edits, (4) a resident-cache-shift prototype whose shifted state is **asserted equal to
IronCalc's re-read authoritative state** after every edit and every undo/redo, (5)
cost-at-scale numbers (10^5–10^6, foreground, force+assert), and (6) `findings.md` with
the locked cache-sync design + the `Model`-vs-`UserModel` recommendation, honestly graded
against the GATE/DISCOVERY criteria.

### Key API facts established from the IronCalc 0.7.1 source (cited)

- `UserModel<'a>` wraps `Model<'a>` (`user_model/common.rs:222`). It exposes
  `insert_rows`/`insert_columns`/`delete_rows`/`delete_columns` (common.rs:882–1022),
  `undo`/`redo`/`can_undo`/`can_redo` (common.rs:306–340), `set_user_input`,
  `update_range_style`, `set_rows_height`/`set_columns_width`,
  `get_row_height`/`get_column_width`, `get_cell_style`/`get_style_for_cell` (via
  `get_model()`), `copy_to_clipboard`/`paste_from_clipboard` (common.rs:1765–1923),
  `get_model()` → `&Model` (common.rs:288), `from_model`/`to_bytes`/`from_bytes`.
- Structural edits **auto-evaluate** (`evaluate_if_not_paused`, common.rs:2106) — an
  insert/delete is `&mut self` and runs a full `evaluate()`. Same blocking shape as
  `Model` (the SP1 seam concern).
- The **diff-list is `pub(crate)`** (`history.rs:20` `enum Diff`), only extractable as
  opaque bitcode via `flush_send_queue()` (common.rs:376). It is NOT publicly
  inspectable field-by-field. → the cache **cannot** consume structured diffs; it must
  **mirror the structural primitive** it issued. (Finding for A + B.)
- Row band styles + heights live in `worksheet.rows: Vec<Row>` (each `Row{r, s, height}`)
  and are **re-keyed** on insert/delete (actions.rs:362–376, 431–446). Column band
  styles + widths live in `worksheet.cols: Vec<Col>{min,max,width,style}`. Sizes read via
  `get_row_height`/`get_column_width`; band styles via `get_row_style`/`get_column_style`
  on `get_model()`.
- `merge_cells: Vec<String>` exists on `Worksheet` (types.rs:113) but there is **no
  public `Model`/`UserModel` setter/getter** — merges are unreachable through the public
  API (confirms overview §2 "merges have no IronCalc API"). → the correctness harness
  omits merges and records the gap.
- Structural-edit cost is driven by `sheet_data.keys()` (populated rows) +
  `displace_cells` (formula rewrite), NOT by 1,048,576 (actions.rs:346–385) — so a
  sparse-but-tall sheet should be far cheaper than worst case. Measure to confirm.
- `UserModel<'static>` should be `Send` iff `Model<'static>` is (SP1 proved it is); a
  compile-time `assert_send::<UserModel<'static>>()` is the authoritative check.
- `.xlsx` round-trip: `ironcalc::export::save_to_xlsx(&Model, path)` /
  `ironcalc::import::load_from_xlsx(path, ...)` operate on `&Model` — reachable via
  `get_model()`. `.icalc`/internal via `to_bytes`/`from_bytes`.

## Steps

1. **`src/probe.rs` — `UserModel` API + `Send` probe.**
   - `assert_send::<UserModel<'static>>()` (compile-time, mirrors SP1
     `seam.rs:52`). Also `assert_send::<Model<'static>>()` as a control.
   - A runtime smoke that constructs a `UserModel::new_empty(..)` with `'static`
     literals, sets a value+formula, evaluates, reads back, and exercises
     `undo`/`redo`/`can_undo`/`can_redo`, capturing the observed API surface into a
     struct returned for `findings.md`.
   - A **worker-seam probe**: spawn a thread, move the `UserModel<'static>` onto it
     (proving `Send` at runtime too), run an `insert_rows` + `evaluate` on the worker,
     hand the model back via the join handle, and assert reads still work — the SP1 seam
     shape applied to `UserModel`.

2. **`src/cache.rs` — the resident style/geometry cache prototype** (per architecture
   §4.1–§4.3; headless, no GPUI):
   - Per axis (rows, cols): `default_size: f64`, `size_overrides: BTreeMap<i64, f64>`,
     `default_band_style: StyleId`, `band_style_overrides: BTreeMap<i64, StyleId>`.
   - Per cell: `cell_style_overrides: BTreeMap<(i64,i64), StyleId>`.
   - `StyleId` interner: `HashMap<Style, StyleId>` + `Vec<Style>` (dedup; styles are
     highly repetitive per SP4/SP5).
   - **Cumulative-size structure — implement candidate (a) DENSE prefix-sum** over an
     explicit `Vec<f64>` sizes array (index→size) with a prefix-sum vector, plus the
     lookups `offset(i)` (cumulative pixels before row/col `i`) and `index_at(pixel)`
     (binary search) the renderer needs. This is architecture §4.3(a); the plan is to
     **measure it before rejecting** for the derived-Fenwick option (b). The dense array
     is sized to the *populated extent* (the tallest touched row), not 1,048,576, and its
     splice cost is what the cost harness measures.
   - **Shift primitive** `shift_axis(axis, at, delta)`:
     - re-key `size_overrides` / `band_style_overrides` for keys `>= at` by `delta`
       (BTreeMap split-off + re-insert; removed-on-delete overrides captured + returned
       so undo can restore them — the *mirror-the-primitive* undo strategy),
     - re-key `cell_style_overrides` on the shifted axis,
     - splice the dense sizes array (insert `delta` default-size entries at `at`, or
       remove them) and rebuild the affected prefix-sum tail.
   - `undo_shift(...)` applies the inverse primitive, restoring captured overrides.

3. **`src/harness.rs` — build the reference sheet + IronCalc↔cache sync driver.**
   - `build_sheet(user_model, rows, ...)`: cross-referencing formulas (e.g. `B_r =
     A_r + A_{r-1}`, and a `SUM(A1:A_n)` total that must re-target on insert/delete),
     row **band** styles + custom row heights on a set of banded rows, column band styles
     + custom widths on banded cols. (No merges — API gap, recorded.)
   - `hydrate_cache_from_engine(user_model)`: pull default+override sizes and band/cell
     styles from IronCalc into a fresh cache (the "on load" path).
   - `apply_structural_edit_to_both(kind, at, count)`: issue the edit on the `UserModel`
     AND the mirrored `shift_axis` on the cache.
   - `assert_cache_agrees_with_engine(user_model, cache, sample_indices)` — **the
     load-bearing contract** (architecture §4.4): for a sample of indices spanning the
     edit point, assert cache size == `get_row_height`/`get_column_width`, cache band
     style == `get_row_style`/`get_column_style`, cache cell style == `get_style_for_cell`,
     and the cumulative offset is consistent with the shifted sizes.

4. **`src/main.rs` — orchestrate the runnable investigation** (prints a report + writes
   `results/*.json`), plus the correctness/undo assertions live in `tests/` so they gate.

5. **Cost harness** (`src/main.rs`, foreground, `timeout`-run): at 10^5 and 10^6
   populated rows, time a single `insert_rows`/`delete_rows` on the `UserModel`
   (IronCalc-side) and, separately, the cache `shift_axis` cost. **Force+assert** each: a
   reference/size/style at an index past the edit point must have moved. Report p50/p99
   via `LatencyStats`, env-stamped via `Environment::detect` + `cpu_model()`, written to
   `results/`. Cap the scale and record the ceiling if a run is too slow.

## Tests (in `tests/`, so they gate; correctness is the GATE)

- `send_probe`: `UserModel<'static>` is `Send` (compile-time assert links).
- `structural_insert_row_shifts_refs_styles_sizes`: after `insert_rows`, a downstream
  formula reference, a banded row's style, and a custom row height all moved down by the
  count; the total `SUM` still covers the original range plus/minus the shift.
- `structural_delete_row_shifts`: symmetric for delete (references re-target; a formula
  pointing into the deleted row becomes `#REF!` where expected, others shift).
- `structural_insert_delete_column_shifts`: same for columns (band col style + width +
  cell references).
- `xlsx_roundtrip_preserves_structural_edit`: apply an insert+delete, save via
  `save_to_xlsx(get_model())`, reload, assert references/band styles/sizes survive.
- `undo_redo_value_edit` / `undo_redo_style_edit` / `undo_redo_structural_edit`:
  edit → undo → assert reverted → redo → assert reapplied; the structural undo fully
  un-shifts (a downstream reference/size/style returns to its pre-edit index).
- `cache_agrees_after_insert_row` / `_delete_row` / `_insert_col` / `_delete_col`: the
  §4.4 contract — mirrored cache == IronCalc re-read, over indices spanning the edit.
- `cache_agrees_after_undo` / `_after_redo`: contract holds through undo/redo.
- `cumulative_offset_matches_sizes`: dense prefix-sum `offset`/`index_at` round-trip and
  agree with the summed override/default sizes after a shift.
- `copy_paste_translates_relative_refs`: `copy_to_clipboard` + `paste_from_clipboard`
  translates a relative reference to the paste target (records values-vs-formulas-vs-
  styles behavior).
- `cache_interns_styles`: repeated identical styles share one `StyleId`.

## Off-ramp watch

If any GATE fails — structural edits don't shift correctly, undo/redo missing/partial,
the cache can't be made to agree with IronCalc, or the shift is worse than O(shifted at
scale) — record it plainly in `findings.md` as a "must change first" finding (that is a
successful Phase-3 outcome), and surface it in the return summary.
