# Phase 3 — A4: Correct the fork docs; state the real fork strategy

**Verdict: CONFIRMED (the docs were stale and in places wrong) — and the review's own claim
about upstreaming is disproved with evidence.**

> ### ⚠️ SUPERSEDED NUMBERS — read this first (2026-08-07)
>
> **Everything below §Method was written against the pre-A1 pin and the 2026-07-28 upstream
> state. Its counts and SHAs are stale. Do not quote them.**
>
> | This document says | Actually |
> |---|---|
> | pinned SHA `cee2859d` | **`ecbf6226`** — A1 re-pinned onto the branch tip |
> | "11 changes: 6 merged upstream, 5 fork-only" | **11 changes: 8 merged upstream, 3 fork-only** (DOLLAR counts in neither) |
> | XMATCH array constants = fork-only | **merged upstream** `54e301b3` (PR #1295, 2026-07-29) |
> | DOLLAR = fork-only | **reverted, not carried**; PR #1293 closed unmerged — the fix was wrong |
> | (no row for the font-`<name>` fix) | **merged upstream** `14790bdd` (PR #1236) — the row this pass missed |
> | "fork's `main` is 99 commits behind" | **188 commits behind** as of 2026-08-07 |
> | discrepancy #3: "tip has moved past the pin" | **resolved** by A1 |
>
> The live record is
> [`specs/projects/ironcalc-upstreaming/implementation_plan.md`](../../ironcalc-upstreaming/implementation_plan.md)
> §Status table. Why the counts moved, and why the method that produced them under-counted, is in
> **§Review remediation** at the end of this file.

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
- ~~**No upstream PR status beyond merged/not-merged.** Distinguishing "open PR" from "never
  submitted" needs the GitHub API against `ironcalc/IronCalc`, which is outside this session's
  repository scope. The table says merged or fork-only and does not guess at PR state.~~
  **WRONG — retracted 2026-08-07.** The justification does not hold: the GitHub MCP
  `search_pull_requests` tool queries `ironcalc/IronCalc` fine despite the repo being outside the
  session's scope. The table now carries PR numbers and states. See §Review remediation.

## Verification

Docs-only. `cargo metadata --locked` clean after the manifest comment edit (comments only — the
resolve is untouched, and A1's pin is unchanged).

---

## Review remediation (2026-08-07)

Code review of A4 raised five moderate items and six mild ones. **All confirmed; all fixed.**
Handled jointly with Phase 1's A1 review, which overlapped on the same fork-inventory documents.
Everything below was **re-derived from the fork and upstream**, not copied from the review brief.

### Evidence base

Read-only, no `add_repo` needed — exactly the route this phase recorded:

```
git clone --bare --filter=blob:none https://github.com/scosman/ironcalc
git remote add upstream https://github.com/ironcalc/IronCalc && git fetch upstream
```

Verified against fork `freecell-fixes` @ `ecbf6226`, fork `main` @ `cedba4ea` (2026-07-10),
upstream `main` @ `91d343c3` (2026-08-07). Upstream PR states via GitHub MCP
`search_pull_requests` (`repo:ironcalc/IronCalc author:scosman`).

### Finding 1 — the inventory missed a merged change (Moderate) · **CONFIRMED, fixed**

`fix(xlsx): preserve font <name> on import` (`14790bdd`, PR
[#1236](https://github.com/ironcalc/IronCalc/pull/1236), merged 2026-07-10) is ours, is merged
upstream, and had **no row**. Independently verified: `14790bdd` is an ancestor of both fork
`main` and `freecell-fixes`, and of `upstream/main`.

Missed for the structural reason the reviewer identified — once upstream merges a change and the
fork rebases `main` onto it, the change stops appearing as a first-parent merge on
`freecell-fixes`. E2 was rescued only by prior knowledge; #1236 was not.

Row added, marked *inherited-after-merge* like E2, and **the method now carries a mandatory
cross-check step** (`git log --author=scosman upstream/main` + the upstream PR list) so the
next re-derivation catches what enumeration cannot see.

**The reviewer's proposed headline of "7 merged" is already out of date — it is 8.** See Finding 3.

### Finding 2 — the documented method did not reproduce the table (Moderate) · **CONFIRMED, fixed**

Reproduced verbatim. `git log --first-parent --merges main..freecell-fixes` yields **11** merges:
10 fix merges plus the revert PR #2 — **including DOLLAR, excluding E2 and the font fix**. The
published set was "those ten, minus DOLLAR, plus E2", i.e. two of three adjustments undocumented.

Fixed by stating the **inclusion rule** explicitly at the head of the table: *every change we
authored that the pin carries, or that upstream merged and the fork re-inherited via its `main`
mirror.* The same rule now heads `app/Cargo.toml`'s comment, replacing "WHAT IS ON THE BRANCH: 10
changes at this pin" — which the reviewer correctly flagged as incoherent, since E2 is not
carried on the branch in any sense that E1/E4/tint aren't.

### Finding 3 — "fork-only" asserted where "open PR" was the truth (Moderate) · **CONFIRMED, fixed**

Confirmed, and **the picture moved in the week since the review** — which is itself the argument
for querying rather than assuming. Re-queried 2026-08-07:

| Change | Review said (a week ago) | **Now** |
|---|---|---|
| XMATCH array constants | #1295 **OPEN** | **#1295 MERGED** 2026-07-29 — upstream `54e301b3`, patch-id identical to fork `f9d1f9ce` (`643cb6a8…`) |
| `set_user_inputs` | #1258 OPEN | #1258 **still OPEN** |
| frozen pane | #1290 OPEN | #1290 **still OPEN** |
| merged cells | no PR | **still no PR** — genuinely fork-only |
| DOLLAR | #1293 CLOSED unmerged | unchanged — and the closure was *correct* (Phase 1, Finding 7) |

**So the count is 8 merged / 3 fork-only, not the reviewer's 7/4.** PR numbers and states are now
table columns.

The strategy conclusion survives — merged cells alone carries it — but the reviewer was right that
`projects/ironcalc-upgrade.md`'s "Not every fix we need is one upstream will take" leaned on the
conflation. Rewritten: upstream takes **8 of our 11**, two of the remaining three have live PRs,
and exactly one (merged cells) has no PR because it is a *feature*, not a fix. The honest case for
the fork is **capability + latency**, not rejection.

### Finding 4 — `phase_3.md` still on the old numbers (Moderate) · **CONFIRMED, fixed**

Confirmed on every count. Fixed with the superseded-numbers banner at the top of this file rather
than by rewriting the body — the body is a record of what that pass did, including what it got
wrong, and rewriting it would erase the evidence for Finding 1's structural lesson.
`projects/architecture-review-remediation.md`'s two paragraphs (the A1 one at ~186 and the A4 one
at ~256) are restated.

### Finding 5 — a document still claimed the pin is temporary (Moderate) · **CONFIRMED, fixed**

`specs/projects/ironcalc-upstreaming/functional_spec.md:116` said "The git-`main` pin is temporary
and clearly marked" — precisely what §A4's Done-when forbids, and in direct contradiction of **D7**
at line 38 of the same file. Struck with a dated superseded note pointing at D7. The two
lower-grade siblings (`functional_spec.md` §5 and `architecture.md` §7, both describing a move to a
released pin "once the fixes ship" and pointing at the very document A4 retitled) got the same
treatment.

### Mild items · all **CONFIRMED**, all fixed

- **Frozen-pane row's "The `freecell-fixes` tip commit"** — false; the tip is `ecbf6226` (the
  revert merge), not `507fe6c7`. Note removed.
- **"Two live discrepancies this rebuild surfaced"** — held one live and one resolved. Renamed to
  *Drift and discrepancies*, each item marked live/resolved.
- **`implementation_plan.md` Phases 2 and 5 unreconciled with the table** — confirmed and worse
  than reported: Phase 2's cited `1c2c477` and `48b0b23` **do not exist in the fork at all** (both
  `git cat-file` as unknown revisions — pre-rebase SHAs). Replaced with the live SHAs. Phase 5 was
  an unticked "open one PR per fix (E2, E5)" months after both merged; ticked, with #1223/#1224.
- **Merged-cells row's branch name** — real branch is `claude/merged-cells-implementation-yv1pr7`.
  Corrected.
- **`projects/ironcalc-upgrade.md`'s orphaned "Coordinate with `projects/style-cache.md`"** —
  left over from deleted content, referring to nothing in the rewritten document. Deleted.
- **`CLAUDE.md:59` at 110 chars in a ~100-char file** — rewrapped.

### Found during remediation (not in either review)

1. **`fix/batch-set-inputs` has advanced past the pin.** `6f086bb9` ("make `set_user_inputs`
   atomic on mid-batch write failure", PR-feedback fix, 2026-07-17) is on the `fix/` branch and in
   open PR #1258, but is **not** on `freecell-fixes` — verified by `git merge-base --is-ancestor`
   (NO) and `git cherry` (`+`). The pin does not carry the atomicity hardening. Recorded in the
   table row and flagged for the next fork bump.
2. **The `=0.7.1` patch-attachment hazard is now live.** Upstream has released **0.8.x**; the fork
   declares `0.7.1` at the pinned rev. The overdue `main` re-sync will bump the fork's version and
   silently detach the `[patch]` (a *warning*, not an error). Recorded in `app/Cargo.toml` and
   `projects/ironcalc-upgrade.md`.
3. **The staleness is accelerating.** Fork `main` went from 99 commits behind (2026-07-28) to
   **188** (2026-08-07). Restated with both datapoints so the trend is visible, since a bare
   snapshot count reads as stable.
4. **One further change of ours is outside the table.** PR
   [#1333](https://github.com/ironcalc/IronCalc/pull/1333) (clipboard whole-multiple paste fill),
   **OPEN**, has no corresponding branch in `scosman/ironcalc` and is not carried at this pin.
   Noted below the table rather than counted, since it satisfies neither limb of the inclusion
   rule.

### Verification

Docs-only except `document.rs` (Phase 1, Finding 7) and `app/Cargo.toml` **comments only** — the
`rev` values are untouched, since a pin bump is a separate deliberate commit.

| Check | Result |
|---|---|
| `cargo test -p freecell-engine --lib` | **406 passed**, 0 failed, 1 ignored |
| `cargo clippy -p freecell-engine --all-targets -- -D warnings` | clean |
| `cargo fmt --all --check` | clean |
| `cargo metadata --locked` (from `app/`) | exits 0 |
| `git diff --stat app/Cargo.lock` | **empty — the pin did not move** |
