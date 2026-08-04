//! System-monitor visual test.
//!
//! Demonstrates htui's script layer end to end: the demo document carries a
//! `<script>` block whose `refresh()` function calls the registered **host
//! function** `sample()` (a Rust callback) and writes the returned values into
//! the DOM via `document.getElementById(...).textContent`. A background
//! sampling thread keeps the data fresh; the main thread only reads the latest
//! snapshot, so clicks / hover stay responsive even though a `wmic` query
//! costs ~0.1–1.1 s. Auto-refresh every 200 ms re-invokes `refresh`.
//!
//! Sampling deliberately uses only the standard library: `wmic` subprocess
//! output on Windows plus a few environment variables. When a sample fails
//! (e.g. `wmic` missing on a modern Windows), the affected field falls back
//! to `n/a` instead of crashing.

use std::collections::HashMap;
use std::env;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use boa_engine::{JsValue, js_string};
use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use htui::events::{AutoRefresh, EventBindings};
use htui::script::HostCall;

/// Register the functions the demo document calls: the host function
/// `sample()` returns the latest snapshot as a JS object, and the document's
/// `<script>` defines `refresh()` to update the DOM with it. The page
/// auto-refreshes every 200 ms.
pub fn bindings() -> EventBindings {
    let mut bindings = EventBindings::default();

    // The published snapshot, written by the sampler thread and read by the
    // host `sample()` on the input thread. The lock is held only for the
    // microsecond-scale publish / clone — the slow `wmic` sampling happens on
    // the background thread *before* acquiring it, so the input loop never
    // blocks on a 1.1 s CPU query.
    let snapshot: Arc<Mutex<Snapshot>> = Arc::new(Mutex::new(Snapshot::default()));
    {
        let snapshot = Arc::clone(&snapshot);
        std::thread::spawn(move || {
            let mut sampler = Sampler::new();
            loop {
                // Slow sampling runs on this thread, lock-free.
                let next = sampler.sample_tick();
                // Publish: brief lock, memcpy-sized.
                if let Ok(mut snap) = snapshot.lock() {
                    *snap = next;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        });
    }

    let snapshot_for_host = Arc::clone(&snapshot);
    bindings.host.insert(
        "sample".to_string(),
        Box::new(move |call: &mut HostCall| {
            let snap = snapshot_for_host
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            sample_to_js(call, &snap)
        }),
    );
    bindings.auto = Some(AutoRefresh {
        every: Duration::from_millis(200),
        action: "refresh".to_string(),
    });
    bindings
}

/// Render a one-off sampled page to an HTML string.
///
/// Backs the `--sample` / `--watch` diagnostic flags so the sampled page can
/// be previewed without an interactive terminal. The snapshot is read from the
/// shared sampler.
pub fn preview_page() -> String {
    static SAMPLER: OnceLock<Arc<Mutex<Sampler>>> = OnceLock::new();
    let sampler = SAMPLER.get_or_init(|| Arc::new(Mutex::new(Sampler::new())));
    let snap = sampler
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .snapshot();
    snap.render_html()
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
// Sampler
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

/// A point-in-time machine snapshot read by the host `sample()` function.
#[derive(Clone, Default)]
struct Snapshot {
    cpu_name: String,
    host: String,
    os: String,
    cores: String,
    /// `(label, total_bytes)` per mounted volume.
    disks: Vec<(String, u64)>,
    /// Per-disk free bytes, aligned with `disks`.
    disk_free: Vec<u64>,
    cpu_load: Option<f32>,
    process_count: usize,
    /// `(used_bytes, total_bytes)`.
    memory: Option<(u64, u64)>,
    now_secs: u64,
    /// Seconds since the sampler started.
    session_secs: u64,
}

impl Snapshot {
    /// Render a static, self-contained HTML page for the diagnostic flags.
    fn render_html(&self) -> String {
        let mut page = String::from(
            "<html><head><title>System Monitor</title></head><body>\n\
             <h1>System Monitor</h1>\n\
             <p id=\"refresh\" onclick=\"refresh()\">[ Refresh ]</p>\n\
             <p id=\"quit\" onclick=\"quit()\">[ Quit ]</p>\n\
             <p>host: ",
        );
        page.push_str(&self.host);
        page.push_str(" · os: ");
        page.push_str(&self.os);
        page.push_str(" · cores: ");
        page.push_str(&self.cores);
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
        page.push_str(&self.cpu_name);
        page.push_str("</p>\n");

        page.push_str("<h2>Memory</h2><p>");
        match self.memory {
            Some((used, total)) => {
                page.push_str(&bar(used as f32 / total.max(1) as f32, 16));
                page.push_str(&format!(" {} / {}", gb(used), gb(total)));
            }
            None => page.push_str("n/a"),
        }
        page.push_str("</p>\n");

        if !self.disks.is_empty() {
            page.push_str("<h2>Disks</h2>\n");
            for (i, (label, total)) in self.disks.iter().enumerate() {
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
        page.push_str(&format_datetime(self.now_secs));
        page.push_str(" · session: ");
        page.push_str(&format!(
            "{:02}:{:02}",
            self.session_secs / 60,
            self.session_secs % 60
        ));
        page.push_str("</p></body></html>\n");
        page
    }
}

/// Samples machine metrics on a rotation and produces fresh [`Snapshot`]s.
/// Owned exclusively by the background sampler thread; the published snapshot
/// (an `Arc<Mutex<Snapshot>>`) is written only after sampling finishes, so the
/// input loop never blocks on a slow `wmic` query.
struct Sampler {
    start: Instant,
    tick: u64,
    snapshot: Snapshot,
}

impl Sampler {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            tick: 0,
            snapshot: Snapshot::default(),
        }
    }

    fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }

    /// Sample one rotation and return the next snapshot. The first tick pays
    /// for everything; afterwards every tick runs a single fast memory query
    /// and the slow queries (`process`, `cpu`, `disk free`) are rotated in.
    /// All `wmic` calls happen here, lock-free, on the background thread.
    fn sample_tick(&mut self) -> Snapshot {
        self.tick += 1;

        let static_info = if self.tick == 1 {
            Some(sample_static())
        } else {
            None
        };
        if let Some(s) = static_info {
            self.snapshot.cpu_name = s.cpu_name;
            self.snapshot.host = s.host;
            self.snapshot.os = s.os;
            self.snapshot.cores = s.cores;
            self.snapshot.disks = s.disks;
            self.snapshot.process_count = sample_process_count();
            self.snapshot.disk_free = sample_disk_free();
            self.snapshot.cpu_load = sample_cpu_load();
        } else {
            if self.tick.is_multiple_of(5) {
                self.snapshot.process_count = sample_process_count();
            }
            if self.tick % 5 == 2 {
                self.snapshot.cpu_load = sample_cpu_load();
            }
            if self.tick.is_multiple_of(10) {
                self.snapshot.disk_free = sample_disk_free();
            }
        }
        self.snapshot.memory = sample_memory();
        self.snapshot.now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.snapshot.session_secs = self.start.elapsed().as_secs();
        self.snapshot.clone()
    }
}

/// Build a JS object from a snapshot (`{ host, os, cores, cpu, mem, … }`).
fn sample_to_js(call: &mut HostCall, snap: &Snapshot) -> JsValue {
    let cpu_text = snap
        .cpu_load
        .map(|l| format!("{l:.0}%"))
        .unwrap_or_else(|| "n/a".to_string());
    let mem_text = match snap.memory {
        Some((used, total)) => format!("{} / {}", gb(used), gb(total)),
        None => "n/a".to_string(),
    };
    let proc_text = snap.process_count.to_string();
    let now_text = format_datetime(snap.now_secs);
    let session_text = format!("{:02}:{:02}", snap.session_secs / 60, snap.session_secs % 60);

    let mut init = ObjectInitializer::new(call.context);
    init.property(js_string!("host"), js_string!(snap.host.as_str()), Attribute::all())
        .property(js_string!("os"), js_string!(snap.os.as_str()), Attribute::all())
        .property(js_string!("cores"), js_string!(snap.cores.as_str()), Attribute::all())
        .property(js_string!("cpuName"), js_string!(snap.cpu_name.as_str()), Attribute::all())
        .property(js_string!("cpu"), js_string!(cpu_text.as_str()), Attribute::all())
        .property(js_string!("mem"), js_string!(mem_text.as_str()), Attribute::all())
        .property(js_string!("proc"), js_string!(proc_text.as_str()), Attribute::all())
        .property(js_string!("now"), js_string!(now_text.as_str()), Attribute::all())
        .property(js_string!("session"), js_string!(session_text.as_str()), Attribute::all());
    init.build().into()
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
