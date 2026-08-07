# Phase 1 — A1: Pin the IronCalc fork by SHA

**Verdict: CONFIRMED — and the hazard was live, not hypothetical.**

> **Revised after owner review.** The first cut of this phase pinned the *locked* SHA (`cee2859d`)
> to avoid changing behaviour, treating the fork's revert of `fix/dollar-negative-zero` as an open
> question. The owner confirmed **the revert is intentional and FreeCell's test was simply missed**.
> So the pin is `ecbf6226` — the branch tip — and the stale `=DOLLAR(-0.001,2)` assertion is
> deleted. The reasoning below is left as written because the *investigation* is what made the
> stale test visible; only the conclusion changed.

## Confirmation

1. `app/Cargo.toml` `[patch.crates-io]` pinned both `ironcalc` and `ironcalc_base` with
   `branch = "freecell-fixes"`. Every other git dependency in the workspace (`gpui`,
   `gpui_platform`, `gpui-component`, `gpui-component-assets`) is `rev`-pinned — the fork was
   the sole outlier, as the review said.

2. The interesting part is what the branch was actually doing. Resolving it:

   ```
   $ git ls-remote https://github.com/scosman/ironcalc refs/heads/freecell-fixes
   ecbf6226ddfbe20bec15f9904647db450efdf2ea

   $ grep freecell-fixes app/Cargo.lock
   ...?branch=freecell-fixes#cee2859dceda65ff64e52192be4ec47a259870e1
   ```

   **The branch tip and the locked commit had already diverged.** Two commits separated them:

   ```
   ecbf622 Merge pull request #2 from scosman/claude/remove-dollar-negative-zero-fzfr2h
   8a79a7f Revert "Merge fix/dollar-negative-zero into freecell-fixes"
   ```

   So the branch tip **reverts** `fix/dollar-negative-zero`, and FreeCell's committed lock
   still builds the pre-revert commit that contains it.

3. That revert is not inert for FreeCell. `app/crates/freecell-engine/src/document.rs:2186`
   asserts:

   ```rust
   ("=DOLLAR(-0.001,2)", "$0.00"),         // fix/dollar-negative-zero
   ```

   which is exactly the behaviour the reverted one-line guard in
   `base/src/functions/text/string_format.rs` provides. Anyone who ran `cargo update` today
   would have silently moved the build onto the branch tip and turned a committed FreeCell
   test red, with nothing in the manifest recording that a decision had been made.

   That is the review's abstract argument ("a force-push + GC makes history unbuildable")
   showing up in its everyday form: a `branch =` pin means the commit you build is decided by
   whoever last pushed, not by your repository.

## What changed

`app/Cargo.toml`:

- Both patch entries now pin `rev = "ecbf6226ddfbe20bec15f9904647db450efdf2ea"`.
- The comment block above `[patch.crates-io]` was rewritten: why a `rev` and not a `branch`,
  and that bumping the fork is a deliberate one-line edit + `cargo update -p ironcalc -p
  ironcalc_base` in its own commit, with this pin's own move cited as the worked example of why a
  bump always re-runs the workspace tests.
- The `ironcalc = "=0.7.1"` / `ironcalc_base = "=0.7.1"` lines were investigated per the unit
  brief. They are **not** inert and must not be deleted: cargo only applies a `[patch]` entry
  whose replacement version satisfies the requirement being replaced, so `=0.7.1` is what the
  patch attaches to. They *are* misleading, so the comment now says so explicitly — read it as
  "the fork's base version", not "we ship IronCalc 0.7.1".

### Which SHA — and the stale test

Pinned the branch tip **`ecbf6226ddfbe20bec15f9904647db450efdf2ea`**.

The investigation above surfaced a genuine question: the tip drops the DOLLAR negative-zero
guard, and `document.rs` asserted it. **Owner's answer: the revert is intentional, the assertion
was just never removed.** So the assertion is the stale artifact, not the pin.

**Superseded 2026-08-07 (review remediation) — the assertion was RESTORED, corrected.** Deleting
it left `DOLLAR` covered by no test anywhere (the fork's revert had also removed the fork-side
`test_dollar_negative_rounds_to_zero` *and* `test_dollar_vectors`, which covered ordinary positive
cases). Worse, "owner confirmed the revert is intentional" was a thin resting place for an
Excel-compatibility product. The substantive answer is now on record — see
**Review remediation → Finding 7** below. `document.rs` asserts:

