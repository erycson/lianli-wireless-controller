use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn silent_command<P: AsRef<Path>>(program: P) -> Command {
    let mut cmd = Command::new(program.as_ref());
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[derive(Debug, Clone)]
pub struct AttachedPort {
    pub port: u8,
    pub speed: String,
    pub status: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub description: String,
    pub uri: String,
}

#[derive(Debug, Clone)]
pub struct UsbipDiagnostics {
    pub usbip_exe: Option<PathBuf>,
    pub wusbip_exe: Option<PathBuf>,
    pub driver_installed: bool,
    pub driver_path: Option<PathBuf>,
    pub instruction: Option<String>,
}

pub fn locate_usbip() -> Option<PathBuf> {
    let candidates = [
        r"C:\Program Files\USBip\usbip.exe",
        r"C:\Program Files (x86)\USBip\usbip.exe",
        r"C:\tools\usbip\usbip.exe",
        r"C:\usbip\usbip.exe",
        r".\usbip.exe",
    ];

    for c in candidates {
        let p = Path::new(c);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }

    if let Ok(out) = silent_command("where.exe").arg("usbip").output()
        && out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(first_line) = s.lines().next() {
                let p = PathBuf::from(first_line.trim());
                if p.exists() {
                    return Some(p);
                }
            }
        }

    None
}

pub fn locate_wusbip() -> Option<PathBuf> {
    let candidates = [
        r"C:\Program Files\USBip\wusbip.exe",
        r"C:\Program Files (x86)\USBip\wusbip.exe",
        r"C:\tools\usbip\wusbip.exe",
        r"C:\usbip\wusbip.exe",
        r".\wusbip.exe",
    ];

    for c in candidates {
        let p = Path::new(c);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }

    None
}

pub fn check_driver() -> (bool, Option<PathBuf>) {
    let p = Path::new(r"C:\Windows\System32\drivers\usbip2_ude.sys");
    if p.exists() {
        return (true, Some(p.to_path_buf()));
    }
    if let Ok(out) = silent_command("sc.exe").args(["query", "usbip2_ude"]).output()
        && out.status.success() {
            return (true, None);
        }
    (false, None)
}

pub fn get_diagnostics() -> UsbipDiagnostics {
    let usbip_exe = locate_usbip();
    let wusbip_exe = locate_wusbip();
    let (driver_installed, driver_path) = check_driver();

    let instruction = if usbip_exe.is_none() {
        Some("USBip client is not installed. Run 'winget install vadimgrn.usbip-win2' or download from https://github.com/vadimgrn/usbip-win2/releases".to_string())
    } else {
        None
    };

    UsbipDiagnostics {
        usbip_exe,
        wusbip_exe,
        driver_installed,
        driver_path,
        instruction,
    }
}

pub fn list_ports() -> Result<Vec<AttachedPort>, String> {
    let exe = locate_usbip().ok_or("usbip.exe not found on system")?;
    let out = silent_command(&exe)
        .arg("port")
        .output()
        .map_err(|e| format!("Failed to run usbip port: {}", e))?;

    let text = String::from_utf8_lossy(&out.stdout);
    let mut ports = Vec::new();

    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.starts_with("Port ") && line.contains(':') {
            let colon_idx = line.find(':').unwrap_or(7);
            let port_part = &line[5..colon_idx];
            let port_num = port_part.trim().parse::<u8>().unwrap_or(0);
            let status = line[colon_idx + 1..].trim().to_string();

            let mut desc = String::new();
            let mut vid = 0u16;
            let mut pid = 0u16;
            let mut uri = String::new();

            if i + 1 < lines.len() {
                let l2 = lines[i + 1].trim();
                desc = l2.to_string();
                if let Some(pos) = l2.rfind('(')
                    && let Some(end) = l2.rfind(')') {
                        let id_part = &l2[pos + 1..end];
                        let parts: Vec<&str> = id_part.split(':').collect();
                        if parts.len() == 2 {
                            vid = u16::from_str_radix(parts[0], 16).unwrap_or(0);
                            pid = u16::from_str_radix(parts[1], 16).unwrap_or(0);
                        }
                    }
            }

            if i + 2 < lines.len() {
                let l3 = lines[i + 2].trim();
                if l3.starts_with("-> usbip://") {
                    uri = l3[3..].trim().to_string();
                }
            }

            ports.push(AttachedPort {
                port: port_num,
                speed: status.clone(),
                status,
                vendor_id: vid,
                product_id: pid,
                description: desc,
                uri,
            });
        }
        i += 1;
    }

    Ok(ports)
}

pub fn detach_port(port: u8) -> Result<(), String> {
    let exe = locate_usbip().ok_or("usbip.exe not found on system")?;
    let out = silent_command(&exe)
        .args(["detach", "-p", &format!("{:02}", port)])
        .output()
        .map_err(|e| format!("Failed to run usbip detach: {}", e))?;

    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub fn detach_all_virtual() -> Result<usize, String> {
    let exe = locate_usbip().ok_or("usbip.exe not found on system")?;

    // 1. Immediately detach all connected virtual devices using native usbip -a
    let _ = silent_command(&exe).args(["detach", "-a"]).output();

    let ports = list_ports().unwrap_or_default();
    let mut detached = 0;
    for p in ports {
        if (p.vendor_id == 0x0416 && (p.product_id == 0x8040 || p.product_id == 0x8041))
            || p.uri.contains("127.0.0.1")
        {
            let _ = detach_port(p.port);
            detached += 1;
        }
    }
    Ok(detached)
}

pub fn attach_virtual() -> Result<(), String> {
    let exe = locate_usbip().ok_or("usbip.exe not found on system")?;

    let _ = detach_all_virtual();

    let out1 = silent_command(&exe)
        .args(["attach", "-r", "127.0.0.1", "-b", "1-1"])
        .output()
        .map_err(|e| format!("Failed to attach 1-1 (TX): {}", e))?;

    if !out1.status.success() {
        return Err(format!("Attach TX failed: {}", String::from_utf8_lossy(&out1.stderr).trim()));
    }

    let out2 = silent_command(&exe)
        .args(["attach", "-r", "127.0.0.1", "-b", "1-2"])
        .output()
        .map_err(|e| format!("Failed to attach 1-2 (RX): {}", e))?;

    if !out2.status.success() {
        return Err(format!("Attach RX failed: {}", String::from_utf8_lossy(&out2.stderr).trim()));
    }

    Ok(())
}

