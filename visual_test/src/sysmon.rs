//! System-monitor visual test.
//!
//! Demonstrates htui's script layer end to end: the demo document carries a
//! `<script>` block whose `refresh()` function calls the registered **host
//! function** `sample()` (a Rust callback) and writes the returned values into
//! the DOM via `document.getElementById(...).textContent`. A background
//! sampling thread keeps the data fresh; the main thread only reads the latest
//! snapshot, so clicks / hover stay responsive. Auto-refresh every 200 ms
//! re-invokes `refresh`.
//!
//! Sampling is backed by the [`sysinfo`] crate (no `wmic` subprocess): the
//! sampler owns one [`System`] and refreshes it on a fixed cadence, which is
//! both faster and more portable than spawning `wmic` per metric.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use htui::events::EventBindings;
use htui::script::{HostCall, HostFn};
use sysinfo::{Disks, System};

/// Latest snapshot published by the sampler thread, read by the `sample()`
/// host function (registered by `#[htui::commands]`) without captures.
static SNAPSHOT: OnceLock<Arc<Mutex<Snapshot>>> = OnceLock::new();

/// Everything the demo document calls, declared in one module and registered
/// by a single `#[htui::commands]` invocation: the three pure formatting
/// commands (`bar` / `gb` / `formatDateTime`), the stateful `sample()`
/// command (returning a `HostFn` closure), and the 200 ms auto-refresh of the
/// page's `refresh()` function.
#[htui::commands(auto = "refresh", every_ms = 200)]
mod commands {
    use super::*;

    /// `sample()` �?read the latest snapshot the sampler thread published and
    /// return it as a JS object. The wall-clock `now` is computed at read time
    /// (every auto refresh); `session` comes from the latest sampled snapshot
    /// (the sampler ticks every 200 ms, so the readout is at most that stale).
    #[htui::command(name = "sample")]
    pub(super) fn sample() -> HostFn {
        Box::new(|call: &mut HostCall| {
            let snap = SNAPSHOT
                .get()
                .map(|s| s.lock().unwrap_or_else(|e| e.into_inner()).clone())
                .unwrap_or_default();
            // Wall-clock readouts are computed at read time (every auto
            // refresh), not frozen at sampling time, so the clock ticks every
            // second regardless of the sampling cadence.
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(snap.now_secs);
            let cpu_bar = snap
                .cpu_load
                .map(|l| bar(l / 100.0, 14))
                .unwrap_or_else(|| "n/a".to_string());
            let cpu_pct = snap
                .cpu_load
                .map(|l| format!("{l:.0}%"))
                .unwrap_or_default();
            let mem_bar = snap
                .memory
                .map(|(u, t)| bar(u as f32 / t.max(1) as f32, 14))
                .unwrap_or_else(|| "n/a".to_string());
            let mem_text = snap
                .memory
                .map(|(u, t)| format!("{} / {}", gb(u), gb(t)))
                .unwrap_or_default();
            let disks = format_disks(&snap.disks, &snap.disk_free);
            Ok(htui::js_obj!(call.context,
                "host"    => snap.host.clone(),
                "os"      => snap.os.clone(),
                "cores"   => snap.cores.clone(),
                "cpuName" => snap.cpu_name.clone(),
                "cpuBar"  => cpu_bar,
                "cpuPct"  => cpu_pct,
                "memBar"  => mem_bar,
                "memText" => mem_text,
                "disks"   => disks,
                "proc"    => snap.process_count,
                "now"     => format_datetime(now_secs),
                "session" => format!(
                    "{:02}:{:02}",
                    snap.session_secs / 60,
                    snap.session_secs % 60
                ),
            ))
        })
    }

