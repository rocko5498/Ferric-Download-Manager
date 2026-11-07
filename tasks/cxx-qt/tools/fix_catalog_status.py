import json, re, glob

p = 'tasks/cxx-qt/catalog/actions.json'
d = json.load(open(p, encoding='utf-8'))

# Find all QML actionIds
qml = set()
for f in glob.glob('crates/ferric-qt/qml/**/*.qml', recursive=True):
    for m in re.finditer(r'actionId:\s*"([^"]+)"', open(f, encoding='utf-8').read()):
        qml.add(m.group(1))

# Mark unwired "done" entries as "planned"
changed = 0
for a in d['actions']:
    if a.get('status') == 'done' and a['id'] not in qml:
        a['status'] = 'planned'
        changed += 1

json.dump(d, open(p, 'w', encoding='utf-8'), indent=2, ensure_ascii=False)
print(f'Fixed {changed} entries: done -> planned')

# Summary
done = sum(1 for a in d['actions'] if a.get('status') == 'done')
planned = sum(1 for a in d['actions'] if a.get('status') == 'planned')
print(f'Remaining: {done} done, {planned} planned')
