"""Draws the app icon source (1024x1024 PNG): two usage bars on a dark tile.

Usage: python3 scripts/make_icon.py src-tauri/icons/source.png
Then:  cargo tauri icon src-tauri/icons/source.png
"""
import sys
from PIL import Image, ImageDraw

S = 1024
SS = 4  # supersampling for smooth edges
img = Image.new("RGBA", (S * SS, S * SS), (0, 0, 0, 0))
d = ImageDraw.Draw(img)


def rr(box, radius, fill):
    d.rounded_rectangle([v * SS for v in box], radius=radius * SS, fill=fill)


rr((100, 100, 924, 924), 185, (24, 27, 34, 255))
for top, frac, color in ((330, 0.72, (240, 160, 60, 255)), (554, 0.38, (86, 196, 140, 255))):
    rr((220, top, 804, top + 140), 70, (255, 255, 255, 38))
    rr((220, top, 220 + int(584 * frac), top + 140), 70, color)

img.resize((S, S), Image.LANCZOS).save(sys.argv[1])
