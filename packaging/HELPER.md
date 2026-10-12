# SweepMac Privileged Helper (SMJobBless production path)

STATUS: argv protocol (`remove`/`trash`) is DEV-ONLY — any local caller
could delete files. Never ship it privileged. Production = SMJobBless + XPC.

Bundle layout (exact, required by SMJobBless):
  SweepMac.app/Contents/Library/LaunchServices/com.sweepmac.helper (tool)
  SweepMac.app/Contents/Info.plist            (SMPrivilegedExecutables)
  helper Contents/Info.plist                  (SMAuthorizedClients)
  Tool must embed its launchd plist (packaging/com.sweepmac.helper.plist)
  via linker flag: -sectcreate __TEXT __launchd_plist <that plist>.

App Info.plist (see bless::app_privileged_executables_plist):
  SMPrivilegedExecutables = { com.sweepmac.helper: <requirement> }
Helper Info.plist (see bless::helper_authorized_clients_plist):
  SMAuthorizedClients = [ <same requirement> ]
Requirement (bless::requirement_string(team_id, label)):
  anchor apple generic and identifier "LABEL" and certificate
  leaf[subject.OU] = "TEAMID"  (same Developer ID team both sides).

Sign (paid Developer ID, same team — TEAMID/NAME are placeholders):
  codesign -s "Developer ID Application: NAME (TEAMID)" --timestamp \
    --options runtime -f SweepMac.app/.../LaunchServices/com.sweepmac.helper
  codesign -s "Developer ID Application: NAME (TEAMID)" --timestamp \
    --options runtime -f SweepMac.app
  Verify: codesign -d -r- <each binary>; spctl -a -vvv --type execute <each>.

Bless flow (app side): AuthorizationCreate(kAuthorizationRightExecute) ->
  SMJobBless(kSMDomainSystemLaunchd, CFSTR("com.sweepmac.helper"), auth,
  &err). One admin prompt; system copies tool to
  /Library/PrivilegedHelperTools/ + plist to /Library/LaunchDaemons/.

Upgrade rule: SMJobBless installs only if bundled CFBundleVersion is
NEWER than installed (see bless::is_newer). Bump CFBundleVersion every
release or the old tool stays. Equal/older = no replace.

Uninstall: sudo launchctl bootout system/com.sweepmac.helper; sudo rm
  /Library/PrivilegedHelperTools/com.sweepmac.helper
  /Library/LaunchDaemons/com.sweepmac.helper.plist

REMAINS: paid Developer ID cert; notarize app+helper (staple); test
install/upgrade path on a clean VM; replace argv with XPC + uid/codesign
checks per call.
