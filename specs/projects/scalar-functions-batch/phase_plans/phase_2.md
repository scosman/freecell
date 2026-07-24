---
status: complete
---

# Phase 2 — TRIM fix (§4). `fix/trim-internal-runs`

## Overview

Phase 2 is the **one real code change** in this batch (per the Phase 0 headline finding: every other
function already exists and computes correctly on the fork's `main`). It is a **correctness fix** to
the already-present `TRIM`, not a new function — implemented as a single `fix/<slug>` branch in the
IronCalc fork (`scosman/ironcalc`) per the standing "fix the fork, don't hack FreeCell" doctrine.

## The bug

`fn_trim` (`base/src/functions/text/common.rs`, body ~L505, bug line L535) was:

```rust
return CalcResult::String(s.trim().to_owned());
```

Two defects against Excel's TRIM contract (functional_spec §4):

1. **Ends-only** — it does not collapse **internal** runs of multiple spaces to a single space.
2. **Wrong whitespace scope** — Rust's `str::trim` strips **all Unicode whitespace** (tab `0x09`,
   NBSP `0xA0`, …), whereas Excel's TRIM operates on the **ASCII space `0x20` only**.

## The fix

Replaced the buggy normalization with a `0x20`-only split/filter/join (the coercion arms —
Number/Boolean/Empty/Error/Range/Array → String — were left exactly as they were; only the string
value's whitespace normalization changed):

```rust
// Excel TRIM operates on the ASCII space (0x20) only: it removes
// leading and trailing spaces and collapses each internal run of two
// or more spaces to a single space. It must NOT touch tab (0x09),
// non-breaking space (0xA0), or any other Unicode whitespace, so we
// deliberately split on ' ' rather than using `str::trim`.
let trimmed = s
    .split(' ')
    .filter(|w| !w.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
return CalcResult::String(trimmed);
```

`split(' ')` splits on the ASCII space only; dropping empty tokens removes leading/trailing spaces
**and** collapses internal runs; `join(" ")` re-inserts a single space between surviving tokens. Tabs,
NBSP, and all other whitespace are never inspected, preserving the `0x20`-only scope.

## Tests added

Two upstream-style test functions in `base/src/test/text_functions/mod.rs` (matching the module's
existing `new_empty_model` / `_set` / `evaluate` / `_get_text` harness convention), covering the §4
before/after table verbatim plus the called-out `0x20`-only edge rows:

- **`test_trim_collapse_internal_runs`** — the §4 collapse/trim rows:
  - `TRIM("  hello   world  ")` → `"hello world"`
  - `TRIM("a    b")` → `"a b"` (load-bearing interior-only regression)
  - `TRIM("no  extra")` → `"no extra"` (load-bearing interior-only regression)
  - `TRIM("single")` → `"single"` (no-op)
  - `TRIM("   ")` → `""` (all-spaces)
- **`test_trim_only_ascii_space`** — the `0x20`-only proofs:
  - `TRIM("a"&CHAR(9)&CHAR(9)&"b")` → `"a\t\tb"` (tabs **not** collapsed/trimmed)
  - `TRIM(CHAR(160)&"x"&CHAR(160))` → NBSP-`x`-NBSP unchanged (NBSP **not** trimmed)

## Verification (crate-scoped, from `/workspace/ironcalc`)

- `cargo test -p ironcalc_base` — **2191 passed, 0 failed** (both new TRIM tests green); doctests 23/23.
- `make lint` — **EXIT 0** (`cargo fmt --check` clean + strict clippy `-D warnings` clean).

Per the fork build-efficiency policy, only the `ironcalc_base` crate was tested — not the whole fork
workspace or the js/python suites.

## Fork branch / commit / push

- **Branch:** `fix/trim-internal-runs` (off clean `main`).
- **Branch commit:** `6c894ba2` — authored `Steve Cosman <848343+scosman@users.noreply.github.com>`,
  clean upstream-style message, **no** session URL / **no** Claude co-author trailer (fork branches are
  destined for upstream PRs). Title: *"Fix TRIM to collapse internal runs of spaces (Excel-compatible)"*.
- **Integration:** merged `--no-ff` into `freecell-fixes` → merge commit `0a36e79e` (no conflicts).
- **Push:** both `fix/trim-internal-runs` and `freecell-fixes` pushed to `scosman/ironcalc` via the
  git-proxy origin (exit 0 — **no 403**, no patch fallback needed).

## Upstream PR prep

Prepared for the owner to open (see `../fork-fixes/README.md` → "Upstream PR prep — completed
branches"): compare link
`https://github.com/ironcalc/IronCalc/compare/main...scosman:ironcalc:fix/trim-internal-runs`, title
and body drafted from functional_spec §4.

## Code review

Passed clean — no issues.
