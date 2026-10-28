use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use byteorder::{BigEndian, ByteOrder};

use crate::bridge::descriptors::handle_control_request;
use crate::bridge::rf_protocol::{apply_air_packet, build_air_status_response, AirNode, DEFAULT_CHANNEL, MASTER_MAC};
use crate::bridge::usbip_protocol::*;

#[derive(Debug, Default, Clone)]
pub struct BridgeStats {
    pub is_running: bool,
    pub tx_connected: bool,
    pub rx_connected: bool,
    pub packets_received: u64,
    pub packets_forwarded: u64,
    /// RF channel L-Connect has selected (sent as byte 1 of USB_GetMac)
    pub master_channel: u8,
    /// Fans heard over the air by the ESP32-C3 radio: they ARE the status report.
    pub air_nodes: Vec<AirNode>,
}

fn now_ts() -> u32 {
    let millis = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() & 0x7FFFFFFF) as u32;
    if millis == 0 { 0x12345678 } else { millis }
}

fn status_response(stats: &Mutex<BridgeStats>, read_len: usize) -> Vec<u8> {
    match stats.lock() {
        Ok(st) => build_air_status_response(read_len, &st.air_nodes, now_ts(), st.master_channel),
        Err(_) => build_air_status_response(read_len, &[], now_ts(), DEFAULT_CHANNEL),
    }
}

pub type PacketForwarder = Arc<dyn Fn(&[u8]) + Send + Sync + 'static>;

pub struct VirtualDongleServer {
    running: Arc<AtomicBool>,
    stats: Arc<Mutex<BridgeStats>>,
    forwarder: Arc<Mutex<Option<PacketForwarder>>>,
    listener_handle: Option<JoinHandle<()>>,
}

impl Default for VirtualDongleServer {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualDongleServer {
    pub fn new() -> Self {
        let initial_stats = BridgeStats { master_channel: DEFAULT_CHANNEL, ..Default::default() };
        Self {
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(Mutex::new(initial_stats)),
            forwarder: Arc::new(Mutex::new(None)),
            listener_handle: None,
        }
    }

    pub fn set_forwarder<F>(&self, f: F)
    where
        F: Fn(&[u8]) + Send + Sync + 'static,
    {
        if let Ok(mut lock) = self.forwarder.lock() {
            *lock = Some(Arc::new(f));
        }
    }

    /// Callback for the ESP32-C3 link: feeds every 64-byte status packet heard on air.
    pub fn air_packet_sink(&self) -> impl Fn(&[u8]) + Send + Sync + 'static {
        let stats = Arc::clone(&self.stats);
        move |p| {
            if let Ok(mut s) = stats.lock() {
                apply_air_packet(&mut s.air_nodes, p, now_ts());
            }
        }
    }

    pub fn get_stats(&self) -> BridgeStats {
        self.stats.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn start(&mut self, addr: &str) -> std::io::Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Ok(());
        }

        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;

        self.running.store(true, Ordering::SeqCst);
        if let Ok(mut st) = self.stats.lock() {
            st.is_running = true;
        }

        let running = Arc::clone(&self.running);
        let stats = Arc::clone(&self.stats);
        let forwarder = Arc::clone(&self.forwarder);

        let handle = thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _addr)) => {
                        let running_c = Arc::clone(&running);
                        let stats_c = Arc::clone(&stats);
                        let forwarder_c = Arc::clone(&forwarder);

                        thread::spawn(move || {
                            let _ = handle_client(stream, running_c, stats_c, forwarder_c);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => {
                        break;
                    }
                }
            }
        });

        self.listener_handle = Some(handle);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Ok(mut st) = self.stats.lock() {
            st.is_running = false;
            st.tx_connected = false;
            st.rx_connected = false;
        }
        if let Some(h) = self.listener_handle.take() {
            let _ = h.join();
        }
    }
}