    /// `bar(ratio, width)` �?progress bar.
    ///
    /// Deliberately avoids the U+2588 FULL BLOCK / U+2591 LIGHT SHADE glyphs:
    /// their East Asian Width is *Ambiguous*, so CJK-configured consoles render
    /// them two columns wide (or backfill a CJK font), which misaligns the bar
    /// and the digits after it; LIGHT SHADE is also near-invisible on light
    /// backgrounds. `#` / `-` are single-column in every terminal.
    ///
    /// Any positive ratio keeps at least one filled cell �?`round()` would
    /// blank the whole bar below ~3% load, which reads as "nothing is
    /// displayed" on a lightly loaded machine.
    #[htui::command]
    pub(super) fn bar(ratio: f32, width: usize) -> String {
        let ratio = ratio.clamp(0.0, 1.0);
        let filled = if ratio > 0.0 {
            ((ratio * width as f32).ceil() as usize).clamp(1, width)
        } else {
            0
        };
        format!("[{}{}]", "#".repeat(filled), "-".repeat(width - filled))
    }

    /// `gb(bytes)` �?human-readable byte count (`GiB` �?`GB`).
    #[htui::command]
    pub(super) fn gb(bytes: u64) -> String {
        format!("{:.1} GB", bytes as f32 / 1_073_741_824.0)
    }

