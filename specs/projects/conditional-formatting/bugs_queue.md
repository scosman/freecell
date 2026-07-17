# Conditional Formatting — bug queue (fix at end of first pass)

Bugs found by the requester while **using the running app**, to be **batch-fixed at the end** of
the first pass (owner directive 2026-07-17). Each fix is delegated to a coding sub-agent + code
review (manager stays out of the code). Root-cause analysis here is for handoff.

Status legend: `OPEN` → `FIXING` → `FIXED` (commit).

---

## BUG-1 — CF does not apply when a rule is added; only applies after a value change  — FIXED (b6e0678 + hardening)

**Fix (commit `b6e0678`):** added an `AppliedKind::CondFmt` variant that records its op like `StyleOnly`
but sets `needs_eval = true`, so a CF add/update/delete/reorder forces the single coalesced
`doc.evaluate()` — refreshing IronCalc's `cf_cache` **before** the `AppliedOp::Rebuild` cache refresh
reads it. CF now applies/updates/clears immediately with no value change. `op_of` unchanged, so the CF
republish + `CondFmtUpdated` reconcile still fires. 3 worker-seam regression tests added
(`cond_fmt_applies_on_add_without_value_change`, `…_removed_on_delete_…`, `…_updated_on_edit_…`), each
asserting the **published render cache** with a value-free batch (so they fail pre-fix).

**Review:** diff-only CR — APPROVE-WITH-NITS. Confirmed the ordering (evaluate runs before the cache
rebuild), no double-rebuild (the post-recompute path skips already-rebuilt sheets), and that **undo/redo
of a CF op was already safe** (Undo/Redo return `AppliedKind::Cell` → unconditional eval; the CF op's
`Touch::Rebuild` re-lands the sheet). Two non-blocking follow-ups folded in as a hardening commit:
(a) render-cache-fill assertions added to `undo_redo_restores_and_republishes_cf` (closes the exact
coverage class BUG-1 slipped through — a path proven safe by reasoning now has an assertion);
(b) a doc-comment note that the forced eval re-rolls volatiles. All checks green (371 lib tests).

<details><summary>Original root-cause analysis (kept for the record)</summary>

**Reported:** "Rendering doesn't update after adding a rule. Need to change the value to make
conditional formatting apply."

**Severity:** High — this is the core feature; adding a rule must apply it immediately.

**Root cause (investigated):**
- In `app/crates/freecell-engine/src/worker/run.rs`, `apply_one` classifies `Command::AddCondFmt`
  (and `UpdateCondFmt` / `DeleteCondFmt` / `RaiseCondFmtPriority` / `LowerCondFmtPriority`) as
  `AppliedKind::StyleOnly` (comment: "A CF rule changes styles, not values → no recompute", ~line
  3522-3526). So the batch's `needs_eval` stays **false** and `doc.evaluate()` is **not** called.
- The worker holds `doc.pause_evaluation()` for the batch, so IronCalc's internal
  `evaluate_if_not_paused()` (called at the end of `add_conditional_formatting` in the fork) is a
  **no-op** → the model's `cf_cache` is **not (re)populated** after the mutation.
- The CF ops map to `AppliedOp::Rebuild { sheet }` (`op_of`, ~line 3740), so `apply_cache_refresh`
  rebuilds that sheet's render cache with `cf = has_cond_fmt(sheet) == true`, calling
  `extended_render_style` → `get_extended_cell_style`, which reads the **stale/empty `cf_cache`** →
  returns the base style → **no CF overlay** in the published cache → grid shows nothing.
- Later, editing a **value** sets `needs_eval = true` → `doc.evaluate()` (fork `model.rs:3030`,
  which calls `self.evaluate_conditional_formatting()` at `model.rs:3082`) → `cf_cache` refreshed →
  `refresh_cf_caches_after_recompute` rebuilds the CF sheet's cache → CF finally appears. Exactly
  the reported symptom.

**Why it slipped through review:** P3's tests only asserted the **value-change flips** the cached
style; no test asserted **"adding a rule (no value change) applies CF to the published cache."** The
P3 CR's "first-CF-rule batch is covered by the cf=true rebuild" reasoning was wrong — it assumed
`cf_cache` was fresh at rebuild time, but nothing evaluates CF for a `StyleOnly` op under pause.

**Suggested fix (for the coding agent — verify before implementing):** a CF mutation **does** require
CF re-evaluation (values unchanged, but the CF results depend on the new/changed/removed rule). Make
each CF command force a CF re-eval so the model's `cf_cache` is fresh **before** the
`AppliedOp::Rebuild` cache refresh reads it. Options:
- **(preferred, targeted)** add `WorkbookDocument::evaluate_conditional_formatting()` wrapping the
  fork's public `Model::evaluate_conditional_formatting()`, and call it in `apply_one` right after
  each CF mutation (CF-only re-eval; avoids a full-workbook value recompute for a formatting op).
  Confirm the ordering so the CF-eval runs before `apply_cache_refresh` rebuilds the style cache.
- **(minimal)** mark CF commands `needs_eval = true` so the existing coalesced `doc.evaluate()` runs
  (refreshes `cf_cache`) and the existing `refresh_cf_caches_after_recompute` fires. Simpler but does
  a full value recompute on every CF add/edit/delete/reorder — acceptable (user-driven, infrequent)
  but heavier than (preferred).

