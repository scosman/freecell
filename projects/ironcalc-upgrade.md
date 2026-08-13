# IronCalc — keeping the fork current

**Status: Ongoing (standing maintenance).** Companion to
`specs/projects/ironcalc-upstreaming`.

> **Rewritten 2026-07-28 (v05-cleanup-1 / unit A4).** This document used to be titled *"move to a
> released pin"* and framed the fork as a temporary state with an exit. That is not the strategy.
> **We keep the fork permanently.** We keep upstreaming fixes as clean single-fix PRs, and we keep
> re-syncing the fork from upstream `main`. Riding `freecell-fixes` is the normal operating
> position, not a stopgap.

## Why the fork is permanent

**Not because upstream refuses our work — it mostly takes it.** Of the 11 changes we have
authored, **8 are merged upstream** (2026-08-07) and only 3 are fork-only; of those 3, two have
*open* upstream PRs ([#1258](https://github.com/ironcalc/IronCalc/pull/1258),
[#1290](https://github.com/ironcalc/IronCalc/pull/1290)) and may well land too. Exactly one has no
upstream PR at all: **merged cells** — a whole feature (`base/src/merge_cells.rs`), not a fix.

So the honest case for the fork is not "upstream won't take our fixes". It is twofold:

- **Not every capability we need is a fix at all.** Merged cells is a feature upstream does not
  have and has not asked for. That single row is enough to make the fork permanent on its own.
- **Latency, not rejection.** Even a fix upstream *will* take lands on upstream's schedule, not
  ours — `set_user_inputs` (the batched single-undo write the paste / Replace-All path depends on)
  has been open since 2026-07-12. FreeCell ships against `freecell-fixes` the day the fix works;
  upstreaming then stops us paying to carry it forever.

Even if upstream took every one of them tomorrow, the *next* thing FreeCell needs would restart
the cycle.

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
- **The fork's `main` mirror goes stale.** As of 2026-08-07 it is at `cedba4ea` (2026-07-10),
  **188 commits behind** upstream `main` — it was 99 behind ten days earlier, so the gap widens
  fast. A sync is overdue.
- **A stale mirror also freezes the crate version.** The fork declares `0.7.1` while upstream has
  released **0.8.x**. `app/Cargo.toml`'s `[workspace.dependencies] ironcalc = "=0.7.1"` is what
  the `[patch]` attaches to, so the re-sync that bumps the fork to 0.8 must move that requirement
  in the same commit. Get it wrong and cargo only *warns* ("patch … was not used in the crate
  graph"), silently resolves the real crates.io crate, and fails much later on a missing fork API.
- **Sometimes *we* are the ones who were wrong.** `fix/dollar-negative-zero` was reverted on
  `freecell-fixes` (PR #2) and closed unmerged upstream
  ([#1293](https://github.com/ironcalc/IronCalc/pull/1293)) — because the "fix" was incorrect.
  **Excel returns `($0.00)`** for `=DOLLAR(-0.001,2)`: it picks the parenthesized form from the
  sign of the input *before* rounding. `$0.00` is Google Sheets' answer, and IronCalc targets
  Excel. FreeCell's pin moved onto the post-revert tip in v05-cleanup-1/A1, and the right response
  was to **correct** `freecell-engine::document`'s expectation to `($0.00)`, not drop the
  assertion. **This is why a bump always runs the workspace tests**: the rev pin guarantees
  nothing moves until you choose, and the suite tells you what changed when you do — a red test
  after a bump is a question to answer, not noise to silence.

## The optional simplification (not a goal)

If a released IronCalc ever contains *every* change `freecell-fixes` carries, the
`[patch.crates-io]` stanza could be dropped for a plain crates.io pin. Given that the fork carries
a feature upstream does not have, that day is hypothetical. Treat it as a simplification available
if it arrives — never as a milestone to plan toward, and never as a reason to avoid forking when
FreeCell needs something.
