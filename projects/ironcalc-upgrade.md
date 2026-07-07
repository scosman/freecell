# IronCalc — move to a released pin

**Status: Future.** Follow-up tail of the `specs/projects/ironcalc-upstreaming` project.

The upstreaming project upgrades FreeCell onto the fork **git-`main` + our E2/E5 fixes** (via
`[patch.crates-io]` → `scosman/ironcalc#freecell-fixes`), migrates FreeCell to `main`'s new
`Color`-enum style API, and deletes the import workarounds (`open_fixups`/`open_repair`,
`roxmltree`/`zip`). That pins FreeCell to **unreleased git-`main`**, which is fine for validation
but not for shipping.

This follow-up, once IronCalc publishes a **release** containing all five fixes (E1/E4/E1′ already
on `main`; E2/E5 via our PRs landing):

1. Bump `ironcalc`/`ironcalc_base` to that release version and **remove the `[patch.crates-io]`
   stanza** (and update `=0.7.1` pins).
2. Re-run the FreeCell test + visual validation against the released crate (no code change expected
   if the release matches `main`; reconcile any API delta if the release diverged).
3. If some of our E2/E5 PRs did **not** make that release, restore only those specific hacks until
   a later release carries them (the fixups are per-bug separable).

Coordinate with `projects/style-cache.md` (the resident style/geometry cache reads fill/font colours
through the same resolved-`Color` path).
