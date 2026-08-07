---
status: complete
---

# chrome-view-split

Break up `app/crates/freecell-app/src/chrome/view.rs` — a 16,099-line file holding one
struct with 71 fields and eight independent feature domains — into a `chrome/view/`
module directory. Mechanical, behaviour-preserving, no visibility changes.

**Plan of record:**
[`projects/architecture-review-remediation.md`](../../../projects/architecture-review-remediation.md)
§F1.
**Underlying review:** `reviews/projects/codebase-architecture-review/phase_1_feedback.md`
(structure + growth history) and `phase_3_feedback.md` (the UI-side view).

## Motivation

`chrome/view.rs` is 16,099 lines: **8,249 production** + ~7,851 inline test. That is ~30% of
`freecell-app`'s production code in one file. It contains a single `ChromeView` struct with
**71 fields** and **267 methods** (580 across both `impl` blocks), 35 `render_*` methods and
91 `cx.listener` closures, spanning at least eight independent feature domains:
conditional-formatting sidebar and rule editor (~1,510 lines), formatting (~1,450), sheet
tabs (~1,350), edit/formula (~990), charts (~790), plus find, stats and shell concerns.

The growth curve is the real finding: **5,618 → 6,281 → 7,425 → 9,365 → 10,718 → 13,865 →
15,472 → 16,099** across 34 commits in sixteen days, monotonically, with exactly **one**
net-negative commit. Nothing in the codebase resists file growth.

Two things to hold in mind:

1. **This is the loudest problem and one of the least dangerous.** It costs velocity and
   bus factor, not users. The review explicitly ranked it outside the top five most dangerous
   findings. Do it well, but do not let it expand.
2. **It is mechanical.** Rust's module-descendant privacy means child modules under
   `chrome/view/` can see the parent's private fields with **zero visibility changes** to any
   of the 71 fields. Nothing structural is blocking the split, which is why it never happened
   — nothing forced it either.

## Confirm before building

The measurements above come from a review agent that read the file but did not compile it.
Re-measure at HEAD before planning the split: production vs. `#[cfg(test)]` line counts,
where the test module starts, the real field and method counts, and the actual domain
boundaries. If the file has already moved materially, re-plan against what's there.

Also worth checking: the codebase already extracted two state types out of this file in the
past. Find them and follow the pattern that worked rather than inventing a new one.

## Scope

- Split `chrome/view.rs` into `chrome/view/` child modules, one per feature domain.
- Move inline tests with the code they test.
- **Behaviour-preserving.** No refactoring of logic, no state-model changes, no field
  consolidation. If you find a bug, write it down; do not fix it here — a mixed diff makes
  this unreviewable, and that is the whole risk of the unit.
- Keep `ChromeView` as one struct. Splitting the *state* is E2's job (a separate v1.0
  project) and doing it here would blow the scope.

**Target:** every resulting file under **2,000 production lines** — the ceiling that
`v05-cleanup-2` will enforce in CI (F2). If a domain can't get under it cleanly, say so and
propose either a deeper split or an explicit listed exemption; do not silently leave a file
over the line, and do not contort the structure to hit the number.

## Out of scope

- `worker/run.rs` and its chart extraction — that half of the original F1 unit belongs to the
  parallel **engine-worker-hardening** project, which owns that file.
- `grid/view.rs` (10,627 lines) — 60% of the way to the same problem, but not this round.
  Note where it stands and file it if it warrants its own unit.
- Any change to UI state ownership, mirrored fields, or the `Rc<OnceCell<WeakEntity>>`
  wiring — that is E2.
- The CI line-count ceiling itself (F2, next round).

## Working agreement

- **Process — follow the `/spec` loop exactly as defined. Do not improvise a faster one.**
  `/spec implement` puts you in a **manager** role: per phase you spawn a coding sub-agent,
  validate its attestation, spawn a *fresh* CR sub-agent, route feedback back and re-review
  until clean, resume the coding agent to commit, then verify with `git status`. Every code
  change — CR fixes included — needs a clean CR before commit. Do not write the code or run
  the review inline yourself, and do not skip the CR loop because a change looks small — a
  mechanical move is exactly the case where an unreviewed "obvious" edit slips through.
  "Autonomous" below means **no human sign-off**; it does not mean a shortened process.
- **Autonomy:** run the full spec + implement flow in one pass. Do not stop for sign-off
  between spec phases. Ask only if a real unknown surfaces — for this project, the plausible
  one is a domain that genuinely cannot be separated without a state change. If you hit that,
  say so rather than quietly starting E2.
- **Verification is the whole game here.** A behaviour-preserving split is only worth doing if
  you can show it preserved behaviour: crate-scoped `cargo build -p freecell-app` +
  `cargo test -p freecell-app --lib` must stay green at every phase, and the test count must
  not drop. `cargo fmt --all --check` (whole workspace) always.
- **Render tests:** `chrome/view.rs` covers the action row, data/formula row, sheet tabs and
  the CF sidebar — **none of which have pixel baselines** (per `CLAUDE.md` §Render tests
  scope). A pure move should not touch grid/cell/sheet or titlebar pixels, so **do not run the
  full pixel suite for this project.** Validate with the crate's gpui view tests plus an Xvfb
  smoke launch (`xvfb-run -a cargo run -p freecell-app`). If a phase unexpectedly touches
  grid-render code, stop and reconsider — that means it isn't a pure move.
- Commit per domain moved, not one giant commit. Push regularly.
