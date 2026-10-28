# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning: [SemVer](https://semver.org/).

## [0.1.0] - 2026-10-28

### Added
- Virtual L-Wireless dongle over USB/IP (TX `0416:8040`, RX `0416:8041`) for L-Connect 3.
- ESP32-C3 RF Master firmware: transmits L-Connect commands on the Lian Li 2.4 GHz network and
  reports fan status packets heard on air.
- RX status report built from real fan packets (fans, RPM, PWM, binding).
- Automatic controller detection; refuses to run with more than one controller.
- Win32 GUI with fan table, live log and tray icon.
- Start with Windows (logon task) that follows the exe when it moves.
- Single-instance guard.
- Optional traffic log (`proxy_traffic.log`): GUI toggle, 10 MB rotation, keeps the last 3 files.
- CLI: `detect`, `status`, `bridge start`, `ports`, `autostart`.
- CI (Rust tests + firmware build + host tests) and tagged releases.
