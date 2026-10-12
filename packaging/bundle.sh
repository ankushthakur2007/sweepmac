#!/usr/bin/env bash
set -euo pipefail
# Assembles SweepMac.app: main app + SweepTray login-item bundle. Idempotent.
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/SweepMac.app"
cargo build --release
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/sweepmac" "$APP/Contents/MacOS/sweepmac"
cp "$ROOT/packaging/Info.plist" "$APP/Contents/Info.plist"
cp "$ROOT/packaging/PkgInfo" "$APP/Contents/PkgInfo"
cp "$ROOT/packaging/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
codesign -s - --deep --force "$APP"
echo "--- bundle ---"
du -sh "$APP"
shasum -a 256 "$APP/Contents/MacOS/sweepmac"
