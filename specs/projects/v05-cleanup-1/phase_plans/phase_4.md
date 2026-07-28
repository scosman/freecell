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

### Concurrency — the bug this nearly introduced

The existing key was `render-${{ github.ref }}` with `cancel-in-progress: true`. Adding a schedule
would have made a manual dispatch on `main` and the weekly scheduled run **share a group and cancel
each other** — silently eating the backstop the phase exists to add. The key now includes the
event:

```yaml
group: render-${{ github.event_name }}-${{ github.ref }}
```

Two dispatches on the same branch still supersede each other, which is what an iterating agent
wants.

## Docs corrected in the same phase

- **`checks.yml` header** claimed the job covers the render suite. It does not: `cargo test
  --workspace` compiles `render-tests` and runs its GPUI-free unit tests, but every pixel case
  self-skips without `FREECELL_RENDER`. Rewritten to say so explicitly, and to name where the pixel
  suite actually runs. (Its stale "documented GPL ztracing exception" phrase was corrected at the
  same time — the exception is gone, per Phase 2.)
- **`app/README.md` §CI** listed "the render suite (Xvfb + lavapipe)" under `checks`. Corrected, and
  `render` now has its own entry describing all three triggers.
- **`render.yml` header** no longer claims to be a required status check, and explains why it is
  off the PR path, with the measured flake data behind the release gate.
- **`CLAUDE.md` "Render tests"** — the section's *scope* and *cost* guidance was accurate and is the
  genuinely useful part, so it was kept verbatim. What changed is the framing: "a **manual** gate …
  there is no safety net" became a table of the three triggers plus an explicit statement that
  **the weekly run does not cover you before merge** — it fires after, on someone else's watch. The
  agent's responsibility is unchanged; what changed is that forgetting no longer means *nobody*
  checked.

## Verification

- All six workflow files re-parse as YAML.
- The `workflow_call` contract checked against GitHub's requirements (called workflow needs
  `on: workflow_call:`; caller uses `uses: ./.github/workflows/render.yml`).
- Cron verified by hand against 5-field UTC semantics.
- **The workflow was dispatched on this branch** and accepted, proving the edited trigger block
  still parses and runs — the one part of this change that is executable pre-merge. D1 must not be
  the change that silently breaks the manual path.

The schedule and release paths cannot be executed from here; they are verifiable only once on
`main` (schedule) and at the next tag (release).

## Not done, deliberately

Bump-token, `paths` filter, per-PR gate — all three explicitly rejected by the owner decision the
unit records.
