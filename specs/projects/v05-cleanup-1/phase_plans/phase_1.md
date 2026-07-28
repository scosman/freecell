# Phase 1 — A1: Pin the IronCalc fork by SHA

**Verdict: CONFIRMED — and the hazard was live, not hypothetical.**

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

- Both patch entries now pin `rev = "cee2859dceda65ff64e52192be4ec47a259870e1"`.
- The comment block above `[patch.crates-io]` was rewritten: why a `rev` and not a `branch`,
  and that bumping the fork is a deliberate one-line edit + `cargo update -p ironcalc -p
  ironcalc_base` in its own commit, with the divergence above cited as the reason to re-run
  the tests when bumping.
- The `ironcalc = "=0.7.1"` / `ironcalc_base = "=0.7.1"` lines were investigated per the unit
  brief. They are **not** inert and must not be deleted: cargo only applies a `[patch]` entry
  whose replacement version satisfies the requirement being replaced, so `=0.7.1` is what the
  patch attaches to. They *are* misleading, so the comment now says so explicitly — read it as
  "the fork's base version", not "we ship IronCalc 0.7.1".

### Which SHA, and why not the tip

Pinned the **locked** commit (`cee2859d`), not the branch tip (`ecbf6226`).

A1 is a supply-chain unit: it changes *how* the dependency is addressed, never *what* it
resolves to. Pinning the tip would have shipped a behaviour change (losing the DOLLAR
negative-zero guard) and turned `document.rs`'s scalar-function test red, smuggled in under a
"pin by SHA" commit. Whether to adopt the fork's revert is a separate, deliberate decision —
it is now a one-line edit, which is exactly the property this unit was after.

`cee2859d` is still reachable and is an ancestor of the current tip (verified with
`git merge-base --is-ancestor` against a fresh clone), so the pin is not a dangling reference.

**Flagged for the owner:** the fork's `freecell-fixes` tip reverts a fix that FreeCell's test
suite depends on. Nothing is broken today (the lock and now the rev pin both hold the
pre-revert commit), but the next fork bump has to reconcile that — either FreeCell drops the
`DOLLAR(-0.001,2)` assertion, or the revert gets reverted on the fork.

## Verification

| Check | Result |
|---|---|
| `Cargo.lock` diff | 4 lines: `?branch=freecell-fixes#<sha>` → `?rev=<sha>#<sha>` on both crates. **Every fragment SHA identical**; no other dependency moved. |
| `cargo build --locked -p freecell-engine` | clean (proves the patch still applies and the lock is consistent with the new source string) |
| `cargo test --locked -p freecell-engine --lib` | 390 passed, 0 failed, 1 ignored |
| `cargo fmt --all --check` | clean |

## Out of scope, as briefed

No tagging, mirroring, or vendoring scheme. The fork inventory and the surrounding doc
corrections belong to A4 (Phase 3), which rewrites the rest of this comment block.
