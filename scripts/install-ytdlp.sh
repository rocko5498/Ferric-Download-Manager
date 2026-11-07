#!/usr/bin/env bash
# Install official yt-dlp release for Ferric (Linux).
# Does NOT vendor yt-dlp into the repo — downloads the GitHub release binary.
set -euo pipefail

DEST_DIR="${1:-${XDG_DATA_HOME:-$HOME/.local/share}/ferric/bin}"
ASSET="yt-dlp"
URL="https://github.com/yt-dlp/yt-dlp/releases/latest/download/${ASSET}"
mkdir -p "$DEST_DIR"
OUT="${DEST_DIR}/${ASSET}"

echo "Downloading official yt-dlp → ${OUT}"
curl -fsSL -o "$OUT" "$URL"
chmod +x "$OUT"

echo "OK: ${OUT}"
echo "Point Ferric Settings → Media → yt-dlp path to this file,"
echo "or export FERRIC_YTDLP=${OUT} and restart ferricd."
