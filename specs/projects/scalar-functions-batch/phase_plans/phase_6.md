---
status: complete
---

# Phase 6 — CLEAN (§3.6). Verified; branch skipped.

## Outcome

**Already present on `freecell-fixes` (inherited from `main`) — verified, no branch created.**
`fn_clean` — `base/src/functions/text/char_code.rs:137` (registry: enum `Clean` mod.rs:223 ·
name-map:772 · dispatch:2411). Implementation is `filter(|c| (c as u32) >= 32)` — strips ASCII
control 0–31 only, keeps 127/160/Unicode (spec O-2).

Verified against functional_spec §3.6 in a scratch module (deleted after verification).

## Vectors run (§3.6) — all PASS

| Vector | Expected | Result |
|---|---|---|
| `CLEAN(CHAR(9)&"text"&CHAR(10))` | `text` (tab + LF removed) | PASS |
| `CLEAN("Hello"&CHAR(7)&"World")` | `HelloWorld` (bell removed) | PASS |
| `CLEAN(CHAR(31)&"x")` | `x` (control 31 removed) | PASS |
| `CLEAN("normal text")` | `normal text` (no-op) | PASS |
| `CLEAN("keep"&CHAR(127))` | `keep` + DEL(127) — **127 kept** (≥ 32) | PASS |
| `CLEAN(CHAR(160)&"y")` | NBSP(160) + `y` — **160 kept** (CLEAN ≠ TRIM) | PASS |

## Test-vector note (not a divergence)

The §3.6 example `CLEAN(CHAR(31)&"x"&CHAR(0))`→`x` cannot be exercised verbatim because `CHAR(0)`
itself returns `#VALUE!` (§3.4 — CHAR domain is 1–255), so the NUL portion is un-producible via
`CHAR`. Verified the control-char strip with code 31 alone; the code-0 path is code-symmetric
with 31 under the single `>= 32` filter. This is a spec-example artifact, not an impl divergence.
No fork source modified.
