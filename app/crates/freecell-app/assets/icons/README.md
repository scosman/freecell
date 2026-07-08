# Lucide icons (vendored, third-party)

This folder holds a small set of **Lucide** icons — the standard, open-source
[Lucide](https://lucide.dev) icon set — vendored so FreeCell's action-bar (chrome)
buttons render real formatting glyphs from a **FreeCell-owned asset source** instead of
Unicode text. They are loaded through the app's combined `AssetSource`
(`crates/freecell-app/src/shell/assets.rs`) and rendered with gpui-component's `Icon`
component via `Icon::empty().path("icons/<name>.svg")`.

**Why vendor?** The pinned `gpui-component-assets` bundle is a curated ~99-icon Lucide
subset that ships **no** typography/formatting icons (bold, italic, alignment, decimals,
…). Rather than repin gpui-component, we vendor exactly the icons we need here and compose
them with the existing bundle at the `AssetSource` layer (the bundle still resolves
`IconName::Loader` etc.).

**This directory contains ONLY Lucide icons and their license.** Do not add FreeCell's own
files here — everything in `icons/` is covered by the license in `LICENSE`, kept isolated
from our source so the license boundary is unambiguous.

## Authoring format

Each file is an unmodified Lucide icon in the same single-line, tintable form the
`gpui-component-assets` bundle uses: `fill="none" stroke="currentColor" stroke-width="2"`,
`24×24` viewBox. `stroke="currentColor"` is what lets gpui-component's `Icon` tint each
glyph to the button's foreground color (normal / selected / disabled), matching the
bundled icons. The only change from the upstream `icons/*.svg` source files is whitespace
minification to one line.

## Contents (16 icons — one per action-bar control)

| File | Action-bar button |
|---|---|
| `bold.svg` | Bold |
| `italic.svg` | Italic |
| `underline.svg` | Underline |
| `strikethrough.svg` | Strikethrough |
| `text-wrap.svg` | Wrap text |
| `baseline.svg` | Text color |
| `paint-bucket.svg` | Fill color |
| `grid-2x2.svg` | Borders |
| `text-align-start.svg` | Align left |
| `text-align-center.svg` | Align center |
| `text-align-end.svg` | Align right |
| `arrow-up-to-line.svg` | Align top |
| `separator-horizontal.svg` | Align middle |
| `arrow-down-from-line.svg` | Align bottom |
| `decimals-arrow-right.svg` | Increase decimals |
| `decimals-arrow-left.svg` | Decrease decimals |

## License

Lucide is distributed under the **ISC License** (with an MIT-licensed subset derived from
the Feather project). The full upstream license text is vendored verbatim in `LICENSE`.
