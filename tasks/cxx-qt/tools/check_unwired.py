import json, re, glob

p = 'tasks/cxx-qt/catalog/actions.json'
d = json.load(open(p, encoding='utf-8'))
done = set(a['id'] for a in d['actions'] if a.get('status') == 'done')

qml = set()
for f in glob.glob('crates/ferric-qt/qml/**/*.qml', recursive=True):
    for m in re.finditer(r'actionId:\s*"([^"]+)"', open(f, encoding='utf-8').read()):
        qml.add(m.group(1))

unwired = done - qml
print(f'Done entries without QML actionId: {len(unwired)}')
for i in sorted(unwired):
    print(f'  {i}')
