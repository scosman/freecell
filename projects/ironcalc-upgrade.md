# IronCalc — keeping the fork current

**Status: Ongoing (standing maintenance).** Companion to
`specs/projects/ironcalc-upstreaming`.

> **Rewritten 2026-07-28 (v05-cleanup-1 / unit A4).** This document used to be titled *"move to a
> released pin"* and framed the fork as a temporary state with an exit. That is not the strategy.
> **We keep the fork permanently.** We keep upstreaming fixes as clean single-fix PRs, and we keep
> re-syncing the fork from upstream `main`. Riding `freecell-fixes` is the normal operating
> position, not a stopgap.

## Why the fork is permanent

Not every fix we need is one upstream will take, and not every capability we need is a fix at all.
Of the eleven changes `freecell-fixes` carries, six are merged upstream and five are fork-only —
including **merged cells** (a whole feature, `base/src/merge_cells.rs`) and
**`UserModel::set_user_inputs`** (the batched single-undo write the paste / Replace-All path
depends on). Even if upstream took every one of them tomorrow, the *next* thing FreeCell needs
would restart the cycle. The fork is how we ship without waiting on someone else's roadmap;
upstreaming is how we avoid paying to carry things forever.

Full per-fix inventory + upstream status:
[`specs/projects/ironcalc-upstreaming/implementation_plan.md`](../specs/projects/ironcalc-upstreaming/implementation_plan.md)
§Status table.

## The standing loop

1. **A fix or capability is needed** → one `fix/<slug>` branch off the fork's `main`, with
   upstream-style tests. One fix = one branch = one focused upstream PR. Never bundle.
2. **Merge it into `freecell-fixes`** (the integration branch FreeCell builds against) and open the
   upstream PR from the `fix/` branch.
3. **Bump FreeCell's pin.** `app/Cargo.toml` pins `rev = "<sha>"`, not a branch (v05-cleanup-1/A1).
   Bumping is a deliberate one-line edit of both revs + `cargo update -p ironcalc -p ironcalc_base`,
   in its own commit, with the workspace tests run.
4. **Periodically re-sync** the fork's `main` from upstream, rebase `fix/*` + `freecell-fixes` onto
   it, and drop any `fix/` branch upstream has merged.

## What to expect when re-syncing

Re-syncing is where the cost of the fork actually lands. It is ordinary, not alarming:

- **Merged fixes disappear from the fork's delta.** E2 (`fix/e2-numfmt`) is the worked example — it
  is upstream as `5b98252` and no longer shows up as a fork-only merge.
- **Upstream may reshape what it merged.** It took our `set_worksheet_index` and then renamed it to
  `move_sheet` (`7ca43c7`). `freecell-engine/src/document.rs:1514` still calls the old name, so the
  next re-sync breaks that call site. This is the "incidental drift to reconcile on the FreeCell
  side" that CLAUDE.md §Engine warns about — expect a handful per sync, and fix them in FreeCell
  rather than pinning the fork away from upstream.
- **The fork's `main` mirror goes stale.** As of 2026-07-28 it is at `cedba4e`, **99 commits
  behind** upstream `main`. A sync is overdue.
- **`freecell-fixes` can move under you.** Its tip is currently two commits past the pinned SHA and
  those commits *revert* `fix/dollar-negative-zero`, which `document.rs:2186` asserts. The rev pin
  means nothing changes until someone chooses to bump — reconciling that is part of the bump.

## The optional simplification (not a goal)

If a released IronCalc ever contains *every* change `freecell-fixes` carries, the
`[patch.crates-io]` stanza could be dropped for a plain crates.io pin. Given that the fork carries
a feature upstream does not have, that day is hypothetical. Treat it as a simplification available
if it arrives — never as a milestone to plan toward, and never as a reason to avoid forking when
FreeCell needs something.

Coordinate with `projects/style-cache.md` (the resident style/geometry cache reads fill/font colours
through the same resolved-`Color` path).
