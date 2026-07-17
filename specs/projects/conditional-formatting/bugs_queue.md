# Conditional Formatting — bug queue (fix at end of first pass)

Bugs found by the requester while **using the running app**, to be **batch-fixed at the end** of
the first pass (owner directive 2026-07-17). Each fix is delegated to a coding sub-agent + code
review (manager stays out of the code). Root-cause analysis here is for handoff.

Status legend: `OPEN` → `FIXING` → `FIXED` (commit).

---

## BUG-1 — CF does not apply when a rule is added; only applies after a value change  — OPEN

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
baselines can't be generated until BUG-1 is fixed).
