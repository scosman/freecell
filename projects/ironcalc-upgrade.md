# IronCalc — move to a released pin

**Status: Future.** Follow-up tail of the `specs/projects/ironcalc-upstreaming` project.

The upstreaming project upgrades FreeCell onto the fork **git-`main` + our E2/E5 fixes** (via
`[patch.crates-io]` → `scosman/ironcalc#freecell-fixes`), migrates FreeCell to `main`'s new
`Color`-enum style API, and deletes the import workarounds (`open_fixups`/`open_repair`,
`roxmltree`/`zip`). That pins FreeCell to **unreleased git-`main`**, which is fine for validation
but not for shipping.

**Premise update (2026-08-06 fork sync).** The original blocker is largely gone: **all five**
E-fixes are now on upstream `main` — E1/E4/E1′ always were; E2 (num-fmt, upstream `5b982529`)
had already landed *before* this sync (it is an ancestor of the pre-sync `main`, `cedba4e`); and
E5 (indexed colors, `ef336e06`+`e481dc65`) is the one that arrived *with* the sync. So a release
cut from `main` at or after `0.8.3` should carry all five. What now keeps the `[patch.crates-io]`
alive is a *different*, later set of not-yet-upstreamed capabilities: merged-cells,
`set_user_inputs`, frozen-pane tracking across structural edits, and Excel-style paste fill.
Dropping the patch means those must land upstream (or be given up) first — see
`specs/projects/ironcalc-upstreaming/implementation_plan.md` §Sync log for the live carry list.

This follow-up, once IronCalc publishes a **release** containing everything `freecell-fixes`
currently carries:

1. Bump `ironcalc`/`ironcalc_base` to that release version and **remove the `[patch.crates-io]`
   stanza**. Update the exact-version requirements in `app/Cargo.toml` — currently `=0.8.3`,
   tracking the fork; they must equal whatever the release declares.
2. Re-run the FreeCell test + visual validation against the released crate (no code change expected
   if the release matches `main`; reconcile any API delta if the release diverged).
3. If some of our fixes did **not** make that release, keep riding `freecell-fixes` (or restore only
   those specific hacks) until a later release carries them — the fixes are per-bug separable.

Coordinate with `projects/style-cache.md` (the resident style/geometry cache reads fill/font colours
through the same resolved-`Color` path).
