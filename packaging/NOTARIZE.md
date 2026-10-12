# Notarization (Developer ID required for distribution)
Ad-hoc sign (`codesign -s -`) works locally but has limits:
- SMJobBless privileged helper install will fail later (needs
Developer ID + embedded provisioning/requirements).
- Gatekeeper quarantines the .app on other Macs (right-click Open
or `xattr -d com.apple.quarantine` needed as workaround).
Real release signing (needs paid Apple Developer account):
codesign -s "Developer ID Application: Your Name (TEAMID)" --deep --force --options runtime SweepMac.app
xcrun notarytool submit SweepMac.zip --apple-id you@example.com --team-id TEAMID --password APP-PW --wait
xcrun stapler staple SweepMac.app
spctl -a -vvv -t install SweepMac.app
Do this only from a clean `packaging/bundle.sh` build artifact.
Ship the stapled .app inside a stapled DMG/ZIP for Gatekeeper pass.
Ad-hoc builds are fine for local dev; never ship them publicly.
Without stapling, first launch on other Macs requires explicit approval.
