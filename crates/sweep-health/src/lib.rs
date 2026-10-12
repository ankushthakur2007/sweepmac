//! Menu-bar health sensors. CleanMyMac polls monitors on dual cadences
//! (`init(timeInterval:backgroundTimeInterval:)`); we do the same but cheaper:
//! values are computed on demand, cached, and refreshed at most every
//! `BACKGROUND_SECS` in the background or on every menu open.
use std::process::Command;
use std::time::{Duration, Instant};

pub const FOREGROUND_SECS: u64 = 15;
pub const BACKGROUND_SECS: u64 = 300;

/// One tile of health state.
#[derive(Debug, Clone, Default)]
pub struct Health {
    pub disk_free: Option<u64>,
    pub disk_total: Option<u64>,
    pub mem_pressure_pct: Option<u8>,
    pub battery_pct: Option<u8>,
    pub battery_mins: Option<u64>,
    pub load_avg: Option<f64>,
    pub down_bps: Option<u64>,
    pub up_bps: Option<u64>,
    /// Best-effort CPU die temperature, e.g. "58.25 C" (slow path only).
    pub thermal: Option<String>,
    /// Best-effort Bluetooth accessory battery, e.g. "AirPods Pro 82%" (slow path only).
    pub bt_battery: Option<String>,
}

fn statvfs_free_total(path: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let free = st.f_bavail as u64 * st.f_frsize as u64;
    let total = st.f_blocks as u64 * st.f_frsize as u64;
    Some((free, total))
}

unsafe extern "C" {
    fn mach_port_deallocate(task: libc::mach_port_t, name: libc::mach_port_t) -> u32;
}

/// Memory pressure via Mach kernel statistics — one syscall, no subprocess.
fn mem_pressure() -> Option<u8> {
    unsafe {
        let host = libc::mach_host_self();
        let mut stats: libc::vm_statistics64 = std::mem::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let ret = libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            &mut stats as *mut _ as *mut _,
            &mut count,
        );
        // mach_host_self returns a send right we must release.
        let _: u32 = mach_port_deallocate(libc::mach_task_self(), host);
        if ret != 0 {
            return None;
        }
        let total = stats.free_count as u64
            + stats.active_count as u64
            + stats.inactive_count as u64
            + stats.speculative_count as u64
            + stats.wire_count as u64
            + stats.compressor_page_count as u64;
        if total == 0 {
            return None;
        }
        let used = stats.active_count as u64 + stats.wire_count as u64 + stats.compressor_page_count as u64;
        Some(((used * 100) / total).min(100) as u8)
    }
}

