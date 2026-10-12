//! SweepMac shell: sidebar router + module pages.
//! Operate-mode UI: one accent, neutral sidebar, tabular sizes, teaching empty
//! states. No decorative motion; the progress bar is state, not choreography.
use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use eframe::egui;
use sweep_core::{fmt_bytes, ignore, snapshot, trash_paths, CleanResult, JunkItem, Progress};

mod updater;
mod loginitems;
mod tray;

const ACCENT: egui::Color32 = egui::Color32::from_rgb(10, 132, 255);

/// Latest update-check outcome, written by the checker thread.
static UPDATE_RESULT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Route {
    Welcome,
    SmartCare,
    Cleanup,
    Applications,
    Privacy,
    Performance,
    Protection,
    Clutter,
    SpaceLens,
}

impl Route {
    fn meta() -> &'static [(Route, &'static str, &'static str, bool)] {
        use Route::*;
        &[
            (Welcome, "Welcome", "First-run setup", true),
            (SmartCare, "Smart Care", "All systems at a glance", true),
            (Cleanup, "Cleanup", "Caches, logs, trash", true),
            (Applications, "Applications", "Leftover app data", true),
            (Privacy, "Privacy", "Browser data", true),
            (Performance, "Performance", "Startup items audit", true),
            (Protection, "Protection", "Suspicious files audit", true),
            (Clutter, "My Clutter", "Old & large files", true),
            (SpaceLens, "Space Lens", "Large folders", true),
        ]
    }
    fn scanners(&self) -> Vec<Box<dyn sweep_core::Scanner>> {
        match self {
            Route::Welcome | Route::SmartCare => vec![],
            Route::Cleanup => sweep_scanners::all(),
            Route::Applications => sweep_scanners::uninstallers(),
            Route::Privacy => sweep_scanners::privacy(),
            Route::Performance => sweep_scanners::performance(),
            Route::Protection => sweep_scanners::protection(),
            Route::Clutter => sweep_scanners::clutter(),
            Route::SpaceLens => sweep_scanners::spacelens(),
        }
    }
}

enum WorkerMsg {
    Progress(Progress),
    Done(Vec<JunkItem>),
    Cleaned(sweep_core::CleanResult),
}

#[derive(Default)]
struct ModulePage {
    items: Vec<JunkItem>,
    busy: bool,
    progress: Progress,
    status: String,
    rx: Option<Receiver<WorkerMsg>>,
    setup_log: Vec<String>,
    setup_rx: Option<Receiver<String>>,
}

