---
status: draft
---

# Phase 2: Worker protocol + published CF rule list

## Overview

Wire the P1 engine seam (`WorkbookDocument` CF methods + conversions, all headless in P1) through the
engine-free worker protocol so the UI can mutate CF and read the current rules synchronously. This
phase adds:

- Engine-free `Command` CF variants + a `WorkerEvent::CondFmtUpdated { sheet }`.
- Dispatch of those commands in the worker's `apply_one` → the matching `WorkbookDocument` method,
  with the engine's `Err(String)` surfaced back the same way style edits surface engine errors
  (`WorkerEvent::EditRejected { reason: Engine(..) }`).
- A published `cond_fmt: Arc<RwLock<HashMap<SheetId, Vec<CfRuleView>>>>` map on `Shared`, refreshed by
  the worker after any CF mutation, on undo/redo of a CF op, on structural CF-range shifts, and once
  on open; read synchronously via `DocumentClient::cond_fmt_rules(sheet)`.

**No cache change** (the value-dependent extended-read is P3) and **no UI** (P4+). The
`StyleCacheUpdated` that a CF mutation emits is produced by the existing style-cache mirror; in P2 it
re-reads base styles (CF fills appear once P3 threads the extended read into the cache build).

## Design notes / deviations

- **CF ops are bucketed with the style/cell edits** (into `apply_edit_batch`), so they ride the
  existing coalesced-eval + `Published` + undo/redo machinery. A CF mutation changes styles, not
  values → `apply_one` returns `AppliedKind::StyleOnly` (no recompute).
- **`op_of` maps every CF command to `AppliedOp::Rebuild { sheet }`** (a full sheet-cache rebuild),
  not `AppliedOp::Cells { sheet, range }` as architecture §4.2 literally states. Rationale: (1)
  `DeleteCondFmt`/`Raise`/`Lower` carry only an `index`, not a range, so a range is unavailable
  without pre-reading the doc; (2) a CF range may be a multi-area address that a single `CellRange`
  can't express; (3) a rule affects its whole (possibly large) range and a reorder affects the union
  — so a full rebuild is the simple, always-correct choice (the same reasoning architecture §6 uses
  for the value-publish path). This is also what threads the CF-range shift on a structural edit back
  into the published map.
