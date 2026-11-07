#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
HOST_PATH="${1:-$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)/target/debug/ferric-host}"
MANIFEST_DIR="${HOME}/.config/google-chrome/NativeMessagingHosts"
MANIFEST_PATH="${MANIFEST_DIR}/com.ferric.host.json"

if [ ! -x "$HOST_PATH" ]; then
  echo "ferric-host not found or not executable at $HOST_PATH. Run cargo build -p ferric-host first." >&2
  exit 1
fi

mkdir -p "$MANIFEST_DIR"
sed "s|FERRIC_HOST_PATH|$HOST_PATH|g" "$SCRIPT_DIR/com.ferric.host.chrome.json" > "$MANIFEST_PATH"

echo "Installed com.ferric.host for Chrome"
echo "Manifest: $MANIFEST_PATH"
echo "Host: $HOST_PATH"