impl ModulePage {
    fn pump(&mut self) {
        let mut finished = false;
        if let Some(rx) = &self.rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    WorkerMsg::Progress(p) => self.progress = p,
                    WorkerMsg::Done(items) => {
                        let ignored = ignore::load();
                        let items: Vec<JunkItem> =
                            items.into_iter().filter(|i| !ignore::is_ignored(&i.path, &ignored)).collect();
                        let total: u64 = items.iter().map(|i| i.size).sum();
                        self.status = if items.is_empty() {
                            "Nothing found — this area is clean.".into()
                        } else {
                            format!("Found {} items ({})", items.len(), fmt_bytes(total))
                        };
                        self.items = items;
                        self.busy = false;
                        finished = true;
                    }
                    WorkerMsg::Cleaned(r) => {
                        self.status = if r.errors.is_empty() {
                            "Moved to Trash. Undo from the Trash if needed.".into()
                        } else {
                            format!("Finished with {} errors — see terminal.", r.errors.len())
                        };
                        for e in r.errors {
                            eprintln!("clean error: {e}");
                        }
                        self.items.clear();
                        self.busy = false;
                        finished = true;
                    }
                }
            }
        }
        // Setup log stream (Protection page first-run flow).
        if let Some(rx) = &self.setup_rx {
            while let Ok(line) = rx.try_recv() {
                let finished = line.starts_with("Protected")
                    || line.contains("already running")
                    || line.contains("is up")
                    || line.contains("failed")
                    || line.contains("not found");
                self.setup_log.push(line);
                if self.setup_log.len() > 60 {
                    let excess = self.setup_log.len() - 60;
                    self.setup_log.drain(..excess);
                }
                if finished {
                    self.status = cached_clam_status();
                    self.busy = false;
                    self.setup_rx = None;
                    break;
                }
            }
        }
        if finished {
            self.rx = None;
        }
    }

    fn scan(&mut self, scanners: Vec<Box<dyn sweep_core::Scanner>>) {
        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        self.rx = Some(rx);
        self.busy = true;
        self.items.clear();
        self.status = "Scanning…".into();
        thread::spawn(move || {
            let mut all = Vec::new();
            for s in &scanners {
                let tag = s.name().to_string();
                for item in s.scan(&|p| {
                    let _ = tx.send(WorkerMsg::Progress(Progress {
                        label: format!("{tag}: {}", p.label),
                        ..p
                    }));
                }) {
                    all.push(item);
                }
            }
            let _ = tx.send(WorkerMsg::Done(all));
        });
    }

    fn clean(&mut self, disabler: Option<fn(&JunkItem) -> Result<String, String>>) {
        let selected: Vec<JunkItem> =
            self.items.iter().filter(|i| i.selected).cloned().collect();
        if selected.is_empty() {
            return;
        }
        // Pre-clean snapshot: never clean what we didn't record.
        let snap_items: Vec<(std::path::PathBuf, u64, String)> =
            selected.iter().map(|i| (i.path.clone(), i.size, i.category.clone())).collect();
        if let Err(e) = snapshot::save_to(&snapshot::dir(), &snap_items) {
            self.status = format!("Snapshot failed ({e}) — clean aborted.");
            self.busy = false;
            return;
        }
        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        self.rx = Some(rx);
        self.busy = true;
        self.status = if disabler.is_some() { "Disabling…".into() } else { "Moving to Trash…".into() };
        thread::spawn(move || {
            let mut out = CleanResult::default();
            match disabler {
                Some(d) => {
                    for item in &selected {
                        match d(item) {
                            Ok(note) => {
                                out.removed += 1;
                                if !note.is_empty() {
                                    eprintln!("disable note {}: {note}", item.path.display());
                                }
                            }
                            Err(e) => out.errors.push(format!("{}: {e}", item.path.display())),
                        }
                    }
                }
                None => {
                    let paths: Vec<std::path::PathBuf> = selected.iter().map(|i| i.path.clone()).collect();
                    out = trash_paths(&paths);
                }
            }
            let _ = tx.send(WorkerMsg::Cleaned(out));
        });
    }
}

/// ClamAV status cached 5s: the check opens a TCP connection, and the
/// Protection banner calls it every frame.
fn cached_clam_status() -> String {
    static CACHE: std::sync::Mutex<Option<(String, std::time::Instant)>> =
        std::sync::Mutex::new(None);
    let mut g = CACHE.lock().unwrap();
    let stale = g
        .as_ref()
        .map(|(_, t)| t.elapsed() > std::time::Duration::from_secs(5))
        .unwrap_or(true);
    if stale {
        *g = Some((sweep_scanners::clamsetup::status(), std::time::Instant::now()));
    }
    g.as_ref().unwrap().0.clone()
}

fn onboarded_file() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".sweepmac/onboarded")
}

fn fda_granted() -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    ["Library/Mail", "Library/Messages"]
        .iter()
        .any(|sub| std::fs::read_dir(format!("{home}/{sub}")).is_ok())
}

struct SweepApp {
    route: Route,
    pages: BTreeMap<u8, ModulePage>,
    totals: BTreeMap<u8, (usize, u64)>,
    bulk_rx: Option<Receiver<(u8, Vec<JunkItem>)>>,
    bulk_busy: bool,
    update_msg: Option<String>,
    update_busy: bool,
    tray_mounted: bool,
    update_failed: bool,
}

impl Default for SweepApp {
    fn default() -> Self {
        let mut pages = BTreeMap::new();
        pages.insert(0, ModulePage::default());
        pages.insert(1, ModulePage::default());
        let start = if onboarded_file().exists() { Route::SmartCare } else { Route::Welcome };
        let mut app = Self { route: start, pages, totals: BTreeMap::new(), bulk_rx: None, bulk_busy: false, update_msg: None, update_busy: false, tray_mounted: false, update_failed: false };
        app.page_mut().status = "Press Scan to see what can be reclaimed.".into();
        app
    }
}

