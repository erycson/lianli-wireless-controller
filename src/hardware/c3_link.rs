//! Serial link to the ESP32-C3 RF Master (firmware/esp32c3): the radio side of the virtual dongle.
//! bridge -> C3: "OUT <128 hex>" per TX Bulk OUT. C3 -> bridge: "FAN <rssi> <hex64>" (status packets
//! heard on air), "TX ..." / "LOG ..." / "IN ...". Every line goes to proxy_traffic.log.
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use windows::Win32::Devices::Communication::{
    EscapeCommFunction, SetCommTimeouts, CLRDTR, CLRRTS, COMMTIMEOUTS, SETRTS,
};
use windows::Win32::Foundation::HANDLE;

pub struct C3Link {
    sender: Sender<Vec<u8>>,
}

impl C3Link {
    /// `on_air_packet` receives every 64-byte CRC-valid status packet the C3 hears.
    pub fn start<F>(port: &str, on_air_packet: F) -> Self
    where
        F: Fn(&[u8]) + Send + 'static,
    {
        let (sender, receiver) = channel();
        let port = port.to_string();
        thread::spawn(move || link_worker(&port, receiver, on_air_packet));
        Self { sender }
    }

    pub fn forward(&self, packet: &[u8]) {
        let _ = self.sender.send(packet.to_vec());
    }
}

/// Live USB-CDC serial ports (HKLM\HARDWARE\DEVICEMAP\SERIALCOMM, `\Device\USBSER*`): where a C3 shows up.
fn usb_serial_ports() -> Vec<String> {
    use windows::Win32::System::Registry::*;
    let mut ports = Vec::new();
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, windows::core::w!(r"HARDWARE\DEVICEMAP\SERIALCOMM"), None, KEY_READ, &mut key).is_err() {
            return ports;
        }
        for i in 0.. {
            let mut name = [0u16; 256];
            let mut data = [0u16; 64];
            let mut name_len = name.len() as u32;
            let mut data_len = (data.len() * 2) as u32;
            if RegEnumValueW(key, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut name_len, None, None,
                Some(data.as_mut_ptr() as *mut u8), Some(&mut data_len)).is_err() {
                break;
            }
            let name = String::from_utf16_lossy(&name[..name_len as usize]);
            let port = String::from_utf16_lossy(&data[..(data_len as usize / 2)]).trim_end_matches('\0').to_string();
            if name.to_ascii_uppercase().contains("USBSER") {
                ports.push(port);
            }
        }
        let _ = RegCloseKey(key);
    }
    ports
}

/// Our firmware answers "STAT" with "LOG stat cfg=..."; nothing else does (no side effects on the radio).
fn is_our_c3(port: &str) -> bool {
    let Ok(mut f) = open_port(port) else { return false };
    if f.write_all(b"STAT\n").is_err() {
        return false;
    }
    let deadline = std::time::Instant::now() + Duration::from_millis(1500);
    let mut seen = Vec::new();
    let mut buf = [0u8; 512];
    while std::time::Instant::now() < deadline {
        match f.read(&mut buf) {
            Ok(n) => seen.extend_from_slice(&buf[..n]),
            Err(_) => return false,
        }
        if seen.windows(13).any(|w| w == b"LOG stat cfg=") {
            return true;
        }
    }
    false
}

/// Original Lian Li dongles plugged in (TX chip 0x0416:0x8040). Call with our virtual ports detached.
fn original_dongles() -> usize {
    nusb::list_devices()
        .map(|l| l.filter(|d| d.vendor_id() == 0x0416 && d.product_id() == 0x8040).count())
        .unwrap_or(0)
}

/// Finds the single RF controller to run with. Ok(port) = exactly one controller and it is our C3.
/// Err(msg, conflict) = none found (conflict=false) or more than one controller (conflict=true).
pub fn detect() -> Result<String, (String, bool)> {
    let c3s: Vec<String> = usb_serial_ports().into_iter().filter(|p| is_our_c3(p)).collect();
    let originals = original_dongles();
    match (c3s.len(), originals) {
        (1, 0) => Ok(c3s[0].clone()),
        (0, 0) => Err(("No controller found. Plug in the ESP32-C3 (RF Master firmware).".into(), false)),
        (0, 1) => Err(("Only the ORIGINAL Lian Li dongle is plugged in; this program drives the ESP32-C3. Use L-Connect directly with the dongle or swap it for the C3.".into(), true)),
        (c, o) => Err((format!(
            "{} controllers connected ({} ESP32-C3 {}, {} original). L-Connect accepts only ONE: unplug the others.",
            c + o, c, c3s.join(" "), o
        ), true)),
    }
}

fn set_rts(f: &File, on: bool) {
    unsafe {
        let _ = EscapeCommFunction(HANDLE(f.as_raw_handle()), if on { SETRTS } else { CLRRTS });
    }
}

