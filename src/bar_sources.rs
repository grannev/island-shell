//! The bar's own workers (everything else it shows comes from the island's
//! existing ones): the details drawer's disk, CPU and memory, sampled only
//! while the drawer is open — Waybar polled them all the time.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::bar::Details;

/// What the bar's workers report.
pub enum BarEvent {
    Details(Details),
    /// the tray's items, in order
    Tray(Vec<crate::tray::TrayItem>),
    /// an item's menu, read for its sheet
    Menu(String, Vec<crate::tray::MenuEntry>),
}

/// Sample disk, CPU and memory every two seconds while `open` holds.
pub fn spawn_details(tx: Sender<BarEvent>, open: Arc<AtomicBool>) {
    thread::Builder::new()
        .name("bar-details".into())
        .spawn(move || {
            let mut last_cpu = read_cpu();
            let mut disk = disk_free("/");
            let mut tick = 0u32;
            // a first reading at once, the CPU's after a short look
            thread::sleep(Duration::from_millis(250));
            while open.load(Ordering::Relaxed) {
                let cpu = read_cpu();
                let usage = match (last_cpu, cpu) {
                    (Some((busy0, total0)), Some((busy1, total1))) if total1 > total0 => {
                        (100.0 * (busy1 - busy0) as f64 / (total1 - total0) as f64).round().clamp(0.0, 100.0) as u8
                    }
                    _ => 0,
                };
                last_cpu = cpu;
                if tick % 30 == 29 {
                    disk = disk_free("/");
                }
                let (memory_gb, memory_total_gb) = memory_gib().unwrap_or((0.0, 0.0));
                let details = Details { disk_free: disk.clone(), cpu: usage, memory_gb, memory_total_gb };
                if tx.send(BarEvent::Details(details)).is_err() {
                    return;
                }
                tick += 1;
                // once a second: the drawer draws the last half minute
                for _ in 0..10 {
                    if !open.load(Ordering::Relaxed) { return; }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        })
        .expect("spawn bar-details");
}

/// (busy, total) jiffies from /proc/stat's first line.
fn read_cpu() -> Option<(u64, u64)> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let fields: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
    let total: u64 = fields.iter().sum();
    // idle and iowait are not busy
    let idle = fields.get(3).copied().unwrap_or(0) + fields.get(4).copied().unwrap_or(0);
    Some((total - idle, total))
}

/// Used memory (total less available) and the total, GiB.
fn memory_gib() -> Option<(f64, f64)> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kib = |key: &str| info.lines().find(|l| l.starts_with(key))?.split_whitespace().nth(1)?.parse::<f64>().ok();
    let total = kib("MemTotal:")?;
    Some(((total - kib("MemAvailable:")?) / 1024.0 / 1024.0, total / 1024.0 / 1024.0))
}

/// Free space on `path`'s filesystem, as Waybar wrote it ("118G", "56.3G").
fn disk_free(path: &str) -> String {
    let Ok(cpath) = std::ffi::CString::new(path) else { return String::new() };
    // SAFETY: a zeroed statvfs is a valid out-parameter for the call
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(cpath.as_ptr(), &mut st) } != 0 {
        return String::new();
    }
    human(st.f_bavail as f64 * st.f_frsize as f64)
}

fn human(bytes: f64) -> String {
    let gib = bytes / 1024.0 / 1024.0 / 1024.0;
    if gib >= 100.0 { format!("{gib:.0}G") } else if gib >= 1.0 { format!("{gib:.1}G") } else { format!("{:.0}M", gib * 1024.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_are_sane() {
        let (busy, total) = read_cpu().unwrap();
        assert!(total > 0 && busy <= total);
        let (mem, total) = memory_gib().unwrap();
        assert!(mem > 0.0 && mem < total && total < 4096.0);
        assert!(disk_free("/").ends_with('G') || disk_free("/").ends_with('M'));
        assert_eq!(human(127.4 * 1024.0 * 1024.0 * 1024.0), "127G");
        assert_eq!(human(56.34 * 1024.0 * 1024.0 * 1024.0), "56.3G");
    }
}