```rust
("=DOLLAR(-0.001,2)", "($0.00)"),
```

so all four fork-fix rows (TRIM, DOLLAR, ADDRESS, XMATCH) are covered, with DOLLAR flagged in
comments as *not* a fix branch.

Consequence for the A4 inventory: ~~`freecell-fixes` now carries **10** changes, not 11 — 6 merged
upstream, 4 fork-only.~~ **Superseded 2026-08-07:** the correct figure is **11 changes we
authored — 8 merged upstream, 3 fork-only** (DOLLAR counts in neither: not carried, not merged).
Re-derived in Phase 3's remediation; updated in `Cargo.toml`, `CLAUDE.md`, `PROJECTS.md`,
`projects/ironcalc-upgrade.md` and the upstreaming status table.

**The general lesson survives the specific answer.** A `branch =` pin meant this divergence would
have arrived via a stray `cargo update` as a mysteriously-red test, at whatever moment someone
happened to regenerate the lock. With a `rev` pin it arrived as a deliberate one-line edit whose
test failure is self-explanatory — which is the whole point of the unit.

## Verification

| Check | Result |
|---|---|
| `Cargo.lock` diff | 4 lines total: `?branch=freecell-fixes#…` → `?rev=ecbf6226…#ecbf6226…` on both crates. No other dependency moved. |
| `cargo build --locked -p freecell-engine` | clean (proves the patch still applies and the lock is consistent with the new source string) |
| `cargo test -p freecell-engine --lib` @ the A1 commit `360aca8` | **398 passed**, 0 failed, 1 ignored |
| `cargo test -p freecell-engine --lib` @ branch HEAD + remediation | **406 passed**, 0 failed, 1 ignored (the branch gained tests after A1) |
| `cargo test -p freecell-engine --tests --no-fail-fast` @ HEAD | **109 passed**, 3 ignored across the 9 runnable integration targets; `charts_roundtrip_libreoffice` (2) fails for lack of a working `soffice` in this container — environmental, pre-existing, unrelated |
| `cargo clippy -p freecell-engine --all-targets -- -D warnings` | clean |
| `cargo fmt --all --check` | clean |
| `cargo metadata --locked` | exits 0; `Cargo.lock` unchanged by the remediation |

*(Counts corrected 2026-08-07. The row previously read "390 passed" and was labelled as the
re-run; 390 was the **first** run's count, before the phase was revised to pin the tip. The
correct figure at the A1 commit is 398 — reconciled by `#[test]` count: `freecell-engine/src`
carries 399 `#[test]` at `360aca8` (398 + 1 `#[ignore]`) and 407 at HEAD. The integration run was
omitted entirely and is now recorded.)*

## Out of scope, as briefed

No tagging, mirroring, or vendoring scheme. The fork inventory and the surrounding doc
corrections belong to A4 (Phase 3), which rewrites the rest of this comment block.

---

## Review remediation (2026-08-07)

Code review of A1 raised four items. All four confirmed; all four fixed. Handled jointly with
Phase 3's A4 review, which overlapped on the same fork-inventory documents.

### Finding 6 — the test's doc comment described the pre-A1 world (Moderate) · **CONFIRMED, fixed**

`document.rs`'s doc comment said the pin points at "the fork's `freecell-fixes` **branch**" (A1
made it a `rev`) and that `fixes` covers "the **4** fork correctness fixes … including
`fix/dollar-negative-zero`" while the array held three. A unit whose thesis is *stale records
cause silent breakage* had left the stalest record inside its own test.

Fixed: the comment now says "a `rev` on the fork's `freecell-fixes` branch" and names the three
real fix branches. The rest of the doc comment is now **byte-identical to `origin/main`'s**, so
the eventual merge agrees rather than conflicts.

### Finding 7 — a behaviour change shipped with nothing pinning it (Moderate) · **CONFIRMED, fixed**

The pin bump changed a user-observable result (`=DOLLAR(-0.001,2)`: `$0.00` → `($0.00)`), and the
fork's revert had also deleted the fork-side DOLLAR tests. After A1, **no test anywhere covered
`DOLLAR`**. Restored to `document.rs`, in the exact form `origin/main` uses:

