---
status: complete
---

# Phase 0 — Fork setup + batch-wide pre-checks (findings)

## Overview

Phase 0 is **discovery only** — no fork source changes, no `fix/*` branches. It (1) confirms the
fork git setup, (2) runs the §5.2 existence sweep for all 12 names on `main`, and (3) resolves
every architecture §1 `[checkpoint]` so later phases don't re-discover.

## Headline finding (project-inverting — read first)

**All 11 functions AND the TRIM target already exist on the fork's `main` (the clean upstream
mirror), fully registered (enum variant + name mapping + dispatch arm + impl fn).** Upstream
IronCalc has implemented essentially this entire batch since the spec was written (the spec's
"345 of ~506 functions, these 11 missing" premise is now stale). Concretely:

- **10 of the 12 are present AND appear correct** against their functional-spec §3 contracts
  (verified by reading the impls, not just the registry): SUMPRODUCT, PROPER, REPLACE, CHAR, CODE,
  CLEAN, DOLLAR, ADDRESS, PERCENTILE(.INC), QUARTILE(.INC), XMATCH.
- **TRIM is present but WRONG** — the one real code change in this batch. `fn_trim`
  (`base/src/functions/text/common.rs:535`) is `s.trim().to_owned()`: ends-only (does not collapse
  internal runs) **and** uses Rust `str::trim` which strips *all* Unicode whitespace (tabs, NBSP,
  …) at the ends, violating the 0x20-only scope. This is exactly the §4 bug. **Phase 2 stays valid
  as written.**
- **CHAR/CODE already use Windows-1252** for 128–255 (spec O-1 satisfied) — with **one minor
  divergence** in the 5 undefined CP1252 slots (see Phase 5 note).

**Net effect on the plan:** this batch is now overwhelmingly a **verify-and-skip** exercise, not a
build. Each `fix/*` phase (except TRIM) should first run its spec §3 test vectors against the
existing impl; if green, **record "already present — branch skipped"** per §5.2 and move on. Only
TRIM (Phase 2) — and *optionally* a tiny CHAR/CODE undefined-slot tweak (Phase 5) — require a
branch. This should be surfaced to the owner: the deliverable collapses to ~1 branch + a pin bump.

## 1. Fork setup state

