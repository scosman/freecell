---
status: complete
---

# Implementation Plan: Scalar Functions Batch (+ TRIM fix)

Twelve phases. **All fork work** in `scosman/ironcalc` (clone/branch via the git-proxy URL
`http://local_proxy@127.0.0.1:<port>/git/scosman/ironcalc`, `<port>` from FreeCell's `git remote -v`;
or `add_repo scosman/ironcalc` **up front** while the user is present). Build order front-loads the
highest-value / most-independent fixes (SUMPRODUCT + TRIM), then the text functions, then the two
biggest (ADDRESS full-R1C1, XMATCH full-modes) last, then a single integration phase.

**Per-phase discipline (every fix/* phase):**
- Branch `fix/<slug>` off fork **`main`**. **First: the pre-branch existence check** (arch §5.2 —
  `git grep` the name + `Function` enum, `git merge-base --is-ancestor` if an upstream commit is
  suspected). If already present + correct → **skip the branch** (record "already present"); if present
  + wrong → the branch becomes a correctness fix.
- Implement the one impl fn (arch §3) + registry entries (enum / name map / dispatch arm). **[checkpoint]**
  the inferred fork symbols on first contact.
- Add upstream-style tests = the §3 worked examples verbatim + the called-out edge rows.
- **Crate-scoped** `cargo test -p ironcalc_base` + `make lint` (fmt + strict clippy) — not the whole
  fork workspace. Author as `Steve Cosman <848343+scosman@users.noreply.github.com>`; clean messages,
  no session URLs. Commit + push the branch (git-proxy; if push 403s, save a durable patch under
  `fork-fixes/` and note it in the tracker).
- Merge into `freecell-fixes`. Prepare the upstream PR (compare link + title + body) in
  `fork-fixes/README.md`.

Section refs are to `architecture.md` / `functional_spec.md` §3.

## Phases

- [x] **Phase 0 — Fork setup + batch-wide pre-checks.** Clone the fork (git-proxy or `add_repo`);
      confirm `main` is a clean upstream mirror; sync `main` from upstream if stale. Run the §5.2
      existence sweep for **all 12** names at once (`SUMPRODUCT, PROPER, REPLACE, CHAR, CODE, CLEAN,
      DOLLAR, ADDRESS, PERCENTILE[.INC], QUARTILE[.INC], XMATCH, TRIM`) and record which already exist
      (esp. CHAR/CODE, PERCENTILE/QUARTILE, TRIM). Confirm the coercion helpers, `CalcResult`/`Error`,
      the array/wildcard/formatter symbols, the test harness, and the volatile-fn set (arch §1)
      exist where inferred — resolve every **[checkpoint]** up front so later phases don't re-discover.
- [x] **Phase 1 — SUMPRODUCT (§3.1). `fix/sumproduct`.** Highest-value item + the template.
      `fn_sumproduct` with the `eval_arg_as_grid` + `to_number_or_zero` helpers; dimension rule → `#VALUE!`;
      the two forms (multi-array booleans→0; single-expression `(A=x)*(B)` via array-context eval —
      resolve that [checkpoint] here). Tests = §3.1 table + error-element propagation.
- [x] **Phase 2 — TRIM fix (§4). `fix/trim-internal-runs`.** Read the current `fn_trim` body; replace
      with the `split(' ')/filter/join` one-liner (collapse internal 0x20 runs, 0x20-only scope).
      Regression tests = the §4 before/after table incl. the tab + NBSP 0x20-only proofs.
- [x] **Phase 3 — PROPER (§3.2). `fix/proper`.** `fn_proper`; word boundary = non-letter; UPPER/LOWER
      case tables. Tests = §3.2 (incl. `e-mail`, `o'brien`, `2-way 76street`).
- [x] **Phase 4 — REPLACE (§3.3). `fix/replace`.** `fn_replace`; Unicode-scalar indexing; `start<1`/
      `num<0` → `#VALUE!`; append/insert/over-trim boundaries. Tests = §3.3.
- [x] **Phase 5 — CHAR + CODE (§3.4/§3.5). `fix/char-code` (paired, arch §4).** `fn_char` + `fn_code`
      sharing the CP1252 128–255 table; inverse-consistency. If the fork already has them raw-Unicode,
      this is the CP1252 correctness fix (owner-escalate only if it breaks green tests, arch §Open-1).
      Tests = §3.4/§3.5 + the `CODE(CHAR(n))==n` invariant over 1..=255.
- [x] **Phase 6 — CLEAN (§3.6). `fix/clean`.** `fn_clean`; strip codes 0–31 only (keep 127/160/Unicode).
      Tests = §3.6 (incl. 127-kept, NBSP-kept).
- [x] **Phase 7 — DOLLAR (§3.7). `fix/dollar`.** `fn_dollar`; explicit ROUND (half-away), reuse the
      TEXT/FIXED formatter (en-US, no trailing space), negative→parens, negative-decimals rounding,
      $0 guard. Tests = §3.7 + `(-0.001,2)`→`$0.00`.
- [x] **Phase 8 — PERCENTILE.INC + QUARTILE.INC (§3.9/§3.10). `fix/percentile-quartile-inc` (paired,
      arch §4).** Shared `collect_numbers` + `percentile_inc_core`; register `.INC` **and** legacy
      `PERCENTILE`/`QUARTILE` onto the two impl fns; QUARTILE maps quart→k over the core. k/quart range →
      `#NUM!`, no-numerics → `#NUM!`. If legacy already exists non-inclusive, reconcile to inclusive
      (arch §Open-2). Tests = §3.9 + §3.10 tables.
- [x] **Phase 9 — ADDRESS (§3.8). `fix/address`.** `fn_address`; column→letters (bijective base-26,
      `16384`→`XFD`); abs_num 1–4 markers; **full R1C1** (a1=FALSE, O-5); sheet_text quoting (O-4, incl.
      empty-sheet `!` edge); range/abs_num → `#VALUE!`. Tests = §3.8 + `(1,1,1,TRUE,"")`→`!$A$1`.
- [x] **Phase 10 — XMATCH (§3.11). `fix/xmatch`.** `fn_xmatch`; all four `match_mode`s (exact / next-
      smaller / next-larger / wildcard) × all four `search_mode`s (first→last / last→first / binary asc /
      binary desc); reuse MATCH comparison + the fork wildcard matcher; type-sensitive + case-insensitive;
      2-D array → `#VALUE!`; not found → `#N/A`; invalid mode → `#N/A` (O-6). Tests = §3.11 + 2-D,
      case-insensitive, binary-desc, and the **binary≡linear-on-sorted** equivalence test.
- [x] **Phase 11 — Integration + FreeCell pickup + PR prep.** Confirm all 10 `fix/*` are merged into
      `freecell-fixes` and pushed (or durable patches recorded). In FreeCell: `cd app && cargo update -p
      ironcalc_base -p ironcalc` to re-pin the lock onto the new `freecell-fixes` HEAD; add + run the
      **FreeCell-side smoke** (`freecell-engine` test: one formula per new function returns its computed
      value, not `#NAME?`) crate-scoped (`cargo test -p freecell-engine --lib`); `cargo fmt --all --check`.
      Finalize all 10 upstream PR preps (compare link + title + body) in `fork-fixes/README.md`.
      **Optional (may defer):** add the 11 names + templates to `freecell-core/src/functions.rs`
      autocomplete catalog. **Owner shepherds the upstream PRs** (human-in-loop).

## Notes for the build

- **Fork policy.** One fix = one branch = one clean upstream PR; never combine **unrelated** fixes.
  The two deliberate pairings (`fix/char-code`, `fix/percentile-quartile-inc`) are **one coupled
  feature each** (shared new helper + definitional coupling) — justified in arch §4, not a violation.
- **Pre-branch check is mandatory** (arch §5.2). Some of these may already exist upstream (as
  hide/unhide did in `gaps_closing_7_15`) → skip or convert to a correctness fix; do not add duplicates.
- **No pixel suite, no benchmarks** — no UI surface; correctness-only (arch §6). The CLAUDE.md render
  gate does not apply to this batch.
- **Ephemeral container:** commit + push after **every** phase; keep durable `fork-fixes/*.patch` copies
  if fork push is blocked (mirrors `conditional-formatting/fork-fixes/`).
- **Build efficiency:** crate-scoped `cargo test -p ironcalc_base` + `make lint` per fork phase; reserve
  any full-workspace run for the final integration check. Run FreeCell cargo from `app/`.

## Status

**All phases complete.** The batch's real deliverable narrowed during verification from the
10 front-loaded `fix/*` branches to **4 real fork branches / PRs** — `fix/trim-internal-runs`
(TRIM), `fix/dollar-negative-zero` (DOLLAR), `fix/address-empty-sheet` (ADDRESS), and
`fix/xmatch-array-constant` (XMATCH) — all merged onto `freecell-fixes` (HEAD `9161a463`) and
pushed. The other **7 functions** (PROPER, REPLACE, CHAR, CODE, CLEAN, PERCENTILE.INC,
QUARTILE.INC) were found **already present upstream + verified in place** (no branch, no PR).
**SUMPRODUCT** is present + verified; its `--`-idiom divergence is an **engine unary-minus
operator** bug, spun out as a **deferred** off-critical-path follow-on
(`projects/unary-minus-boolean-coercion.md`), not fixed here.

Phase 11 re-pinned FreeCell's lock onto `freecell-fixes` HEAD (`81feec40` → `9161a463`) and
added a `freecell-engine` smoke test (13 assertions: 9 presence + 4 fixes) proving the batch
computes end-to-end through the FreeCell engine seam. The per-branch upstreaming state + the
4 finalized upstream-PR preps (for the owner to open) live in `fork-fixes/README.md` (the
tracker).