fn open_port(port: &str) -> std::io::Result<File> {
    let f = OpenOptions::new().read(true).write(true).open(format!(r"\\.\{}", port))?;
    let h = HANDLE(f.as_raw_handle());
    // Reads return as soon as bytes arrive, or after 20 ms with nothing: one thread does both directions.
    let t = COMMTIMEOUTS {
        ReadIntervalTimeout: u32::MAX,
        ReadTotalTimeoutMultiplier: u32::MAX,
        ReadTotalTimeoutConstant: 20,
        WriteTotalTimeoutMultiplier: 0,
        WriteTotalTimeoutConstant: 500,
    };
    unsafe {
        SetCommTimeouts(h, &t).map_err(std::io::Error::other)?;
        // DTR=0 RTS=0 = run (USB-Serial-JTAG maps RTS to chip reset)
        let _ = EscapeCommFunction(h, CLRDTR);
    }
    set_rts(&f, false);
    Ok(f)
}

/// Feeds one C3 line; returns true when the C3 asks for a hardware reset.
fn handle_line(port: &str, line: &str, on_air_packet: &impl Fn(&[u8])) -> bool {
    let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
    match tag {
        "FAN" => {
            if let Some(p) = rest.split_once(' ').and_then(|(_, h)| hex::decode(h.trim()).ok()) {
                on_air_packet(&p);
                crate::traffic_log::log_packet("C3 RF -> PROXY", port, &format!("FAN rssi={}", rest.split(' ').next().unwrap_or("")), &p);
            }
        }
        "RF" => {} // raw captures (every packet heard, incl. noise): too chatty to log
        _ => crate::traffic_log::log_event("C3", port, line),
    }
    // Known limitation: the C3 BLE controller leaks one buffer per TX and asserts after ~400 TX;
    // a hardware reset (RTS pulse, like esptool) is the only known recovery. Root fix is in the ROM hook.
    (tag == "LOG" && line.contains("reset me")) || tag == "assert" || line.contains("abort()")
}

fn link_worker(port: &str, receiver: Receiver<Vec<u8>>, on_air_packet: impl Fn(&[u8])) {
    loop {
        let mut f = match open_port(port) {
            Ok(f) => f,
            Err(e) => {
                crate::traffic_log::log_event("C3", port, &format!("open failed: {} (retrying)", e));
                thread::sleep(Duration::from_secs(2));
                // Drop what queued up while the radio was away: stale commands, L-Connect resends
                while receiver.try_recv().is_ok() {}
                continue;
            }
        };
        crate::traffic_log::log_event("C3", port, "link up");

        let mut line = Vec::new();
        let mut buf = [0u8; 512];
        let reason = 'session: loop {
            loop {
                match receiver.try_recv() {
                    Ok(pkt) => {
                        if let Err(e) = f.write_all(format!("OUT {}\n", hex::encode(&pkt)).as_bytes()) {
                            break 'session format!("write failed: {}", e);
                        }
                        crate::traffic_log::log_packet("PROXY -> C3", port, &format!("OUT ({} bytes)", pkt.len()), &pkt);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            let n = match f.read(&mut buf) {
                Ok(n) => n,
                Err(e) => break 'session format!("read failed: {}", e),
            };
            for &b in &buf[..n] {
                if b != b'\n' && b != b'\r' {
                    line.push(b);
                    continue;
                }
                if line.is_empty() {
                    continue;
                }
                let text = String::from_utf8_lossy(&line).into_owned();
                line.clear();
                if handle_line(port, &text, &on_air_packet) {
                    set_rts(&f, true);
                    thread::sleep(Duration::from_millis(100));
                    set_rts(&f, false);
                    break 'session "hardware reset (BLE TX leak)".to_string();
                }
            }
        };
        crate::traffic_log::log_event("C3", port, &format!("link down: {}", reason));
        drop(f);
        thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lines() {
        let got = std::cell::RefCell::new(Vec::new());
        let sink = |p: &[u8]| got.borrow_mut().push(p.to_vec());
        assert!(!handle_line("T", "FAN -47 12ab", &sink));
        assert!(!handle_line("T", "FAN -47 1", &sink)); // odd hex ignored
        assert_eq!(*got.borrow(), vec![vec![0x12, 0xab]]);
        assert!(handle_line("T", "LOG tx count 350 (reset me)", &sink));
        assert!(handle_line("T", "assert ble_util_buf.c 291, param 000000d9 00000000", &sink));
    }

    /// Hardware check: `cargo test --lib detect_real -- --ignored --nocapture` with the C3 plugged in.
    #[test]
    #[ignore]
    fn detect_real() {
        println!("serial ports: {:?}", usb_serial_ports());
        println!("detect: {:?}", detect());
    }
}
