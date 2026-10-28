// Virtual Master MAC address (clean identity that registers as Local Controller)
pub const MASTER_MAC: [u8; 6] = [0xAA, 0xBB, 0xCC, 0x11, 0x22, 0x99];

/// Default RF channel (slv3 MasterDevice.MasterChannel initial value).
pub const DEFAULT_CHANNEL: u8 = 8;

/// Entry 0: the Master, exactly as rfrx builds it from the TX beacon it hears on channel 0
/// (`[0x12, MAC, 0*6, ch, 0xFF, clock, 0xFF, 0...]` -> mac, master 0, ch, rx 0xFF, ts, dev_type 0xFF).
/// The master field MUST be zero: with our own MAC there, slv3 takes the Master for a fan bound to
/// us but not bind_to_master and keeps "UnBind reunbind"-ing it.
pub fn make_master_entry_with_ts(ts: u32, channel: u8) -> [u8; 42] {
    let mut b = [0u8; 42];
    b[0..6].copy_from_slice(&MASTER_MAC);
    b[12] = channel;
    b[13] = 0xFF;
    b[14..18].copy_from_slice(&ts.to_be_bytes());
    b[18] = 0xFF; // dev_type = 0xFF (Master Controller itself)
    b[40] = 0xFA;
    b[41] = 0x1C;
    b
}

fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// A node heard over the air (rfrx fan table slot): `a` = packet[1..41], kept raw.
#[derive(Debug, Clone)]
pub struct AirNode {
    pub a: [u8; 40],
    /// fan clock - our clock (ms), so the reported timestamp keeps advancing like rfrx does
    pub ts_off: u32,
    pub last_heard: std::time::Instant,
}

/// Air nodes silent for this long are dropped (rfrx: ~20 aging passes of 0x640 ms).
pub const AIR_NODE_TTL: std::time::Duration = std::time::Duration::from_secs(32);

/// rfrx `rx_packet`: validates a 64-byte 0x12 status packet captured on air and upserts its node.
pub fn apply_air_packet(nodes: &mut Vec<AirNode>, p: &[u8], now_ts: u32) -> bool {
    if p.len() < 64 || p[0] != 0x12 || crc16_xmodem(&p[..0x3c]) != u16::from_be_bytes([p[0x3e], p[0x3f]]) {
        return false;
    }
    let mut a = [0u8; 40];
    a.copy_from_slice(&p[1..41]);
    let ts = u32::from_be_bytes([a[14], a[15], a[16], a[17]]);
    let ts_off = if a[18] == 0xFF { 0 } else { ts.wrapping_sub(now_ts) };
    let node = AirNode { a, ts_off, last_heard: std::time::Instant::now() };
    match nodes.iter_mut().find(|n| n.a[0..6] == a[0..6]) {
        Some(n) => *n = node,
        None => nodes.push(node),
    }
    true
}

/// rfrx `build_report`: entry 0 = our Master, then every live air node as `a` + `fa 1c`.
/// With no node heard yet, the report holds only the Master (count 1).
pub fn build_air_status_response(read_len: usize, nodes: &[AirNode], ts: u32, master_channel: u8) -> Vec<u8> {
    let mut resp = vec![0u8; read_len.max(434)];
    resp[0] = 0x10;
    resp[2] = 0x80;
    resp[3] = master_channel;
    resp[4..46].copy_from_slice(&make_master_entry_with_ts(ts, master_channel));
    let mut count = 1usize;
    for n in nodes.iter().filter(|n| n.last_heard.elapsed() < AIR_NODE_TTL).take(9) {
        let e = &mut resp[4 + count * 42..4 + (count + 1) * 42];
        e[..40].copy_from_slice(&n.a);
        let t = if n.a[18] == 0xFF { n.ts_off } else { ts.wrapping_add(n.ts_off) } & 0x7FFF_FFFF;
        e[14..18].copy_from_slice(&t.to_be_bytes());
        e[40] = 0xFA;
        e[41] = 0x1C;
        count += 1;
    }
    resp[1] = count as u8;
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_air_packet_to_report() {
        // FAN line in the real layout, MACs replaced (group 02:00:00:00:00:01, 3 fans, RPM 1241/1229/1230)
        let mut p = hex("12020000000001020000000099080456ca5def00030334edf61414140004d904cd04ce00009b9b9b9b");
        p.resize(64, 0);
        let crc = crc16_xmodem(&p[..0x3c]);
        p[0x3e..0x40].copy_from_slice(&crc.to_be_bytes());
        let mut nodes = Vec::new();
        assert!(apply_air_packet(&mut nodes, &p, 1000));
        p[5] ^= 1; // corrupted -> rejected
        assert!(!apply_air_packet(&mut nodes, &p, 1000));
        assert_eq!(nodes.len(), 1);
        let r = build_air_status_response(434, &nodes, 2000, 8);
        assert_eq!(r[1], 2);
        let e = &r[46..88];
        assert_eq!(&e[0..6], &[0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
        assert_eq!((e[19], u16::from_be_bytes([e[28], e[29]])), (3, 0x04d9));
        assert_eq!(u32::from_be_bytes([e[14], e[15], e[16], e[17]]), 0x56ca5def + 1000);
        assert_eq!((e[40], e[41]), (0xFA, 0x1C));
    }

    #[test]
    fn test_report_without_nodes_is_master_only() {
        let r = build_air_status_response(448, &[], 12345678, 8);
        assert_eq!((r.len(), r[0], r[1]), (448, 0x10, 1));
        assert_eq!(&r[4..10], &MASTER_MAC);
        assert_eq!(r[22], 0xFF); // dev_type 0xFF = Master
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

}
