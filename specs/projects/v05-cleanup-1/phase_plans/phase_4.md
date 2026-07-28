# Phase 4 — D1: Run the render gate weekly on `main` and at release

**Verdict: CONFIRMED — and the review's statistics check out exactly.**

## Confirmation

`render.yml` was `on: workflow_dispatch:` only, while its own header said it "MUST be a required
status check". Those two cannot both be true: a dispatch-only workflow reports no context on a PR,
so requiring it would block every merge rather than gate anything.

The review's run statistics were verified against the Actions API rather than taken on trust,
because one of them (the flake rate) determines whether blocking a release on the suite is
defensible:

| Claim | Measured |
|---|---|
| 29 dispatched runs | 29, **all** `workflow_dispatch` |
| across 13 branches | 13 distinct `head_branch` values |
| **zero on `main`, ever** | **0** |
| failing ~7% of runs | 26 success, 2 failure, 1 cancelled |

So `main` has never been render-verified, against 44 merged PRs. Confirmed.

### The flake rate is not flake

The two failures were on the **same branch** (`claude/charts-spec-implement-dcdypq`) on the **same
day**, mid-development of a chart-rendering change, followed by successes on that same branch.
That is the gate doing its job on genuinely-moved pixels — not lavapipe misbehaving. **Zero
infrastructure flake in 29 runs.**

This is the fact the design hinged on. Had the 7% been real driver flake, blocking a release on the
suite would have meant a coin-flip release process, and the right answer would have been an
advisory job with a loud summary. It isn't, so the gate blocks.

## What changed

### Triggers

```yaml
on:
  workflow_dispatch:      # unchanged — agent-driven confirmation on a branch
  workflow_call:          # invoked by release.yml
  schedule:
    - cron: "17 7 * * 0"  # weekly on main
```

- **`schedule`** runs on the default branch by definition, which is precisely the hole. Sunday
  07:17 UTC — off-peak, and deliberately off the hour, because GitHub's scheduler is heavily
  contended at `:00` and delays runs.
- **`workflow_call`** over the alternatives. `on: release: types: [published]` was rejected because
  it fires *after* publication — that is observation, not a gate. A duplicated job in `release.yml`
  was rejected as a second copy of the suite to keep in sync. `workflow_call` makes the render
  result a visible dependency of the release run itself.

### The release gate is a hard `needs:`

`release.yml` gains a `render` job (`uses: ./.github/workflows/render.yml`) and **all three**
packaging jobs (`macos`, `linux`, `windows`) declare `needs: render`. A red pixel diff therefore
produces **no artifacts at all**, rather than artifacts plus a red X somebody is expected to notice.

It costs the suite's wall-clock at the front of a release. On an infrequent, deliberate operation
that is the right trade, and it is stated in both headers so it is not a surprise.

### Concurrency — the bug this nearly introduced, and the one it first introduced

The existing key was `render-${{ github.ref }}` with `cancel-in-progress: true`. Adding a schedule
would have made a manual dispatch on `main` and the weekly scheduled run **share a group and cancel
each other** — silently eating the backstop the phase exists to add.

The first fix, `render-${{ github.event_name }}-${{ github.ref }}`, was **not sufficient**, and the
reasoning behind it was wrong. Inside a **reusable** workflow the `github` context is the
**CALLER's**: on the `workflow_call` path `github.event_name` is whatever triggered `release.yml`
(`push` for a tag, `workflow_dispatch` for a manual release) — it is *never* the string
`workflow_call`. So a release dispatched on `main` produced the same group as a direct render
dispatch on `main`, and with `cancel-in-progress: true` one killed the other — also overriding
`release.yml`'s deliberate `cancel-in-progress: false`. The key now also carries the caller's
workflow identity:

```yaml
group: render-${{ github.workflow }}-${{ github.event_name }}-${{ github.ref }}
```

`github.workflow` is `render` on the direct paths and `release` on the called path, which is what
actually separates the two. Resulting groups: schedule-on-`main`, dispatch-on-`main`,
release-by-tag (`refs/tags/vX.Y.Z`), and release-by-dispatch are four distinct groups; two
dispatches on the same branch still share one and supersede each other, which is what an iterating
agent wants.

