import re

path = 'crates/ferric-qt/qml/main.qml'
content = open(path, encoding='utf-8').read()
lines = content.split('\n')

changed = 0
for i, line in enumerate(lines):
    # Skip lines already using qsTr
    if 'qsTr(' in line:
        continue

    # Pattern: text: "literal" (no concatenation)
    # Match text: "..." but not text: "..." + something
    new_line = re.sub(
        r'(text:\s*)"((?:[^"\\]|\\.)*)"(?!\s*\+)',
        lambda m: f'{m.group(1)}qsTr("{m.group(2)}")',
        line
    )

    # Pattern: title: "literal"
    new_line = re.sub(
        r'(title:\s*)"((?:[^"\\]|\\.)*)"(?!\s*\+)',
        lambda m: f'{m.group(1)}qsTr("{m.group(2)}")',
        new_line
    )

    # Pattern: placeholderText: "literal"
    new_line = re.sub(
        r'(placeholderText:\s*)"((?:[^"\\]|\\.)*)"(?!\s*\+)',
        lambda m: f'{m.group(1)}qsTr("{m.group(2)}")',
        new_line
    )

    if new_line != line:
        lines[i] = new_line
        changed += 1

open(path, 'w', encoding='utf-8').write('\n'.join(lines))
print(f'Wrapped {changed} lines in qsTr()')

# Verify
content2 = open(path, encoding='utf-8').read()
qs_count = len(re.findall(r'qsTr\(', content2))
print(f'Total qsTr() calls now: {qs_count}')
