//! System-monitor visual test.
//!
//! Demonstrates htui's dynamic re-rendering end to end: the `refresh` handler
//! samples the machine (CPU load, memory, disks, process count, wall clock)
//! and rebuilds the *entire* document via [`RenderCmd::RenderHtml`]; the
//! render thread re-parses the new HTML, re-lays it out and diff-renders it.
//! Hover highlight comes from the CSS `:hover` rules in `style.css`.
//!
//! Sampling deliberately uses only the standard library: `wmic` subprocess
//! output on Windows plus a few environment variables. When a sample fails
//! (e.g. `wmic` missing on a modern Windows), the affected field falls back
//! to `n/a` instead of crashing.
//!
//! ## Refresh rate vs. sampling cost
//!
//! The auto-refresh interval is 200 ms (`EventBindings::auto`), but a `wmic`
//! subprocess query costs ~0.1–0.3 s each and `wmic cpu get loadpercentage`
//! alone takes ~1.1 s, so the [`Monitor`] splits sampling into tiers:
//!
//! - **static** (once): CPU model, host, OS, core count, disk labels + total
//!   size;
//! - **fast** (every tick): memory usage (one `wmic` call);
//! - **slow rotation**: process count / CPU load every 5th tick, disk free
//!   every 10th tick.
//!
//! Measured on a Windows box this bottoms out around 0.4 s per tick — the
//! `wmic` subprocess + WMI provider overhead. A true 200 ms cadence needs an
//! in-process sampler (e.g. the `sysinfo` crate, currently unreachable from
//! crates.io).

use std::collections::HashMap;
use std::env;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use htui::events::{AutoRefresh, EventBindings, EventHandler, HtmlEvent};
use htui::tui::graph_api::RenderCmd;

/// Register the two functions the demo document calls: `refresh()` rebuilds
/// the page with fresh samples, `quit()` exits the render loop. The page also
/// auto-refreshes every 200 ms (see [`Monitor`] for the sampling budget).
pub fn bindings() -> EventBindings {
    let mut bindings = EventBindings::default();
    let monitor = Arc::new(Mutex::new(Monitor::new()));

    let quit: EventHandler = Box::new(|_| Some(RenderCmd::Quit));
    bindings.actions.insert("quit".to_string(), quit);

    bindings.actions.insert(
        "refresh".to_string(),
        Box::new(move |_: &HtmlEvent| {
            let mut monitor = monitor.lock().unwrap_or_else(|e| e.into_inner());
            Some(RenderCmd::RenderHtml {
                html: monitor.page().into(),
            })
        }),
    );
    bindings.auto = Some(AutoRefresh {
        every: Duration::from_millis(200),
        action: "refresh".to_string(),
    });
    bindings
}

/// Render the sampled page to an HTML string.
///
/// Backs the `--sample` / `--watch` diagnostic flags so the page the
/// `refresh` handler produces can be previewed without an interactive
/// terminal. The monitor (and its session timer) is shared across previews.
pub fn preview_page() -> String {
    static MONITOR: OnceLock<Arc<Mutex<Monitor>>> = OnceLock::new();
    let monitor = MONITOR.get_or_init(|| Arc::new(Mutex::new(Monitor::new())));
    monitor
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .page()
}

