# SweepMac

Free, open-source Mac cleaner in Rust. No telemetry, no account, no animations tax.

## What it does

- **Cleanup** — caches, logs, trash (per-app breakdown)
- **Applications** — leftover data from uninstalled apps
- **Privacy** — browser histories and cookies
- **Performance** — startup/login items audit with reversible disable
- **Protection** — quarantine audit, daemon audit, ClamAV malware scan (opt-in)
- **My Clutter** — old installers, stale downloads, large files
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

## Layout

- `crates/sweep-core` — scan→clean pipeline traits
- `crates/sweep-scanners` — one module per feature
- `crates/sweep-health` — menu-bar sensors (syscalls, cached)
- `crates/sweep-shell` — egui app + native menu bar
- `crates/sweep-helper` — privileged worker scaffold (dev-only until signed)
- `packaging/` — bundle script, plists, signing notes