impl SweepApp {
    fn key(route: Route) -> u8 {
        match route {
            Route::Welcome => 8,
            Route::SmartCare => 7,
            Route::Cleanup => 0,
            Route::Applications => 1,
            Route::Privacy => 2,
            Route::Performance => 3,
            Route::SpaceLens => 4,
            Route::Protection => 5,
            Route::Clutter => 6,
        }
    }
    fn page_mut(&mut self) -> &mut ModulePage {
        self.pages.entry(Self::key(self.route)).or_default()
    }

    fn render_page(&mut self, ui: &mut egui::Ui, title: &str, blurb: &str) {
        let route = self.route;
        ui.heading(title);
        ui.label(egui::RichText::new(blurb).weak());
        // First-run protection setup banner (Protection page only).
        // Auto-starts once per session when protection isn't ready — the user
        // should never have to touch a terminal.
        if route == Route::Protection {
            static AUTO_SETUP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            let state = cached_clam_status();
            let needs_setup = !state.starts_with("Protected");
            if needs_setup
                && !AUTO_SETUP.swap(true, std::sync::atomic::Ordering::SeqCst)
                && self.page_mut().setup_rx.is_none()
            {
                let (tx, rx) = mpsc::channel::<String>();
                self.page_mut().setup_rx = Some(rx);
                self.page_mut().busy = true;
                thread::spawn(move || {
                    sweep_scanners::clamsetup::setup(tx);
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(cached_clam_status()).strong());
                let running = self.page_mut().setup_rx.is_some();
                if ui.add_enabled(!running, egui::Button::new("Set up protection")).clicked() {
                    let (tx, rx) = mpsc::channel::<String>();
                    self.page_mut().setup_rx = Some(rx);
                    self.page_mut().setup_log.clear();
                    self.page_mut().busy = true;
                    thread::spawn(move || {
                        sweep_scanners::clamsetup::setup(tx);
                    });
                }
            });
            let log: Vec<String> = self.page_mut().setup_log.iter().rev().take(6).rev().cloned().collect();
            for line in log {
                ui.label(egui::RichText::new(line).small().weak());
            }
            ui.add_space(4.0);
        }
        ui.add_space(8.0);

        // Actions row.
        let busy = self.page_mut().busy;
        ui.horizontal(|ui| {
            let scan = ui.add_enabled(!busy, egui::Button::new("Scan").fill(ACCENT));
            if scan.clicked() {
                self.page_mut().scan(route.scanners());
            }
            let has_sel = self.page_mut().items.iter().any(|i| i.selected);
            let clean_btn = ui.add_enabled(
                !busy && has_sel,
                egui::Button::new(egui::RichText::new("Clean selected").color(egui::Color32::from_rgb(200, 40, 40))),
            );
            if clean_btn.clicked() {
                let disabler = match route {
                    // Login items are disabled (moved aside + bootout), never trashed.
                    Route::Performance => Some(sweep_scanners::performance::disable as fn(&JunkItem) -> Result<String, String>),
                    _ => None,
                };
                self.page_mut().clean(disabler);
            }
            if busy {
                ui.spinner();
            }
        });

        // Progress = state feedback.
        {
            let page = self.page_mut();
            if page.busy && page.progress.total > 0 {
                ui.add(
                    egui::ProgressBar::new(page.progress.done as f32 / page.progress.total.max(1) as f32)
                        .text(&page.progress.label),
                );
            }
            ui.label(egui::RichText::new(&page.status).weak());
        }
        ui.separator();

        // Selection summary + select all/none.
        {
            let page = self.page_mut();
            let sel: u64 = page.items.iter().filter(|i| i.selected).map(|i| i.size).sum();
            ui.horizontal(|ui| {
                ui.strong(format!("Selected: {}", fmt_bytes(sel)));
                if ui.small_button("All").clicked() {
                    for i in &mut page.items {
                        i.selected = true;
                    }
                }
                if ui.small_button("None").clicked() {
                    for i in &mut page.items {
                        i.selected = false;
                    }
                }
            });
        }
        ui.add_space(4.0);

        // Grouped list: category header + subtotal, rows with right-aligned sizes.
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        {
            let page = self.page_mut();
            for (idx, item) in page.items.iter().enumerate() {
                groups.entry(item.category.clone()).or_default().push(idx);
            }
        }
        if groups.is_empty() && !self.page_mut().busy {
            ui.label(egui::RichText::new(
                "Nothing here yet. Press Scan — results appear grouped by source, biggest first.",
            ).weak());
        }
        let mut ignored_drop: Vec<usize> = Vec::new();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (cat, idxs) in &groups {
                let sub: u64 = idxs.iter().map(|&k| self.page_mut().items[k].size).sum();
                ui.horizontal(|ui| {
                    ui.strong(cat.as_str());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(fmt_bytes(sub));
                    });
                });
                for &k in idxs {
                    let page = self.page_mut();
                    let item = &mut page.items[k];
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut item.selected, "");
                        ui.monospace(fmt_bytes(item.size));
                        ui.label(item.path.display().to_string());
                        if ui.small_button("Ignore").clicked() {
                            let _ = ignore::add(&item.path.display().to_string());
                            item.selected = false;
                            ignored_drop.push(k);
                        }
                    });
                }
                ui.add_space(6.0);
            }
        });
        // Drop ignored rows (persisted above); descending keeps indices valid.
        ignored_drop.sort_unstable();
        ignored_drop.dedup();
        for &k in ignored_drop.iter().rev() {
            let page = self.page_mut();
            if k < page.items.len() {
                page.items.remove(k);
            }
        }
    }

    fn scan_all(&mut self) {
        // One bulk run at a time, and never under a manual page scan.
        if self.bulk_busy || self.pages.values().any(|p| p.busy) {
            return;
        }
        let (tx, rx) = mpsc::channel::<(u8, Vec<JunkItem>)>();
        self.bulk_rx = Some(rx);
        self.bulk_busy = true;
        for (route, _, _, live) in Route::meta() {
            if *live && !matches!(route, Route::SmartCare | Route::Welcome) {
                self.pages.entry(Self::key(*route)).or_default().busy = true;
            }
        }
        thread::spawn(move || {
            for (route, _, _, live) in Route::meta() {
                if !live || matches!(route, Route::SmartCare | Route::Welcome) {
                    continue;
                }
                let mut all = Vec::new();
                for s in route.scanners() {
                    for item in s.scan(&|_| {}) {
                        all.push(item);
                    }
                }
                let key = SweepApp::key(*route);
                if tx.send((key, all)).is_err() {
                    break;
                }
            }
        });
    }

    fn render_dashboard(&mut self, ui: &mut egui::Ui) {
        ui.heading("Smart Care");
        ui.label(egui::RichText::new("All systems at a glance").weak());
        ui.add_space(8.0);
        let (n, bytes): (usize, u64) =
            self.totals.values().fold((0, 0), |(c, b), (cc, bb)| (c + cc, b + bb));
        if self.totals.is_empty() {
            ui.label("Not scanned yet — press Scan all for a full checkup.");
        } else {
            ui.label(format!("{} items · {} reclaimable", n, fmt_bytes(bytes)));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let any_busy = self.bulk_busy || self.pages.values().any(|p| p.busy);
            let btn = ui.add_enabled(!any_busy, egui::Button::new("Scan all").fill(ACCENT));
            if btn.clicked() {
                self.scan_all();
            }
            if self.bulk_busy {
                ui.spinner();
            }
        });
        ui.separator();
        for (route, label, _sub, live) in Route::meta() {
            if !live || matches!(route, Route::SmartCare | Route::Welcome) {
                continue;
            }
            let key = Self::key(*route);
            let summary = match self.totals.get(&key) {
                Some((c, b)) => format!("{c} items · {}", fmt_bytes(*b)),
                None => "not scanned yet".into(),
            };
            ui.horizontal(|ui| {
                ui.strong(*label);
                ui.label(egui::RichText::new(summary).weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Open").clicked() {
                        self.route = *route;
                    }
                });
            });
        }
    }

    fn render_welcome(&mut self, ui: &mut egui::Ui) {
        ui.heading("Welcome to SweepMac");
        ui.label(egui::RichText::new("A Mac cleaner that moves things to Trash — nothing is deleted outright.").weak());
        ui.add_space(8.0);
        ui.label("• Reclaim space: caches, logs, trash, leftover app data, old downloads.");
        ui.label("• Stay private: browser histories and cookies (close browsers first).");
        ui.label("• Stay safe: startup and protection areas are audit-first, reversible where possible.");
        ui.add_space(8.0);
        ui.strong("Permissions");
        ui.label("Privacy and system caches need Full Disk Access (FDA). Without it, those scans return less.");
        ui.label(format!("Full Disk Access: {}", if fda_granted() { "granted" } else { "limited" }));
        ui.horizontal(|ui| {
            if ui.button("Open System Settings").clicked() {
                let _ = std::process::Command::new("open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
                    .spawn();
            }
            if ui.add(egui::Button::new("Get started").fill(ACCENT)).clicked() {
                let f = onboarded_file();
                if let Some(p) = f.parent() {
                    let _ = std::fs::create_dir_all(p);
                }
                let _ = std::fs::write(&f, "1");
                self.route = Route::SmartCare;
            }
        });
    }
}

