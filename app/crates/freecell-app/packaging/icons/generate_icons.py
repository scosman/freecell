#!/usr/bin/env python3
"""Generate FreeCell's app icons in every format cargo-packager needs.

These are the FINAL icons, not placeholders. The artwork lives in `app/icon.icon`
and is exported from the macOS "Icon Builder" (Icon Composer) app as two finished
1024x1024@2x (2048x2048) source PNGs. This script resamples those sources and insets
each to the 824/1024 icon grid (~10% transparent padding per side) before deriving
outputs — it does no drawing, rounding, or clipping (the shape is already baked in):

  - sourceMacOS-1024x1024@2x.png     the macOS squircle (rounded corners baked in,
                                     transparent corners) -> icon.icns
  - sourceWinLinux-1024x1024@2x.png  the square variant -> everything else

Two sources -> two pipelines (both padded to the 824/1024 content grid):

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

# Icon-grid content fraction: place the icon body at 824x824 inside the 1024x1024 canvas
# (100px transparent margin per side, 100 + 824 + 100 = 1024). The macOS icon grid and
# Windows/Linux guidance both suggest ~10% padding per side so the icon reads at a
# consistent visual size next to native apps. Our sources are full-bleed, so we inset
# BOTH to this fraction before deriving outputs.
CONTENT_FRAC = 824 / 1024


def load_source(path: str) -> Image.Image:
    """Load a 16-bit RGBA source and normalize it to 8-bit RGBA for resampling."""
    return Image.open(path).convert("RGBA")


def resized(source: Image.Image, size: int) -> Image.Image:
    """LANCZOS downscale of `source` to `size`x`size` (RGBA)."""
    if source.size == (size, size):
        return source.copy()
    return source.resize((size, size), Image.LANCZOS)


def padded_to_content_grid(source: Image.Image) -> Image.Image:
    """Inset `source` to the icon-grid content fraction on a transparent canvas.

    Scales the artwork to `CONTENT_FRAC` of the frame (824/1024) and centers it on a
    fully transparent square canvas the same size as `source`, leaving a symmetric margin
    (200px per side at the 2048px @2x master = 100px @1x). Building one padded master and
    downscaling it (LANCZOS) to every target size keeps the 824/1024 ratio at every
    resolution. Applied to BOTH sources so no output is full-bleed.
    """
    canvas_size = source.width
    content_size = round(canvas_size * CONTENT_FRAC)
    body = resized(source, content_size)
    canvas = Image.new("RGBA", source.size, (0, 0, 0, 0))
    offset = (canvas_size - content_size) // 2
    canvas.paste(body, (offset, offset))
    return canvas


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
    # Both sources are padded to the 824/1024 icon grid so no output is full-bleed.
    macos = padded_to_content_grid(load_source(SOURCE_MACOS))
    winlinux = padded_to_content_grid(load_source(SOURCE_WINLINUX))

    write_icns(macos)
    write_pngs(winlinux)
    write_ico(winlinux)

    print("wrote:", ", ".join(sorted(os.listdir(HERE))))


if __name__ == "__main__":
    main()
