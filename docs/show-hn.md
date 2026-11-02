Show HN: SweepMac – free, open-source Mac cleaner in Rust

I got quoted $40/yr to renew CleanMyMac and decided to build my own instead. SweepMac is a native macOS cleaner: caches, logs, trash, leftover app data, browser privacy data, malware scanning, startup audit, and a disk-usage map. Free, open-source (BSD-3), no telemetry, no accounts, no subscription.

Tech: Rust + eframe/egui for the UI, raw syscalls (sysctl, getifaddrs, host_statistics64) for a menu-bar health monitor, ClamAV over localhost TCP for malware scanning. The menu-bar icon is a native NSStatusItem — no Electron.

Safety model (the part I'm most proud of): I reverse-engineered CleanMyMac's guardrails and rebuilt them. Everything moves to Trash — nothing is hard-deleted. SIP-protected paths (SF_RESTRICTED) are skipped on every filesystem walk. A snapshot manifest is written before any clean so you can see exactly what was moved. Per-item ignore list persisted across scans. Malware-infected files are never preselected.

Honest caveats: unsigned for now (right-click → Open on first launch), Apple Silicon only, macOS 13+. The "Update signatures" button runs freshclam with a generated config because Homebrew's default lies about the database mirror.

78 tests, 5 crates, ~3.2 MB app. DMG in releases.

GitHub: https://github.com/ankushthakur2007/sweepmac

Happy to answer questions about the Rust/egui architecture, the ClamAV TCP protocol, or how I extracted CleanMyMac's safety checks.