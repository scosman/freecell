# IronCalc Upgrade — adopt the fixed engine, delete the import hacks

**Status: Future.** Gated on an IronCalc **release** that carries all five import fixes (see
`specs/projects/ironcalc-upstreaming/` + `fork_audit.md`). Do not pin FreeCell to git-`main`.

## Why

FreeCell pins `ironcalc = "=0.7.1"` and carries workarounds for five import bugs
(`GAPS.md` E1–E5). Upstream `main` has **already fixed three of them** — E4 (`xfId`
optionality), E1 (file-theme resolution), E1′ (`hue_to_rgb` tint) — and the
`ironcalc-upstreaming` project upstreams the remaining two (E2 num-fmt table, E5 indexed
override). Once a release ships with all five, FreeCell should **upgrade and delete the hacks**
rather than keep compensating.

## What this project does

1. **Bump** `ironcalc`/`ironcalc_base` to the release containing the fixes.
2. **Migrate to the new `Color` API.** `main` replaced the style-color model: `Fill` is now
   `{ color: Color }` (enum `Rgb|Theme|None|…`), not `pattern_type` + `fg_color`/`bg_color:
   Option<String>`; `Font.color`/borders are `Color` too. FreeCell's `open_fixups`, `cache`, and
   style-reading assume the old `Option<String>` fields and will not compile — migrate them to
   read `Color` (resolving `Color::Theme`/`Rgb` as needed for the grid).
3. **Delete the now-dead compensations:** `open_fixups.rs`, `open_repair.rs`, and drop the
   `roxmltree` + `zip` deps from `freecell-engine`. (Confirm each bug is fixed in the pinned
   release first — E2/E5 depend on our PRs landing.)
4. **In-app visual validation** (the pass we could not do during upstreaming, because FreeCell
   can't build against `main`): open the mortgage + Numbers fixtures, eyeball theme colours,
   indexed colours, number formats, and the `xfId`-less file; open→save→reopen an affected file
   to confirm the save round-trip.

## Risks / notes

- If some of our E2/E5 PRs are **not** in the chosen release, keep just those specific hacks and
  delete the rest — the fixups are already per-bug separable.
- The `Color` migration is the real work here; scope it as its own phase (touches the resident
  style cache's colour reads).
- Coordinate with `projects/style-cache.md` (the resident style/geometry cache also reads fill/
  font colours).
