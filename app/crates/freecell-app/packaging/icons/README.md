# FreeCell app icons

These are **placeholder** icons: a dark-green rounded square with a light spreadsheet
grid, a darker header row/column, and a couple of accent cells. They exist so the packaged
apps (macOS `.app`/`.dmg`, Windows installer, Linux `.deb`/`.AppImage`) have a real icon in
every format cargo-packager needs. They are intentionally simple — swap in a real,
professionally-designed icon when one is ready (see "Dropping in real icons" below).

## Files here

| File | Used by | Notes |
|---|---|---|
| `generate_icons.py` | — | **Source of truth.** Draws the design parametrically and emits every file below. Requires `pillow`. |
| `icon.svg` | reference | Vector version of the same design (emitted by the script), for a designer to build on. Not consumed by the build. |
| `icon.png` | — | 1024×1024 master (reference / high-res source). |
| `16x16.png` … `512x512.png` | Linux (`deb`, `AppImage`) | Installed into `usr/share/icons/hicolor/<size>/apps/`. cargo-packager reads each PNG's pixel dimensions from the file. |
| `128x128@2x.png` | Linux (retina) | The `@2x` suffix tells cargo-packager it's a high-density (256px) variant → `hicolor/256x256@2/apps/`. |
| `icon.icns` | macOS (`.app`, `.dmg`) | Copied verbatim into `Contents/Resources/icon.icns`. |
| `icon.ico` | Windows (`nsis`) | Multi-resolution (16/32/48/64/128/256). |

The `[package.metadata.packager].icons` list in `../../Cargo.toml` references these by path.
**cargo-packager `cd`s into the crate manifest dir before packaging**, so those paths are
relative to `crates/freecell-app/` (e.g. `packaging/icons/icon.icns`), not the workspace
root. Per platform: the `.icns` is used on macOS, the `.ico` on Windows, and the sized PNGs
on Linux (cargo-packager can also synthesize an `.icns`/`.ico` from PNGs, but we ship real
ones so nothing depends on that).

## Regenerating

```sh
pip install pillow
python3 generate_icons.py     # rewrites every file in this directory
```

The script is deterministic; edit the design parameters at the top (colors, cell count,
corner radius, accent cells) and re-run.

## Dropping in real icons

You (the user) said you can generate real icons. Here's exactly what to provide so they
slot straight in with **no config or code changes** — just replace the files here:

**What a real icon set needs:**

- **A source at ≥ 1024×1024**, square, with a transparent background — ideally a **vector
  SVG** (clean scaling) or a 1024×1024 (or larger) PNG. macOS icons should already include
  the rounded-square ("squircle") shape with transparent corners baked in; macOS does *not*
  round them for you in the `.app`.
- **macOS `icon.icns`** containing 16, 32, 64, 128, 256, 512, and 1024 px, each with an
  `@2x` retina variant (Apple's full set). Build it with `iconutil` from an `.iconset`
  (macOS), or `png2icns` / Pillow (cross-platform).
- **Windows `icon.ico`** containing 16, 32, 48, 64, 128, 256 px.
- **Linux PNGs** at 16, 32, 48, 64, 128, 256, 512 px (plus the 256px `128x128@2x.png`
  retina variant), transparent background.

**Two ways to drop them in:**

1. **Easiest:** replace `icon.png` (the ≥1024 master) with the real artwork and re-run
   `generate_icons.py` — it re-derives every sized PNG, the `.ico`, and the `.icns`. (For a
   truly polished `.icns` with hand-tuned small sizes, prefer option 2.)
2. **Full control:** generate `icon.icns`, `icon.ico`, and the sized PNGs with your own
   pipeline and drop them in here, **keeping the same filenames**. The packager config and
   scripts need no changes.

Keep the filenames stable — the `icons` list in `Cargo.toml` references them by name.
