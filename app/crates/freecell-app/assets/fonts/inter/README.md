# Inter (vendored, third-party)

This folder holds the **Inter** typeface — a third-party font vendored so FreeCell
renders in one predictable family on **every** platform (macOS, Linux, CI). It is the
app's UI/grid font, registered at startup via `cx.text_system().add_fonts(...)` (see
`crates/freecell-app/src/shell/fonts.rs`).

**This directory contains ONLY Inter and its license.** Do not add FreeCell's own files
here — everything in `inter/` is covered by the SIL Open Font License in `OFL.txt`, kept
isolated from our source so the license boundary is unambiguous.

## Contents (the four static RIBBI text faces)

| File | Family | Style | Weight |
|---|---|---|---|
| `Inter-Regular.ttf`    | Inter | Regular     | 400 |
| `Inter-Bold.ttf`       | Inter | Bold        | 700 |
| `Inter-Italic.ttf`     | Inter | Italic      | 400 |
| `Inter-BoldItalic.ttf` | Inter | Bold Italic | 700 |

All four share the family name **"Inter"**, so GPUI's font matcher selects the correct
face for a `font_weight(BOLD)` / `italic()` request. We vendor the **static** desktop
faces (not the variable font) so weight/italic resolve deterministically — important for
bit-stable render-test baselines. The other Inter weights (Thin…Black) and the `Display`
optical size are intentionally omitted; the app only uses Regular/Bold + their italics.

**Why TrueType (`.ttf`), not OpenType/CFF (`.otf`):** GPUI registers embedded fonts on macOS
via `CGFont::from_data_provider`, which cannot load CFF outlines — an `.otf` here loads fine
on Linux but makes `add_fonts` fail on macOS, so the whole UI silently reverts to the system
font. Inter's release ships both formats; we vendor the upstream **TrueType** (`glyf`) faces,
which load on every platform.

## Provenance & license

- **Upstream:** Inter — https://github.com/rsms/inter (© 2016 The Inter Project Authors).
- **Version:** Inter **4.1** (font `Version 4.001`), the four static RIBBI faces from the
  `ttf/` folder of the official `Inter-4.1.zip` release. **Unmodified** upstream files (verified
  byte-for-byte against the release).
- **License:** SIL Open Font License 1.1 — full text in [`OFL.txt`](./OFL.txt) (the license as
  shipped in the release). The OFL permits bundling and embedding in software.
