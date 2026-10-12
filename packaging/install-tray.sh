#!/usr/bin/env bash
# App autostart: the main app now carries the menu icon itself (single
# process), so autostart = login item for com.sweepmac.app. Preferred path is
# in-app (Settings → Start at login, via SMAppService); this script is the
# manual fallback using the bundled plist.
set -euo pipefail
PLIST="$HOME/Library/LaunchAgents/com.sweepmac.app.plist"
SRC="$(cd "$(dirname "$0")" && pwd)/com.sweepmac.app.plist"
if [ ! -f "$SRC" ]; then
  echo "missing $SRC — create it from packaging/com.sweepmac.tray.plist as a template"
  exit 1
fi
launchctl bootout "gui/$(id -u)/com.sweepmac.app" 2>/dev/null || true
cp "$SRC" "$PLIST"
launchctl bootstrap "gui/$(id -u)" "$PLIST" 2>&1 | head -n 3
echo "autostart installed (or use in-app Settings → Start at login)"
