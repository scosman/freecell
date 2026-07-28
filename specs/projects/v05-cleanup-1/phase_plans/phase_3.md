# Phase 3 — A4: Correct the fork docs; state the real fork strategy

**Verdict: CONFIRMED (the docs were stale and in places wrong) — and the review's own claim
about upstreaming is disproved with evidence.**

## Method

The unit says to establish the true state *from the fork's history*, not from any status table.
Read-only access turned out to need no ceremony: a plain

```
git clone --bare --filter=blob:none https://github.com/scosman/ironcalc
git remote add upstream https://github.com/ironcalc/IronCalc && git fetch upstream
```

works through the container's outbound proxy — no `add_repo`, no git-proxy URL. (Recorded in
CLAUDE.md, since the existing "autonomous-run gotchas" note only covers the push case.)

Classification method, chosen after a first attempt went wrong:

1. `git log --first-parent` on `freecell-fixes` → the fix branches, one merge each.
2. Each merge's **second parent** is the fix branch head.
3. `git cherry upstream/main <head> <merge-base>` → patch-id equivalence.

**Subject matching is not reliable and I got two rows wrong with it.** It missed
`fix/xlsx-bool-import` (upstream reworded/rewrote; patch-id found it at `13fb8f4b`), and content
probes were worse still — grepping upstream for guessed identifiers produced false negatives
(`split_whitespace` for TRIM) and false positives (`merge_cells` matching an unrelated field).
Patch-id is the only method here that is both cheap and sound; the status table now records it so
the next person re-derives instead of trusting.

## The inventory

Verified against fork `freecell-fixes` @ `cee2859d` (the pinned SHA) and upstream `main` @
`2e2465c`. **11 changes: 6 merged upstream, 5 fork-only.**

Merged upstream: E2 num-fmt table (`5b98252`), E5 negative-indexed-colors guard (`e481dc6`),
`set_worksheet_index` (`2f53937`), `xsd:boolean` import forms (`13fb8f4b`), TRIM internal runs
(`2e2465c`), ADDRESS empty-sheet prefix (`2b8672a`).

Fork-only: `set_user_inputs`, merged cells (5 commits, adds `base/src/merge_cells.rs`),
DOLLAR negative-zero, XMATCH array constants, frozen-pane structural-edit tracking.

### The review's claim, disproved

> "The review's further claim that none of these are upstreamed is wrong."

The overview already said so; this phase supplies the evidence. Upstream `main`'s **tip commit**
is `2e2465c Fix TRIM to collapse internal runs of spaces (Excel-compatible)` — one of ours. Five
more are in its history. More than half the delta has been upstreamed.

### Two live discrepancies the rebuild surfaced

These were not in the unit brief; they fell out of doing the inventory properly.

1. **Upstream merged our API and then renamed it.** `set_worksheet_index` → `move_sheet`
   (`7ca43c7`). `freecell-engine/src/document.rs:1514` still calls the old name, so **the next
   re-sync of the fork onto upstream `main` breaks FreeCell's build at that call site.** One-line
   fix, but it should be expected rather than discovered at 2am.
2. **The fork's `main` mirror is 99 commits behind upstream** (`cedba4e`). The re-sync the
   operating model calls for is overdue.

(The third — `freecell-fixes`' tip having moved past the pin and reverted a fix `document.rs`
asserts — was found in Phase 1 and is recorded there and in the status table.)

## What changed

| Document | Change |
|---|---|
| `specs/projects/ironcalc-upstreaming/implementation_plan.md` | Status table **rebuilt**. The old one listed 2 fixes, both "⏳ awaiting sign-off", and was ~3 months stale. New table: 11 rows, per-fix upstream status with the upstream SHA, the re-derivation method, and the two discrepancies. Also replaced the "Optional optimisation" section, which framed the released pin as the goal. |
| `projects/ironcalc-upgrade.md` | **Rewritten and retitled** — was *"IronCalc — move to a released pin"*, `Status: Future`, and read as a plan to get off the fork. Now *"keeping the fork current"*, `Status: Ongoing`, with the standing loop and a "what to expect when re-syncing" section grounded in the real drift found above. The released-pin idea survives as an explicitly hypothetical simplification. |
| `CLAUDE.md` §Engine | Added the permanent-fork statement up front; corrected the pin description (`rev`, not branch) and the bump procedure; noted that upstream may *reshape* what it merges, with the rename as the example; pointed at the status table with "don't trust a stale summary". Also recorded the read-only clone route. |
| `app/Cargo.toml` | The `[patch.crates-io]` comment no longer names two fixes and no longer says "TEMPORARY: revert to a released crates.io pin once the fixes ship". It states the permanent-fork position, gives the 6-merged/5-fork-only headline, and **points at the table rather than duplicating it** — this comment has been wrong about the branch contents before, and an inlined list is how that happens. |

## Judgement calls

- **The manifest comment does not inline the inventory.** The unit asked for it to be "corrected
  to match". Reproducing an 11-row table in a `Cargo.toml` comment recreates exactly the failure
  being fixed — two copies, one of which rots. It carries the headline and a pointer.
- **`CLAUDE.md` §Engine was mostly right already.** It already said "this is the standing way of
  working, not a one-off". The edits are additive; the only thing removed was the implication in
  *"the sum of our not-yet-upstreamed fixes"* that the branch is a queue that drains to empty.
- **No upstream PR status beyond merged/not-merged.** Distinguishing "open PR" from "never
  submitted" needs the GitHub API against `ironcalc/IronCalc`, which is outside this session's
  repository scope. The table says merged or fork-only and does not guess at PR state.

## Verification

Docs-only. `cargo metadata --locked` clean after the manifest comment edit (comments only — the
resolve is untouched, and A1's pin is unchanged).
