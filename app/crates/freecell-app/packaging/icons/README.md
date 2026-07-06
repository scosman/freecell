# FreeCell app icons

These are the **final** app icons (no longer placeholders). They exist in every format
cargo-packager needs so the packaged apps (macOS `.app`/`.dmg`, Windows installer, Linux
`.deb`/`.AppImage`) carry the real FreeCell icon.

## Where the artwork comes from

The master artwork lives in **`app/icon.icon`** and is edited with the macOS **Icon
Builder** (Icon Composer) app. Icon Builder exports two finished 1024×1024@2x
(2048×2048) source PNGs, checked in here:

| Source | Shape | Feeds |
|---|---|---|
| `sourceMacOS-1024x1024@2x.png` | macOS squircle — rounded corners + padding baked in, transparent corners | `icon.icns` |
| `sourceWinLinux-1024x1024@2x.png` | square, full-bleed | `icon.ico` + every sized PNG + `icon.png` |

`generate_icons.py` only **resamples** these finished sources (LANCZOS downscale to each
target size) — it does no drawing, rounding, or clipping. The shape and padding are
already baked into the sources.

## Files here

| File | Used by | Notes |
|---|---|---|
| `generate_icons.py` | — | **Source of truth for the derived files.** Resamples the two sources into every format below. Requires `pillow`. |
| `sourceMacOS-1024x1024@2x.png` | `generate_icons.py` | 2048×2048 macOS squircle export from Icon Builder. Not consumed by the build directly. |
| `sourceWinLinux-1024x1024@2x.png` | `generate_icons.py` | 2048×2048 square full-bleed export from Icon Builder. Not consumed by the build directly. |
| `icon.png` | — | 1024×1024 master (reference / high-res source, from the Win/Linux source). |
| `16x16.png` … `512x512.png` | Linux (`deb`, `AppImage`) | Installed into `usr/share/icons/hicolor/<size>/apps/`. cargo-packager reads each PNG's pixel dimensions from the file. |
| `128x128@2x.png` | Linux (retina) | The `@2x` suffix tells cargo-packager it's a high-density (256px) variant → `hicolor/256x256@2/apps/`. |
| `icon.icns` | macOS (`.app`, `.dmg`) | Copied verbatim into `Contents/Resources/icon.icns`. Built from the macOS source. |
| `icon.ico` | Windows (`nsis`) | Multi-resolution (16/32/48/64/128/256). Built from the Win/Linux source. |

The `[package.metadata.packager].icons` list in `../../Cargo.toml` references these by
path. **cargo-packager `cd`s into the crate manifest dir before packaging**, so those
paths are relative to `crates/freecell-app/` (e.g. `packaging/icons/icon.icns`), not the
workspace root. Per platform: the `.icns` is used on macOS, the `.ico` on Windows, and
the sized PNGs on Linux (cargo-packager can also synthesize an `.icns`/`.ico` from PNGs,
but we ship real ones so nothing depends on that).

## Regenerating

Re-export the two `source*-1024x1024@2x.png` files from `app/icon.icon` in Icon Builder
(only needed if the artwork changed), then:

```sh
pip install pillow
python3 generate_icons.py     # rewrites every derived file in this directory
```

The script is deterministic: `icon.icns` comes from the macOS source; `icon.ico`, the
sized PNGs, `128x128@2x.png`, and `icon.png` come from the Win/Linux source. Keep the
output filenames stable — the `icons` list in `Cargo.toml` references them by name.
