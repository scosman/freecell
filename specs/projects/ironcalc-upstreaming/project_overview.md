---
status: draft
---

# IronCalc Upstreaming

Become a proper OSS client of IronCalc: take the workarounds ("hacks") FreeCell
currently carries for IronCalc 0.7.1 bugs and API gaps, fix them at the source in a
fork, validate the fixes in the real FreeCell app, and — with owner sign-off — submit
them upstream as PRs. The end state is a FreeCell that carries the *minimum* engine-
compensation code, ideally deleting the import fix-up modules entirely.

## Fork

`https://github.com/scosman/ironcalc` — our fork of `ironcalc/IronCalc` (a Cargo
workspace: `base` → `ironcalc_base`, `xlsx`/umbrella → `ironcalc`; dual MIT/Apache-2.0).
FreeCell currently pins `ironcalc = "=0.7.1"` / `ironcalc_base = "=0.7.1"` from crates.io
(0.7.1 is the newest release — there is no version bump that fixes these).

## What we're doing

- **Plan a patch order.** Rank the fixes by value/effort and dependency (hard open-failures
  first, then the correctness bugs, then the API-visibility cleanups, then enhancements).
- **Per issue: a patch on its own branch, with tests.** Each fix is an isolated, independently
  submittable commit/branch in the fork, carrying upstream-style tests. Each is its own step
  in the implementation plan.
- **Merge all patches to a target branch** we can build FreeCell against for validation
  (decide: track upstream `main`, or a dedicated integration branch — this project makes the
  call).
- **Point FreeCell at the fork** (via crate dep) and **remove the local hacks** the fixes make
  redundant (`open_fixups`, `open_repair`, and the shims the API-visibility fixes retire),
  plus their extra deps (`roxmltree`, `zip`, and the `serde_json`-for-private-types usage).
- **One human validation pass.** Owner confirms every fix works in the real app and that
  FreeCell behaves correctly with the hacks removed (open the mortgage / Numbers fixtures,
  eyeball the previously-wrong rendering).
- **On sign-off, open the upstream PRs** — one per fix, each with its minimal repro and tests.

## Scope

The workarounds and their "upstream vs. keep-ours" verdicts are already catalogued (this
session + `GAPS.md` E1–E5 + the Round-3 API audit). In scope for upstreaming are the items
whose ideal home is IronCalc core:

- **Import correctness bugs** (delete our fix-ups when landed): E4 missing-`xfId` hard open
  failure; E2/E3 wrong built-in `numFmtId` table (`#VALUE!` on valid numbers); E1 theme
  palette ignored; E5 `<indexedColors>` override ignored; the `hue_to_rgb` tint-math overflow.
- **Public-API visibility gaps** (retire our shims): un-nameable `Clipboard`, constructor-less
  `BorderArea`, private `Function` enum module, missing workbook-default-font accessor.
- **Robustness/enhancement** (we keep our guard either way): the missing parser recursion
  depth cap (uncatchable SIGABRT); band fast-paths for range ops; minor missing APIs
  (CSV dry-run dims, format-code classification).

Out of scope (kept FreeCell-side by design, or too large / product decisions, not upstreamed
here): the date-format heuristic and format-color→RGB mapping (legitimately consumer-side),
zoom/hidden-column view state, and the big features (merged-cell write API, conditional
formatting, dynamic arrays) — those stay on the backlog with their own designs.

## Constraints & non-negotiables

- **The fork is a bridge, not a home.** Each patch is a rebased-on-`main` topic branch, one
  commit per upstream PR, so it's independently submittable and droppable as it merges.
- **Nothing is submitted upstream without explicit owner sign-off.**
- **FreeCell must stay green throughout** — the dep swap + hack removal is gated on the human
  validation pass; the app must build, test, and render correctly with the compensation code
  gone.
- Prefer Cargo's `[patch.crates-io]` → fork git branch for the FreeCell-side wiring (lowest
  churn, cleanest removal) over a submodule; revisit only if lockstep engine/app co-dev is
  needed.
