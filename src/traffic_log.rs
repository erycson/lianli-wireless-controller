use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

#[repr(C)]
#[derive(Default)]
struct SystemTimeWin {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLocalTime(lpSystemTime: *mut SystemTimeWin);
}

pub fn get_timestamp() -> String {
    let mut st = SystemTimeWin::default();
    unsafe {
        GetLocalTime(&mut st);
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.year, st.month, st.day, st.hour, st.minute, st.second, st.milliseconds
    )
}

/// Rotate `proxy_traffic.log` past this size; keep `.1`..`.3` (newest first).
const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const KEEP_ROTATED: u32 = 3;
const REG_KEY: windows::core::PCWSTR = windows::core::w!(r"Software\LianLiWirelessController");
const REG_LOG: windows::core::PCWSTR = windows::core::w!("LogToFile");

/// (open file, its size). File logging is off by default; the GUI toggle persists in HKCU.
static LOG_FILE: Mutex<Option<(File, u64)>> = Mutex::new(None);
static ENABLED: std::sync::OnceLock<std::sync::atomic::AtomicBool> = std::sync::OnceLock::new();

fn enabled() -> &'static std::sync::atomic::AtomicBool {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    ENABLED.get_or_init(|| {
        let mut v = 0u32;
        let mut len = 4u32;
        let ok = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, REG_KEY, REG_LOG, RRF_RT_REG_DWORD, None, Some(&mut v as *mut _ as *mut _), Some(&mut len))
        };
        std::sync::atomic::AtomicBool::new(ok.is_ok() && v != 0)
    })
}

pub fn file_logging() -> bool {
    enabled().load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_file_logging(on: bool) {
    use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD};
    enabled().store(on, std::sync::atomic::Ordering::Relaxed);
    let v = on as u32;
    unsafe {
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, REG_KEY, REG_LOG, REG_DWORD.0, Some(&v as *const _ as *const _), 4);
    }
    if !on {
        *LOG_FILE.lock().unwrap() = None;
    }
}

/// Next to the exe (main() also sets the working directory there).
pub fn log_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("proxy_traffic.log")))
        .unwrap_or_else(|| "proxy_traffic.log".into())
}

fn rotated(path: &std::path::Path, n: u32) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(format!(".{}", n));
    s.into()
}

fn rotate(path: &std::path::Path) {
    let _ = std::fs::remove_file(rotated(path, KEEP_ROTATED));
    for n in (1..KEEP_ROTATED).rev() {
        let _ = std::fs::rename(rotated(path, n), rotated(path, n + 1));
    }
    let _ = std::fs::rename(path, rotated(path, 1));
}

fn write_line(line: &str) {
    if !file_logging() {
        return;
    }
    let Ok(mut guard) = LOG_FILE.lock() else { return };
    let path = log_path();
    if guard.as_ref().is_some_and(|(_, size)| size + line.len() as u64 > MAX_LOG_BYTES) {
        *guard = None;
        rotate(&path);
    }
    if guard.is_none() {
        let Ok(f) = OpenOptions::new().create(true).append(true).open(&path) else { return };
        let size = f.metadata().map(|m| m.len()).unwrap_or(0);
        *guard = Some((f, size));
    }
    if let Some((f, size)) = guard.as_mut() {
        let _ = f.write_all(line.as_bytes());
        *size += line.len() as u64;
    }
}
/// Logs a packet or event with precise millisecond timestamp.
/// direction: e.g. "L-CONNECT -> PROXY", "PROXY -> L-CONNECT", "PROXY -> C3"
/// channel: e.g. "TX 0x8040", "RX 0x8041", "COM3"
/// details: description of command, seqnum, status
/// data: packet payload to format as hex
pub fn log_packet(direction: &str, channel: &str, details: &str, data: &[u8]) {
    let ts = get_timestamp();
    let hex_dump = if data.is_empty() {
        String::new()
    } else if data.len() <= 64 {
        hex::encode(data)
    } else {
        format!(
            "{}... [{} bytes total]",
            hex::encode(&data[..64]),
            data.len()
        )
    };

    let line = if hex_dump.is_empty() {
        format!("[{}] [{}] [{}] {}\n", ts, direction, channel, details)
    } else {
        format!("[{}] [{}] [{}] {} | {}\n", ts, direction, channel, details, hex_dump)
    };

    write_line(&line);

    // Fan status packets arrive ~40/s and are shown as a table, not in the live log
    if direction != "C3 RF -> PROXY"
        && let Ok(mut r) = RECENT.lock() {
            if r.1.len() >= RECENT_MAX {
                r.1.pop_front();
            }
            r.1.push_back(line.trim_end().to_string());
            r.0 += 1;
        }
}

const RECENT_MAX: usize = 200;
/// (lines ever pushed, last RECENT_MAX lines) for the GUI live log.
static RECENT: Mutex<(u64, std::collections::VecDeque<String>)> = Mutex::new((0, std::collections::VecDeque::new()));

/// Returns (sequence, recent lines); the sequence changes whenever a line is added.
pub fn recent_lines() -> (u64, Vec<String>) {
    RECENT.lock().map(|r| (r.0, r.1.iter().cloned().collect())).unwrap_or_default()
}

pub fn log_event(direction: &str, channel: &str, message: &str) {
    log_packet(direction, channel, message, &[]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_keeps_current_plus_three() {
        let dir = std::env::temp_dir().join(format!("lianli-rot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("proxy_traffic.log");
        for n in 0..6 {
            std::fs::write(&log, n.to_string()).unwrap();
            rotate(&log);
        }
        std::fs::write(&log, "6").unwrap();
        let read = |p: std::path::PathBuf| std::fs::read_to_string(p).unwrap();
        assert_eq!(read(log.clone()), "6");
        assert_eq!((read(rotated(&log, 1)), read(rotated(&log, 2)), read(rotated(&log, 3))), ("5".into(), "4".into(), "3".into()));
        assert!(!rotated(&log, 4).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
