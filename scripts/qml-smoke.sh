#!/usr/bin/env bash
# Starts ferric-qt in smoke mode (needs a display, e.g. xvfb-run) and fails on any QML problem.
# usage: scripts/qml-smoke.sh path/to/ferric-qt
set -u
exe="${1:?path to ferric-qt}"
log="$(mktemp)"
timeout 90 "$exe" --smoke > "$log" 2>&1
code=$?
steps="$(grep -c "SMOKE \(tab\|dialog\)" "$log")"
bad="$(grep -E "TypeError|ReferenceError|Binding loop|is not a type|Cannot (read|assign|anchor)|qml.*(error|warning)|QML .*:[0-9]+:[0-9]+:" "$log" | sort -u | head -20)"
if ! grep -q FERRIC-SMOKE-DONE "$log"; then echo "FAIL: smoke run did not finish (exit $code); log: $log"; tail -5 "$log"; exit 1; fi
if [ "$steps" -ne 19 ]; then echo "FAIL: expected 19 smoke steps, saw $steps"; exit 1; fi
if [ -n "$bad" ]; then echo "FAIL: QML problems:"; echo "$bad"; exit 1; fi
echo "PASS: $steps steps (5 tabs, 14 dialogs) opened without a QML error"
