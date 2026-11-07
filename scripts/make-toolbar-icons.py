"""Draws the toolbar icons: one consistent set (round badges, orange and slate) instead of mixed stock icons.
Run: python scripts/make-toolbar-icons.py   (needs Pillow). Writes crates/ferric-qt/qml/icons/tb-*-48.png.
The files are 96 px wide (drawn at 4x, then reduced) and shown at about 28 px, so they stay sharp at 200% scaling."""
import math
import os
from PIL import Image, ImageDraw

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "ferric-qt", "qml", "icons")
S = 384  # drawing size, reduced to 96
ORANGE = (232, 104, 28, 255)
SLATE = (62, 66, 82, 255)
RED = (214, 69, 69, 255)
GREEN = (40, 160, 108, 255)
WHITE = (255, 255, 255, 255)
NONE = (0, 0, 0, 0)


def canvas():
    img = Image.new("RGBA", (S, S), NONE)
    return img, ImageDraw.Draw(img)


def badge(d, color):
    d.ellipse([24, 24, S - 24, S - 24], fill=color)


def ring(d, color):
    d.ellipse([30, 30, S - 30, S - 30], outline=color, width=26)


def plus(d, color=WHITE):
    d.rounded_rectangle([S // 2 - 14, 100, S // 2 + 14, S - 100], 14, fill=color)
    d.rounded_rectangle([100, S // 2 - 14, S - 100, S // 2 + 14], 14, fill=color)


def play(d, color=WHITE):
    d.polygon([(146, 108), (146, 276), (286, 192)], fill=color)


def square(d, color=WHITE, box=(120, 120, 264, 264)):
    d.rounded_rectangle(list(box), 22, fill=color)


def check(d, color=WHITE):
    pts = [(104, 200), (166, 262), (284, 128)]
    d.line(pts, fill=color, width=36, joint="curve")
    for x, y in (pts[0], pts[-1]):
        d.ellipse([x - 18, y - 18, x + 18, y + 18], fill=color)


def cross(d, color=WHITE):
    d.line([(120, 120), (264, 264)], fill=color, width=34)
    d.line([(264, 120), (120, 264)], fill=color, width=34)
    for x, y in ((120, 120), (264, 264), (264, 120), (120, 264)):
        d.ellipse([x - 17, y - 17, x + 17, y + 17], fill=color)


def clock(d, color=WHITE):
    c = S // 2
    d.line([(c, c), (c, 110)], fill=color, width=30)
    d.line([(c, c), (264, 220)], fill=color, width=30)
    d.ellipse([c - 20, c - 20, c + 20, c + 20], fill=color)


def gear(img):
    d = ImageDraw.Draw(img)
    c = S // 2
    pts = []
    for i in range(8):
        a = math.radians(i * 45)
        for da, r in ((-13, 160), (13, 160), (19, 128), (26, 128)):
            pass
        for da, r in ((-17, 128), (-11, 168), (11, 168), (17, 128)):
            ang = a + math.radians(da)
            pts.append((c + r * math.cos(ang), c + r * math.sin(ang)))
    d.polygon(pts, fill=SLATE)
    d.ellipse([c - 112, c - 112, c + 112, c + 112], fill=SLATE)
    d.ellipse([c - 52, c - 52, c + 52, c + 52], fill=NONE)  # the hole (replace, not blend)


def make(name, draw):
    img, d = canvas()
    draw(img, d)
    img.resize((96, 96), Image.LANCZOS).save(os.path.join(OUT, f"tb-{name}-48.png"), optimize=True)


def add(img, d):      badge(d, ORANGE); plus(d)
def resume(img, d):   badge(d, ORANGE); play(d)
def stop(img, d):     badge(d, SLATE); square(d)
def stopall(img, d):  badge(d, SLATE); square(d, box=(100, 128, 180, 256)); square(d, box=(204, 128, 284, 256))
def delete(img, d):   badge(d, RED); cross(d)
def deldone(img, d):  badge(d, GREEN); check(d)
def options(img, d):  gear(img)
def scheduler(img, d): badge(d, SLATE); clock(d)
def startq(img, d):   ring(d, ORANGE); play(d, ORANGE)
def stopq(img, d):    ring(d, SLATE); square(d, SLATE)


for name, fn in [("add", add), ("resume", resume), ("stop", stop), ("stopall", stopall), ("delete", delete),
                 ("deldone", deldone), ("options", options), ("scheduler", scheduler), ("startq", startq),
                 ("stopq", stopq)]:
    make(name, fn)
print("wrote", len([f for f in os.listdir(OUT) if f.startswith("tb-")]), "icons to", os.path.normpath(OUT))
