# ESP32-C3 RF Master firmware

Firmware for an ESP32-C3 that replaces the Lian Li wireless dongle radio (UNI FAN SL/TL Wireless).
The Rust bridge (`src/hardware/c3_link.rs`) emulates the dongle to L-Connect over USB/IP and forwards
each TX Bulk OUT packet to this board over its USB-Serial-JTAG port. The firmware:

- reassembles the 4 x 60-byte `Usb_SendRf` chunks into the 242-byte on-air frame, stamps the clock and
  transmits it;
- answers `USB_GetMac` and sends the channel-0 presence beacon, like the dongle TX chip;
- listens continuously on channel 0 and reports every valid fan status packet (`0x12`, CRC-16/XMODEM).

One radio does TX and RX (time-division): after each TX it listens at least `RXHOLD` ms, and repeated
frames for the same target are coalesced in a 16-slot queue.

## Radio parameters

| Parameter      | Value                                                         |
|----------------|---------------------------------------------------------------|
| Access address | `0x71764129`                                                  |
| Data rate      | 2 Mbps                                                        |
| Channel        | BLE data channel index n, f = 2404 + 2n MHz (RX on n = 0)     |
| CRC            | CRC-24 BLE polynomial, init `0xA0F955` (reflected `0xAA9F05`), computed in software |
| Whitening      | BLE whitening seeded by the channel index, applied in software |

The radio is driven through [ESPeranto](components/esperanto) (ROM hooks on the BLE controller).
TX sends the full Lian Li packet (preamble, AA, whitened PDU, CRC) as the payload of a DTM test packet
("packet in packet"), so the payload limit is 244 bytes.

## Serial protocol

USB-Serial-JTAG (shows up as a `USBSER*` COM port on Windows), newline-terminated ASCII.

Host to C3:

| Line              | Meaning                                                                  |
|-------------------|--------------------------------------------------------------------------|
| `OUT <128 hex>`   | One 64-byte Bulk OUT packet from L-Connect (`0x10` SendRf, `0x11` GetMac) |
| `STAT`            | Status; reply starts with `LOG stat cfg=` (the bridge uses it to detect the board) |
| `MAC`             | Self-test: same as a GetMac on channel 8                                 |
| `MASTER <12 hex>` | Master MAC used in GetMac replies and beacons (default `aabbcc112299`)   |
| `RXHOLD <ms>`     | Minimum listening time between two TX (default 100)                      |
| `TXMS <ms>`       | How long each PDU is repeated on air (default 20)                        |

C3 to host:

| Line                  | Meaning                                                              |
|-----------------------|----------------------------------------------------------------------|
| `IN <128 hex>`        | 64-byte reply for L-Connect (SendRf ack, GetMac answer)              |
| `TX <ch> <len> <hex>` | PDU put on air: type, len, payload (before CRC/whitening)            |
| `RF <rssi> <len> <hex>` | Every captured packet: type, len, payload                          |
| `FAN <rssi> <hex64>`  | Captured `0x12` status packet with a valid CRC-16/XMODEM             |
| `LOG <text>`          | Status. A line containing `(reset me)` asks the bridge to pulse RTS (hardware reset) |

`MASTER`, `MAC`, `RXHOLD` and `TXMS` are manual tools; the bridge only sends `OUT` and `STAT`.

## Build

Requires ESP-IDF **v4.4** (ESPeranto's ROM offsets do not match IDF 5.x). With Docker:

```bash
# Git Bash: export MSYS_NO_PATHCONV=1 first
docker run --rm -v "$PWD/firmware/esp32c3:/project" -w /project \
  espressif/idf:v4.4.4 idf.py build
```

`sdkconfig` is generated from `sdkconfig.defaults`; do not commit it. Output: `build/lianli_c3.bin`
(about 310 KB) plus bootloader and partition table.

## Flash

Connect the board's USB port (USB-Serial-JTAG) and pick its COM port.

Separate images:

```bash
cd build
python -m esptool --chip esp32c3 -p COM3 -b 460800 --before default_reset --after hard_reset \
  write_flash --flash_mode dio --flash_size 2MB --flash_freq 80m \
  0x0 bootloader/bootloader.bin 0x8000 partition_table/partition-table.bin 0x10000 lianli_c3.bin
```

Single merged image at `0x0`:

```bash
cd build
python -m esptool --chip esp32c3 merge_bin -o lianli_c3_merged.bin @flash_args
python -m esptool --chip esp32c3 -p COM3 -b 460800 write_flash 0x0 lianli_c3_merged.bin
```

Or with IDF: `idf.py -p COM3 flash` (Docker cannot reach Windows COM ports; run esptool natively).

## Host test

Pure helpers (CRC-24, CRC-16/XMODEM, whitening, frame reassembly, clock stamping, GetMac reply,
beacon, packet-in-packet sealing) build without ESP-IDF:

```bash
make -C test          # gcc; exits non-zero on failure
```

Or in Docker: `docker run --rm -v "$PWD":/p -w /p/test gcc:13 make test`.

## Recovery

The ROM bootloader cannot be overwritten. If the firmware hangs or the port does not enumerate:

1. Hold **BOOT** (GPIO9), press and release **RESET** (or replug USB), release BOOT.
2. The board enters download mode on the same USB port.
3. Flash again with the commands above, or erase with `python -m esptool --chip esp32c3 -p COM3 erase_flash`.

## License

`components/esperanto/` is ESPeranto by Romain Cayre, Damien Cauquil and Aurélien Francillon
(github.com/RCayre/woot2023_espwn32_artifacts), MIT License, see `components/esperanto/LICENSE`.
Local changes: `PRIV_REQUIRES bt` in CMake; the C3 RX callback always chains to the original scan
handler; the RX task frees each packet after the callback; configurable channel index for
HOPCTRL/frequency table and RX software dewhitening; HCI command-complete counters; DTM length from
`hci_tx_len`; STAT counters.