/// Diagnostic: compare the file-redirect and pipe wmic capture paths for
/// timing and content completeness. Backs the `--probe-wmic` flag.
pub fn probe_wmic() -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (selector, fields) in [
        ("OS", "TotalVisibleMemorySize"),
        ("process", "processid"),
        ("cpu", "name"),
    ] {
        let path = env::temp_dir().join(format!("htui-probe-{}.txt", std::process::id()));
        // File redirect: read immediately after status(), then after 300 ms.
        let Ok(file) = std::fs::File::create(&path) else {
            writeln!(out, "file-redirect {selector}: cannot create temp file").ok();
            continue;
        };
        let start = Instant::now();
        let status = Command::new("wmic")
            .args([selector, "get", fields, "/value"])
            .stdout(std::process::Stdio::from(file))
            .status();
        let status_ms = start.elapsed().as_millis();
        let bytes_now = std::fs::read(&path).unwrap_or_default();
        std::thread::sleep(Duration::from_millis(300));
        let bytes_later = std::fs::read(&path).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        writeln!(
            out,
            "file-redirect {selector} {fields}: status_ok={} status_ms={status_ms} now={} later={}",
            status.is_ok_and(|s| s.success()),
            bytes_now.len(),
            bytes_later.len()
        )
        .ok();

        // Pipe (Command::output): how long until EOF closes?
        let start = Instant::now();
        let output = Command::new("wmic")
            .args([selector, "get", fields, "/value"])
            .output();
        let pipe_ms = start.elapsed().as_millis();
        let pipe_len = output.as_ref().map(|o| o.stdout.len()).unwrap_or(0);
        writeln!(
            out,
            "pipe {selector} {fields}: ok={} pipe_ms={pipe_ms} bytes={pipe_len}",
            output.is_ok_and(|o| o.status.success())
        )
        .ok();
    }
    // Call the production `wmic()` helper directly and report what it parses.
    for (selector, fields, key) in [
        ("OS", "TotalVisibleMemorySize", "TotalVisibleMemorySize"),
        ("process", "processid", "ProcessId"),
        ("cpu", "name", "Name"),
        ("cpu", "loadpercentage", "LoadPercentage"),
    ] {
        let start = Instant::now();
        let stanzas = wmic(selector, fields);
        let ms = start.elapsed().as_millis();
        let first = stanzas
            .first()
            .and_then(|m| m.get(key))
            .cloned()
            .unwrap_or_else(|| "<missing>".to_string());
        writeln!(
            out,
            "real wmic() {selector}: stanzas={} ms={ms} first={first:?}",
            stanzas.len()
        )
        .ok();
    }
    out
}

// ============================================================
// Monitor
// ============================================================

/// Static, rarely-changing machine facts (sampled once).
struct StaticInfo {
    cpu_name: String,
    host: String,
    os: String,
    cores: String,
    /// `(label, total_bytes)` per mounted volume.
    disks: Vec<(String, u64)>,
}

/// Tracks the sampling tiers and renders the page.
struct Monitor {
    start: Instant,
    tick: u64,
    static_info: Option<StaticInfo>,
    cpu_load: Option<f32>,
    process_count: usize,
    /// Per-disk free bytes, aligned with `static_info.disks` (refreshed on a
    /// slow rotation).
    disk_free: Vec<u64>,
}

