#!/usr/bin/env python3
"""Generate FreeCell's PLACEHOLDER app icon in every format cargo-packager needs.

This is the single source of truth for the placeholder icon: it draws a simple
"spreadsheet app" mark (dark-green rounded square + a light cell grid with a
header row/column and a couple of accent cells) parametrically, then emits every
raster format the packager consumes:

  - icon.svg              vector reference (same design, for a designer to riff on)
  - icon.png              1024x1024 master
  - <n>x<n>.png           16/32/48/64/128/256/512/1024 (Linux hicolor + build inputs)
  - 128x128@2x.png        retina alias of the 256px raster (deb marks @2x as high-density)
  - icon.icns             macOS .app / .dmg icon
  - icon.ico              Windows NSIS installer + .exe icon

This is a PLACEHOLDER. See README.md in this directory for exactly what a real,
professionally-designed icon needs to drop in here.

Usage:  python3 generate_icons.py       # writes all files next to this script
Requires: Pillow  (pip install pillow)
"""

from __future__ import annotations

import os

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))

# --- design parameters (all proportional to the master size) ---------------------
MASTER = 1024
CORNER_RADIUS_FRAC = 0.222          # macOS-ish rounded square
BG_TOP = (22, 101, 52)              # #166534  (green-700)
BG_BOTTOM = (15, 74, 39)            # #0f4a27  (deeper green)
GRID_INSET_FRAC = 0.175             # inset of the cell panel from the icon edge
CELLS = 5                           # 5x5 cell grid
LINE_COLOR = (209, 250, 229, 235)   # light mint grid lines
HEADER_FILL = (5, 46, 22, 255)      # #052e16  darker header band
ACCENT_CELLS = {                    # (col, row): fill  -- suggest "data"
    (0, 0): (255, 255, 255, 255),
    (2, 1): (134, 239, 172, 255),   # green-300
    (4, 3): (255, 255, 255, 235),
    (1, 3): (134, 239, 172, 200),
}

# Raster sizes we emit as <n>x<n>.png (used by Linux packages + as icns/ico inputs).
PNG_SIZES = [16, 32, 48, 64, 128, 256, 512, 1024]
# Sizes embedded in the multi-resolution Windows .ico.
ICO_SIZES = [16, 32, 48, 64, 128, 256]


def _lerp(a: tuple[int, int, int], b: tuple[int, int, int], t: float) -> tuple[int, int, int]:
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def _rounded_mask(size: int, radius: int) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    return mask


def render_master(size: int = MASTER) -> Image.Image:
    """Draw the icon at `size`x`size` (RGBA, transparent outside the rounded square)."""
    # Vertical green gradient background.
    bg = Image.new("RGB", (size, size))
    px = bg.load()
    for y in range(size):
        color = _lerp(BG_TOP, BG_BOTTOM, y / (size - 1))
        for x in range(size):
            px[x, y] = color
    icon = bg.convert("RGBA")

    draw = ImageDraw.Draw(icon)
    inset = round(size * GRID_INSET_FRAC)
    panel = size - 2 * inset
    step = panel / CELLS
    line_w = max(1, round(size * 0.010))

    # Header row + header column (darker band) to read as a spreadsheet.
    draw.rectangle([inset, inset, inset + panel, inset + step], fill=HEADER_FILL)
    draw.rectangle([inset, inset, inset + step, inset + panel], fill=HEADER_FILL)

    # Accent "data" cells.
    for (col, row), fill in ACCENT_CELLS.items():
        x0 = inset + col * step
        y0 = inset + row * step
        draw.rectangle([x0, y0, x0 + step, y0 + step], fill=fill)

    # Grid lines.
    for i in range(CELLS + 1):
        off = inset + i * step
        draw.line([(inset, off), (inset + panel, off)], fill=LINE_COLOR, width=line_w)
        draw.line([(off, inset), (off, inset + panel)], fill=LINE_COLOR, width=line_w)

    # Clip to the rounded square.
    radius = round(size * CORNER_RADIUS_FRAC)
    mask = _rounded_mask(size, radius)
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    out.paste(icon, (0, 0), mask)
    return out


def write_svg(path: str) -> None:
    """Emit a vector reference of the same design (hand-built from the parameters)."""
    s = 1024
    inset = round(s * GRID_INSET_FRAC)
    panel = s - 2 * inset
    step = panel / CELLS
    radius = round(s * CORNER_RADIUS_FRAC)
    lw = max(1, round(s * 0.010))

    def hexc(c):
        return "#%02x%02x%02x" % (c[0], c[1], c[2])

    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{s}" height="{s}" viewBox="0 0 {s} {s}">',
        "  <!-- PLACEHOLDER FreeCell icon. See README.md in this directory. -->",
        "  <defs>",
        '    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">',
        f'      <stop offset="0" stop-color="{hexc(BG_TOP)}"/>',
        f'      <stop offset="1" stop-color="{hexc(BG_BOTTOM)}"/>',
        "    </linearGradient>",
        f'    <clipPath id="round"><rect width="{s}" height="{s}" rx="{radius}" ry="{radius}"/></clipPath>',
        "  </defs>",
        f'  <g clip-path="url(#round)">',
        f'    <rect width="{s}" height="{s}" fill="url(#bg)"/>',
        f'    <rect x="{inset}" y="{inset}" width="{panel}" height="{step:.1f}" fill="{hexc(HEADER_FILL)}"/>',
        f'    <rect x="{inset}" y="{inset}" width="{step:.1f}" height="{panel}" fill="{hexc(HEADER_FILL)}"/>',
    ]
    for (col, row), fill in ACCENT_CELLS.items():
        x0 = inset + col * step
        y0 = inset + row * step
        parts.append(
            f'    <rect x="{x0:.1f}" y="{y0:.1f}" width="{step:.1f}" height="{step:.1f}" fill="{hexc(fill)}"/>'
        )
    grid = hexc(LINE_COLOR)
    for i in range(CELLS + 1):
        off = inset + i * step
        parts.append(
            f'    <line x1="{inset}" y1="{off:.1f}" x2="{inset + panel}" y2="{off:.1f}" '
            f'stroke="{grid}" stroke-width="{lw}"/>'
        )
        parts.append(
            f'    <line x1="{off:.1f}" y1="{inset}" x2="{off:.1f}" y2="{inset + panel}" '
            f'stroke="{grid}" stroke-width="{lw}"/>'
        )
    parts.append("  </g>")
    parts.append("</svg>")
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(parts) + "\n")


def main() -> None:
    master = render_master(MASTER)

    # Master + per-size PNGs (Lanczos downscale keeps grid lines crisp).
    master.save(os.path.join(HERE, "icon.png"))
    rasters: dict[int, Image.Image] = {}
    for n in PNG_SIZES:
        img = master if n == MASTER else master.resize((n, n), Image.LANCZOS)
        rasters[n] = img
        img.save(os.path.join(HERE, f"{n}x{n}.png"))
    # Retina alias (256px raster named @2x so the packager tags it high-density).
    rasters[256].save(os.path.join(HERE, "128x128@2x.png"))

    # Windows multi-resolution .ico.
    master.save(
        os.path.join(HERE, "icon.ico"),
        format="ICO",
        sizes=[(n, n) for n in ICO_SIZES],
    )

    # macOS .icns (Pillow packs the standard icns sizes from the master).
    master.save(os.path.join(HERE, "icon.icns"), format="ICNS")

    # Vector reference.
    write_svg(os.path.join(HERE, "icon.svg"))

    print("wrote:", ", ".join(sorted(os.listdir(HERE))))


if __name__ == "__main__":
    main()