```rust
("=DOLLAR(-0.001,2)", "($0.00)"),
```

**Verified green** against the pinned engine — this is a real run, not an assumption:
`cargo test -p freecell-engine --lib` → 406 passed, 0 failed.

**The substantive answer, so this cannot be re-litigated in either direction.** `origin/main` had
already settled it independently (commit `8702e4a`): upstream
[ironcalc/IronCalc#1293](https://github.com/ironcalc/IronCalc/pull/1293) was **closed unmerged**
after the maintainer tested **Excel** directly and a second reviewer reproduced — **Excel returns
`($0.00)`**. `$0.00` is *Google Sheets*' answer; IronCalc targets Excel.

The mechanism, verified by reading the pinned engine's source rather than taking the PR thread on
faith — `fn_dollar`, `base/src/functions/text/string_format.rs` @ `ecbf6226`:

```rust
let formatted = format_abs(value.abs(), decimals, true);
let result = if value < 0.0 { format!("(${})", formatted) } else { format!("${}", formatted) };
```

The parenthesized branch is selected from the **raw sign of the input**, while rounding happens
inside `format_abs(value.abs(), …)`. **Section selection is by sign, pre-rounding**, so a negative
that rounds to zero still takes the negative section. That is Excel's contract, and IronCalc's
behaviour was correct all along — the reverted "fix" was the defect.

So: the revert is correct and complete, there is no outstanding gap, and no owner decision is
pending. Recorded in `specs/projects/scalar-functions-batch/fork-fixes/README.md` (the document
`document.rs` names) and in the upstreaming §Status table.

### Finding 8 — the deliberate spec deviation lived only in the phase plan (Moderate) · **CONFIRMED, fixed**

`v05-cleanup-1/functional_spec.md` §A1 item 2 demands "no dependency drift … as a side effect of
this unit" and `architecture.md` §1 calls byte-identical resolution "the load-bearing constraint
of the unit". A1 satisfied item 1 by breaking item 2, on owner instruction — correctly, since the
two were not simultaneously satisfiable once the tip had moved. But both files are
`status: complete` and asserted the constraint unqualified.

Fixed by **annotation, not rewrite** (the convention the other phases used): one dated block in
each, pointing here.

### Mild — `Cargo.toml`'s "WHY A rev" overstated two of three clauses · **CONFIRMED, fixed**

Two of the three clauses did not survive scrutiny: under `branch =` the lock *already* recorded
the exact commit, so an old revision built with the committed lock **did** resolve reproducibly;
and a force-push + GC breaks a `rev` pin just as thoroughly, since the lock stores a SHA either
way. The middle clause is the true, load-bearing, empirically-demonstrated one — **any lock
regeneration silently moves the fork, with nothing in the manifest recording it**. The comment now
leads with that and drops the other two, citing this pin's own move as the worked example.

### Mild — the `=0.7.1` comment conflated the declaration with the `=` · **CONFIRMED, fixed**

What must exist is *a* requirement the patch can satisfy (`0.7` would do); the `=` is not
load-bearing. And the failure mode is a **warning**, not an error: cargo prints "patch … was not
used in the crate graph", silently resolves the real crates.io `ironcalc`, and only dies later at
compile on a missing merged-cells API. Both now stated explicitly.

**Newly found while checking this:** the hazard is no longer hypothetical. Upstream has released
**0.8.x** while the fork declares `0.7.1` at the pinned rev, so the overdue `main` re-sync will
bump the fork's version and break the patch's attachment. Called out in the manifest comment and
in `projects/ironcalc-upgrade.md`.

### Mild — `phase_1.md:103`'s verification table reported the wrong count · **CONFIRMED, fixed**

The table reported 390 (the *first* run, before the phase was revised to pin the tip) labelled as
the second run. See the corrected §Verification table above: **398** at the A1 commit `360aca8`,
**406** at HEAD, plus the previously-omitted integration run.

**One number in the review brief is wrong.** It stated "the real count at `360aca8` and at HEAD is
**398** lib". 398 is right for `360aca8` **only** — at HEAD it is **406**, because C1/G1/F3a added
tests to `freecell-engine` after A1. Reconciled by `#[test]` count (399 → 407 in
`freecell-engine/src`, one `#[ignore]`d in both). Both figures are now recorded separately rather
than conflated.
