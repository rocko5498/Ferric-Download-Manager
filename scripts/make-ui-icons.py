"""Draws the file-type badges, sidebar status icons and the empty-state picture (flat, rounded, one palette).
Run: python scripts/make-ui-icons.py   (needs Pillow). Writes crates/ferric-qt/qml/icons/{ft,sb}-*.png, empty-downloads.png
and ferric-logo-96.png. Drawn at 4x and reduced, so the edges stay clean at 200% scaling."""
import os
from PIL import Image, ImageDraw

HERE = os.path.dirname(__file__)
OUT = os.path.join(HERE, "..", "crates", "ferric-qt", "qml", "icons")
LOGO = os.path.join(HERE, "..", "assets", "ferric-logo.png")
S, N = 288, 72  # drawing size, final size
ORANGE, SLATE = (232, 104, 28, 255), (62, 66, 82, 255)
BLUE, PINK, VIOLET = (74, 120, 194, 255), (196, 90, 143, 255), (138, 92, 208, 255)
GREEN, RED, AMBER, GREY = (40, 160, 108, 255), (214, 69, 69, 255), (232, 163, 28, 255), (141, 138, 134, 255)
WHITE, NONE = (255, 255, 255, 255), (0, 0, 0, 0)


def new():
    im = Image.new("RGBA", (S, S), NONE)
    return im, ImageDraw.Draw(im)


def badge(d, color):
    d.rounded_rectangle([14, 14, S - 14, S - 14], 58, fill=color)


def save(im, name, size=N):
    im.resize((size, size), Image.LANCZOS).save(os.path.join(OUT, name), optimize=True)


def thick(d, pts, w, color=WHITE):
    d.line(pts, fill=color, width=w, joint="curve")
    for x, y in (pts[0], pts[-1]):
        d.ellipse([x - w // 2, y - w // 2, x + w // 2, y + w // 2], fill=color)


# ---- file types: a coloured rounded badge with a white glyph
def archive(im, d):
    badge(d, ORANGE)
    for y in range(64, 224, 36):
        d.rectangle([132, y, 156, y + 18], fill=WHITE)
        d.rectangle([156, y + 18, 180, y + 36], fill=WHITE)
    d.rounded_rectangle([124, 190, 188, 236], 10, fill=WHITE)
    d.rounded_rectangle([144, 206, 168, 222], 5, fill=ORANGE)


def document(im, d):
    badge(d, BLUE)
    d.rounded_rectangle([82, 56, 206, 232], 14, fill=WHITE)
    for y in (100, 132, 164):
        d.rounded_rectangle([102, y, 186, y + 14], 7, fill=BLUE)


def music(im, d):
    badge(d, PINK)
    d.ellipse([84, 168, 138, 212], fill=WHITE)
    d.rectangle([124, 70, 142, 190], fill=WHITE)
    d.polygon([(124, 70), (206, 96), (206, 134), (124, 110)], fill=WHITE)


def program(im, d):
    badge(d, SLATE)
    d.rounded_rectangle([62, 74, 226, 214], 16, fill=WHITE)
    d.rounded_rectangle([62, 74, 226, 112], 16, fill=ORANGE)
    d.rectangle([62, 96, 226, 112], fill=ORANGE)
    d.rounded_rectangle([86, 134, 138, 190], 8, fill=SLATE)
    d.rounded_rectangle([156, 134, 204, 156], 8, fill=SLATE)
    d.rounded_rectangle([156, 168, 204, 190], 8, fill=SLATE)


def video(im, d):
    badge(d, VIOLET)
    d.polygon([(112, 84), (112, 204), (212, 144)], fill=WHITE)


def image(im, d):
    badge(d, GREEN)
    d.ellipse([170, 70, 214, 114], fill=WHITE)
    d.polygon([(54, 214), (118, 120), (162, 178), (188, 150), (234, 214)], fill=WHITE)


def other(im, d):
    badge(d, GREY)
    d.polygon([(86, 56), (176, 56), (210, 90), (210, 232), (86, 232)], fill=WHITE)
    d.polygon([(176, 56), (176, 90), (210, 90)], fill=(205, 202, 198, 255))


# ---- sidebar: plain glyphs, no badge
def sb_all(im, d):
    for y in (82, 130, 178):
        d.rounded_rectangle([60, y, 228, y + 30], 15, fill=SLATE)


def sb_down(im, d):
    d.rounded_rectangle([124, 48, 164, 150], 14, fill=ORANGE)
    d.polygon([(76, 130), (212, 130), (144, 214)], fill=ORANGE)
    d.rounded_rectangle([60, 232, 228, 252], 10, fill=ORANGE)


def sb_pause(im, d):
    d.rounded_rectangle([78, 62, 124, 226], 14, fill=AMBER)
    d.rounded_rectangle([164, 62, 210, 226], 14, fill=AMBER)


def sb_done(im, d):
    d.ellipse([30, 30, 258, 258], fill=GREEN)
    thick(d, [(84, 150), (128, 194), (206, 100)], 30)


def sb_error(im, d):
    d.ellipse([30, 30, 258, 258], fill=RED)
    d.rounded_rectangle([128, 78, 160, 160], 16, fill=WHITE)
    d.ellipse([126, 184, 162, 220], fill=WHITE)


for name, fn in [("ft-archive", archive), ("ft-document", document), ("ft-music", music), ("ft-program", program),
                 ("ft-video", video), ("ft-image", image), ("ft-other", other), ("sb-all", sb_all),
                 ("sb-downloading", sb_down), ("sb-paused", sb_pause), ("sb-completed", sb_done), ("sb-error", sb_error)]:
    im, d = new()
    fn(im, d)
    save(im, f"{name}.png")

# ---- empty state: a soft orange disc with a download arrow dropping into a tray
E = 4
im = Image.new("RGBA", (192 * E, 192 * E), NONE)
d = ImageDraw.Draw(im)
d.ellipse([12 * E, 12 * E, 180 * E, 180 * E], fill=(253, 238, 227, 255))
d.rounded_rectangle([82 * E, 40 * E, 110 * E, 104 * E], 8 * E, fill=ORANGE)
d.polygon([(54 * E, 92 * E), (138 * E, 92 * E), (96 * E, 140 * E)], fill=ORANGE)
d.rounded_rectangle([50 * E, 128 * E, 142 * E, 146 * E], 9 * E, fill=SLATE)
d.rounded_rectangle([50 * E, 128 * E, 62 * E, 112 * E] if False else [50 * E, 112 * E, 62 * E, 146 * E], 6 * E, fill=SLATE)
d.rounded_rectangle([130 * E, 112 * E, 142 * E, 146 * E], 6 * E, fill=SLATE)
im.resize((192, 192), Image.LANCZOS).save(os.path.join(OUT, "empty-downloads.png"), optimize=True)

Image.open(LOGO).convert("RGBA").resize((96, 96), Image.LANCZOS).save(os.path.join(OUT, "ferric-logo-96.png"), optimize=True)
print("icons written:", len([f for f in os.listdir(OUT) if f.startswith(("ft-", "sb-", "empty-", "ferric-logo"))]))