fn handle_client(
    mut stream: TcpStream,
    running: Arc<AtomicBool>,
    stats: Arc<Mutex<BridgeStats>>,
    forwarder: Arc<Mutex<Option<PacketForwarder>>>,
) -> std::io::Result<()> {
    // slv3 gives each 64-byte read only 10 ms (WinUsb.ReadAll). The accepted socket inherits the listener's
    // non-blocking mode on Windows, so every URB waited out a 10 ms WouldBlock sleep and the read timed out
    // (5 timeouts = ResetRx). Blocking reads + no Nagle wake on arrival.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_nodelay(true)?;

    while running.load(Ordering::SeqCst) {
        let mut hdr = [0u8; 8];
        match stream.read_exact(&mut hdr) {
            Ok(()) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(_) => return Ok(()),
        }

        let code = BigEndian::read_u16(&hdr[2..4]);

        if code == OP_REQ_DEVLIST {
            let mut rep = Vec::new();
            let mut rep_hdr = [0u8; 12];
            BigEndian::write_u16(&mut rep_hdr[0..2], USBIP_VERSION);
            BigEndian::write_u16(&mut rep_hdr[2..4], OP_REP_DEVLIST);
            BigEndian::write_u32(&mut rep_hdr[4..8], 0);
            BigEndian::write_u32(&mut rep_hdr[8..12], 2);
            rep.extend_from_slice(&rep_hdr);

            rep.extend_from_slice(&pack_usb_device("1-1", 0x8040));
            rep.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0]);

            rep.extend_from_slice(&pack_usb_device("1-2", 0x8041));
            rep.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0]);

            stream.write_all(&rep)?;
            stream.flush()?;
        } else if code == OP_REQ_IMPORT {
            let mut busid_raw = [0u8; 32];
            stream.read_exact(&mut busid_raw)?;
            let busid_str = String::from_utf8_lossy(&busid_raw);
            let busid = busid_str.trim_matches('\0');
            let is_rx = busid.contains("1-2");
            let pid = if is_rx { 0x8041 } else { 0x8040 };

            let mut rep = Vec::new();
            let mut rep_hdr = [0u8; 8];
            BigEndian::write_u16(&mut rep_hdr[0..2], USBIP_VERSION);
            BigEndian::write_u16(&mut rep_hdr[2..4], OP_REP_IMPORT);
            BigEndian::write_u32(&mut rep_hdr[4..8], 0);
            rep.extend_from_slice(&rep_hdr);
            rep.extend_from_slice(&pack_usb_device(busid, pid));

            stream.write_all(&rep)?;
            stream.flush()?;

            if let Ok(mut st) = stats.lock() {
                if is_rx {
                    st.rx_connected = true;
                } else {
                    st.tx_connected = true;
                }
            }

            let res = urb_loop(&mut stream, is_rx, &running, &stats, &forwarder);

            if let Ok(mut st) = stats.lock() {
                if is_rx {
                    st.rx_connected = false;
                } else {
                    st.tx_connected = false;
                }
            }
            return res;
        }
    }
    Ok(())
}

