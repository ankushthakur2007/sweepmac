#!/usr/bin/env bash
# Builds SweepMac-<ver>.dmg: drag-to-Applications installer. Idempotent.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VER="${1:-0.2.0}"
DMG="$ROOT/SweepMac-$VER.dmg"
STAGE="$(mktemp -d /tmp/sweepmac-dmg.XXXXXX)"
trap 'rm -rf "$STAGE"' EXIT

bash "$ROOT/packaging/bundle.sh" >/dev/null
cp -R "$ROOT/SweepMac.app" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "SweepMac $VER" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
echo "--- dmg ---"
ls -lh "$DMG"
shasum -a 256 "$DMG"
