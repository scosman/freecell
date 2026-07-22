---
status: complete
---

# Phase 10 — XMATCH (§3.11). Verified; branch skipped (array-constant divergence, owner decision).

## Outcome

**Already present on `freecell-fixes` (inherited from `main`) with all four match_modes × four
search_modes — verified, no branch created.** `fn_xmatch` —
`base/src/functions/lookup_and_reference/xmatch.rs` (registry: enum `Xmatch` mod.rs:184 ·
name-map:733 · dispatch:2383).

Verified against functional_spec §3.11 in a scratch module (deleted after verification). Because
the fork rejects inline array constants (see divergence), core logic was verified with **ranges**.

## Vectors run (§3.11, arr={10,20,30,40,50} in a range)

| Vector | Expected | Result |
|---|---|---|
| `XMATCH(30,arr)` | `3` (exact) | PASS |
| `XMATCH(35,arr)` | `#N/A` (exact not found) | PASS |
| `XMATCH(35,arr,-1)` | `3` (next smaller = 30) | PASS |
| `XMATCH(35,arr,1)` | `4` (next larger = 40) | PASS |
| `XMATCH(20,{10,20,20,30},0,1)` (range) | `2` (first match) | PASS |
| `XMATCH(20,{10,20,20,30},0,-1)` (range) | `3` (last match, reverse) | PASS |
| `XMATCH("ban*",{apple,banana,cherry},2)` (range) | `2` (wildcard) | PASS |
| `XMATCH(30,arr,0,2)` | `3` (binary ascending) | PASS |
| `XMATCH(99,arr,0)` | `#N/A` (not found) | PASS |
| `XMATCH("BANANA",words)` | `2` (case-insensitive) | PASS |
| `XMATCH(30,{50,40,30,20,10},0,-2)` (range) | `3` (binary descending) | PASS |
| `XMATCH(5,{"5","x"})` (range) | `#N/A` (type-sensitive: 5 ≠ "5") | PASS |
| `XMATCH(3,J1:K2)` (2-D lookup_array) | `#VALUE!` | PASS |
| `XMATCH(30,arr,0,2) == XMATCH(30,arr,0,1)` (binary≡linear on sorted) | equal | PASS |

All four match_modes (exact / next-smaller / next-larger / wildcard), all four search_modes
(first→last / last→first / binary-asc / binary-desc), type-sensitivity, case-insensitive text
matching, tie direction, 2-D→`#VALUE!`, and the binary≡linear-on-sorted equivalence are correct.

## DIVERGENCE — needs owner decision

An **inline array-constant** lookup_array yields `#VALUE!` (both horizontal `{10,20,30}` and
vertical `{10;20;30}`); only a **range** lookup_array is accepted. This diverges from spec §2.5,
which lists XMATCH among the functions that accept array constants. The core matching logic is
fully correct with ranges (the overwhelmingly common real-world usage). Narrow divergence
confined to array-constant argument acceptance; flagged for owner decision / a possible
`fix/xmatch` enhancement. No fork source modified.