fn battery() -> (Option<u8>, Option<u64>) {
    // NOTE: plain -rn output (no -a: that emits XML plist, not text lines).
    let out = match Command::new("/usr/sbin/ioreg")
        .args(["-rn", "AppleSmartBattery"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return (None, None),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut cur: Option<u64> = None;
    let mut max: Option<u64> = None;
    let mut mins: Option<u64> = None;
    for line in text.lines() {
        let t = line.trim();
        // Exact-key match: avoids AppleRawCurrentCapacity colliding.
        if let Some(rest) = t.strip_prefix("\"CurrentCapacity\" = ") {
            cur = rest.trim().parse().ok();
        } else if let Some(rest) = t.strip_prefix("\"MaxCapacity\" = ") {
            max = rest.trim().parse().ok();
        } else if let Some(rest) = t.strip_prefix("\"TimeRemaining\" = ") {
            mins = rest.trim().parse().ok();
        }
    }
    let pct = match (cur, max) {
        (Some(c), Some(m)) if m > 0 => Some(((c * 100) / m).min(100) as u8),
        _ => None,
    };
    let mins = mins.filter(|&m| m < 60000);
    (pct, mins)
}

fn load() -> Option<f64> {
    let mut avg = [0f64; 3];
    if unsafe { libc::getloadavg(avg.as_mut_ptr(), 3) } == -1 {
        return None;
    }
    Some(avg[0])
}

/// Total rx/tx bytes across physical + VPN interfaces (en*, utun*),
/// one syscall via getifaddrs. AF_LINK filter counts each iface once.
fn net_counters() -> Option<(u64, u64)> {
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 || head.is_null() {
            return None;
        }
        let mut rx: u64 = 0;
        let mut tx: u64 = 0;
        let mut cur = head;
        while !cur.is_null() {
            let ifa = &*cur;
            let tracked = !ifa.ifa_name.is_null()
                && !ifa.ifa_addr.is_null()
                && (*ifa.ifa_addr).sa_family as libc::c_int == libc::AF_LINK
                && !ifa.ifa_data.is_null()
                && {
                    let n = std::ffi::CStr::from_ptr(ifa.ifa_name).to_bytes();
                    n.starts_with(b"en") || n.starts_with(b"utun")
                };
            if tracked {
                let d = &*(ifa.ifa_data as *const libc::if_data);
                rx = rx.wrapping_add(d.ifi_ibytes as u64);
                tx = tx.wrapping_add(d.ifi_obytes as u64);
            }
            cur = (*cur).ifa_next;
        }
        libc::freeifaddrs(head);
        Some((rx, tx))
    }
}

/// Pure rate math: bits per second from two counter samples. saturating_sub
/// treats counter resets as zero delta instead of a giant spike.
fn net_rates(prev: (u64, u64), prev_at: Instant, cur: (u64, u64), now: Instant) -> Option<(u64, u64)> {
    let secs = now.checked_duration_since(prev_at)?.as_secs_f64();
    if !(secs > 0.0) {
        return None;
    }
    let down = (cur.0.saturating_sub(prev.0) as f64 * 8.0 / secs) as u64;
    let up = (cur.1.saturating_sub(prev.1) as f64 * 8.0 / secs) as u64;
    Some((down, up))
}

fn parse_thermal(text: &str) -> Option<String> {
    for line in text.lines() {
        if line.to_lowercase().contains("die temperature") {
            let v = line.split(':').nth(1)?.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Best-effort CPU temperature via `powermetrics --samplers smc -n1`.
/// Slow path only: needs root and the Intel SMC sampler (absent on
/// Apple Silicon), so this is usually None. Never panics, never blocks fast path.
fn thermal() -> Option<String> {
    let out = Command::new("/usr/bin/powermetrics")
        .args(["--samplers", "smc", "-n1"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_thermal(&String::from_utf8_lossy(&out.stdout))
}

fn parse_bt_battery(text: &str) -> Option<String> {
    // system_profiler -json nests devices as {"Name": {"device_batteryLevelMain": "82%", ...}}.
    // No serde in this libc-only crate: scan for battery keys and attribute each
    // to the nearest enclosing `"name" : {`, preferring the Main cell per device.
    let mut best: Vec<(String, String, bool)> = Vec::new();
    let mut rest = text;
    while let Some(k) = rest.find("\"device_batteryLevel") {
        let after_key = &rest[k..];
        let colon = after_key.find(':')?;
        let is_main = after_key[..colon].to_lowercase().contains("main");
        let val = after_key[colon + 1..]
            .trim_start()
            .strip_prefix('"')
            .and_then(|s| s.split('"').next())
            .unwrap_or("");
        let digits: String = val.chars().filter(|c| c.is_ascii_digit()).collect();
        let pre = &rest[..k];
        let name = pre
            .rfind(": {")
            .and_then(|o| pre[..o].rfind('"'))
            .and_then(|e| pre[..e].rfind('"').map(|s| pre[s + 1..e].to_string()))
            .filter(|n| !n.is_empty() && n.len() <= 64);
        rest = &after_key[colon + 1..];
        let (Some(name), Ok(pct)) = (name, digits.parse::<u8>()) else {
            continue;
        };
        if let Some(slot) = best.iter_mut().find(|(n, _, _)| *n == name) {
            if is_main && !slot.2 {
                *slot = (name, format!("{pct}%"), true);
            }
        } else {
            best.push((name, format!("{pct}%"), is_main));
        }
    }
    if best.is_empty() {
        return None;
    }
    Some(
        best.iter()
            .take(4)
            .map(|(n, p, _)| format!("{n} {p}"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// Best-effort Bluetooth accessory battery via system_profiler.
/// Slow (~2s): slow path only. None when no accessory reports a level.
fn bt_battery() -> Option<String> {
    let out = Command::new("/usr/sbin/system_profiler")
        .args(["SPBluetoothDataType", "-json"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_bt_battery(&String::from_utf8_lossy(&out.stdout))
}

impl Health {
    /// Fast sample: syscalls only (statvfs, Mach VM, loadavg) — ~microseconds,
    /// safe to run every couple of seconds. Battery excluded (slow ioreg).
    pub fn sample_fast() -> Self {
        let (disk_free, disk_total) = statvfs_free_total("/").unzip();
        Self {
            disk_free,
            disk_total,
            mem_pressure_pct: mem_pressure(),
            battery_pct: None,
            battery_mins: None,
            load_avg: load(),
            down_bps: None,
            up_bps: None,
            thermal: None,
            bt_battery: None,
        }
    }

    /// Fresh sample. Each source is independent — one failing never blanks the rest.
    /// Network rates stay None here (they need Sampler rate history).
    pub fn sample() -> Self {
        let mut h = Self::sample_fast();
        let (battery_pct, battery_mins) = battery();
        h.battery_pct = battery_pct;
        h.battery_mins = battery_mins;
        h.thermal = thermal();
        h.bt_battery = bt_battery();
        h
    }

    /// Overall grade from factors (their Fair/Good/Excellent pattern, simplified).
    pub fn grade(&self) -> &'static str {
        let mut bad = 0;
        if let (Some(f), Some(t)) = (self.disk_free, self.disk_total) {
            if f * 100 / t.max(1) < 10 {
                bad += 1;
            }
        }
        if self.mem_pressure_pct.unwrap_or(0) > 85 {
            bad += 1;
        }
        if self.battery_pct.unwrap_or(100) < 20 {
            bad += 1;
        }
        match bad {
            0 => "Good",
            1 => "Fair",
            _ => "Poor",
        }
    }
}

/// Cached sample with dual cadence: foreground refresh every
/// FOREGROUND_SECS, background at most every BACKGROUND_SECS.
pub struct CachedHealth {
    last: Option<(Health, Instant)>,
}

/// Realtime sampler: fast syscall sample every tick; slow battery + thermal
/// merged at most every 60s, bluetooth at most every 120s; network rates
/// from getifaddrs counters (first tick arms the baseline, so rates start None).
pub struct Sampler {
    battery: (Option<u8>, Option<u64>),
    battery_at: Option<Instant>,
    thermal: Option<String>,
    thermal_at: Option<Instant>,
    bt: Option<String>,
    bt_at: Option<Instant>,
    net_prev: Option<((u64, u64), Instant)>,
    net: (Option<u64>, Option<u64>),
}

pub const TICK_SECS: u64 = 2;
pub const BATTERY_SECS: u64 = 60;
pub const THERMAL_SECS: u64 = 60;
pub const BT_SECS: u64 = 120;

impl Sampler {
    pub fn new() -> Self {
        Self {
            battery: (None, None),
            battery_at: None,
            thermal: None,
            thermal_at: None,
            bt: None,
            bt_at: None,
            net_prev: None,
            net: (None, None),
        }
    }

    pub fn tick(&mut self) -> Health {
        let now = Instant::now();
        if self.battery_at.is_none_or(|t| t.elapsed() > Duration::from_secs(BATTERY_SECS)) {
            self.battery = battery();
            self.battery_at = Some(now);
        }
        if self.thermal_at.is_none_or(|t| t.elapsed() > Duration::from_secs(THERMAL_SECS)) {
            self.thermal = thermal();
            self.thermal_at = Some(now);
        }
        if self.bt_at.is_none_or(|t| t.elapsed() > Duration::from_secs(BT_SECS)) {
            self.bt = bt_battery();
            self.bt_at = Some(now);
        }
        if let Some(cur) = net_counters() {
            if let Some((prev, at)) = self.net_prev {
                if let Some((d, u)) = net_rates(prev, at, cur, now) {
                    self.net = (Some(d), Some(u));
                }
            }
            self.net_prev = Some((cur, now));
        }
        let mut h = Health::sample_fast();
        h.battery_pct = self.battery.0;
        h.battery_mins = self.battery.1;
        h.thermal = self.thermal.clone();
        h.bt_battery = self.bt.clone();
        h.down_bps = self.net.0;
        h.up_bps = self.net.1;
        h
    }
}

impl CachedHealth {
    pub fn new() -> Self {
        Self { last: None }
    }

    /// Returns cached value if younger than `max_age`, else re-samples.
    pub fn get(&mut self, max_age: Duration) -> Health {
        let fresh = self.last.as_ref().is_some_and(|(_, t)| t.elapsed() < max_age);
        if !fresh {
            self.last = Some((Health::sample(), Instant::now()));
        }
        self.last.as_ref().unwrap().0.clone()
    }

    pub fn get_foreground(&mut self) -> Health {
        self.get(Duration::from_secs(FOREGROUND_SECS))
    }

    pub fn get_background(&mut self) -> Health {
        self.get(Duration::from_secs(BACKGROUND_SECS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_never_panics_and_grades() {
        let h = Health::sample();
        assert!(["Good", "Fair", "Poor"].contains(&h.grade()));
    }

    #[test]
    fn cache_serves_stale_within_age() {
        let mut c = CachedHealth::new();
        let a = c.get(Duration::from_secs(3600));
        let b = c.get(Duration::from_secs(3600));
        assert_eq!(a.grade(), b.grade());
    }

    #[test]
    fn disk_numbers_sane() {
        if let (Some(f), Some(t)) = (statvfs_free_total("/").unzip()) {
            assert!(f <= t && t > 0);
        }
    }

    #[test]
    fn sampler_ticks_twice_without_panic_and_stays_sane() {
        let mut s = Sampler::new();
        let a = s.tick();
        let b = s.tick();
        assert!(["Good", "Fair", "Poor"].contains(&b.grade()));
        // Rates arrive together or not at all (first tick only arms the baseline).
        assert_eq!(b.down_bps.is_some(), b.up_bps.is_some());
        if let (Some(f), Some(t)) = (b.disk_free, b.disk_total) {
            assert!(f <= t);
        }
        let _ = a;
    }

    #[test]
    fn net_rate_math() {
        let t0 = Instant::now() - Duration::from_secs(2);
        let now = Instant::now();
        let (d, u) = net_rates((1000, 500), t0, (3000, 1500), now).unwrap();
        assert!((7000..=8000).contains(&d)); // 2000B*8/~2s down
        assert!((3500..=4000).contains(&u)); // 1000B*8/~2s up
        assert!(net_rates((0, 0), now, (10, 10), now).is_none()); // zero elapsed
        assert!(net_counters().is_some());
    }

    #[test]
    fn thermal_and_bt_parsers() {
        assert_eq!(parse_thermal("CPU die temperature: 58.25 C\n").as_deref(), Some("58.25 C"));
        assert!(parse_thermal("nothing here").is_none());
        let js = r#"{"AirPods Pro" : {"device_batteryLevelMain" : "82%", "device_address" : "AA"}}"#;
        assert_eq!(parse_bt_battery(js).as_deref(), Some("AirPods Pro 82%"));
        assert!(parse_bt_battery("{}").is_none());
    }
}