impl Monitor {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            tick: 0,
            static_info: None,
            cpu_load: None,
            process_count: 0,
            disk_free: Vec::new(),
        }
    }

    /// Sample and render one frame.
    fn page(&mut self) -> String {
        self.tick += 1;

        // First tick pays for everything; afterwards every tick runs a single
        // fast memory query (~90 ms) and the slow queries are rotated in.
        // `wmic cpu get loadpercentage` alone costs ~1.1 s, so CPU load is
        // cached and refreshed on a rotation instead of every tick.
        if self.static_info.is_none() {
            self.static_info = Some(sample_static());
            self.process_count = sample_process_count();
            self.disk_free = sample_disk_free();
            self.cpu_load = sample_cpu_load();
        } else {
            if self.tick.is_multiple_of(5) {
                self.process_count = sample_process_count();
            }
            if self.tick % 5 == 2 {
                self.cpu_load = sample_cpu_load();
            }
            if self.tick.is_multiple_of(10) {
                self.disk_free = sample_disk_free();
            }
        }
        let mem = sample_memory();

        let s = self
            .static_info
            .as_ref()
            .expect("static tier sampled on the first tick");

        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let session = self.start.elapsed();

        let mut page = String::from(
            "<html><head><title>System Monitor</title></head><body>\n\
             <h1>System Monitor</h1>\n\
             <p id=\"refresh\" onclick=\"refresh()\">[ Refresh ]</p>\n\
             <p id=\"quit\" onclick=\"quit()\">[ Quit ]</p>\n\
             <p>host: ",
        );
        page.push_str(&s.host);
        page.push_str(" · os: ");
        page.push_str(&s.os);
        page.push_str(" · cores: ");
        page.push_str(&s.cores);
        page.push_str("</p>\n");

        page.push_str("<h2>CPU</h2><p>");
        match self.cpu_load {
            Some(load) => {
                page.push_str(&bar(load / 100.0, 16));
                page.push_str(&format!(" {load:.0}%"));
            }
            None => page.push_str("n/a"),
        }
        page.push_str("</p><p>");
        page.push_str(&s.cpu_name);
        page.push_str("</p>\n");

        page.push_str("<h2>Memory</h2><p>");
        match mem {
            Some((used, total)) => {
                page.push_str(&bar(used as f32 / total.max(1) as f32, 16));
                page.push_str(&format!(" {} / {}", gb(used), gb(total)));
            }
            None => page.push_str("n/a"),
        }
        page.push_str("</p>\n");

        if !s.disks.is_empty() {
            page.push_str("<h2>Disks</h2>\n");
            for (i, (label, total)) in s.disks.iter().enumerate() {
                let free = self.disk_free.get(i).copied().unwrap_or(0);
                let used = (*total).saturating_sub(free);
                page.push_str(&format!(
                    "<p>{label} {} {} / {}</p>\n",
                    bar(used as f32 / (*total).max(1) as f32, 12),
                    gb(used),
                    gb(*total)
                ));
            }
        }

        page.push_str("<h2>System</h2><p>processes: ");
        page.push_str(&self.process_count.to_string());
        page.push_str(" · now: ");
        page.push_str(&format_datetime(now_secs));
        page.push_str(" · session: ");
        page.push_str(&format!(
            "{:02}:{:02}",
            session.as_secs() / 60,
            session.as_secs() % 60
        ));
        page.push_str("</p></body></html>\n");
        page
    }
}

// ============================================================
// Sampling tiers (std-only)
// ============================================================

/// Run `wmic <selector> get <fields> /value` and parse the `Key=Value`
/// stanzas. Returns one map per stanza; empty when `wmic` is unavailable.
///
/// Output is redirected to a temp file instead of a pipe: `wmic` spawns a
/// WMI-provider grandchild that keeps the stdout pipe handle open, so
/// `Command::output()` (which waits for pipe EOF) would block for hundreds of
/// milliseconds past wmic's exit. `status()` only waits for wmic itself, and
/// the file is read afterwards.
fn wmic(selector: &str, fields: &str) -> Vec<HashMap<String, String>> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let path = env::temp_dir().join(format!(
        "htui-wmic-{}-{}.txt",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let file = match std::fs::File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("failed to create wmic output file: {e}");
            return Vec::new();
        }
    };
    let bytes = match Command::new("wmic")
        .args([selector, "get", fields, "/value"])
        .stdout(std::process::Stdio::from(file))
        .status()
    {
        Ok(status) if status.success() => std::fs::read(&path).unwrap_or_default(),
        Ok(_) => Vec::new(),
        Err(e) => {
            log::warn!("failed to run wmic {selector}: {e}");
            Vec::new()
        }
    };
    let _ = std::fs::remove_file(&path);
    parse_wmic_stanzas(&decode_wmic_output(&bytes))
}

