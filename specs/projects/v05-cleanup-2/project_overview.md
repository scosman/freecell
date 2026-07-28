---
status: complete
---

# v05-cleanup-2

Second batched cleanup round closing the **v0.5** architecture-review findings. Everything
here depends on a first-round project having landed, which is the only reason it is a
separate round.

**Plan of record:**
[`projects/architecture-review-remediation.md`](../../../projects/architecture-review-remediation.md)
— unit IDs below refer to it.
**Underlying review:**
[`reviews/projects/codebase-architecture-review/`](../../../reviews/projects/codebase-architecture-review/)

## Prerequisites

Do not start this round until all three have merged:

| Needed | Provides |
|---|---|
| **v05-cleanup-1** | C1 (the part-inventory test C2 must not regress) and D1 (the render gate D2 tightens) |
| **chrome-view-split** | the `chrome/view.rs` split F2's ceiling must hold against |
| **engine-worker-hardening** | the `worker/run.rs` chart extraction, same reason |

Verify at HEAD rather than assuming — if `worker/run.rs` is still over the ceiling, F2 needs
to know before it picks a number.

## Read this first — the findings are hypotheses, not facts

The review agents read a lot of code and compiled none of it, and one unit elsewhere in this
remediation was already disproved on re-check. **Every unit starts by confirming the root
cause still exists at HEAD.** Re-derive from source; check `git log -p`; go deeper than the
review did. **A unit closed as "not a real problem, here's the evidence" is a successful
outcome** — record it, correct the remediation doc, move on. Never implement a fix for a
problem you have not personally confirmed.

## Scope — one unit per phase

### C2 — Save-fidelity warning dialog
**Depends on C1.** The most valuable single item in the whole remediation.

FreeCell's save path is lossy by default: `is_carry_part` allowlists two prefixes
(`xl/charts/`, `xl/drawings/`) over a package IronCalc regenerates from scratch, so opening a
real `.xlsx`, editing one cell and saving permanently deletes pivot tables, macros, data
validation, hyperlinks, comments, Excel Tables, autofilters, sheet protection, print setup,
images and in-cell rich text. **A warn-before-strip dialog was designed and cut on
2026-07-13** (`GAPS.md` line 451); the surviving mitigation is a `.back` file with no UI, no
menu entry and no mention in any dialog.

**Priority split — this unit is allowed to ship the first half only:**

- **P1 (required):** the warning exists at all. Saving a file that came from Excel tells the
  user, *before* it happens, that unmodelled content will be dropped. This is the shippable
  minimum.
- **P2 (may punt to v1.0):** enumerate exactly what is being dropped. `reinject` already reads
  the original zip and enumerates parts, so a raw part-name list is close at hand — but a
  *user-meaningful* list ("2 pivot tables, 14 comments, 1 macro module") is real work. If the
  part-name→user-noun mapping balloons the scope, **ship P1, file a `GAPS.md` entry for the
  enumeration, and stop.**

The structural fix (inverting the allowlist to default-carry / explicit-drop) is C3, a
separate v1.0 project. This unit makes the loss *visible*, not smaller.

→ evidence: `phase_5_feedback.md`

### D2 — Tighten pixel tolerance; add exact-match text cases
**Depends on D1.**

`fail_fraction: 0.005` permits **384–1,596** differing pixels in scenes whose entire
non-background content is 8k–25k pixels and whose glyph ink is on the order of **74 pixels**.
A wrong digit in a cell passes silently. And because `#[gpui::test]` installs a
`NoopTextSystem`, the pixel suite is the *only* place real font metrics are ever exercised —
so **nothing in the project currently validates that the right number is drawn correctly**,
which for a spreadsheet is the thing most worth validating.

Tighten for the text-centric cases (`cell_*`, `spill_*`, `autogrow_*`). lavapipe has 29 runs
of demonstrated stability, and `diff.rs`'s own comment says to tighten when it does.

**Do not force it.** If tightened thresholds prove flaky across runners or driver versions,
back off — a flaky gate is worse than a loose one. Land whatever tightening holds cleanly,
record the rest in `GAPS.md`, and stop. This is the one unit in the remediation where
partial success is the expected outcome.