The one combination that shares a group — two *manual* release dispatches of the same ref — was
first written up as "a collision left by construction, accepted". Review corrected that: it is
**unreachable**, because `release.yml` sets `concurrency: release-${{ github.ref }}` with
`cancel-in-progress: false`, and a run entering an occupied group under that setting goes
**pending** — starting no jobs at all — so the second release's render job never launches and
there is nothing to cancel. The header now says which mechanism makes it unreachable (and that
deleting `release.yml`'s group would make it real), which is the useful thing to record.

### Least privilege

`render.yml` declares `permissions: contents: read` at workflow level. This is stated as what the
**job** needs, not as a claim about the repo-wide default (which a workflow cannot see): if that
default is read/write, the block genuinely narrows the token — `contents: read`, every other scope
`none`. Either way no step here needs more (`actions/checkout` needs `contents: read`;
`Swatinem/rust-cache` and `actions/upload-artifact` use the runtime token, not `GITHUB_TOKEN`).
The point is that a workflow now embedded in the **release** path states its scope explicitly, so
a future default change or a broader-permissioned caller can't widen it silently. Valid on the
`workflow_call` path too: a called workflow may only *downgrade* the caller's token, and
`release.yml` sets no `permissions:` block.

## Docs corrected in the same phase

The first pass of this sweep was **not complete** — it corrected the two workflow headers and
`app/README.md` but missed two documents that still asserted exactly what §D1's "Done when" says
must be gone from the repo. Both are now fixed:

- **`checks.yml` header** claimed the job covers the render suite. It does not: `cargo test
  --workspace` compiles `render-tests` and runs its GPUI-free unit tests, but every pixel case
  self-skips without `FREECELL_RENDER`. Rewritten to say so explicitly, and to name where the pixel
  suite actually runs. (Its stale "documented GPL ztracing exception" phrase was corrected at the
  same time — the exception is gone, per Phase 2.)
- **`app/README.md` §CI** listed "the render suite (Xvfb + lavapipe)" under `checks`. Corrected, and
  `render` now has its own entry describing all three triggers. The `roundtrip` workflow, which
  exists but had never been listed, was added at the same time.
- **`app/render-tests/README.md`** (missed on the first pass) still called the gate "a required step
  in `checks.yml`". Corrected to the real mechanism: `checks` compiles the crate and runs its
  GPUI-free unit tests but diffs no pixels; the gate is `render.yml` under its three triggers.
- **`specs/projects/mvp/architecture.md` §9** (missed on the first pass) still described render as
  `workflow_dispatch`-only, listed `checks`/`render`/`perf-gates` as all-required status checks, and
  instructed that render be wired into branch protection under context `render (Xvfb + lavapipe)`.
  This mattered more than the others: **both `render.yml` and `checks.yml` cite "architecture.md §9"
  as their authority**, so the workflows were pointing at a spec that contradicted them. Updated to
  post-D1 reality with a dated note recording that D1 changed it, and the `workflow_dispatch`
  bootstrap caveat extended to say a branch dispatch validates steps/env but exercises neither
  `schedule` nor `workflow_call`. Unrelated parts of that document were left alone.
- **`render.yml` header** no longer claims to be a required status check, and explains why it is
  off the PR path, with the measured flake data behind the release gate.
- **`CLAUDE.md` "Render tests"** — the section's *scope* and *cost* guidance was accurate and is the
  genuinely useful part, so it was kept verbatim. What changed is the framing: "a **manual** gate …
  there is no safety net" became a table of the three triggers plus an explicit statement that
  **the weekly run does not cover you before merge** — it fires after, on someone else's watch. The
  agent's responsibility is unchanged; what changed is that forgetting no longer means *nobody*
  checked. Two follow-ups: items 1 and 3 still scoped the rule to "grid/cell/sheet or titlebar"
  while item 2 and the Scope paragraph include **chart-render** — all three now say the same thing;
  and the "a feature-branch run uses that branch's `render.yml`" bullet gained the caveat that a
  dispatch validates steps/env only, not the `schedule` or `workflow_call` triggers.

## Verification

- All six workflow files re-parse as YAML.
- The `workflow_call` contract checked against GitHub's requirements (called workflow needs
  `on: workflow_call:`; caller uses `uses: ./.github/workflows/render.yml`).
- Cron verified by hand against 5-field UTC semantics.
- **The workflow was dispatched on this branch** and accepted, proving the edited trigger block
  still parses and runs — the one part of this change that is executable pre-merge. D1 must not be
  the change that silently breaks the manual path.

  **Caveat (review, 2026-07-28): that dispatch is stale evidence for part of the change.** It is
  run #30 (`30375672745`), head SHA `e20baac` — *before* the concurrency-key and `permissions:`
  edits. `concurrency.group` and `permissions:` are evaluated at **run start**, not at parse time,
  so `yaml.safe_load` passing does not cover them; a malformed group expression produces a startup
  failure only a real dispatch catches. The bullet's literal claim still holds (the `on:` block was
  not touched after the dispatch), but a fresh dispatch is what actually validates the current
  file. One is scheduled as part of this project's render-validation phase, which covers both
  concerns in one run.