/// Decode wmic's redirected output.
///
/// When stdout is a file (not a pipe / console), `wmic` writes UTF-16LE, so
/// decoding as UTF-8 would produce NUL-stuffed garbage. This falls back to
/// UTF-16LE when the byte pattern looks like it, and plain UTF-8 otherwise.
fn decode_wmic_output(bytes: &[u8]) -> String {
    let body = if bytes.starts_with(&[0xFF, 0xFE]) {
        &bytes[2..]
    } else {
        bytes
    };
    let looks_utf16 = body.len() >= 2
        && body
            .as_chunks::<2>()
            .0
            .iter()
            .take(16)
            .all(|c| c[1] == 0 || c[0] == 0);
    if looks_utf16 {
        let words: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&words)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// Split `Key=Value` output into stanzas (blank-line separated records).
fn parse_wmic_stanzas(text: &str) -> Vec<HashMap<String, String>> {
    let mut result: Vec<HashMap<String, String>> = Vec::new();
    let mut current = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            current.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

/// Static tier: CPU model, host / OS / cores and disk labels + sizes.
fn sample_static() -> StaticInfo {
    let cpu_name = wmic("cpu", "name")
        .first()
        .and_then(|m| m.get("Name").cloned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "n/a".to_string());
    let host = env::var("COMPUTERNAME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "unknown".to_string());
    let os = env::var("OS").unwrap_or_default();
    let cores = env::var("NUMBER_OF_PROCESSORS").unwrap_or_default();
    let disks = wmic("logicaldisk", "caption,size,freespace")
        .into_iter()
        .filter_map(|m| {
            let label = m.get("Caption")?.clone();
            let total = m.get("Size")?.parse::<u64>().ok().filter(|t| *t > 0)?;
            Some((label, total))
        })
        .collect();
    StaticInfo {
        cpu_name,
        host,
        os,
        cores,
        disks,
    }
}

/// Fast tier (every tick): memory usage in `(used_bytes, total_bytes)`.
fn sample_memory() -> Option<(u64, u64)> {
    wmic("OS", "TotalVisibleMemorySize,FreePhysicalMemory")
        .into_iter()
        .next()
        .and_then(|m| {
            let total_kb: u64 = m.get("TotalVisibleMemorySize")?.parse().ok()?;
            let free_kb: u64 = m.get("FreePhysicalMemory")?.parse().ok()?;
            Some((total_kb.saturating_sub(free_kb) * 1024, total_kb * 1024))
        })
}

/// Slow tier: CPU load percentage (a ~1.1 s `wmic` query, so it is cached
/// and refreshed on a rotation rather than every tick).
fn sample_cpu_load() -> Option<f32> {
    wmic("cpu", "loadpercentage")
        .first()
        .and_then(|m| m.get("LoadPercentage"))
        .and_then(|v| v.parse::<f32>().ok())
}

/// Slow tier: process count.
fn sample_process_count() -> usize {
    wmic("process", "processid").len()
}

/// Slow tier: per-disk free bytes (aligned with the static disk list).
fn sample_disk_free() -> Vec<u64> {
    wmic("logicaldisk", "caption,size,freespace")
        .into_iter()
        .filter_map(|m| {
            let has_size = m.get("Size").and_then(|v| v.parse::<u64>().ok()).is_some_and(|t| t > 0);
            has_size.then(|| m.get("FreeSpace").and_then(|v| v.parse().ok()).unwrap_or(0))
        })
        .collect()
}

// ============================================================
// Formatting helpers
// ============================================================

/// A text bar like `[██████░░░░]` for the given 0..=1 ratio.
fn bar(ratio: f32, width: usize) -> String {
    let filled = ((ratio.clamp(0.0, 1.0)) * width as f32).round() as usize;
    let filled = filled.min(width);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(width - filled))
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f32 / 1_073_741_824.0)
}

/// Format a UNIX timestamp as `YYYY-MM-DD HH:MM:SS` without external crates.
fn format_datetime(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let day_secs = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        day_secs / 3600,
        (day_secs % 3600) / 60,
        day_secs % 60
    )
}

/// Convert a day count since 1970-01-01 to a `(year, month, day)` calendar
/// date (Howard Hinnant's `civil_from_days` algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
