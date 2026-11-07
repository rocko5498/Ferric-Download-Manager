"""Scans the QML for layout mistakes that make an interface look unaligned. Run: python scripts/qml-layout-lint.py [--detail]
Checks (each prints a count per file):
  V  a Text/dot/icon in a Row with no vertical centring (sits on the Row's top edge)
  T  a Text inside a fixed-height box with no centring (sits at the top of the box)
  H  hard-coded sizes that are not on the scale (heights and widths of controls)
  R  corner radii that are not Theme.radius / Theme.sp2
  C  colours written as literals instead of Theme colours
  F  font sizes that are not Theme.fontSize*"""
import glob
import os
import re
import sys
from collections import Counter, defaultdict

QML = os.path.join(os.path.dirname(__file__), "..", "crates", "ferric-qt", "qml")
detail = "--detail" in sys.argv
CENTRED = re.compile(r"anchors\.(verticalCenter|centerIn|fill)|verticalAlignment|anchors\.top|anchors\.bottom|height: parent\.height|Layout\.alignment")
ind = lambda s: len(s) - len(s.lstrip())


def block_end(L, i):
    depth = 0
    for k in range(i, len(L)):
        depth += L[k].count("{") - L[k].count("}")
        if depth <= 0:
            return k
    return i


found = defaultdict(lambda: defaultdict(list))
SCALE_H = {0, 1, 2, 3}
for path in sorted(glob.glob(os.path.join(QML, "*.qml"))):
    name = os.path.basename(path)
    if name == "Theme.qml":
        continue
    L = open(path, encoding="utf-8-sig").read().replace("\r\n", "\n").split("\n")
    for i, s in enumerate(L):
        t = s.strip()
        # V: direct children of a Row
        if re.match(r"^Row\s*\{", t):
            end = block_end(L, i)
            kids = [k for k in range(i + 1, end + 1) if ind(L[k]) == ind(s) + 4 and re.match(r"^\s*(Text|Rectangle|Image)\s*\{", L[k])]
            if len(kids) > 1:
                for k in kids:
                    kend = block_end(L, k)
                    if not CENTRED.search("\n".join(L[k:kend + 1])):
                        found[name]["V"].append(k + 1)
        # T: a Text directly inside a fixed-height Rectangle/Item
        if re.match(r"^Text\s*\{", t):
            j = i - 1
            while j >= 0 and (not L[j].strip() or ind(L[j]) >= ind(s)):
                j -= 1
            if j >= 0 and re.match(r"^\s*(Rectangle|Item)\s*\{", L[j]):
                head = "\n".join(L[j:min(j + 5, i)])
                tend = block_end(L, i)
                if re.search(r"\bheight:\s*(\d+|Theme\.\w+)", head) and not CENTRED.search("\n".join(L[i:tend + 1])):
                    found[name]["T"].append(i + 1)
        if re.search(r"\bradius:\s*\d+", s) and not re.search(r"radius:\s*[01]\b", s):
            found[name]["R"].append(i + 1)
        if re.search(r"""color:\s*"#[0-9a-fA-F]{3,8}"|border\.color:\s*"#""", s) and "Theme" not in s and '"#fff"' not in s and "#CC000000" not in s:
            found[name]["C"].append(i + 1)
        if re.search(r"font\.pixelSize:\s*\d+", s):
            found[name]["F"].append(i + 1)
        for m in re.finditer(r"(?<![\w.])(?:height|width):\s*(\d+)(?=[;} ]|$)", s):
            if int(m.group(1)) not in SCALE_H and not re.search(r"Layout|implicit", s):
                found[name]["H"].append(i + 1)
                break

cats = "VTHRCF"
print(f"{'file':32}" + "".join(f"{c:>6}" for c in cats))
tot = Counter()
for name in sorted(found, key=lambda n: -sum(len(v) for v in found[n].values())):
    row = {c: len(found[name].get(c, [])) for c in cats}
    tot.update(row)
    print(f"{name:32}" + "".join(f"{row[c]:>6}" for c in cats))
    if detail:
        for c in "VT":
            if found[name].get(c):
                print(f"    {c}: lines {found[name][c][:20]}")
print(f"{'TOTAL':32}" + "".join(f"{tot[c]:>6}" for c in cats))
