"""Draws the app icon source (1024x1024 PNG): two usage lanes and a pace mark.

Usage: python3 scripts/make_icon.py src-tauri/icons/source.png
Then:  cargo tauri icon src-tauri/icons/source.png
"""
import sys
from PIL import Image, ImageDraw

S = 1024
SS = 4  # supersampling for smooth edges
img = Image.new("RGBA", (S * SS, S * SS), (0, 0, 0, 0))

# Tile: vertical gradient, clipped to a rounded square with the macOS margin.
TOP, BOTTOM = (36, 42, 58), (16, 19, 27)
grad = Image.new("RGBA", (S * SS, S * SS))
gd = ImageDraw.Draw(grad)
for y in range(S * SS):
    t = y / (S * SS - 1)
    gd.line([(0, y), (S * SS, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)) + (255,))
mask = Image.new("L", (S * SS, S * SS), 0)
ImageDraw.Draw(mask).rounded_rectangle([100 * SS, 100 * SS, 924 * SS, 924 * SS], radius=190 * SS, fill=255)
img.paste(grad, (0, 0), mask)

d = ImageDraw.Draw(img)


def rr(box, radius, fill):
    d.rounded_rectangle([v * SS for v in box], radius=radius * SS, fill=fill)


LEFT, RIGHT, H = 250, 774, 124
for top, frac, color in ((346, 0.68, (240, 162, 59, 255)), (554, 0.39, (76, 195, 138, 255))):
    rr((LEFT, top, RIGHT, top + H), H // 2, (255, 255, 255, 36))
    rr((LEFT, top, LEFT + int((RIGHT - LEFT) * frac), top + H), H // 2, color)
# Pace mark across both lanes.
rr((676, 304, 696, 720), 10, (255, 255, 255, 255))

img.resize((S, S), Image.LANCZOS).save(sys.argv[1])
