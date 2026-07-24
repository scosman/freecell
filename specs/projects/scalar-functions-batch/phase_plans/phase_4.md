---
status: complete
---

# Phase 4 — REPLACE (§3.3). Verified; branch skipped.

## Outcome

**Already present on `freecell-fixes` (inherited from `main`) — verified, no branch created.**
`fn_replace` — `base/src/functions/text/string_format.rs:207` (registry: enum `Replace`
mod.rs:235 · name-map:784 · dispatch:2423).

Verified against functional_spec §3.3 by running every worked-example vector through the fork's
test harness in a scratch module (deleted after verification).

## Vectors run (§3.3) — all PASS

| Vector | Expected | Result |
|---|---|---|
| `REPLACE("abcdefg",3,2,"XY")` | `abXYefg` | PASS |
| `REPLACE("2009",3,2,"10")` | `2010` | PASS |
| `REPLACE("Hello",6,0," World")` | `Hello World` (insertion at end) | PASS |
| `REPLACE("Hello",1,0,">>")` | `>>Hello` (insertion at start) | PASS |
| `REPLACE("abc",2,10,"X")` | `aX` (num_chars past end trims to end) | PASS |
| `REPLACE("abc",10,2,"XYZ")` | `abcXYZ` (start past end → append) | PASS |
| `REPLACE("abc",0,1,"X")` | `#VALUE!` (start_num < 1) | PASS |
| `REPLACE("abc",2,-1,"X")` | `#VALUE!` (num_chars < 0) | PASS |

Insertion, append, over-trim boundaries and both `#VALUE!` guards behave per spec. No
divergence. No fork source modified.
