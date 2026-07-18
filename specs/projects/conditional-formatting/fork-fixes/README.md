# Fork fix required for conditional formatting — BLOCKED ON PUSH

During Phase 2 of the conditional-formatting project we found a bug in **our IronCalc fork**
(`scosman/ironcalc`, the `freecell-fixes` branch FreeCell pins): the `Diff::DeleteSheet` **undo**
arm restored every worksheet field *except* `conditional_formatting`, so undoing a sheet delete
silently dropped that sheet's CF rules. Per the standing "fix the fork, don't hack FreeCell"
doctrine (`CLAUDE.md`), the fix belongs in the fork.

The one-line fix + an upstream-style regression test are captured in
[`0001-cf-undo-restore-conditional-formatting-on-sheet-delete.patch`](./0001-cf-undo-restore-conditional-formatting-on-sheet-delete.patch)
(committed on the fork locally as `1a2c63c`, on top of `freecell-fixes` HEAD `81feec4`).

## Why it's not applied yet — push blocked

**This session cannot push to `scosman/ironcalc`.** The container git-proxy returns **403 on push**
to that repo (org policy — only `scosman/freecell` is writable this session; `git push --dry-run`
to `http://127.0.0.1:<port>/git/scosman/ironcalc` → `403`). Fetch/clone work; push does not. So the
fork commit lives only in an ephemeral container clone — hence this durable patch copy.

Because CF exists **only** on `freecell-fixes` (upstream `main` has no CF code at all), a
`fix/<slug>`-off-`main` branch can't host this fix today — `freecell-fixes` is its only valid home
until CF itself is upstreamed. (When CF lands upstream, this fix should ride along / be re-homed on
the CF `fix/` branch for the upstream PR.)

## Owner action to unblock

1. Apply the patch onto `scosman/ironcalc` `freecell-fixes` and push, e.g. from a fork checkout:
   ```
   git checkout freecell-fixes
   git am < 0001-cf-undo-restore-conditional-formatting-on-sheet-delete.patch   # (or git apply + commit)
   git push origin freecell-fixes
   ```
   (Or just re-apply the one-liner in `base/src/user_model/undo_redo.rs` — restore
   `worksheet.conditional_formatting = old_data.conditional_formatting.clone();` in the
   `Diff::DeleteSheet` undo arm — plus the test.)
2. In FreeCell: `cd app && cargo update -p ironcalc_base -p ironcalc` to re-pin onto the new
   `freecell-fixes` HEAD.
3. Remove the `#[ignore = "blocked on fork CF-undo fix …"]` from
   `undo_of_sheet_delete_repopulates_cf_map` in `app/crates/freecell-engine/src/worker/run.rs`
   (search for the ignore reason) and confirm it passes.

## What FreeCell already does (correct without the fork fix)

The FreeCell-side half of the fix **is** in place and correct on its own: on undo of a sheet
delete, the worker reconciles the **reappeared** sheets and republishes their CF rules + emits
`CondFmtUpdated`. Until the fork fix ships it simply has nothing to republish (the engine hasn't
restored the rules yet); once the fork fix is pinned, the reappeared-sheet reconcile surfaces the
restored rules and the `#[ignore]`d test goes green. No FreeCell workaround/hack was added.
