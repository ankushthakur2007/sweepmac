//! In-process menu-bar health monitor, native AppKit (no third-party tray lib).
//! Same proven recipe as the standalone tray: sharedApplication, accessory
//! policy, finishLaunching, NSStatusItem with SF Symbol icon, NSMenu with
//! live tiles, NSTimer on CommonModes. Sampling on worker threads, UI on main.
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use cocoa::appkit::{NSApp, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::{NSAutoreleasePool, NSString};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

use sweep_core::fmt_bytes;
use sweep_health::{Health, Sampler, TICK_SECS};

static mut CTX: *const TrayCtx = std::ptr::null();

struct TrayCtx {
    grade: id,
    disk: id,
    mem: id,
    batt: id,
    load: id,
    net: id,
    scan: id,
    tx_scan: Sender<()>,
}

fn nsstring(s: &str) -> id {
    unsafe { NSString::alloc(nil).init_str(s) }
}

fn menu_label(title: &str) -> id {
    unsafe {
        let item: id = msg_send![class!(NSMenuItem), alloc];
        let item: id = msg_send![item,
            initWithTitle: nsstring(title)
            action: nil
            keyEquivalent: nsstring("")];
        let _: () = msg_send![item, setEnabled: NO];
        item
    }
}

fn paint(h: &Health, ctx: &TrayCtx) {
    unsafe {
        let set = |item: id, s: String| {
            let _: () = msg_send![item, setTitle: nsstring(&s)];
        };
        set(ctx.grade, format!("Mac health: {}", h.grade()));
        set(
            ctx.disk,
            match (h.disk_free, h.disk_total) {
                (Some(f), Some(t)) => format!("Disk free: {} of {}", fmt_bytes(f), fmt_bytes(t)),
                _ => "Disk: unavailable".into(),
            },
        );
        set(
            ctx.mem,
            match h.mem_pressure_pct {
                Some(p) => format!("Memory pressure: {p}%"),
                None => "Memory: unavailable".into(),
            },
        );
        set(
            ctx.batt,
            match (h.battery_pct, h.battery_mins) {
                (Some(p), Some(m)) => format!("Battery: {p}% ({m} min left)"),
                (Some(p), None) => format!("Battery: {p}%"),
                _ => "Battery: unavailable".into(),
            },
        );
        set(
            ctx.load,
            match h.load_avg {
                Some(l) => format!("Load: {l:.2}"),
                None => "Load: unavailable".into(),
            },
        );
        set(
            ctx.net,
            match (h.down_bps, h.up_bps) {
                (Some(d), Some(u)) => format!("Net: ↓ {}/s · ↑ {}/s", fmt_bytes(d), fmt_bytes(u)),
                _ => "Net: …".into(),
            },
        );
    }
}

extern "C" fn on_scan(this: &Object, _cmd: Sel, _sender: id) {
    unsafe {
        if !CTX.is_null() {
            let _ = (*CTX).tx_scan.send(());
        }
        let _ = this;
    }
}

extern "C" fn on_open(_this: &Object, _cmd: Sel, _sender: id) {
    unsafe {
        let app = NSApp();
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    }
}

extern "C" fn on_quit(_this: &Object, _cmd: Sel, _sender: id) {
    unsafe {
        let app = NSApp();
        let _: () = msg_send![app, terminate: nil];
    }
}

extern "C" fn on_tick(_this: &Object, _cmd: Sel, _sender: id) {
    unsafe {
        if CTX.is_null() {
            return;
        }
        let ctx = &*CTX;
        // statics are main-thread confined (timer callbacks); raw-const
        // reads avoid shared `static mut` references.
        if let Some(rx) = unsafe { (*std::ptr::addr_of!(HEALTH_RX)).as_ref() } {
            let mut latest = None;
            while let Ok(h) = rx.try_recv() {
                latest = Some(h);
            }
            if let Some(h) = latest {
                paint(&h, ctx);
            }
        }
        if let Some(rx) = unsafe { (*std::ptr::addr_of!(SCAN_RX)).as_ref() } {
            let mut latest = None;
            while let Ok(s) = rx.try_recv() {
                latest = Some(s);
            }
            if let Some(label) = latest {
                let _: () = msg_send![ctx.scan, setTitle: nsstring(&label)];
            }
        }
    }
}

static mut HEALTH_RX: Option<Receiver<Health>> = None;
static mut SCAN_RX: Option<Receiver<String>> = None;
static mut TRAY_ITEM: id = std::ptr::null_mut();
static mut TRAY_TIMER: id = std::ptr::null_mut();
/// Bumped on every mount; sampler threads from older generations exit
/// instead of piling up behind replaced channels.
static MOUNT_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".sweepmac/config")
}

/// Menu-bar icon preference. Default ON; persisted as `menu_icon=0/1`.
pub fn menu_icon_enabled() -> bool {
    let Ok(text) = std::fs::read_to_string(config_path()) else {
        return true;
    };
    !text.lines().any(|l| l.trim() == "menu_icon=0")
}

pub fn set_menu_icon_enabled(on: bool) {
    let path = config_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim_start().starts_with("menu_icon="))
        .map(|l| l.to_string())
        .collect();
    lines.push(format!("menu_icon={}", if on { 1 } else { 0 }));
    let _ = std::fs::write(&path, lines.join("\n") + "\n");
}

