# Split `grid/view.rs`

**Status:** Future
**Filed:** 2026-07-28, from `specs/projects/chrome-view-split` (which was told to "note where
it stands and file it if it warrants its own unit").
**Related:** `projects/architecture-review-remediation.md` §F1 (the `chrome/view.rs` half, now
done) and §F2 (the CI production-line ceiling, which this blocks).

## What

`app/crates/freecell-app/src/grid/view.rs` — the custom virtualized spreadsheet grid — is
**10,627 lines: 6,575 production + 4,052 inline test** (measured at `3475998`). That is
**3.3×** the 2,000-line production ceiling F2 will enforce, and the largest single file in the
workspace by production lines.

Split it into `grid/view/` child modules by concern, the same way `chrome/view.rs` was just
split into `chrome/view/`.

## Why it's worth doing (and why it isn't urgent)

Same argument as the chrome split, one notch weaker. It costs velocity and bus factor, not
users. The architecture review ranked file size outside the top five most dangerous findings,
and nothing here is broken.

What makes it worth filing now rather than forgetting:

1. **F2 cannot be enforced while it exists.** A CI check at 2,000 production lines has to
   either fail on `grid/view.rs` or carry it as an explicit exemption. An exemption on the
   biggest file in the repo is a weak ceiling.
2. **The chrome split has now proved the method is cheap.** Rust's module-descendant privacy
   means child modules see the parent's private fields with **no** visibility change; only
   private items called *across* the new sibling boundary need `pub(super)`, and the compiler
   names every one. The chrome split moved 16,099 lines across eight phases without a single
   behaviour change or a dropped test.
3. It is the natural next unit and needs no design work beyond measuring.

## Before planning: measure, don't trust this note

The chrome project's overview carried field/line counts from a review agent that had read the
file without compiling it; re-measuring at HEAD found the field count off by two. Do the same
here before planning: production vs `#[cfg(test)]` line counts, where the test module starts,
the real method inventory, and the actual domain boundaries.

**Check first whether `grid/view.rs` is banner-sectioned the way `chrome/view.rs` was.** That
single property is what made the chrome split mechanical rather than a judgement call: 17
production banners and 29 test banners whose names *agreed*, so each production section mapped
to its test section by name and every cut was a contiguous range. If `grid/view.rs` has the
same structure, expect a comparable cost. If it does not, the domain boundaries have to be
derived, and the estimate goes up.

## Method (proven, reusable)

The full procedure is in `specs/projects/chrome-view-split/architecture.md` — §3 (privacy),
§4 (move discipline), §7 (the range→destination mapping). In short:

1. `git mv view.rs view/mod.rs` **as its own commit**, so git records every later phase as a
   move out of `mod.rs` rather than an unrelated add/delete pair.
2. Extract shared test scaffolding into a `#[cfg(test)] mod test_support` first — it exercises
   the whole mechanism on a small surface before any production code moves.
3. One domain per phase, cut on item boundaries with doc comments and banners intact, in
   source order, bodies byte-identical. Tests move with the code they test.
4. Gate **every** phase: build, tests with a test-name multiset check against a baseline
   captured before the first move, `fmt --check`, and `clippy --all-targets -D warnings`
   (a split strands imports, and CI gates on warnings).

## Scope discipline

Behaviour-preserving. No logic refactors, no state-model changes, no field consolidation. A
mixed diff is the only real risk — bugs found go in a `findings.md`, not into this diff.

## Also over the ceiling

For whoever plans F2, the other files above 2,000 production lines at the same commit:

| Production | Total | File |
|---:|---:|---|
| 3,984 | 9,289 | `freecell-engine/src/worker/run.rs` — owned by **engine-worker-hardening** |
| 2,914 | 2,914 | `freecell-engine/tests/worker_seam.rs` — an integration-test file with no `#[cfg(test)]` inside it, so a naive rule counts all of it as production |
| 2,153 | 2,185 | `freecell-app/src/shell/window.rs` |
| 2,142 | 3,834 | `freecell-engine/src/document.rs` |
