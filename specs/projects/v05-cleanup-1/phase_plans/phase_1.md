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

Deleted from `document.rs`'s pinned-engine test:

```rust
("=DOLLAR(-0.001,2)", "$0.00"),         // fix/dollar-negative-zero
```

with a comment recording that the fix was reverted on the fork (PR #2, `8a79a7f`) and why the
assertion went with it. The remaining three fork-fix assertions (TRIM, ADDRESS, XMATCH) still
prove the pin carries their branches.

Consequence for the A4 inventory: `freecell-fixes` now carries **10** changes, not 11 — 6 merged
upstream, 4 fork-only. Updated in `Cargo.toml`, `CLAUDE.md`, `projects/ironcalc-upgrade.md` and the
upstreaming status table.

**The general lesson survives the specific answer.** A `branch =` pin meant this divergence would
have arrived via a stray `cargo update` as a mysteriously-red test, at whatever moment someone
happened to regenerate the lock. With a `rev` pin it arrived as a deliberate one-line edit whose
test failure is self-explanatory — which is the whole point of the unit.

## Verification

| Check | Result |
|---|---|
| `Cargo.lock` diff | 4 lines total: `?branch=freecell-fixes#…` → `?rev=ecbf6226…#ecbf6226…` on both crates. No other dependency moved. |
| `cargo build --locked -p freecell-engine` | clean (proves the patch still applies and the lock is consistent with the new source string) |
| `cargo test --locked -p freecell-engine --lib` | 390 passed, 0 failed, 1 ignored (re-run green on `ecbf6226` after deleting the stale DOLLAR assertion) |
| `cargo fmt --all --check` | clean |

## Out of scope, as briefed

No tagging, mirroring, or vendoring scheme. The fork inventory and the surrounding doc
corrections belong to A4 (Phase 3), which rewrites the rest of this comment block.