fn urb_loop(
    stream: &mut TcpStream,
    is_rx: bool,
    running: &AtomicBool,
    stats: &Mutex<BridgeStats>,
    forwarder: &Mutex<Option<PacketForwarder>>,
) -> std::io::Result<()> {
    let mut in_queue: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
    // (seqnum, devid, transfer_len) of a Bulk IN waiting for data
    let mut pending_in: Option<(u32, u32, usize)> = None;

    while running.load(Ordering::SeqCst) {
        let mut hdr = [0u8; 48];
        match stream.read_exact(&mut hdr) {
            Ok(()) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(_) => break,
        }

        let cmd = BigEndian::read_u32(&hdr[0..4]);
        let seqnum = BigEndian::read_u32(&hdr[4..8]);
        let devid = BigEndian::read_u32(&hdr[8..12]);
        let direction = BigEndian::read_u32(&hdr[12..16]);
        let ep = BigEndian::read_u32(&hdr[16..20]);
        let transfer_len = BigEndian::read_u32(&hdr[24..28]) as usize;

        if cmd == USBIP_CMD_SUBMIT {
            let setup = &hdr[40..48];
            let mut payload = Vec::new();
            if direction == 0 && transfer_len > 0 {
                payload.resize(transfer_len, 0);
                stream.read_exact(&mut payload)?;
            }

            let mut ret_data = Vec::new();
            let status: i32 = 0;

            if ep == 0 {
                ret_data = handle_control_request(setup, is_rx);
                crate::traffic_log::log_packet(
                    "L-CONNECT <-> PROXY",
                    if is_rx { "RX 0x8041 EP 0" } else { "TX 0x8040 EP 0" },
                    &format!("Control seq={} req=0x{:02X} val=0x{:04X} ret_len={}", seqnum, setup[1], BigEndian::read_u16(&setup[2..4]), ret_data.len()),
                    &ret_data,
                );
            } else if ep == 1 && direction == 0 {
                // Bulk OUT
                if is_rx {
                    let desc = if !payload.is_empty() && payload[0] == 16 {
                        let page = if payload.len() > 1 { payload[1] } else { 1 };
                        format!("Bulk OUT Poll page={}", page)
                    } else {
                        format!("Bulk OUT ({} bytes)", payload.len())
                    };
                    crate::traffic_log::log_packet(
                        "L-CONNECT -> PROXY",
                        "RX 0x8041",
                        &format!("seq={} {}", seqnum, desc),
                        &payload,
                    );

                    if !payload.is_empty() && payload[0] == 16 {
                        let page = if payload.len() > 1 { payload[1] as usize } else { 1 };
                        // rfrx streams pages x 448 bytes (header + 10 entries per page)
                        in_queue.clear();
                        in_queue.extend(status_response(stats, 448 * page.clamp(1, 4)));
                    }
                } else {
                    // TX
                    let cmd_byte = payload.first().copied().unwrap_or(0);
                    let cmd_name = match cmd_byte {
                        16 => {
                            if payload.len() >= 6 && payload[4] == 18 {
                                match payload[5] {
                                    16 => "Usb_SendRf: SyncPwm",
                                    32 => "Usb_SendRf: SyncRgbData",
                                    20 => "Usb_SendRf: SyncMasterClock",
                                    18 => "Usb_SendRf: SelectGroup",
                                    _ => "Usb_SendRf: RF_Select",
                                }
                            } else {
                                "Usb_SendRf"
                            }
                        }
                        17 => "USB_GetMac",
                        18 => "USB_SendHeart",
                        19 => "SetFansRPM",
                        21 => "USB_ResetAnother",
                        22 => "SelectedGroup",
                        24 => "SendLightPackage",
                        25 => "ApplyLightingPackage",
                        32 => "Usb_FwErase",
                        33 => "Usb_FwUpload",
                        34 => "Usb_FwVerify",
                        35 => "Usb_FwReboot",
                        _ => "CMD",
                    };

                    crate::traffic_log::log_packet(
                        "L-CONNECT -> PROXY",
                        "TX 0x8040",
                        &format!("seq={} CMD=0x{:02X} ({}) len={}", seqnum, cmd_byte, cmd_name, payload.len()),
                        &payload,
                    );

                    if !payload.is_empty() && payload[0] == 17 {
                        // slv3 QuerryMasterMac sends its MasterChannel in byte 1
                        if payload.len() > 1 && payload[1] != 0
                            && let Ok(mut st) = stats.lock() {
                                st.master_channel = payload[1];
                            }
                        let mut resp = vec![0u8; 64];
                        resp[0] = 0x11;
                        resp[1..7].copy_from_slice(&MASTER_MAC);
                        resp[7..11].copy_from_slice(&now_ts().to_be_bytes());
                        resp[11] = 0x00; // real dongle: 00 09
                        resp[12] = 0x09;
                        in_queue.clear();
                        in_queue.extend(resp);
                    } else if let Ok(mut st) = stats.lock() {
                        st.packets_received += 1;
                    }

                    // Forward ANY Bulk OUT payload on TX (GetMac too: the radio beacons after it)
                    if let Ok(fwd_guard) = forwarder.lock()
                        && let Some(fwd) = &*fwd_guard {
                            fwd(&payload);
                            if let Ok(mut st) = stats.lock() {
                                st.packets_forwarded += 1;
                            }
                        }
                }
            } else if ep == 1 && direction == 1 {
                // Bulk IN: like the real chips, data only exists after a request and is read in
                // transfer_len chunks; with nothing queued the URB stays pending (device NAKs) until
                // a reply arrives or the host times out and unlinks it. slv3 ReadAll loops 64-byte
                // reads until one times out; answering instantly (or with made-up data) makes its
                // reads fail and it resets the chip after 5 failures.
                if in_queue.is_empty() {
                    crate::traffic_log::log_event("L-CONNECT -> PROXY", if is_rx { "RX 0x8041" } else { "TX 0x8040" },
                        &format!("Bulk IN seq={} want={} pending (no data)", seqnum, transfer_len));
                    pending_in = Some((seqnum, devid, transfer_len));
                    continue;
                }
                crate::traffic_log::log_event("L-CONNECT -> PROXY", if is_rx { "RX 0x8041" } else { "TX 0x8040" },
                    &format!("Bulk IN seq={} want={} queued={}", seqnum, transfer_len, in_queue.len()));
                // One 64-byte packet per URB, like the real chip: slv3's ReadAll copies each read into a
                // 64-byte array, so a longer transfer throws, the read returns zeros and 5 of those = ResetRx.
                ret_data = in_queue.drain(..transfer_len.min(64).min(in_queue.len())).collect();
                log_bulk_in(is_rx, seqnum, &ret_data);
            }

            let actual_len = if direction == 0 { transfer_len as u32 } else { ret_data.len() as u32 };
            send_ret_submit(stream, seqnum, devid, direction, ep, status, actual_len, &ret_data)?;

            if let Some((seq, dev, len)) = pending_in
                && !in_queue.is_empty() {
                    pending_in = None;
                    let data: Vec<u8> = in_queue.drain(..len.min(64).min(in_queue.len())).collect();
                    log_bulk_in(is_rx, seq, &data);
                    send_ret_submit(stream, seq, dev, 1, 1, 0, data.len() as u32, &data)?;
                }
        } else if cmd == USBIP_CMD_UNLINK {
            let unlink_seq = BigEndian::read_u32(&hdr[20..24]);
            // -ECONNRESET: the URB was still pending and is now cancelled (no RET_SUBMIT for it)
            let status = if pending_in.map(|p| p.0) == Some(unlink_seq) {
                pending_in = None;
                -104
            } else {
                0
            };
            crate::traffic_log::log_event("L-CONNECT -> PROXY", if is_rx { "RX 0x8041" } else { "TX 0x8040" },
                &format!("UNLINK seq={} target={} status={}", seqnum, unlink_seq, status));
            let mut ret_hdr = [0u8; 48];
            BigEndian::write_u32(&mut ret_hdr[0..4], USBIP_RET_UNLINK);
            BigEndian::write_u32(&mut ret_hdr[4..8], seqnum);
            BigEndian::write_i32(&mut ret_hdr[20..24], status);
            stream.write_all(&ret_hdr)?;
            stream.flush()?;
        }
    }

    Ok(())
}

