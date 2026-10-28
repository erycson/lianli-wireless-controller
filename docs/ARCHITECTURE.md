# Architecture

## Components

| Component | File | Role |
|---|---|---|
| Entry point | `src/main.rs` | GUI by default, CLI subcommands, single-instance mutex `Global\LianLiWirelessController`. |
| GUI | `src/gui/window.rs` | Startup sequence, fan table, live log, tray. |
| USB/IP server | `src/bridge/server.rs` | Accepts the usbip-win2 connections, runs one URB loop per device. |
| Descriptors | `src/bridge/descriptors.rs` | EP0 control requests (device/config/string/MS OS 2.0 descriptors). |
| RX report | `src/bridge/rf_protocol.rs` | Air-node table and the 448-byte status report. |
| USB/IP client | `src/bridge/client.rs` | Wraps `usbip.exe`: attach, detach, list. |
| Radio link | `src/hardware/c3_link.rs` | Serial line protocol with the ESP32-C3, detection, hardware reset. |
| Autostart | `src/hardware/autostart.rs` | Logon task `LianLiWirelessController`. |
| Services | `src/hardware/service_mgr.rs` | Start/query `LConnectService`, `LConnectServiceWatcher`. |
| Log | `src/traffic_log.rs` | Last 200 lines for the GUI; optional `proxy_traffic.log` (10 MB rotation, keeps `.1`..`.3`, toggle in HKCU `LogToFile`). |

## Startup (GUI and `bridge start`)

1. `detach_all_virtual()` (`usbip detach -a`): stale ports from a crashed run look like a second
   controller to L-Connect.
2. `c3_link::detect()`:
   - list `\Device\USBSER*` ports from `HKLM\HARDWARE\DEVICEMAP\SERIALCOMM`;
   - send `STAT\n`, accept the port that answers `LOG stat cfg=` within 1.5 s;
   - count original dongles (`0416:8040`) with nusb.
   - Exactly one ESP32-C3 and no dongle → continue. Otherwise show the reason and retry every 3 s.
3. Start `C3Link`, start the USB/IP server on `127.0.0.1:3240`, attach TX and RX.
4. Start the L-Connect services.

## USB/IP URB loop rules

L-Connect (`slv3.dll`) is strict about timing. Keep these invariants in `urb_loop`:

- **Blocking socket + `TCP_NODELAY`.** slv3 gives each 64-byte read 10 ms (`WinUsb.ReadAll`). The
  accepted socket inherits the listener's non-blocking mode on Windows; reset it to blocking.
- **One 64-byte packet per Bulk IN URB.** slv3 copies each read into a 64-byte buffer; a longer
  transfer throws, the read returns zeros, and 5 failed reads trigger `ResetRx`/`ResetTx`
  (`USB_ResetAnother 0x15`), which clears the fan list.
- **Bulk IN data exists only after a request.** A Bulk OUT poll (`10 <pages>`) or `USB_GetMac (0x11)`
  replaces the IN queue. With an empty queue, keep the IN URB pending until data arrives or the host
  sends `CMD_UNLINK`. Answer `RET_UNLINK` with `-ECONNRESET` (-104) when the URB was still pending,
  0 otherwise.
- **Never invent data.** Only the emulated controller identity (MAC `AA:BB:CC:11:22:99`, clock,
  version) is synthetic. Fans come only from packets heard over the air.
- **Mask every timestamp with `& 0x7FFFFFFF`.** slv3 parses them as signed `int`.

## RX status report

Built by `build_air_status_response` exactly like the dongle RX chip (`rfrx`):

- Size: `448 × pages` (pages from the poll, clamped 1..4). One page = header + 10 entries.
- `[0]=0x10`, `[1]=entry count`, `[2]=0x80`, `[3]=channel`.
- Entry 0 = the controller beacon: MAC, master `00`, channel, rx `0xFF`, clock, dev_type `0xFF`,
  `fa 1c`. Keep the master field zero; our own MAC there makes slv3 loop "UnBind reunbind".
- Entries 1..9 = live air nodes: `packet[1..41]` + `fa 1c`, timestamp re-based onto our clock.
- A node silent for 32 s (`AIR_NODE_TTL`) is dropped.

## Radio link (`c3_link.rs`)

One thread owns the COM port (read timeout 20 ms) and handles both directions.

| Direction | Line | Meaning |
|---|---|---|
| bridge → C3 | `OUT <128 hex>` | One TX Bulk OUT packet from L-Connect (GetMac included). |
| bridge → C3 | `STAT` | Status query; used for detection. |
| C3 → bridge | `FAN <rssi> <hex>` | 64-byte status packet heard on air (CRC-checked again by the bridge). |
| C3 → bridge | `LOG ...` | Diagnostics. `LOG stat cfg=...` answers `STAT`. |
| C3 → bridge | `LOG ... (reset me)`, `assert ...`, `abort()` | Firmware asks for a hardware reset. |
| C3 → bridge | `RF ...` | Raw captures; ignored. |

On a reset request the link pulses RTS for 100 ms (USB-Serial-JTAG maps RTS to chip reset), closes
the port and reopens it. Commands queued while the port is down are dropped.
