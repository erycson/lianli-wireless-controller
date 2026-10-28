//! Start with Windows: a logon task with highest privileges. The exe requires admin (UAC manifest),
//! and an HKCU\...\Run entry cannot elevate at logon, so Task Scheduler is the only silent option.
use std::os::windows::process::CommandExt;
use std::process::Command;

const TASK: &str = "LianLiWirelessController";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn schtasks(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("schtasks").args(args).creation_flags(CREATE_NO_WINDOW).output()
}

fn exe() -> Result<String, String> {
    std::env::current_exe().map(|p| p.display().to_string()).map_err(|e| e.to_string())
}

pub fn is_enabled() -> bool {
    schtasks(&["/Query", "/TN", TASK]).map(|o| o.status.success()).unwrap_or(false)
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    let out = if on {
        let tr = format!("\"{}\" --tray", exe()?);
        schtasks(&["/Create", "/TN", TASK, "/TR", &tr, "/SC", "ONLOGON", "/RL", "HIGHEST", "/F"])
    } else {
        schtasks(&["/Delete", "/TN", TASK, "/F"])
    }
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The exe was moved: re-point an existing task to where it runs now.
pub fn follow_exe() {
    let Ok(out) = schtasks(&["/Query", "/TN", TASK, "/XML"]) else { return };
    if !out.status.success() {
        return;
    }
    let Ok(me) = exe() else { return };
    // schtasks emits UTF-16 XML; a lossy view is enough to find the path
    let xml = String::from_utf8_lossy(&out.stdout).replace('\0', "").to_lowercase();
    if !xml.contains(&me.to_lowercase()) {
        let _ = set_enabled(true);
    }
}
