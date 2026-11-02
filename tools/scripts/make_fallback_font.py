#!/usr/bin/env python3
"""Rasterizes DejaVu Sans Mono into a 1-bit Playdate-style font (.fnt + table PNG).

The simulator uses this font when no Playdate SDK is available. With an SDK it
uses the SDK's own system font instead. Requires Pillow and DejaVu fonts.

    python3 tools/scripts/make_fallback_font.py crates/pdsim-gfx/assets
"""
import sys
from PIL import Image, ImageDraw, ImageFont

out = sys.argv[1]
W, H, SIZE = 8, 14, 12
font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", SIZE)
chars = [chr(c) for c in range(32, 127)]
cols = 16
rows = (len(chars) + cols - 1) // cols
img = Image.new("LA", (cols * W, rows * H), (0, 0))
lines = ["tracking=0"]
for i, ch in enumerate(chars):
    cell = Image.new("L", (W * 4, H * 4), 0)
    ImageDraw.Draw(cell).text((0, 0), ch, font=font, fill=255)
    cx, cy = (i % cols) * W, (i // cols) * H
    for y in range(H):
        for x in range(W):
            if cell.getpixel((x, y + 1)) >= 110:
                img.putpixel((cx + x, cy + y), (0, 255))
    lines.append(("space" if ch == " " else ch) + "\t" + str(W - 1))
img.save(f"{out}/fallback-table-{W}-{H}.png")
open(f"{out}/fallback.fnt", "w").write("\n".join(lines) + "\n")