    /// `formatDateTime(secs)` �?format a UNIX timestamp as `YYYY-MM-DD
    /// HH:MM:SS` without external crates.
    #[htui::command(name = "formatDateTime")]
    pub(super) fn format_datetime(secs: u64) -> String {
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
}

/// Register the functions the demo document calls: the host function
/// `sample()` returns the latest snapshot as a JS object, and the document's
/// `<script>` defines `refresh()` to update the DOM with it. The page
/// auto-refreshes every 200 ms.
pub fn bindings() -> EventBindings {
    // Session anchor shared by the sampler (for the `session` readout). The
    // `sample()` host reads the sampled `session_secs` from the published
    // snapshot, so no capture is needed.
    let start = Instant::now();

    // The published snapshot, written by the sampler thread and read by the
    // host `sample()` on the input thread. The lock is held only for the
    // microsecond-scale publish / clone �?all sysinfo sampling happens on the
    // background thread *before* acquiring it, so the input loop never blocks
    // on a query.
    let snapshot: Arc<Mutex<Snapshot>> = Arc::new(Mutex::new(Snapshot::default()));
    // Publish a clone so the capture-less `sample()` host can reach it.
    SNAPSHOT.get_or_init(|| Arc::clone(&snapshot));
    {
        let snapshot = Arc::clone(&snapshot);
        std::thread::spawn(move || {
            let mut sampler = Sampler::with_start(start);
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

    // One call registers `bar` / `gb` / `formatDateTime`, the `sample()` host
    // One call registers `bar` / `gb` / `formatDateTime`, the `sample()`
    // command and the auto-refresh (see `mod commands` above).
    let mut bindings = EventBindings::default();
    commands::register(&mut bindings);
    bindings
}

/// Render a one-off sampled page to an HTML string.
///
/// Backs the `--sample` / `--watch` diagnostic flags so the sampled page can
/// be previewed without an interactive terminal. Each call samples one frame
/// synchronously (headless mode has no background thread and no UI to keep
/// responsive).
pub fn preview_page() -> String {
    static SAMPLER: OnceLock<Arc<Mutex<Sampler>>> = OnceLock::new();
    let sampler = SAMPLER.get_or_init(|| Arc::new(Mutex::new(Sampler::new())));
    let snap = {
        let mut s = sampler.lock().unwrap_or_else(|e| e.into_inner());
        s.sample_tick()
    };
    snap.render_html()
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
    /// Render a static, self-contained HTML page for the diagnostic flags,
    /// mirroring the interactive page's markup and styling (minus the script).
    fn render_html(&self) -> String {
        let cpu_bar = self
            .cpu_load
            .map(|load| commands::bar(load / 100.0, 14))
            .unwrap_or_else(|| "n/a".to_string());
        let cpu_pct = self
            .cpu_load
            .map(|load| format!("{load:.0}%"))
            .unwrap_or_default();
        let mem_bar = self
            .memory
            .map(|(used, total)| commands::bar(used as f32 / total.max(1) as f32, 14))
            .unwrap_or_else(|| "n/a".to_string());
        let mem_text = self
            .memory
            .map(|(used, total)| format!("{} / {}", commands::gb(used), commands::gb(total)))
            .unwrap_or_default();
        let disks_html = if self.disks.is_empty() {
            "<div id=\"disks\" class=\"dim pad-x1\">n/a</div>".to_string()
        } else {
            let lines: Vec<String> = self
                .disks
                .iter()
                .enumerate()
                .map(|(i, (label, total))| {
                    let total = *total;
                    let free = self.disk_free.get(i).copied().unwrap_or(0);
                    let used = total.saturating_sub(free);
                    format!(
                        "{label} {} {} / {}",
                        commands::bar(used as f32 / total.max(1) as f32, 10),
                        commands::gb(used),
                        commands::gb(total)
                    )
                })
                .collect();
            format!(
                "<div id=\"disks\" class=\"dim pad-x1\">{}</div>",
                lines.join("\n")
            )
        };
        let proc_text = self.process_count.to_string();
        let now_text = commands::format_datetime(self.now_secs);
        let session_text = format!(
            "{:02}:{:02}",
            self.session_secs / 60,
            self.session_secs % 60
        );
        let cpu_name = escape_html(&self.cpu_name);
        let host = escape_html(&self.host);
        let os = escape_html(&self.os);

        format!(
            concat!(
                "<html><head><title>System Monitor</title></head><body>\n",
                "<div class=\"app\">\n",
                "  <div class=\"row header\">\n",
                "    <div class=\"brand grow\">SYSTEM MONITOR</div>\n",
                "    <div class=\"row gap-1\">\n",
                "      <div class=\"btn\">[ Refresh ]</div>\n",
                "      <div class=\"btn\">[ Quit ]</div>\n",
                "    </div>\n",
                "  </div>\n",
                "  <div id=\"meta\" class=\"meta dim\">{host} · {os} · cores {cores}</div>\n",
                "  <div class=\"row gap-2\">\n",
                "    <div class=\"card grow\">\n",
                "      <div class=\"card-head cyan\">CPU</div>\n",
                "      <div class=\"row pad-x1 gap-1\">\n",
                "        <div id=\"cpuBar\" class=\"bar\">{cpu_bar}</div>\n",
                "        <div id=\"cpuPct\" class=\"pct cyan\">{cpu_pct}</div>\n",
                "      </div>\n",
                "      <div id=\"cpuName\" class=\"dim pad-x1\">{cpu_name}</div>\n",
                "    </div>\n",
                "    <div class=\"card grow\">\n",
                "      <div class=\"card-head yel\">MEMORY</div>\n",
                "      <div class=\"row pad-x1 gap-1\">\n",
                "        <div id=\"memBar\" class=\"bar\">{mem_bar}</div>\n",
                "        <div id=\"memText\" class=\"pct yel\">{mem_text}</div>\n",
                "      </div>\n",
                "    </div>\n",
                "  </div>\n",
                "  <div class=\"card\">\n",
                "    <div class=\"card-head purp\">DISKS</div>\n",
                "    {disks_html}\n",
                "  </div>\n",
                "  <div class=\"card\">\n",
                "    <div class=\"card-head green\">SYSTEM</div>\n",
                "    <div id=\"proc\" class=\"val pad-x1\">processes: {proc_text}</div>\n",
                "    <div id=\"now\" class=\"val pad-x1\">{now_text} · session {session_text}</div>\n",
                "  </div>\n",
                "</div>\n",
                "</body></html>\n",
            ),
            host = host,
            os = os,
            cores = self.cores,
            cpu_bar = cpu_bar,
            cpu_pct = cpu_pct,
            cpu_name = cpu_name,
            mem_bar = mem_bar,
            mem_text = mem_text,
            disks_html = disks_html,
            proc_text = proc_text,
            now_text = now_text,
            session_text = session_text,
        )
    }
}

/// Samples machine metrics on a fixed cadence and produces fresh [`Snapshot`]s.
/// Owned exclusively by the background sampler thread; the published snapshot
/// (an `Arc<Mutex<Snapshot>>`) is written only after sampling finishes, so the
/// input loop never blocks on a slow query.
///
/// Holds one [`System`] for the sampler's whole lifetime: sysinfo needs two
/// consecutive refreshes to compute CPU usage, and keeping the instance avoids
/// re-enumerating hardware every tick.
struct Sampler {
    start: Instant,
    tick: u64,
    system: System,
    disks: Disks,
    snapshot: Snapshot,
    /// Machine facts that barely change (CPU model, host, disks); sampled once.
    static_info: Option<StaticInfo>,
}

impl Sampler {
    fn new() -> Self {
        Self::with_start(Instant::now())
    }

    fn with_start(start: Instant) -> Self {
        Self {
            start,
            tick: 0,
            system: System::new_all(),
            disks: Disks::new_with_refreshed_list(),
            snapshot: Snapshot::default(),
            static_info: None,
        }
    }

    /// Refresh every metric and return the next snapshot.
    ///
    /// [`System::refresh_all`] is fast enough to run every tick (no subprocess
    /// spawning), so each tick samples everything: CPU usage (0-100), memory,
    /// process count and per-disk free space. Static facts are cached after
    /// the first tick.
    fn sample_tick(&mut self) -> Snapshot {
        self.tick += 1;
        self.system.refresh_all();
        self.disks.refresh(true);

        if self.static_info.is_none() {
            self.static_info = Some(collect_static(&self.system, &self.disks));
        }
        if let Some(s) = &self.static_info {
            self.snapshot.cpu_name = s.cpu_name.clone();
            self.snapshot.host = s.host.clone();
            self.snapshot.os = s.os.clone();
            self.snapshot.cores = s.cores.clone();
            self.snapshot.disks = s.disks.clone();
        }

        // `refresh_all` above is the second consecutive refresh, so the usage
        // is a real measurement rather than the initial baseline.
        self.snapshot.cpu_load = Some(self.system.global_cpu_usage());
        let total = self.system.total_memory();
        let used = self.system.used_memory();
        self.snapshot.memory = Some((used, total));
        self.snapshot.process_count = self.system.processes().len();
        // Aligned with `static_info.disks` (both walk the same `Disks` list in
        // the same order).
        self.snapshot.disk_free = self
            .disks
            .list()
            .iter()
            .map(|d| d.available_space())
            .collect();
        self.snapshot.now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.snapshot.session_secs = self.start.elapsed().as_secs();
        self.snapshot.clone()
    }
}

/// Snapshot the rarely-changing machine facts from a refreshed [`System`].
fn collect_static(system: &System, disks: &Disks) -> StaticInfo {
    let cpu_name = system
        .cpus()
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_default();
    let host = System::host_name().unwrap_or_else(|| "unknown".to_string());
    let os = match (System::name(), System::os_version()) {
        (Some(name), Some(version)) if !version.is_empty() => format!("{name} {version}"),
        (Some(name), _) => name,
        (None, Some(version)) => version,
        (None, None) => String::new(),
    };
    let cores = system.cpus().len().to_string();
    // Label disks by their mount point (`C:\` → `C:`) so the page reads like
    // a classic monitor; `disk.name()` would show the device model instead.
    let disk_list = disks
        .list()
        .iter()
        .map(|d| {
            let label = d
                .mount_point()
                .to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_string();
            (label, d.total_space())
        })
        .collect();
    StaticInfo {
        cpu_name,
        host,
        os,
        cores,
        disks: disk_list,
    }
}

/// Build a JS object from a snapshot (`{ host, os, cores, cpuBar, memBar, … }`).
///
/// Bars and numeric readouts are split into separate fields so the page can
/// style the percentage with its accent colour next to the bar. `now_secs` /
/// `session_secs` are the wall-clock readouts the caller computed at read
/// Escape text inserted into generated HTML (`&`, `<`, `>`).
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ============================================================
// Formatting helpers
// ============================================================

/// Format the per-disk usage lines (`label` + bar + used/total) for the
/// page's `disks` field; `disk_free` is aligned with `disks` by index.
fn format_disks(disks: &[(String, u64)], disk_free: &[u64]) -> String {
    if disks.is_empty() {
        return "n/a".to_string();
    }
    disks
        .iter()
        .enumerate()
        .map(|(i, (label, total))| {
            let total = *total;
            let free = disk_free.get(i).copied().unwrap_or(0);
            let used = total.saturating_sub(free);
            format!(
                "{label} {} {} / {}",
                commands::bar(used as f32 / total.max(1) as f32, 10),
                commands::gb(used),
                commands::gb(total)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
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
