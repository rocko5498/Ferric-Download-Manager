#!/usr/bin/env python3
"""Wire audit (FQT-018): fail if any QML file references an action id that
isn't in tasks/cxx-qt/catalog/actions.json. See tasks/cxx-qt/AGENTS.md
"Wire audit" for the `actionId: "..."` convention this expects QML to use.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CATALOG = ROOT / "tasks" / "cxx-qt" / "catalog" / "actions.json"
QML_DIR = ROOT / "crates" / "ferric-qt" / "qml"
ACTION_ID_RE = re.compile(r'actionId\s*:\s*"([^"]+)"')


def main() -> int:
    catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
    known_ids = {a["id"] for a in catalog["actions"]}

    unknown = []
    for qml_file in sorted(QML_DIR.rglob("*.qml")):
        text = qml_file.read_text(encoding="utf-8")
        for match in ACTION_ID_RE.finditer(text):
            action_id = match.group(1)
            if action_id not in known_ids:
                unknown.append((qml_file.relative_to(ROOT), action_id))

    if unknown:
        print("Wire audit FAILED - QML references action ids missing from catalog/actions.json:")
        for path, action_id in unknown:
            print(f"  {path}: {action_id!r}")
        return 1

    print(
        f"Wire audit OK - 0 unknown action ids in "
        f"{QML_DIR.relative_to(ROOT)}/**/*.qml ({len(known_ids)} known ids in catalog)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