→ evidence: `phase_6_feedback.md`

### F2 — Production-line ceiling in CI
**Depends on chrome-view-split and engine-worker-hardening.**

Nothing has ever resisted file growth in this repo — `chrome/view.rs` went 5,618 → 16,099 in
sixteen days with one net-negative commit. Add a CI check on per-file **production** line
count with a ceiling of **2,000**.

Count production lines only: several files are 40–50% inline `#[cfg(test)]` and that is
healthy, not debt. Verify every file actually lands under the ceiling after the two split
projects; anything that can't be split cleanly gets an **explicit, listed exemption** rather
than a raised global ceiling — the point is that growth becomes a visible decision, not that
the number is sacred.

### H3 — Update `CLAUDE.md` to prevent this class of defect
Add standing rules targeting the review's root-cause pattern, **each paired with its enforcer
rather than stated as intent**. A `CLAUDE.md` rule is itself prose, which is exactly the
failure mode — so a rule with no named enforcer must be explicitly marked as a judgment call.

Core rule: *an invariant you write down is one you enforce — `debug_assert!`, clamp, type, or
test — or you delete the claim.*

Supporting rules, each traceable to a real defect found in this review:
- every loop over sheet-derived dimensions is bounded by a constant, never by a value from a
  file or a click (B2)
- every engine call from the worker goes through the `catch_unwind` pattern, no exceptions (B1)
- a round-trip test over a workbook you authored yourself is not a fidelity test (C1)
- a doc claim about a CI gate is checked against the workflow's trigger block when written (D1)
- a new feature domain gets a module, not an append (F1)
- adding a mirrored copy of existing state requires explicit justification (E2)
- a plan is a hypothesis: confirm the root cause before implementing, and push back when it
  isn't there

Also pick up the orphan doc fix from the dropped D3 unit: `app/README.md:8`,
`specs/projects/mvp/architecture.md:53`, `functional_spec.md:21` and the `macos-verify.yml`
header all call macOS the **"primary"** platform. FreeCell is cross-platform; make them say
so.

**Keep it tight.** Only rules that changed because of this review. A wholesale rewrite would
bury the ones that matter in a document that is already long and currently wrong in several
places about what CI protects.

## Out of scope

- **H1 (invariant-enforcement sweep)** — its own project, running in parallel with this one.
  H3 writes the *rule*; H1 does the *sweep*. If H1 has already landed when you write H3, cite
  what it found.
- C3 (default-carry preservation), G2, G3, F3, D4, D5, E2 — all v1.0, specced after v0.5 lands.

## Working agreement

- **Process — follow the `/spec` loop exactly as defined. Do not improvise a faster one.**
  `/spec implement` puts you in a **manager** role: per phase you spawn a coding sub-agent,
  validate its attestation, spawn a *fresh* CR sub-agent, route feedback back and re-review
  until clean, resume the coding agent to commit, then verify with `git status`. Every code
  change — CR fixes included — needs a clean CR before commit. Do not write the code or run
  the review inline yourself, and do not skip the CR loop because a change looks small.
  "Autonomous" below means **no human sign-off**; it does not mean a shortened process.
- **Autonomy:** run the full spec + implement flow in one pass. Do not stop for sign-off
  between spec phases. Ask only if a real unknown surfaces — for this round, the plausible one
  is C2's P1/P2 boundary if the part inventory turns out to be unusable for a user-facing
  message. Decide and document rather than asking permission to proceed.
- Units are independent of each other; one phase each. A disproved or blocked unit ends with a
  written finding and the rest continue.
- Crate-scoped checks per phase; `cargo fmt --all --check` (whole workspace) always. Commit and
  push regularly.
- **Render tests:** D2 is a render-harness change by definition — it needs a **full** pixel
  suite run to prove the tightened thresholds hold, under a ~10-minute watchdog, foreground,
  never backgrounded. Run it **once, at the end of the D2 phase**, not per iteration; iterate
  with `render_tests.sh test <prefix>` subsets. C2 (a dialog) and F2/H3 are out of pixel scope.