impl eframe::App for SweepApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // update() is called from ObjC draw_rect: any panic crossing back
        // into C aborts the process with its message lost. Catch it here.
        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.update_inner(ctx, frame);
        })) {
            eprintln!("SWEEP UPDATE PANIC: {:?}", e);
            self.update_failed = true;
        }
        if self.update_failed {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("SweepMac hit a rendering bug");
                ui.label("The UI caught a panic and stopped painting to stay alive. Please report what you clicked.");
            });
        }
    }
}

impl SweepApp {
    fn update_inner(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // In-process menu-bar monitor (mounted lazily: eframe owns NSApp by now).
        // Mount failure is non-fatal: the app simply runs without the icon.
        // SWEEP_NO_TRAY=1 disables it (debug bisect).
        if !self.tray_mounted && std::env::var("SWEEP_NO_TRAY").is_err() {
            self.tray_mounted = true;
            if tray::menu_icon_enabled() {
                let _ = tray::mount();
            }
            let _ = ctx;
        }
        for (key, page) in self.pages.iter_mut() {
            page.pump();
            if page.items.is_empty() {
                // Cleared (or never carried anything): drop stale dashboard figures.
                self.totals.remove(key);
            } else {
                let total: u64 = page.items.iter().map(|i| i.size).sum();
                self.totals.insert(*key, (page.items.len(), total));
            }
        }
        if self.bulk_rx.is_some() {
            let msgs: Vec<(u8, Vec<JunkItem>)> =
                self.bulk_rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
            let ignored = ignore::load();
            for (key, items) in msgs {
                let items: Vec<JunkItem> =
                    items.into_iter().filter(|i| !ignore::is_ignored(&i.path, &ignored)).collect();
                let total: u64 = items.iter().map(|i| i.size).sum();
                let n = items.len();
                if let Some(page) = self.pages.get_mut(&key) {
                    page.items = items;
                    page.busy = false;
                    page.status = if n == 0 {
                        "Nothing found — this area is clean.".into()
                    } else {
                        format!("Found {n} items ({})", fmt_bytes(total))
                    };
                }
                self.totals.insert(key, (n, total));
            }
            self.bulk_busy = self.pages.values().any(|p| p.busy);
        }
        if self.pages.values().any(|p| p.busy) {
            ctx.request_repaint();
        }

        egui::SidePanel::left("nav").resizable(false).show(ctx, |ui| {
            ui.heading("SweepMac");
            ui.label(egui::RichText::new("v0.2 · P2").weak());
            ui.separator();
            for (route, label, sub, live) in Route::meta() {
                if *route == Route::Welcome && self.route != Route::Welcome {
                    continue;
                }
                if !live {
                    ui.add_enabled_ui(false, |ui| {
                        let _ = ui.selectable_label(false, format!("{label}  ·  {sub}"));
                        let _ = ui.small(format!("{sub}"));
                    });
                    continue;
                }
                let resp = ui.selectable_label(self.route == *route, *label);
                if resp.clicked() {
                    self.route = *route;
                    // A fresh route gets a fresh chance: transient render
                    // failures clear instead of bricking the session.
                    self.update_failed = false;
                }
                ui.label(egui::RichText::new(*sub).small().weak());
                ui.add_space(2.0);
            }
            ui.separator();
            // Start at login (SMAppService; single process now carries the menu).
            {
                let on = matches!(loginitems::status(), loginitems::LoginState::Enabled);
                let label = if on { "Start at login: on" } else { "Start at login: off" };
                if ui.small_button(label).clicked() {
                    match loginitems::set_enabled(!on) {
                        Ok(m) => self.update_msg = Some(m),
                        Err(e) => self.update_msg = Some(format!("Login item failed: {e}")),
                    }
                }
            }
            // Menu-bar icon show/hide (persisted, applied live).
            {
                let on = tray::menu_icon_enabled();
                let label = if on { "Menu bar icon: on" } else { "Menu bar icon: off" };
                if ui.small_button(label).clicked() {
                    tray::set_menu_icon_enabled(!on);
                    if on {
                        tray::unmount();
                    } else {
                        let _ = tray::mount();
                    }
                }
            }
            // Self-update (feed URL is a placeholder until first release).
            if ui.small_button("Check for updates").clicked() && !self.update_busy {
                self.update_busy = true;
                self.update_msg = Some("Checking…".into());
                let ctx2 = ctx.clone();
                thread::spawn(move || {
                    let msg = match updater::check("https://sweepmac.app/feed.json") {
                        Ok(Some(r)) => format!("Update {}: {}\n{}", r.version, r.url, r.notes),
                        Ok(None) => "You're up to date.".into(),
                        Err(e) => format!("Update check failed: {e}"),
                    };
                    UPDATE_RESULT.lock().unwrap().replace(msg);
                    ctx2.request_repaint();
                });
            }
            if self.update_busy {
                if let Some(m) = UPDATE_RESULT.lock().unwrap().take() {
                    self.update_msg = Some(m.clone());
                    self.update_busy = false;
                } else {
                    ctx.request_repaint();
                }
            }
            if let Some(m) = &self.update_msg {
                ui.label(egui::RichText::new(m).small().weak());
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.route {
            Route::Welcome => self.render_welcome(ui),
            Route::SmartCare => self.render_dashboard(ui),
            Route::Cleanup => self.render_page(ui, "Cleanup", "Caches, logs and trash. Everything goes to Trash — nothing is deleted outright."),
            Route::Applications => self.render_page(ui, "Applications", "Data left behind by apps. Only flags leftovers of apps no longer installed, plus orphan bundle-id folders over 1 MB."),
            Route::Privacy => self.render_page(ui, "Privacy", "Browser histories and cookies. Close your browsers first for a complete clean."),
            Route::Performance => self.render_page(ui, "Performance", "Startup and login items. Disable moves the entry aside (reversible in ~/.sweepmac/disabled) — loginwindow entries must go via System Settings."),
            Route::Protection => self.render_page(ui, "Protection", "Quarantined downloads, non-Apple daemons, and ClamAV malware scan (needs clamd running — brew install clamav). Infected files are never preselected."),
            Route::Clutter => self.render_page(ui, "My Clutter", "Old installers and stale downloads are preselected; large files are listed for your call."),
            Route::SpaceLens => self.render_page(ui, "Space Lens", "Biggest folders per location. Informational — nothing is selected by default."),
        });
    }
}

fn main() -> eframe::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("SWEEP PANIC: {info}");
        if let Some(loc) = info.location() {
            eprintln!("SWEEP PANIC AT: {loc}");
        }
    }));
    // Log any ObjC exception before it crosses into Rust (fatal there).
    unsafe {
        use cocoa::base::{id, nil};
        use objc::runtime::Object;
        use objc::{msg_send, sel, sel_impl};
        extern "C" fn handler(exc: &Object, _cmd: objc::runtime::Sel) {
            unsafe {
                let name: id = msg_send![exc, name];
                let reason: id = msg_send![exc, reason];
                let cs: *const std::os::raw::c_char = msg_send![name, UTF8String];
                let rs: *const std::os::raw::c_char = msg_send![reason, UTF8String];
                eprintln!(
                    "SWEEP OBJC EXCEPTION: {} / {}",
                    std::ffi::CStr::from_ptr(cs).to_string_lossy(),
                    std::ffi::CStr::from_ptr(rs).to_string_lossy()
                );
            }
        }
        // NSSetUncaughtExceptionHandler is a C function, not a method.
        extern "C" {
            fn NSSetUncaughtExceptionHandler(h: extern "C" fn(&Object, objc::runtime::Sel));
        }
        NSSetUncaughtExceptionHandler(handler);
        let _ = nil;
    }
    eframe::run_native(
        "SweepMac",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(SweepApp::default()))),
    )
}