The schedule and release paths cannot be executed from here; they are verifiable only once on
`main` (schedule) and at the next tag (release).

## Not done, deliberately

Bump-token, `paths` filter, per-PR gate — all three explicitly rejected by the owner decision the
unit records.

## Reviewer findings not actioned

Recorded rather than implemented, with reasons.

- **`skip_render` escape hatch on `release.yml`'s dispatch input — rejected.** The suggestion was a
  `workflow_dispatch` boolean letting an owner package a build without waiting for the pixel suite
  (e.g. a doc-only re-tag, or a suite outage). D1 exists *precisely* to make the pixel gate
  unskippable at release; a documented bypass reintroduces exactly the hole the unit closes, and
  bypasses get used under time pressure, which is when they are least safe. The current failure mode
  is acceptable: a red or broken suite blocks packaging until a human either fixes it or edits the
  workflow — a deliberate, reviewable act rather than a checkbox. If the owner ever wants this, it
  is an owner decision to record in the unit, not an agent's call.
- **Cron slot inconsistency with `macos-verify.yml` — noted, not changed.** `render.yml` runs
  `17 7 * * 0` (off the hour, to dodge GitHub's heavily-contended `:00` scheduling queue) while
  `macos-verify.yml` runs `0 6 * * 1` (on the hour, different day). The reasoning behind the offset
  applies equally to `macos-verify`, but that file is not in this unit's scope and changing it would
  be a drive-by edit to another workflow's schedule. Worth folding into a later CI-hygiene pass.
- **Scheduled-failure notification discoverability — unresolved; a post-merge check for the owner.**
  A weekly backstop only helps if a red run is *noticed*. GitHub emails failed-scheduled-run
  notifications to the account associated with the commit that most recently changed the cron — and
  here that commit is authored `Claude <noreply@anthropic.com>`, which is not a GitHub account, so
  the notification may go nowhere. This cannot be verified from inside the container (it depends on
  the owner's GitHub notification settings and on how the commit's author maps to an account). The
  owner should confirm after merge that a failed weekly `render` run actually reaches a human —
  e.g. by watching the repo's Actions notifications, or by re-committing the cron line under their
  own identity. An unnoticed weekly failure makes the backstop exactly as ignorable as the
  never-run-on-`main` state it replaces.

---

## Review remediation (2026-07-28)

A second review round confirmed the concurrency-key fix is genuinely right — it re-derived the
reusable-workflow rule that the whole `github` context is the *caller's* (so `github.workflow` is
the discriminator that `github.event_name` provably could not be), walked all six firing
combinations, and could construct **no** reachable collision the key misses. It also confirmed the
two stale documents are fixed and that the repo-wide sweep for "render is required" now returns
only correct historical record. Everything it raised was **Mild** and documentation-precision:

| Finding | Fix |
|---|---|
| `render.yml`'s header claimed a remaining collision between two manual release dispatches of the same ref — which is unreachable, and the very next sentence explained why while presenting it as mitigation. | Header and §Concurrency above rewritten to state the mechanism (`release.yml`'s `cancel-in-progress: false` group makes the second run *pend* before any job starts) and to name the condition under which it would become real (deleting that group). |
| "Behaviourally identical to the repo default today" is unverifiable from a workflow, and probably imprecise — if the default is read/write this block genuinely narrows the token. | Both the workflow comment and §Least privilege now state the claim as *what this job needs*, enumerate why no step needs more, and drop the assertion about the repo default. |
| The live-dispatch verification is stale evidence: run #30 (`e20baac`) predates the concurrency/`permissions:` edits, and both are evaluated at run start rather than parse time. | Recorded inline in §Verification, with a fresh dispatch folded into this project's render-validation phase. |
| `specs/projects/mvp/architecture.md` §9 enumerated only four workflows while six exist — inconsistent with the same commit adding the missing `roundtrip` entry to `app/README.md`. | Added `roundtrip` and `release` entries, both checked against the workflow files (triggers, paths filter, `workflow_call` wiring, unsigned-artifact caveat). Noted `macos-verify.yml`'s own "§9 item 3" self-description as harmless pre-existing drift rather than renumbering another workflow's header. |
