---
title: "How I Built a Mac Cleaner in Rust (and Reverse-Engineered CleanMyMac's Safety Model)"
published: false
description: "A native macOS cleaner in Rust + egui: menu-bar health monitor, ClamAV malware scanning, and the safety guardrails I extracted by reverse-engineering CleanMyMac."
tags: rust, macos, opensource, egui
canonical_url: https://github.com/ankushthakur2007/sweepmac
---

# How I Built a Mac Cleaner in Rust

CleanMyMac wanted $40 to renew. I cancelled, looked for a free alternative, and didn't find one that felt like a real Mac app. So I spent a few weeks writing my own. This is the story of SweepMac — and the parts that were genuinely interesting to build.

## What it does

SweepMac is a native macOS cleaner. The usual surface: caches, logs, trash, leftover app data, browser privacy data, a disk-usage map, startup audit. Plus a menu-bar health monitor (RAM, CPU, disk, network, thermals) and malware scanning via ClamAV. Free, open-source (BSD-3), no telemetry, no accounts, no subscription. ~3.2 MB app.

## The safety model (the part I'm most proud of)

Cleaning apps are dangerous software. One wrong click and you've trashed `~/Library`. So before writing a single scanner, I spent time reverse-engineering CleanMyMac's guardrails using REA (a reverse-engineering toolchain). Here's what I found and rebuilt:

**1. Trash-first, always.** The only remover in CleanMyMac is `MoveToTrashFileRemover` — everything goes to Trash, nothing is hard-deleted. SweepMac does the same: `trash_paths()` moves files to Trash, and the UI says "Undo from the Trash if needed."

**2. SIP check everywhere.** CleanMyMac calls `isFileProtectedByRootlessAtPath:` from 10 different frameworks. I traced it — it forwards to Apple's own `NSFileManager isFileProtectedByRootless:`. SweepMac uses the public equivalent: `stat` flags & `SF_RESTRICTED` (0x80000). Every scanner walk skips restricted paths. Verified live against `/System/Library/CoreServices` → protected.

**3. Ignore list.** CleanMyMac has an `IgnorableNodeType` framework with live publishers. SweepMac has a persisted set at `~/.sweepmac/ignored` — per-item "Ignore" button, skipped by all scans.

**4. Pre-clean snapshots.** CleanMyMac has `SnapshotsStorage` with `makeSnapshot`/`restore`. SweepMac writes a JSON manifest (`~/.sweepmac/snapshots/<ts>.json`) of every item before cleaning — and aborts the clean if the snapshot fails to save. Never clean what you didn't record.

**5. System-app data is never "orphan."** CleanMyMac's orphan sweep flags any reverse-DNS folder over 1 MB under Library whose bundle ID isn't in the installed-apps set. But that set is built only from `/Applications` + `~/Applications` — Apple's system apps live in `/System/Applications` and are never in the set. So `com.apple.mediaanalysisd` (428 MB), `com.apple.Maps` (13 MB) all qualified as "orphan" and were **preselected** for deletion. One click trashed live system data. Fix: skip `com.apple.*` keys (the same rule CleanMyMac's Protection module already uses).

**6. Space Lens can't trash user-data roots.** SpaceLens lists top-level children of `~` (Library, Documents, Movies) — and showed the same "Clean selected" button as every other page. One checkbox + click = trashing `~/Library`. Fix: disable the Clean button on that route entirely.

## The architecture

Five crates, one process:

- **sweep-core** — pipeline traits (`Scanner`), `JunkItem`, shared helpers (`home`, `dir_size`, `file_size`, `trash_paths`), ignore list, snapshots, SIP guard.
- **sweep-scanners** — one file per feature: SystemJunk, TrashBin, Uninstaller, Privacy, Performance, Protection, Clutter, SpaceLens, Malware, ClamAV setup. Scanners enumerate only, never delete.
- **sweep-health** — menu-bar sensors (raw syscalls + Mach APIs, std+libc only). 2s sampler thread, channels only.
- **sweep-shell** — egui app, native menu bar (NSStatusItem/NSMenu via objc), updater, login items. UI state lives here, not in scanners.
- **sweep-helper** — privileged worker scaffold (dev-only, never run privileged).

## The interesting bugs

**Pump deadlock.** The Protection page's first-run ClamAV setup streams log lines. I ended setup mode only when a log line matched `"Protected"` / `"already running"` / `"is up"` / `"failed"` / `"not found"`. But `"Cannot write config files."`, `"cannot launch clamd: …"`, and `"clamd started but not answering yet — retry shortly."` (the *expected* first-run outcome) matched none. Busy stayed true forever — Scan, Clean, and "Set up protection" all disabled until restart. Fix: treat channel disconnect (sender dropping) as "done." Added 2 tests for the exact case.

**240 subprocesses per second.** `update()` runs on every input event (~120 Hz). Per frame: `loginitems::status()` spawned `id -u` + `launchctl print`, `tray::menu_icon_enabled()` read the config file, `fda_granted()` did two `read_dir` calls. Fix: 5s TTL cache with invalidation on toggle — same pattern as the clamd ping cache.

**Clutter scanned the same trees 2–3×.** Roots included `/`, `/Users`, `/Applications`, `/Library`, and `~/Downloads` simultaneously, each walked 4 levels deep. `~/Downloads/old.dmg` was found through `/` (depth 4), `/Users` (depth 3), and the Downloads root (depth 1) — three identical rows, tripled totals, cleaning failed twice with "No such file." Fix: dedupe output by path. Also added `/Volumes` to the skip list — external disks and NAS shares were being walked 4 levels deep.

**Malware scan failures were invisible.** Read errors and clamd errors only `eprintln!`'d; the item list stayed empty and the UI said "Nothing found — this area is clean." Fix: count failed files, show the count in progress, and never claim "clean" after an incomplete pass.

## The ClamAV dance

ClamAV lives at `/opt/homebrew/sbin/clamd` (not in GUI PATH — `find_bin` covers it). First `freshclam` needs `DatabaseMirror` in the conf (Homebrew's default lacks it). clamd takes minutes to load signatures on first start — the ping loop waits 60s, don't shorten it. The setup flow handles all of this automatically: `brew install clamav` → generate configs → `freshclam` → launch `clamd`. No terminal commands for the user.

## What I skipped

- **iCloud placeholder awareness** — no cloud files are scanned, so nothing to guard.
- **Bulk scan progress/cancel** — I excluded Protection from "Scan all" (opt-in) rather than adding a cancel button. Clutter stays in bulk but is much faster now with root dedup and `/Volumes` skip.
- **Snapshot restore UI** — manifests are written and pruned to 10 newest. Restore is manual (open the JSON in `~/.sweepmac/snapshots/`).
- **Intel build** — Apple Silicon only. `rustup target add x86_64-apple-darwin` + release flag when someone asks.

## Numbers

- 78 tests (was 70 before this audit round)
- 5 crates, ~3.2 MB app
- 0 new warnings (only pre-existing objc-macro cfg noise + libc deprecation notes)
- 17 audit findings fixed in one pass

## Links

- GitHub: https://github.com/ankushthakur2007/sweepmac
- Latest release (DMG): https://github.com/ankushthakur2007/sweepmac/releases/latest
- Update checks run against GitHub Releases (`releases/latest`) — no self-hosted feed

Happy to answer questions about the Rust/egui architecture, the ClamAV TCP protocol, or how I extracted CleanMyMac's safety checks.