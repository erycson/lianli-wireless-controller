# Protocol

Reference for the Lian Li L-Wireless network as implemented by this project. Byte ranges are
half-open (`a..b` = bytes a to b-1). Multi-byte integers are big-endian unless stated.

## 1. Radio (PHY)

| Parameter | Value |
|---|---|
| Modulation | BLE-style GFSK, 2 Mbps |
| Access address (sync word) | `0x71764129` |
| CRC | CRC-24 (BLE polynomial), init `0xA0F955`; pass `0xAA9F05` (bit-reversed) to a reflected implementation |
| Whitening | BLE whitening, seeded by the channel index |
| Channel | BLE index `n`, frequency `2404 + 2n` MHz |
| Channel 0 (2404 MHz) | fan status packets and the controller beacon |
| Working channel | set by L-Connect (default 8 = 2420 MHz); carries commands |

## 2. Dongle USB interface

Two devices, each WinUSB with Bulk OUT/IN on EP 1, 64-byte packets.

| Device | VID:PID | Role |
|---|---|---|
| TX | `0416:8040` | Sends commands; answers `USB_GetMac`. |
| RX | `0416:8041` | Serves the fan table (status report). |

### 2.1 TX commands (Bulk OUT byte 0)

| Byte 0 | Name | Notes |
|---|---|---|
| `0x10` | `Usb_SendRf` | `[0x10, idx, channel, txType, payload[60]]`; 4 packets form a 240-byte frame. |
| `0x11` | `USB_GetMac` | `[0x11, channel, ...]`. Triggers one beacon on channel 0. |
| `0x15` | `USB_ResetAnother` | Sent by L-Connect after 5 failed reads/writes (reset the other chip). |
| `0x20..0x25` | IAP | Erase / program / jump / version. Not implemented. |

### 2.2 `USB_GetMac` reply (Bulk IN, 64 bytes)

`[0x11, MAC[6], clock u32 (& 0x7FFFFFFF), 0x00, 0x09, 0...]` — `0x09` is the firmware version.

### 2.3 Command frame on air (`Usb_SendRf`)

1. Reassemble `frame[idx*60 .. idx*60+60] = pkt[4..64]` for `idx = 0..3` (240 bytes).
2. Stamp the controller clock (u32) by subcommand `frame[1]`:
   - `0x14` SyncMasterClock → `frame[2..6]`;
   - `0x20` SyncRgb → copy `frame[0x0e..0x10]` to `frame[0xf0..0xf2]`; if `frame[0x12] == 0`, clock → `frame[0x1c..0x20]`;
   - others → unchanged.
3. Transmit 242 bytes on channel `pkt[2]` with `txType = pkt[3]`.

Frame payload (`frame[0] = 0x12`):

| Bytes | Field |
|---|---|
| `1` | subcommand: `0x10` bind/PWM, `0x12` select, `0x14` clock sync, `0x20` RGB, others = control |
| `2..8` | target MAC (`FF×6` = broadcast) |
| `8..14` | controller MAC |
| `14` | target rx type (group id) |
| `15` | target channel |
| `17..21` | PWM per fan (`0..255`), subcommand `0x10` |

### 2.4 Controller beacon (64 bytes, channel 0, once per `USB_GetMac`)

| Bytes | Value |
|---|---|
| `0` | `0x12` |
| `1..7` | controller MAC |
| `7..13` | 0 |
| `13` | working channel |
| `14` | `0xFF` |
| `15..19` | clock u32 |
| `19` | `0xFF` |
| `62..64` | CRC-16/XMODEM (poly `0x1021`, init 0) over bytes `0..60` |

## 3. Fan status packet (on air, 64 bytes, channel 0)

| Bytes | Field |
|---|---|
| `0` | `0x12` |
| `1..7` | group MAC |
| `7..13` | bound controller MAC (`00×6` = unbound) |
| `13` | channel |
| `14` | rx type (group id) |
| `15..19` | fan clock u32 |
| `19` | dev_type (`0` = fan group, `0xFF` = controller) |
| `20` | fan count (1..4) |
| `21..25` | RGB effect index |
| `25..29` | fan type per fan (e.g. `0x14` SL Wireless) |
| `29..37` | RPM per fan, u16; high nibble of the first byte holds flags |
| `37..41` | PWM per fan |
| `62..64` | CRC-16/XMODEM over bytes `0..60` |

Drop packets with a wrong `[0]` or CRC.

## 4. RX status report (Bulk IN of `0416:8041`)

Request: Bulk OUT `[0x10, pages, ...]`. Reply: `448 × pages` bytes, delivered as 64-byte packets.

| Bytes | Field |
|---|---|
| `0` | `0x10` |
| `1` | entry count (1 + fans) |
| `2` | `0x80` |
| `3` | channel |
| `4 + 42k ..` | entry `k`, 42 bytes |

Entry = status packet bytes `1..41` (timestamp re-based, `& 0x7FFFFFFF`) + `0xFA 0x1C`.
Entry 0 = the controller (from its own beacon): master field `00×6`, rx `0xFF`, dev_type `0xFF`.

Within an entry (offsets relative to the entry): `0..6` MAC, `6..12` bound controller, `12` channel,
`13` rx type, `14..18` clock, `18` dev_type, `19` fan count, `20..24` effect, `24..28` fan types,
`28..36` RPM, `36..40` PWM, `40..42` `FA 1C`.

L-Connect marks a group *bound* when its controller field equals the `USB_GetMac` MAC.
