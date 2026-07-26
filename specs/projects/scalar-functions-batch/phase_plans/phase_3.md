---
status: complete
---

# Phase 3 — PROPER (§3.2). Verified; branch skipped.

## Outcome

**Already present on `freecell-fixes` (inherited from `main`) — verified, no branch created.**
`fn_proper` — `base/src/functions/text/string_format.rs:193` (registry: enum `Proper`
mod.rs:234 · name-map:783 · dispatch:2422).

Verified against functional_spec §3.2 by running every worked-example vector through the fork's
test harness in a scratch module (deleted after verification).

## Vectors run (§3.2) — all PASS

| Vector | Expected | Result |
|---|---|---|
| `PROPER("john smith")` | `John Smith` | PASS |
| `PROPER("JOHN SMITH")` | `John Smith` | PASS |
| `PROPER("e-mail address")` | `E-Mail Address` (m follows `-`) | PASS |
| `PROPER("o'brien")` | `O'Brien` (b follows `'`) | PASS |
| `PROPER("2-way 76street")` | `2-Way 76Street` (letters after `-`/digit capitalized) | PASS |
| `PROPER("")` | `""` | PASS |

Word boundary = any non-letter; first letter of each word capitalized, rest lowercased. No
divergence. No fork source modified.