| Item | State |
|---|---|
| Clone | `/workspace/ironcalc`, checked out on `main`, **clean working tree** (`## main...origin/main`, no diffs). |
| History depth | **Unshallowed** — `git fetch --unshallow` succeeded; `origin/main` and `origin/freecell-fixes` both have full history (later phases can branch off `main` and merge into `freecell-fixes`). |
| `origin` | Container git-proxy `http://local_proxy@127.0.0.1:41729/git/scosman/ironcalc` (fetch + push, credential embedded). |
| `main` HEAD | `cedba4ea` — "FIX: Take applyBorder out of the React mount loop". `HEAD == origin/main`. |
| `main` = clean upstream mirror? | **Yes.** `main` has **no** `fix/*` / FreeCell-specific commits ahead of it; the recent commits (xlsx font, CF ranges, indexed colors, applyBorder) are upstream-style. All 12 target functions live in `main`, i.e. they came from **upstream**, not our fork — consistent with "clean mirror." |
| `freecell-fixes` | Exists; **15 commits ahead of `main`**. Contents are our integration work only: merged-cells (5 commits + spec), xlsx bool import, `set_user_inputs` batching, `set_worksheet_index`, indexedColors. **None of the 15 touch any of our 12 functions** — so `freecell-fixes` neither adds nor conflicts with this batch; it inherits all 12 from `main`. Current HEAD `b922df5e`. (FreeCell's lock rev in the spec was `81feec4`, which is present in this history at `81feec40` — the pin is ~4 commits behind `freecell-fixes` HEAD.) |
| Upstream-staleness note | Could not diff against live `ironcalc/IronCalc` (the proxy routes only `scosman/ironcalc`; no upstream remote). But the presence of the full scalar batch + recent CF/xlsx fixes shows the mirror is reasonably current. **No upstream sync needed** for this batch — everything required is already in `main`. |

## 2. Existence sweep (all 12 names, on `main`)

Registry evidence is `base/src/functions/mod.rs`: **E**=enum variant line, **N**=name-map line,
**D**=dispatch arm line. Impl-fn path/line follows. Correctness column = read of the impl vs the
functional-spec §3 contract.

| # | Name | Present? | Registry (mod.rs) | Impl fn (file:line) | Correct vs §3? |
|---|---|---|---|---|---|
| 1 | **SUMPRODUCT** | ✅ present | E132 `Sumproduct` · N681 · D2645 | `fn_sumproduct` — `math_and_trigonometry/sumproduct.rs:19` | **Correct.** Dimension check → `#VALUE!`; non-numeric (bool/str/empty)→0; error elements propagate; uses `eval_to_array` for array-context (§3.1 single-expression idiom covered). |
| 2 | **PROPER** | ✅ present | E234 `Proper` · N783 · D2422 | `fn_proper` — `text/string_format.rs:193` | Present; run §3.2 vectors to confirm (word boundary = non-letter). No red flags. |
| 3 | **REPLACE** | ✅ present | E235 `Replace` · N784 · D2423 | `fn_replace` — `text/string_format.rs:207` | Present; run §3.3 vectors (Unicode-scalar index, start<1/num<0 → `#VALUE!`). No red flags. |
| 4 | **CHAR** | ✅ present | E222 `Char` · N771 · D2410 | `fn_char` — `text/char_code.rs` (guard ~L93, `WIN1252_128_159` table L12) | **CP1252 (O-1 satisfied).** Minor divergence: undefined slots 129/141/143/144/157 → `#VALUE!` (not identity). See Phase 5 note. Explicit §3.4 vectors (128→€, 169→©, 65.9→A, 0/256→`#VALUE!`) pass. |
| 5 | **CODE** | ✅ present | E224 `Code` · N773 · D2412 | `fn_code` — `text/char_code.rs:` (`char_to_win1252` L64) | CP1252 inverse of CHAR; empty→`#VALUE!`. `CODE(CHAR(n))==n` holds for all **defined** n; fails only for the 5 undefined slots (they error in CHAR). See Phase 5 note. |
| 6 | **CLEAN** | ✅ present | E223 `Clean` · N772 · D2411 | `fn_clean` — `text/char_code.rs:137` | **Correct (O-2).** `filter(|c| (c as u32) >= 32)` — strips 0–31 only, keeps 127/160/Unicode. |
| 7 | **DOLLAR** | ✅ present | E227 `Dollar` · N776 · D2415 | `fn_dollar` — `text/string_format.rs:60` | Present; run §3.7 vectors (rounding half-away, negative→parens, negative-decimals, `$0` guards). No red flags. |
| 8 | **ADDRESS** | ✅ present | E161 `Address` · N710 · D2359 | `fn_address` — `lookup_and_reference/address_areas.rs:18` | **Full feature set present:** abs_num 1–4 (range-checked → `#VALUE!`), a1/**R1C1**, sheet_text. Run §3.8 vectors + empty-sheet edge to confirm quoting predicate (O-4). |
| 9 | **PERCENTILE(.INC)** | ✅ present | `PercentileInc` E316·N1098·D2778; `PercentileCompat` (=legacy PERCENTILE) E379·N903·D2745 | `fn_percentile_inc` — `statistical/percentile.rs:44` (core `percentile_inc_impl` L110) | **Correct + inclusive.** `idx=k*(n-1)`, floor/frac linear interp; empty→`#NUM!`; k∉[0,1]→`#NUM!`. **Legacy `PERCENTILE` already routes to the inclusive impl** (D2745) — spec Open-2 reconciliation is already done. Also present: `PERCENTILE.EXC`. |
| 10 | **QUARTILE(.INC)** | ✅ present | `QuartileInc` E325·N1105·D2785; `QuartileCompat` (=legacy QUARTILE) E382·N906·D2748 | `fn_quartile_inc` — `statistical/quartile.rs` | Present; legacy `QUARTILE`→ inclusive impl (D2748). Run §3.10 vectors. Also present: `QUARTILE.EXC`. |
| 11 | **XMATCH** | ✅ present | E184 `Xmatch` · N733 · D2383 | `fn_xmatch` — `lookup_and_reference/xmatch.rs` | **All modes present:** `MatchMode` {exact/−1/1/Wildcard=2}, `SearchMode` {FirstToLast=1/…/BinaryAscending=2/BinaryDescending=−2}; reuses `compare_values` + `from_wildcard_to_regex` + `binary_search_*`. Run §3.11 vectors incl. binary≡linear. |
| 12 | **TRIM** | ⚠️ **present-but-WRONG** | E219 `Trim` · N768 · D2407 | `fn_trim` — `text/common.rs:505` (bug: `s.trim().to_owned()` L535) | **Wrong per §4.** Ends-only (no internal-run collapse) **and** strips all Unicode whitespace at ends (Rust `str::trim`), violating 0x20-only. **The one required code fix.** |

## 3. Resolved checkpoints (architecture §1)

| §1 checkpoint | Resolved symbol / path (evidence) | Notes vs spec inference |
|---|---|---|
| `Function` enum + name map + dispatch | All three in **`base/src/functions/mod.rs`** (2864 lines): `pub enum Function` (variants ~L120–420); a **name-map macro** (lowercase, dot-stripped keys, e.g. `sumproduct => Sumproduct` L681) resolving the parser; central dispatch `match` in `evaluate_function` (arms ~L2350–2790). Confirmed as inferred. |
| Impl-fn modules | **Directories, not single files.** text → `functions/text/` (`common.rs` TRIM, `string_format.rs` PROPER/REPLACE/**DOLLAR**, `char_code.rs` CHAR/CODE/CLEAN); statistical → `functions/statistical/` (`percentile.rs`, `quartile.rs`); math → `functions/math_and_trigonometry/` (`sumproduct.rs`); lookup → `functions/lookup_and_reference/` (`address_areas.rs`, `xmatch.rs`). **DOLLAR lives in text (`string_format.rs`), not financial.** |
| Coercion helpers | **`base/src/cast.rs`** — real names: `get_number:235`, `get_number_no_bools:281`, `get_string:297`, `get_boolean:338`, plus `get_number_or_array:77`, `get_string_or_array:168`. Matches spec's inferred `get_number`/`get_string`/`get_boolean`. Integer-trunc is done at the callsite (`.trunc()`), not inside the helper. |
| `CalcResult` variants | **`base/src/calc_result.rs:12`** `pub(crate) enum CalcResult` — `Number/String/Boolean/Error{error,origin,message}/Range/EmptyCell/EmptyArg/Array/Lambda`. (`Array(_)` and `Lambda(_)` present → richer than the spec's list.) |
| `Error` enum + construction | **`base/src/expressions/token.rs:84`** `pub enum Error` — `REF NAME VALUE DIV NA NUM ERROR NIMPL SPILL CALC CIRC NULL`. Construct via **`CalcResult::new_error(Error, origin, msg)`** (`calc_result.rs:32`) or the direct struct `CalcResult::Error{error,origin,message}`; arity via `CalcResult::new_args_number_error(cell)` (`:39`). |
| Number formatter (DOLLAR) | **`base/src/formatter/format.rs::format_number(value, format, locale) -> Formatted`** (`:59`), backed by `number_format::to_precision`. This is the TEXT/FIXED path DOLLAR reuses. |
| Wildcard/criteria matcher | **`base/src/functions/util.rs`** — `from_wildcard_to_regex:139` (`?`/`*`/`~`), `compare_values:67`, `result_matches_regex`. Already reused by `xmatch.rs`, `database.rs`, `xlookup.rs`. |
| Range/array materialization + **array-context eval** | **Exists.** `Model::eval_to_array` (`functions/spill_functions.rs:148`) + `ArrayNode` enum (`expressions/parser/mod.rs:104`); also `evaluate_node_in_context` (`model.rs:549`). The fork has **full spill/dynamic-array machinery** (`spill_functions.rs`, TRANSPOSE/SEQUENCE/HSTACK/etc.), so the SUMPRODUCT single-expression idiom is fully supported (and SUMPRODUCT already uses `eval_to_array`). |
| Unit-test harness | **`base/src/test/util.rs`** — `new_empty_model() -> Model` (`:7`), `Model::_set(cell, value)` (`:21`), `Model::_get_text(cell)` (`:44`), `_get_text_at(sheet,row,col)` (`:41`); evaluate via `Model::evaluate(&mut self)` (`model.rs:3030`). Exactly the spec's inferred shape. Existing per-function tests live beside/within each function module (e.g. inline `#[cfg(test)]` + `base/src/test/`). |
| Volatile-function set | **No explicit volatile registry found** in `base/src` (grep for `volatile`/`is_volatile`/`needs_recalc` → empty). Our 12 are pure and are **not** registered as volatile anywhere. Nothing to add or avoid. |
| `make lint` / crate-scoped test | **`make lint`** = `cargo fmt -- --check` + `cargo clippy --all-targets --all-features -- -W clippy::unwrap_used -W clippy::expect_used -W clippy::panic -D warnings` (Makefile L2–4, run from fork root). Crate-scoped tests: `cargo test -p ironcalc_base`. (`make tests` also runs js/python/node — not needed per-branch.) |

## 4. Per-phase impact notes (later phases start correct from here)

The branch order below is the implementation_plan's. **Every "add function" phase is now a
"verify-then-skip" phase** unless noted. Standard procedure for a skip: run the §3 test vectors
against the existing impl (as new `#[cfg(test)]` cases is optional/redundant since upstream already
tests them); if green, mark the branch **"already present — skipped"** in `fork-fixes/README.md`
and the status table; do **not** create a duplicate `fix/*` branch.

- **Phase 1 — SUMPRODUCT:** ALREADY PRESENT & correct (`sumproduct.rs:19`, uses `eval_to_array`).
  → verify §3.1 vectors, then **skip the branch**. (Was billed as "the template"; there is nothing
  to template.)
- **Phase 2 — TRIM fix:** **REAL WORK — proceed.** Bug confirmed at `text/common.rs:535`
  (`s.trim()`). Apply the §4 fix (`split(' ').filter(!empty).join(" ")`), keeping the existing
  coercion arms (Number/Boolean/Empty/Error/Range/Array) above it. Add the §4 before/after +
  0x20-only regression tests. This is the primary (likely only-required) branch of the batch.
- **Phase 3 — PROPER:** present (`string_format.rs:193`) → verify §3.2, skip.
- **Phase 4 — REPLACE:** present (`string_format.rs:207`) → verify §3.3 (esp. start<1/num<0 →
  `#VALUE!`, append/insert boundaries), skip.
- **Phase 5 — CHAR + CODE:** present & **CP1252** (`char_code.rs`, `WIN1252_128_159`). This is
  **not** a raw-Unicode reconciliation — O-1's main concern is already satisfied. **One open
  discrepancy:** the 5 undefined CP1252 slots (129,141,143,144,157) currently map to `#VALUE!` in
  CHAR (table entry `\u{FFFD}` → `None`), whereas spec §3.4 wants identity C1 mapping so
  `CODE(CHAR(n))==n` holds over the *full* 1..=255. All **explicit** §3.4/§3.5 vectors pass
  (128→€, 169→©, 65.9→A, 0/256→`#VALUE!`, `CODE(CHAR(200))==200`). *Recommendation:* **skip the
  branch** — the divergence is only on 5 codes Excel itself treats ambiguously, and changing it is
  a judgment call, not a clear bug. If the owner wants strict full-range round-trip, a tiny
  `fix/char-code` correctness branch maps those 5 slots to `char::from_u32(n)` identity. Escalate
  the choice; do not silently change upstream behavior (per arch §F2 — only if it wouldn't break
  green tests).
- **Phase 6 — CLEAN:** present & correct (O-2, `char_code.rs:137`, `>= 32` filter) → verify §3.6,
  skip.
- **Phase 7 — DOLLAR:** present (`string_format.rs:60`) → verify §3.7 vectors carefully (the
  spec called out trailing-space, negative-parens, negative-decimals, `$0`/`$0.00` and the added
  `(-0.001,2)→$0.00` negative-zero guard). If any vector fails, that specific behavior becomes a
  `fix/dollar` correctness branch; otherwise skip.
- **Phase 8 — PERCENTILE.INC + QUARTILE.INC:** present, inclusive, **legacy names already route to
  the inclusive impl** (D2745/D2748) — spec Open-2 is resolved in-fork already. `.EXC` variants
  also present. → verify §3.9/§3.10, skip.
- **Phase 9 — ADDRESS:** present with **full R1C1** + sheet_text (`address_areas.rs:18`) → verify
  §3.8 vectors incl. R1C1 rows, `$XFD$1`, `'My Sheet'`, and the empty-sheet `!$A$1` edge (O-4
  quoting predicate — the one thing worth actually checking). Skip unless a vector fails.
- **Phase 10 — XMATCH:** present with all four match_modes × four search_modes incl. binary +
  wildcard (`xmatch.rs`) → verify §3.11 vectors incl. 2-D→`#VALUE!`, case-insensitive,
  binary-desc, binary≡linear. Skip unless a vector fails.
- **Phase 11 — Integration + FreeCell pickup:** Largely unchanged, but **much smaller**: if only
  Phase 2 (TRIM) produced a branch, `freecell-fixes` gets one merge, then FreeCell re-pins
  (`cd app && cargo update -p ironcalc_base -p ironcalc`) and runs the `freecell-engine` smoke.
  **Note:** the smoke test ("one formula per new function returns a computed value, not `#NAME?`")
  will pass for *all 12 today* even before any branch — because they already resolve on the pinned
  `freecell-fixes` (which inherits them from `main`). The pin is ~4 commits behind `freecell-fixes`
  HEAD; a `cargo update` is still worthwhile to pick up the newer integration work. The 10-PR
  upstream-prep collapses to **0–2 PRs** (TRIM fix; optional CHAR/CODE slot fix).

## Verification note

No fork source was modified this phase (discovery only). The sweep + checkpoint resolution were
done by reading the actual fork tree on `main` (`git grep`, enum/name-map/dispatch inspection, and
direct reads of each impl fn), not from the spec's inferences. Findings above cite `file:line`.