- **CF-map republish hook lives in `apply_edit_batch`**, keyed on the batch's *rebuilt* sheets. That
  set = {CF mutations} ∪ {their undo/redo (each pushes a `Touch::Rebuild`)} ∪ {structural edits that
  shift a sheet's CF ranges}. Each such sheet is reconciled: recompute `cond_fmt_rules`, and if the
  published list changed, update the map and emit `CondFmtUpdated { sheet }`. Gated so a non-CF sheet
  with no published entry is skipped (`has_cond_fmt(sheet) || map.contains(sheet)`), preserving the
  "non-CF workbooks pay nothing" invariant. Plain cell/style edits produce `AppliedOp::Cells` (not
  Rebuild) → never trigger a CF list read.
- **Raise/Lower no-op detection.** The fork records *no* undo diff when a raise/lower is a boundary
  no-op (rule already top/bottom). To keep the worker undo stack 1:1 with IronCalc's, `apply_one`
  compares the rule list before/after the reorder and returns `AppliedKind::NoOp` when unchanged (so
  no phantom undo entry / ops_seen bump). Add/Update/Delete always push a diff → always `StyleOnly`.
- **Sheet-set-change prune.** When the sheet set changes in a batch (delete / undo-of-add), the CF
  map is pruned to the live sheet ids next to the existing `SheetsChanged` emit, so a deleted sheet's
  CF entry never outlives it.

## Steps

1. **`worker/protocol.rs`** — add `CfRuleSpec` to the `freecell_core` import; add five `Command`
   variants carrying only engine-free types:
   `AddCondFmt { sheet: SheetId, range: String, spec: CfRuleSpec }`,
   `UpdateCondFmt { sheet, index: u32, range: String, spec: CfRuleSpec }`,
   `DeleteCondFmt { sheet, index: u32 }`,
   `RaiseCondFmtPriority { sheet, index: u32 }`,
   `LowerCondFmtPriority { sheet, index: u32 }`.
   Add `WorkerEvent::CondFmtUpdated { sheet: SheetId }` (window rebuilds the rows).

2. **`worker/client.rs`** — import `HashMap` + `CfRuleView`; add
   `cond_fmt: Arc<RwLock<HashMap<SheetId, Vec<CfRuleView>>>>` to `Shared` and init it empty in
   `Shared::new`; add `DocumentClient::cond_fmt_rules(&self, sheet: SheetId) -> Vec<CfRuleView>`
   (read lock + clone, empty when absent). Unit-test the client read.

3. **`worker/run.rs` — routing.** Add the five CF commands to the `edit @ (...)` bucket in
   `process_batch` (alongside the style edits). Add their `apply_one` arms (resolve sheet → the
   matching `WorkbookDocument` method; Raise/Lower use the no-op detection above). Add the `op_of`
   arm → `AppliedOp::Rebuild { sheet }`.

4. **`worker/run.rs` — publish.** In `apply_edit_batch`, after `collect_edited_ranges`, capture the
   rebuilt-sheet set; after `apply_cache_refresh`, call `reconcile_published_cond_fmt(&rebuilt)`; in
   the `sheets_after != sheets_before` block, prune the CF map to live ids. Add
   `reconcile_published_cond_fmt(&mut self, sheets)` (gated recompute + change-detected map update +
   `CondFmtUpdated`) and `publish_all_cond_fmt_on_open(&self)` (populate the map for every sheet with
   rules); call the latter once in `load_and_run` after the on-open cache build.

5. **`document/cond_fmt.rs`** — remove the module-level `#![allow(dead_code)]` (its mutators + reads
   are now consumed by the worker); add a targeted `#[allow(dead_code)]` on `extended_render_style`
   only (consumed in P3's value-dependent cache).

6. **`cond_fmt_convert.rs`** — remove the module-level `#![allow(dead_code)]` (all conversions are now
   reachable from the consumed `WorkbookDocument` CF methods).

## Tests

Worker-seam (in `freecell-engine`, `worker/run.rs` test module, via `test_worker()` + `process_batch`):

- `add_cond_fmt_publishes_rules_and_emits_events` — an `AddCondFmt` makes `shared.cond_fmt[sheet]`
  reflect the rule, and emits both `CondFmtUpdated { sheet }` and `StyleCacheUpdated { sheet }`.
- `update_cond_fmt_republishes` — `UpdateCondFmt` changes the published rule's range/summary + emits
  `CondFmtUpdated`.
- `delete_cond_fmt_removes_from_map` — `DeleteCondFmt` drops the sheet's entry + emits.
- `raise_lower_reorders_published_list` — reorder changes the published list order + emits.
- `raise_at_top_is_noop` — a boundary raise records no undo op (ops_seen unchanged) and emits no
  `CondFmtUpdated` (undo-stack alignment guard).
- `undo_redo_restores_and_republishes_cf` — add → undo (rules gone, map empty, `CondFmtUpdated`) →
  redo (rules back, `CondFmtUpdated`).
- `bad_range_add_surfaces_error` — an `AddCondFmt` with an invalid range emits
  `EditRejected { reason: Engine(_) }`, adds no rule, and emits no `CondFmtUpdated`.
- `non_cf_sheet_has_empty_published_map` — a plain edit on a sheet with no CF leaves the map empty.

Client (in `worker/client.rs` test module):

- `cond_fmt_rules_reads_published_map` — a `DocumentClient` over a `Shared` with a seeded map returns
  the sheet's rules and empty for an unseeded sheet.
