# Lian Li Wireless Controller

Open-source replacement for the Lian Li L-Wireless dongle: an ESP32-C3 is the RF Master, and
`lianli-controller.exe` presents it to L-Connect 3 as the original dongle over USB/IP.

Read first: `README.md` (usage), `docs/ARCHITECTURE.md` (components, URB loop rules),
`docs/PROTOCOL.md` (USB and radio formats), `firmware/esp32c3/README.md` (firmware).

## Rules

1. **No fake data.** Only the emulated controller identity (MAC `AA:BB:CC:11:22:99`, clock,
   version) is synthetic. Fans, RPM, PWM and binding come only from packets heard over the air.
2. **One controller.** Run `detach_all_virtual()` before attaching and on exit. Refuse to start with
   more than one controller (ESP32-C3s + original dongles).
3. **Timestamps sent to L-Connect: `& 0x7FFFFFFF`.**
4. **Keep the URB loop invariants** in `docs/ARCHITECTURE.md` (blocking socket + `TCP_NODELAY`,
   ≤ 64 bytes per Bulk IN URB, pending IN until data or UNLINK). Breaking any of them makes
   L-Connect reset the dongle every ~1.5 s.
5. **Firmware wire protocol is a contract** between `src/hardware/c3_link.rs` and
   `firmware/esp32c3/main`. Change both sides together.
6. **Software and docs in English.** Docs are imperative and technical: what, how, where. No history.

## Build and test

```powershell
cargo test --lib
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Firmware (Docker, ESP-IDF 4.4.4):

```bash
MSYS_NO_PATHCONV=1 docker run --rm -v "$PWD/firmware/esp32c3:/project" -w /project espressif/idf:v4.4.4 idf.py build
make -C firmware/esp32c3/test        # host unit tests, plain gcc
```

The exe requires admin (UAC manifest). Close a running instance before `cargo build --release`:
the linker cannot replace a running exe.

## Debug

- BSOD with `nt!RtlpHp*` in the stack (0x0A, 0x139): kernel pool corruption from usbip-win2 < 0.9.8.1
  (request completion vs. UNLINK race). Require ≥ 0.9.8.1; confirm with Driver Verifier on
  `usbip2_ude.sys` / `usbip2_filter.sys`.

- `proxy_traffic.log` (next to the exe, enable "Save to proxy_traffic.log" in the GUI): every USB/IP
  URB, UNLINK and serial line. Rotates at 10 MB, keeps `.1`..`.3`.
- `C:\ProgramData\Lian-Li\L-Connect 3\logs\Slv3.log` (UTC): `ResetRx Success` / `ResetTx Success`
  means L-Connect saw 5 failed reads or writes.

## Release

1. Bump `version` in `Cargo.toml` and add a section to `CHANGELOG.md` (Added / Changed / Fixed / Removed).
   Replace `docs/screenshot.png` when the GUI changes.
2. Tag `vX.Y.Z` and push the tag. `.github/workflows/release.yml` builds the exe and the firmware,
   runs the tests and publishes the release.
