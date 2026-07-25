---
status: complete
---

# Phase 5 — CHAR + CODE (§3.4/§3.5). Verified; branch skipped (known undefined-slot divergence, owner SKIP).

## Outcome

**Already present on `freecell-fixes` (inherited from `main`), and already Windows-1252 for
128–255 (spec O-1 satisfied) — verified, no branch created.**

- `fn_char` — `base/src/functions/text/char_code.rs` (guard ~L93, `WIN1252_128_159` table L12)
  (registry: enum `Char` mod.rs:222 · name-map:771 · dispatch:2410).
- `fn_code` — `base/src/functions/text/char_code.rs` (`char_to_win1252` ~L64) (registry: enum
  `Code` mod.rs:224 · name-map:773 · dispatch:2412).

Verified against functional_spec §3.4/§3.5 by running the explicit vectors plus the
`CODE(CHAR(n))==n` round-trip over the **defined** CP1252 range in a scratch module (deleted
after verification).

## Vectors run — all PASS

CHAR (§3.4): `CHAR(65)`→`A`, `CHAR(97)`→`a`, `CHAR(33)`→`!`, `CHAR(9)`→tab (U+0009),
`CHAR(65.9)`→`A` (truncation), `CHAR(0)`→`#VALUE!`, `CHAR(256)`→`#VALUE!`, `CHAR(128)`→`€`
(U+20AC), `CHAR(169)`→`©` (U+00A9).

CODE (§3.5): `CODE("A")`→`65`, `CODE("abc")`→`97` (first char), `CODE(" ")`→`32`,
`CODE("!")`→`33`, `CODE("")`→`#VALUE!`, `CODE(CHAR(200))`→`200` (round-trip).

Round-trip: `CODE(CHAR(n))==n` verified for **every n in 1..=255 except the 5 undefined CP1252
slots** {129, 141, 143, 144, 157} — all PASS.

## Known divergence — owner has decided to SKIP

The 5 undefined CP1252 slots (129, 141, 143, 144, 157) map to `#VALUE!` in CHAR (table entry
`\u{FFFD}` → `None`) rather than the C1 identity mapping that would make `CODE(CHAR(n))==n` hold
over the *full* 1..=255. Per Phase 0's escalation, **the owner has decided to SKIP this** — it
is the one acceptable divergence in the batch (Excel itself treats these codes ambiguously), so
the round-trip is asserted only over the defined range and no `fix/char-code` branch is created.
All explicit §3.4/§3.5 vectors pass. No fork source modified.
