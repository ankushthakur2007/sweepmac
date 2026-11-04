# SweepMac

Free, open-source Mac cleaner in Rust. No telemetry, no account, no animations tax.

## What it does

- **Cleanup** — caches, logs (per-app breakdown)
- **Applications** — leftover data from uninstalled apps
- **Privacy** — browser histories and cookies
- **Performance** — startup/login items audit with reversible disable
- **Protection** — quarantine audit, daemon audit, ClamAV malware scan (opt-in)
- **My Clutter** — old installers, stale downloads, large files
- **Duplicates** — byte-identical duplicate files (nothing preselected)
- **Space Lens** — biggest folders per location
- **Menu bar** — live health tiles (disk, memory, battery, load, net), in-process
- **Smart Care** — dashboard over all engines

Everything moves to Trash (reversible) except login-item disables (parked under
`~/.sweepmac/disabled`, restorable) and malware hits (never preselected).

## Build

```sh
cargo build --workspace   # debug
cargo test --workspace
bash packaging/bundle.sh # SweepMac.app (ad-hoc signed)
```

## Permissions

Privacy/system-cache scans need Full Disk Access. Protection setup installs
ClamAV via Homebrew on first visit (one click, ~300MB signatures).

## Download

Grab the DMG from [Releases](https://github.com/ankushthakur2007/sweepmac/releases/latest)
(drag to Applications, then right-click → Open on first launch — ad-hoc signed).
Apple Silicon, macOS 13+.

## Safety (the part most cleaners skip)

- **Trash-first** — everything moves to Trash, never hard-deleted
- **SIP-aware** — `SF_RESTRICTED` paths are skipped on every scan walk
- **Space Lens can't trash `~/Library`** — Clean button disabled on that route
- **System-app data is never "orphan"** — `com.apple.*` keys skipped
- **Pre-clean snapshots** — JSON manifest of every item before cleaning; aborts if save fails
- **Ignore list** — per-item, persisted, respected by all scans
- **Malware hits are never preselected** — opt-in cleaning only
- **Unsigned build** — ad-hoc signed, no cert required to build/run

## Layout

- `crates/sweep-core` — scan→clean pipeline traits
- `crates/sweep-scanners` — one module per feature
- `crates/sweep-health` — menu-bar sensors (syscalls, cached)
- `crates/sweep-shell` — egui app + native menu bar
- `crates/sweep-helper` — privileged worker scaffold (dev-only until signed)
- `packaging/` — bundle script, plists, signing notes
