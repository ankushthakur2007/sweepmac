#!/usr/bin/env bash
# Install SweepMac into /Applications + autostart on/off. Works unsigned
# (in-app SMAppService needs a Developer ID; this script is the path for
# ad-hoc builds). Idempotent.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_NAME="SweepMac.app"
LABEL="com.sweepmac.app"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"

usage() { echo "usage: install.sh [install|uninstall|on|off|status]"; }

cmd="${1:-install}"
case "$cmd" in
  install)
    bash "$ROOT/packaging/bundle.sh" >/dev/null
    rm -rf "/Applications/$APP_NAME"
    cp -R "$ROOT/$APP_NAME" "/Applications/$APP_NAME"
    xattr -dr com.apple.quarantine "/Applications/$APP_NAME" 2>/dev/null || true
    echo "installed to /Applications/$APP_NAME"
    "$0" on
    ;;
  uninstall)
    "$0" off || true
    rm -rf "/Applications/$APP_NAME"
    echo "uninstalled"
    ;;
  on)
    sed "s#/Applications/SweepMac.app#$(printf %s "/Applications/$APP_NAME")#" \
      "$ROOT/packaging/com.sweepmac.app.plist" > "$PLIST"
    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$PLIST"
    echo "autostart ON (app launches at login)"
    ;;
  off)
    launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
    rm -f "$PLIST"
    echo "autostart OFF"
    ;;
  status)
    if launchctl print "gui/$(id -u)/$LABEL" >/dev/null 2>&1; then
      echo "autostart ON"
    else
      echo "autostart OFF"
    fi
    [ -d "/Applications/$APP_NAME" ] && echo "app installed" || echo "app NOT installed"
    ;;
  *) usage; exit 2 ;;
esac
