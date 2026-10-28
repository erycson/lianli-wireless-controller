use byteorder::{BigEndian, ByteOrder};

pub const USBIP_VERSION: u16 = 0x0111;
pub const OP_REQ_DEVLIST: u16 = 0x8005;
pub const OP_REP_DEVLIST: u16 = 0x0005;
pub const OP_REQ_IMPORT: u16 = 0x8003;
pub const OP_REP_IMPORT: u16 = 0x0003;

pub const USBIP_CMD_SUBMIT: u32 = 0x00000001;
pub const USBIP_RET_SUBMIT: u32 = 0x00000003;
pub const USBIP_CMD_UNLINK: u32 = 0x00000002;
pub const USBIP_RET_UNLINK: u32 = 0x00000004;

/// Serializes a USB device representation in USB/IP wire format.
/// Total length: 312 bytes.
pub fn pack_usb_device(busid: &str, pid: u16) -> Vec<u8> {
    let mut b = vec![0u8; 312];

    // path: 256 bytes null-padded string
    let path = format!("/sys/devices/pci0000:00/0000:00:14.0/usb1/{}", busid);
    let path_bytes = path.as_bytes();
    let copy_len = path_bytes.len().min(255);
    b[..copy_len].copy_from_slice(&path_bytes[..copy_len]);

    // busid: 32 bytes null-padded string at offset 256
    let busid_bytes = busid.as_bytes();
    let copy_len2 = busid_bytes.len().min(31);
    b[256..256 + copy_len2].copy_from_slice(&busid_bytes[..copy_len2]);

    // busnum: u32 at offset 288
    BigEndian::write_u32(&mut b[288..292], 1);
    // devnum: u32 at offset 292
    let devnum = if busid.contains("1-2") { 2 } else { 1 };
    BigEndian::write_u32(&mut b[292..296], devnum);
    // speed: u32 at offset 296 (2 = USB_SPEED_FULL 12Mbps, legally required for 64-byte Bulk EPs)
    BigEndian::write_u32(&mut b[296..300], 2);

    // idVendor: u16 at offset 300 (0x0416 = Winbond / Lian Li)
    BigEndian::write_u16(&mut b[300..302], 0x0416);
    // idProduct: u16 at offset 302 (0x8040 = TX, 0x8041 = RX)
    BigEndian::write_u16(&mut b[302..304], pid);
    // bcdDevice: u16 at offset 304
    BigEndian::write_u16(&mut b[304..306], 0x0000);

    // bDeviceClass, bDeviceSubClass, bDeviceProtocol, bConfigurationValue, bNumConfigurations, bNumInterfaces
    b[306] = 0xFF;
    b[307] = 0xFF;
    b[308] = 0xFF;
    b[309] = 1;
    b[310] = 1;
    b[311] = 1;

    b
}

