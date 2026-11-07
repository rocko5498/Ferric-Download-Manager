import re

content = open('crates/ferric-qt/qml/main.qml', encoding='utf-8').read()
# Count text: "..." patterns
text_matches = re.findall(r'text:\s*"([^"]+)"', content)
# Count title: "..." patterns
title_matches = re.findall(r'title:\s*"([^"]+)"', content)
# Count placeholderText: "..." patterns
placeholder_matches = re.findall(r'placeholderText:\s*"([^"]+)"', content)

print(f'text: "..." patterns: {len(text_matches)}')
print(f'title: "..." patterns: {len(title_matches)}')
print(f'placeholderText: "..." patterns: {len(placeholder_matches)}')
print(f'Total user-visible strings: {len(text_matches) + len(title_matches) + len(placeholder_matches)}')

# Check how many already have qsTr
qs_tr_text = re.findall(r'text:\s*qsTr\("([^"]+)"\)', content)
print(f'Already wrapped in qsTr(): {len(qs_tr_text)}')
