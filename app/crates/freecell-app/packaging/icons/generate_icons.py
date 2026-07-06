#!/usr/bin/env python3
"""Generate FreeCell's app icons in every format cargo-packager needs.

These are the FINAL icons, not placeholders. The artwork lives in `app/icon.icon`
and is exported from the macOS "Icon Builder" (Icon Composer) app as two finished
1024x1024@2x (2048x2048) source PNGs that this script only *resamples* — it does
no drawing, rounding, or clipping of its own (the shape/padding is already baked in):

  - sourceMacOS-1024x1024@2x.png     the macOS squircle (rounded corners + padding
                                     baked in, transparent corners) -> icon.icns
  - sourceWinLinux-1024x1024@2x.png  the square, full-bleed variant -> everything else

Two sources -> two pipelines:

  macOS source ->
    - icon.icns             macOS .app / .dmg icon (the only macOS output packaged)

  Win/Linux source ->
    - icon.ico              Windows NSIS installer + .exe icon (16/32/48/64/128/256)
    - <n>x<n>.png           16/32/48/64/128/256/512 (Linux hicolor entries)
    - 128x128@2x.png        retina alias of the 256px raster (deb marks @2x high-density)
    - icon.png              1024x1024 master (reference / high-res source)

Usage:  python3 generate_icons.py       # writes all files next to this script
Requires: Pillow  (pip install pillow)
"""

from __future__ import annotations

import os

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))

SOURCE_MACOS = os.path.join(HERE, "sourceMacOS-1024x1024@2x.png")
SOURCE_WINLINUX = os.path.join(HERE, "sourceWinLinux-1024x1024@2x.png")

# Raster sizes we emit as <n>x<n>.png (Linux hicolor entries; the 256px one doubles as
# the 128x128@2x retina alias). The .ico and .icns are built from independent in-memory
# resamples of the sources below, NOT from these files.
PNG_SIZES = [16, 32, 48, 64, 128, 256, 512]
# Sizes embedded in the multi-resolution Windows .ico.
ICO_SIZES = [16, 32, 48, 64, 128, 256]
# Pixel sizes Pillow packs into a .icns (16/32 stored only as @2x, i.e. 32/64px).
ICNS_SIZES = [32, 64, 128, 256, 512, 1024]

# The 1024px master reference (icon.png); larger than any sized PNG above.
MASTER = 1024


def load_source(path: str) -> Image.Image:
    """Load a 16-bit RGBA source and normalize it to 8-bit RGBA for resampling."""
    return Image.open(path).convert("RGBA")


def resized(source: Image.Image, size: int) -> Image.Image:
    """LANCZOS downscale of `source` to `size`x`size` (RGBA)."""
    if source.size == (size, size):
        return source.copy()
    return source.resize((size, size), Image.LANCZOS)


def write_pngs(source: Image.Image) -> None:
    """Emit every sized PNG, the retina @2x alias, and the icon.png master."""
    rasters = {n: resized(source, n) for n in PNG_SIZES}
    for n, img in rasters.items():
        img.save(os.path.join(HERE, f"{n}x{n}.png"))
    # Retina alias: a 256px raster named @2x so the packager tags it high-density.
    rasters[256].save(os.path.join(HERE, "128x128@2x.png"))
    # 1024px master reference (not one of the sized PNGs above).
    resized(source, MASTER).save(os.path.join(HERE, "icon.png"))


def write_ico(source: Image.Image) -> None:
    """Emit the multi-resolution Windows .ico.

    Pillow's ICO encoder skips any requested size larger than the base image and
    uses exact-size matches from `append_images` verbatim, so we hand it every size
    pre-resampled with LANCZOS and use the largest as the base.
    """
    frames = {n: resized(source, n) for n in ICO_SIZES}
    largest = max(ICO_SIZES)
    frames[largest].save(
        os.path.join(HERE, "icon.ico"),
        format="ICO",
        sizes=[(n, n) for n in ICO_SIZES],
        append_images=[frames[n] for n in ICO_SIZES if n != largest],
    )


def write_icns(source: Image.Image) -> None:
    """Emit the macOS .icns.

    Pillow's ICNS encoder resizes internally with a bicubic default, so we supply
    LANCZOS-resampled images for every packed size via `append_images` (keyed by
    width) to control downscale quality.
    """
    frames = [resized(source, n) for n in ICNS_SIZES]
    frames[-1].save(
        os.path.join(HERE, "icon.icns"),
        format="ICNS",
        append_images=frames[:-1],
    )


def main() -> None:
    macos = load_source(SOURCE_MACOS)
    winlinux = load_source(SOURCE_WINLINUX)

    write_icns(macos)
    write_pngs(winlinux)
    write_ico(winlinux)

    print("wrote:", ", ".join(sorted(os.listdir(HERE))))


if __name__ == "__main__":
    main()