/// Remove the status item (menu icon off). Safe to call anytime on main thread.
pub fn unmount() {
    unsafe {
        if !TRAY_ITEM.is_null() {
            let bar = NSStatusBar::systemStatusBar(nil);
            let _: () = msg_send![bar, removeStatusItem: TRAY_ITEM];
            // Balance the retain from mount().
            let _: () = msg_send![TRAY_ITEM, release];
            TRAY_ITEM = std::ptr::null_mut();
        }
        if !TRAY_TIMER.is_null() {
            let _: () = msg_send![TRAY_TIMER, invalidate];
            TRAY_TIMER = std::ptr::null_mut();
        }
        if !CTX.is_null() {
            let _ = Box::from_raw(CTX as *mut TrayCtx);
            CTX = std::ptr::null();
        }
    }
}

/// Builds the menu-bar monitor. Safe to call once, on the main thread, after
/// eframe owns NSApplication. Returns false when setup failed (app runs on
/// without the icon rather than crashing).
pub fn mount() -> bool {
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);

        let (tx_scan, rx_scan) = mpsc::channel::<()>();
        let (tx_health, rx_health) = mpsc::channel::<Health>();
        let (tx_label, rx_label) = mpsc::channel::<String>();
        HEALTH_RX = Some(rx_health);
        SCAN_RX = Some(rx_label);

        let menu: id = msg_send![class!(NSMenu), alloc];
        let menu: id = msg_send![menu, init];
        let grade = menu_label("SweepMac — sampling…");
        let disk = menu_label("Disk: …");
        let mem = menu_label("Memory: …");
        let batt = menu_label("Battery: …");
        let load = menu_label("Load: …");
        let net = menu_label("Net: …");
        for item in [grade, disk, mem, batt, load, net] {
            let _: () = msg_send![menu, addItem: item];
        }
        let sep: id = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![menu, addItem: sep];

        // Class registration panics if repeated (toggle off/on remounts),
        // so reuse the registered class when present.
        let target_cls = match Class::get("SweepTrayTargetHost") {
            Some(c) => c,
            None => {
                let mut decl =
                    ClassDecl::new("SweepTrayTargetHost", class!(NSObject)).unwrap();
                decl.add_method(sel!(doScan:), on_scan as extern "C" fn(&Object, Sel, id));
                decl.add_method(sel!(doOpen:), on_open as extern "C" fn(&Object, Sel, id));
                decl.add_method(sel!(doQuit:), on_quit as extern "C" fn(&Object, Sel, id));
                decl.add_method(sel!(doTick:), on_tick as extern "C" fn(&Object, Sel, id));
                decl.register()
            }
        };
        let target: id = msg_send![target_cls, alloc];
        let target: id = msg_send![target, init];

        let mk_action = |title: &str, sel_: Sel| -> id {
            let item: id = msg_send![class!(NSMenuItem), alloc];
            let item: id = msg_send![item,
                initWithTitle: nsstring(title)
                action: sel_
                keyEquivalent: nsstring("")];
            let _: () = msg_send![item, setTarget: target];
            item
        };
        let scan = mk_action("Quick scan", sel!(doScan:));
        let open = mk_action("Open SweepMac", sel!(doOpen:));
        let quit = mk_action("Quit", sel!(doQuit:));
        for item in [scan, open, quit] {
            let _: () = msg_send![menu, addItem: item];
        }

        CTX = Box::into_raw(Box::new(TrayCtx { grade, disk, mem, batt, load, net, scan, tx_scan }));

        // Status item: SF Symbol template icon (adapts to light/dark bar).
        let bar = NSStatusBar::systemStatusBar(nil);
        let item = bar.statusItemWithLength_(NSVariableStatusItemLength);
        if item == nil {
            eprintln!("SWEEP TRAY: statusItemWithLength_ returned nil");
            return false;
        }
        TRAY_ITEM = item;
        // statusItemWithLength_ returns an autoreleased object: retain it,
        // or TRAY_ITEM dangles after the next pool drain (crash on toggle).
        let _: () = msg_send![item, retain];
        let button = item.button();
        let img: id = msg_send![class!(NSImage),
            imageWithSystemSymbolName: nsstring("wind")
            accessibilityDescription: nsstring("SweepMac")];
        if img != nil {
            let _: () = msg_send![img, setTemplate: YES];
            let _: () = msg_send![button, setImage: img];
        } else {
            let _: () = msg_send![button, setTitle: nsstring("Sweep")];
        }
        let _: () = msg_send![button, setToolTip: nsstring("SweepMac")];
        let _: () = msg_send![item, setMenu: menu];

        // Sampler + scan workers (UI updates happen on main thread only).
        // Generation guard: remounting (icon toggle) retires older threads.
        let gen = MOUNT_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        thread::spawn(move || {
            let mut sampler = Sampler::new();
            loop {
                if MOUNT_GEN.load(std::sync::atomic::Ordering::SeqCst) != gen {
                    break;
                }
                let _ = tx_health.send(sampler.tick());
                thread::sleep(Duration::from_secs(TICK_SECS));
            }
        });
        thread::spawn(move || {
            while rx_scan.recv().is_ok() {
                let _ = tx_label.send("Scanning…".into());
                let mut all = Vec::new();
                for s in sweep_scanners::all() {
                    all.extend(s.scan(&|_| {}));
                }
                let bytes: u64 = all.iter().map(|j| j.size).sum();
                let _ = tx_label.send(format!("Reclaimable: {}", fmt_bytes(bytes)));
            }
        });

        // 1s UI timer on CommonModes: fires even while the menu is open.
        let timer: id = msg_send![class!(NSTimer),
            timerWithTimeInterval: 1.0
            target: target
            selector: sel!(doTick:)
            userInfo: nil
            repeats: YES];
        let runloop: id = msg_send![class!(NSRunLoop), currentRunLoop];
        let _: () = msg_send![runloop, addTimer: timer forMode: nsstring("kCFRunLoopCommonModes")];
        TRAY_TIMER = timer;
        eprintln!("SWEEP TRAY: mounted ok");
        true
    }
}
