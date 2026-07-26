---
status: complete
---

# Phase 8 — PERCENTILE.INC + QUARTILE.INC (§3.9/§3.10). Verified; branch skipped.

## Outcome

**Already present on `freecell-fixes` (inherited from `main`), inclusive, with legacy names
already routing to the inclusive impl — verified, no branch created.**

- `fn_percentile_inc` — `base/src/functions/statistical/percentile.rs:44` (core
  `percentile_inc_impl` ~L110). Registry: `PercentileInc` enum:316 · name-map:1098 ·
  dispatch:2778; legacy `PercentileCompat` (=PERCENTILE) enum:379 · name-map:903 ·
  dispatch:2745 → routes to the inclusive impl.
- `fn_quartile_inc` — `base/src/functions/statistical/quartile.rs`. Registry: `QuartileInc`
  enum:325 · name-map:1105 · dispatch:2785; legacy `QuartileCompat` (=QUARTILE) enum:382 ·
  name-map:906 · dispatch:2748 → routes to the inclusive impl.

Verified against functional_spec §3.9/§3.10 in a scratch module (deleted after verification).

## Vectors run — all PASS

PERCENTILE.INC (§3.9, array={1,2,3,4}): `k=0`→`1` (min), `k=1`→`4` (max), `k=0.5`→`2.5`,
`k=0.25`→`1.75`, `k=0.75`→`3.25`; `{5},0.3`→`5` (n=1); `k=1.1`→`#NUM!`; `k=-0.1`→`#NUM!`;
`PERCENTILE({1,2,3,4},0.5)`→`2.5` (legacy routes to inclusive); no-numerics range→`#NUM!`.

QUARTILE.INC (§3.10, data={1,2,4,7,8,9,10,12}): `quart=0`→`1`, `quart=1`→`3.5`, `quart=2`→`7.5`,
`quart=3`→`9.25`, `quart=4`→`12`; `quart=5`→`#NUM!`; `quart=-1`→`#NUM!`; `QUARTILE(data,1)`→`3.5`
(legacy routes to inclusive).

Linear interpolation (idx = k·(n−1), floor/frac), the quart→k mapping, k/quart out-of-range →
`#NUM!`, no-numerics → `#NUM!`, and legacy-alias routing to the inclusive impl are all correct
(spec Open-2 already resolved in-fork). No divergence. No fork source modified.