fn log_bulk_in(is_rx: bool, seqnum: u32, data: &[u8]) {
    crate::traffic_log::log_packet(
        "PROXY -> L-CONNECT",
        if is_rx { "RX 0x8041" } else { "TX 0x8040" },
        &format!("Bulk IN EP 1 seq={} len={}", seqnum, data.len()),
        data,
    );
}

#[allow(clippy::too_many_arguments)]
fn send_ret_submit(stream: &mut TcpStream, seqnum: u32, devid: u32, direction: u32, ep: u32, status: i32, actual_len: u32, data: &[u8]) -> std::io::Result<()> {
    let mut ret_hdr = [0u8; 48];
    BigEndian::write_u32(&mut ret_hdr[0..4], USBIP_RET_SUBMIT);
    BigEndian::write_u32(&mut ret_hdr[4..8], seqnum);
    BigEndian::write_u32(&mut ret_hdr[8..12], devid);
    BigEndian::write_u32(&mut ret_hdr[12..16], direction);
    BigEndian::write_u32(&mut ret_hdr[16..20], ep);
    BigEndian::write_i32(&mut ret_hdr[20..24], status);
    BigEndian::write_u32(&mut ret_hdr[24..28], actual_len);
    stream.write_all(&ret_hdr)?;
    if !data.is_empty() {
        stream.write_all(data)?;
    }
    stream.flush()
}