**Required regression test (the exact missing coverage):** a worker-seam test that sets cell values,
then sends `AddCondFmt` (a `> N` fill rule) **with no subsequent value edit**, and asserts the
**published render cache** (`DocumentClient::caches()` / the SheetCache RenderStyle) shows the fill on
the matching cell — i.e. CF applies on add. Add analogous coverage for delete (fill removed) and
update. Then re-verify in the live app.

**Also unblocks:** P10 render validation (its render scenes drive this same worker path — correct CF
baselines can't be generated until BUG-1 is fixed). ✅ Now unblocked.

</details>

---

## BUG-2 — CF not loading from `demospreadsheet.xlsx` — INVESTIGATED (no code fix; needs file re-share to confirm)

**Reported:** "not loading conditional formatting in attached spreadsheet. Works in Numbers, but not
FreeCell. Others work, so loading works, just this one."

**Severity:** Medium (file-specific load fidelity) — but see finding.

**Investigation (raw xlsx inspection, 2026-07-17):** unzipped the attached
`e80e7207-demospreadsheet.xlsx` and scanned every worksheet part. **The file contains no
conditional-formatting markup at all:**
- All 4 sheets: `legacy <conditionalFormatting>=0`, `cfRule=0` (catches both legacy and `x14:cfRule`),
  `<extLst>=0`, `x14:conditionalFormatting=0`.
- `xl/styles.xml` has no `<dxfs>` differential-format block (CF fills/fonts live in `<dxfs>`).
- The workbook does contain charts (`chart1..5`) + drawings — so it's a real, non-trivial file; it
  simply carries **zero** CF in the OOXML.

**Conclusion:** there is nothing for FreeCell (or IronCalc, or any OOXML reader) to load — the CF is
absent from the xlsx. Most likely **Apple Numbers did not export its conditional formatting into the
xlsx** (Numbers CF→OOXML export is known-lossy): the user sees CF in the Numbers-native document, but
the exported `.xlsx` lost it. A less-likely alternative is that a different file was intended. Either
way this is **not a FreeCell load defect** reproducible from this file, so there is **no code change
to make**.

**Disposition:** BLOCKED on evidence, not on code. To turn this into an actionable bug we need a file
that actually contains `<conditionalFormatting>`/`<dxfs>` markup and still fails to load in FreeCell.
Action item for the owner: re-share the file (uploads are ephemeral and this one is already gone), or
confirm whether the CF only exists in the `.numbers` original. If a re-shared xlsx *does* contain CF
markup and still doesn't load, reopen as a real load bug (then chase IronCalc's xlsx CF reader).

---

## BUG-3 — Sidebar rule list should show only rules intersecting the current selection — OPEN

**Reported:** "the sidebar list should only show rules in the list that intersect the currently
selected cell(s). Large sheets can have hundreds."

**Severity:** Medium (UX / scalability of the list) — feature refinement, not a correctness defect.

**Current behavior:** `render_cf_list` (in `app/crates/freecell-app/src/chrome/view.rs`) builds one row
per rule returned by `client.cond_fmt_rules(sheet)` — i.e. **every** rule on the sheet, regardless of
selection. On a sheet with hundreds of rules the list is unusable.

**Desired behavior:** the list shows only rules whose target range **intersects the current
selection**; it re-filters live as the selection changes.

**Design notes for the coding agent (verify against the code before implementing):**
- `CfRuleView.range` is the rule's sqref string and **may be a multi-area address** (space-separated,
  e.g. `"A1:A10 C1:C10"`). Parse all sub-areas and test intersection against the current selection
  (also potentially multi-area). Reuse the existing A1/range parsing + rectangle-intersection helpers
  in `freecell-core` rather than hand-rolling — find them first (grid selection already parses ranges).
- **Preserve the true engine index.** Raise/Lower/Update/Delete operate on `CfRuleView.index`, which is
  the rule's position in the *full* sheet list. Filtering is **display-only** — never renumber; keep
  passing the real `index` to the worker commands so reorder/edit/delete still target the right rule.
- **Refresh on selection change.** Today `on_selection_changed` (~view.rs:772) deliberately does **not**
  close the CF sidebar (good), but it also doesn't re-render the filtered list. It must now trigger a
  re-render / `cx.notify()` so the list tracks the selection. Keep the existing "selection change does
  not close the sidebar" behavior.
- **Empty state.** When no rule intersects the selection, show a short empty message (e.g. "No rules
  apply to the selected cells") plus the existing **Add rule** button — adding must still work
  (a new rule's range defaults from the current selection, as today).
- **Priority-reorder semantics under a filtered view:** CF priority is sheet-global, so raising a rule
  that's shown (filtered) still reorders it against hidden rules. That's acceptable for the first pass;
  note it, don't try to make priority selection-relative.

**Tests:** view test(s) — with two rules on a sheet (one intersecting the selection, one not), the list
renders only the intersecting row; changing the selection to the other rule's range swaps which row
shows; empty selection intersection shows the empty state + Add button; a filtered row's Delete/reorder
still sends the command with the correct original index.
