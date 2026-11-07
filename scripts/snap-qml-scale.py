"""Moves the plain-number gaps, margins and font sizes in the QML files onto the shared scale in Theme.qml.
Run: python scripts/snap-qml-scale.py [--dry]   It rewrites only literals (`spacing: 12`), never expressions.
  gaps / margins / padding: 0-2 stay, then snap to Theme.sp1 (5), sp2 (8), sp3 (13), sp4 (21), sp5 (34), sp6 (55)
  font sizes: <=11 -> fontSizeSmall (11), 12-14 -> fontSizeNormal (13), 15-18 -> fontSizeMedium (16), 19+ -> fontSizeLarge (21)"""
import glob
import os
import re
import sys

QML = os.path.join(os.path.dirname(__file__), "..", "crates", "ferric-qt", "qml")
GAP = r"(?:spacing|columnSpacing|rowSpacing|margins|leftMargin|rightMargin|topMargin|bottomMargin|padding|leftPadding|rightPadding|topPadding|bottomPadding)"
PATTERN = re.compile(r"(?P<head>(?<![\w.])(?:anchors\.)?(?P<prop>" + GAP + r"|font\.pixelSize)\s*:\s*)(?P<n>\d+)(?P<tail>\s*(?=[;}\n\r]|$))")


def snap_gap(n):
    if n <= 2:
        return None
    for limit, name in ((6, "sp1"), (10, "sp2"), (17, "sp3"), (27, "sp4"), (44, "sp5")):
        if n <= limit:
            return f"Theme.{name}"
    return "Theme.sp6"


def snap_font(n):
    for limit, name in ((11, "Small"), (14, "Normal"), (18, "Medium")):
        if n <= limit:
            return f"Theme.fontSize{name}"
    return "Theme.fontSizeLarge"


def rewrite(m):
    n = int(m.group("n"))
    new = snap_font(n) if m.group("prop") == "font.pixelSize" else snap_gap(n)
    return m.group("head") + new + m.group("tail") if new else m.group(0)


dry = "--dry" in sys.argv
total = 0
for path in sorted(glob.glob(os.path.join(QML, "*.qml"))):
    if os.path.basename(path) == "Theme.qml":
        continue
    raw = open(path, encoding="utf-8-sig", newline="").read()
    bom = open(path, "rb").read(3) == b"\xef\xbb\xbf"
    new, count = PATTERN.subn(rewrite, raw)
    changed = sum(1 for a, b in zip(PATTERN.findall(raw), PATTERN.findall(new)) if a != b)
    if new != raw:
        total += 1
        print(f"{os.path.basename(path):34} {len(PATTERN.findall(raw)):4} literals")
        if not dry:
            open(path, "w", encoding="utf-8-sig" if bom else "utf-8", newline="").write(new)
print(f"{total} files {'would change' if dry else 'changed'}")
