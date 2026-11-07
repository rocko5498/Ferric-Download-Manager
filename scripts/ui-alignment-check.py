"""Measures a screenshot of the Downloads list and fails if the rows are not aligned.
Run: python scripts/ui-alignment-check.py screenshot.png [sidebar_width=233] [row_height=26] [top=185]
For every row it finds the vertical centre of the row number, the status dot and the name text, and checks:
  - the number and the dot are centred on the same line (within 1 px)
  - the name text is centred on that line too (within 2 px: letters like g and y reach below the baseline)
  - consecutive rows are exactly row_height apart.
`top` is the y of the first data row's top edge. Needs Pillow."""
import sys
from collections import Counter
from PIL import Image

path = sys.argv[1]
sidebar = int(sys.argv[2]) if len(sys.argv) > 2 else 233
pitch = int(sys.argv[3]) if len(sys.argv) > 3 else 26
top = int(sys.argv[4]) if len(sys.argv) > 4 else 185
im = Image.open(path).convert("RGB")
W, H = im.size
px = im.load()
bottom = H - 120  # keep clear of the detail panel and status bar

bg = Counter(px[x, y] for x in range(sidebar + 60, W - 60, 9) for y in range(top, bottom, 9)).most_common(1)[0][0]

def ink(x, y):
    r, g, b = px[x, y]
    return abs(r - bg[0]) + abs(g - bg[1]) + abs(b - bg[2]) > 70

def runs(vals, gap=1):
    out, start, last = [], None, None
    for i, v in enumerate(vals):
        if v:
            if start is None: start = i
            last = i
        elif start is not None and i - last > gap:
            out.append((start, last)); start = None
    if start is not None: out.append((start, last))
    return out

# Left edge of the table: the first column right of the sidebar that has ink in the data rows.
x0 = next(x for x in range(sidebar, W) if any(ink(x, y) for y in range(top, bottom, 2)))

def centres(xa, xb):
    rows = [any(ink(x, y) for x in range(xa, xb)) for y in range(H)]
    return [((a + b) / 2, a, b) for a, b in runs(rows) if top <= a and b < bottom]

num = centres(x0, x0 + 26)
dot = centres(x0 + 28, x0 + 44)
txt = centres(x0 + 54, x0 + 54 + 90)
n = min(len(num), len(dot), len(txt))
print(f"table starts at x={x0}; rows found: numbers {len(num)}, dots {len(dot)}, names {len(txt)}")
bad = 0
for i in range(n):
    cn, cd, ct = num[i][0], dot[i][0], txt[i][0]
    ok = abs(cn - cd) <= 1 and abs(cd - ct) <= 2
    bad += not ok
    print(f"row {i + 1:2}: number {cn:6.1f}  dot {cd:6.1f}  text {ct:6.1f}   {'ok' if ok else 'OFF'}")
gaps = [dot[i + 1][0] - dot[i][0] for i in range(n - 1)]
uneven = [g for g in gaps if abs(g - pitch) > 1]
if gaps:
    print(f"row pitch {min(gaps):.1f}..{max(gaps):.1f} (want {pitch}): {'ok' if not uneven else 'UNEVEN'}")
fail = bad or uneven or n < 3
print("RESULT:", "PASS" if not fail else f"FAIL ({bad} misaligned rows, {len(uneven)} uneven gaps)")
sys.exit(1 if fail else 0)
