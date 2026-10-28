// Exact Descriptors for Lian Li L-Wireless Dongles
// TX (Sender): VID 0x0416, PID 0x8040
// RX (Receiver): VID 0x0416, PID 0x8041

pub const DEV_DESC_TX: [u8; 18] = [
    0x12, 0x01, 0x01, 0x02, 0xff, 0xff, 0xff, 0x40, // bLength, bDescriptorType, bcdUSB, bDevClass, SubClass, Protocol, bMaxPacketSize0
    0x16, 0x04, 0x40, 0x80, 0x00, 0x00, 0x01, 0x02, // idVendor (0x0416), idProduct (0x8040), bcdDevice (0x0000), iManufacturer, iProduct
    0x00, 0x01,                                     // iSerialNumber, bNumConfigurations
];

pub const DEV_DESC_RX: [u8; 18] = [
    0x12, 0x01, 0x01, 0x02, 0xff, 0xff, 0xff, 0x40,
    0x16, 0x04, 0x41, 0x80, 0x00, 0x00, 0x01, 0x02, // idVendor (0x0416), idProduct (0x8041)
    0x00, 0x01,
];

pub const CONF_DESC: [u8; 32] = [
    0x09, 0x02, 0x20, 0x00, 0x01, 0x01, 0x00, 0xa0, 0x23, // Configuration Descriptor (len=32, 1 interface, bus powered, 70mA)
    0x09, 0x04, 0x00, 0x00, 0x02, 0xff, 0x00, 0x00, 0x00, // Interface 0 (2 endpoints, vendor class 0xFF)
    0x07, 0x05, 0x81, 0x02, 0x40, 0x00, 0x00,             // Endpoint 1 IN (Bulk, 64 bytes)
    0x07, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00,             // Endpoint 1 OUT (Bulk, 64 bytes)
];

pub const BOS_DESC: [u8; 33] = [
    0x05, 0x0f, 0x21, 0x00, 0x01,                         // BOS Header (len=33, 1 capability)
    0x1c, 0x10, 0x05, 0x00,                               // Device Capability (len=28, type=0x10 Platform)
    0xdf, 0x60, 0xdd, 0xd8, 0x89, 0x45, 0xc7, 0x4c,       // Microsoft OS 2.0 Platform Capability UUID
    0x9c, 0xd2, 0x65, 0x9d, 0x9e, 0x64, 0x8a, 0x9f,
    0x00, 0x00, 0x03, 0x06,                               // Windows version (0x06030000 = Windows 8.1+)
    0xa2, 0x00, 0x55, 0x00,                               // MS OS 2.0 descriptor set length (162 bytes), bMS_VendorCode (0x55)
];

// MS OS 2.0 Descriptor Set (162 bytes) to bind Microsoft WINUSB driver automatically with custom DeviceInterfaceGUID
pub const MS_OS_20_SET: [u8; 162] = [
    0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x06, 0xa2, 0x00, // Descriptor Set Header (len=10, Windows 8.1+, total=162)
    0x14, 0x00, 0x03, 0x00, 0x57, 0x49, 0x4e, 0x55, 0x53, 0x42, // Compatible ID Feature (len=20, "WINUSB")
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x84, 0x00, 0x04, 0x00, 0x07, 0x00, 0x2a, 0x00,             // Registry Property Feature (len=132, type=REG_MULTI_SZ)
    // "DeviceInterfaceGUIDs\0" (UTF-16LE)
    0x44, 0x00, 0x65, 0x00, 0x76, 0x00, 0x69, 0x00, 0x63, 0x00, 0x65, 0x00,
    0x49, 0x00, 0x6e, 0x00, 0x74, 0x00, 0x65, 0x00, 0x72, 0x00, 0x66, 0x00,
    0x61, 0x00, 0x63, 0x00, 0x65, 0x00, 0x47, 0x00, 0x55, 0x00, 0x49, 0x00,
    0x44, 0x00, 0x73, 0x00, 0x00, 0x00,
    0x50, 0x00, // Property Data Length (80 bytes)
    // "{1D4B2365-4749-48EA-B38A-7C6FDDDD7E26}\0\0" (UTF-16LE)
    0x7b, 0x00, 0x31, 0x00, 0x44, 0x00, 0x34, 0x00, 0x42, 0x00, 0x32, 0x00,
    0x33, 0x00, 0x36, 0x00, 0x35, 0x00, 0x2d, 0x00, 0x34, 0x00, 0x37, 0x00,
    0x34, 0x00, 0x39, 0x00, 0x2d, 0x00, 0x34, 0x00, 0x38, 0x00, 0x45, 0x00,
    0x41, 0x00, 0x2d, 0x00, 0x42, 0x00, 0x33, 0x00, 0x38, 0x00, 0x41, 0x00,
    0x2d, 0x00, 0x37, 0x00, 0x43, 0x00, 0x36, 0x00, 0x46, 0x00, 0x44, 0x00,
    0x44, 0x00, 0x44, 0x00, 0x44, 0x00, 0x37, 0x00, 0x45, 0x00, 0x32, 0x00,
    0x36, 0x00, 0x7d, 0x00, 0x00, 0x00, 0x00, 0x00,
];

pub fn encode_utf16_desc(s: &str) -> Vec<u8> {
    let utf16: Vec<u16> = s.encode_utf16().collect();
    let len = 2 + (utf16.len() * 2);
    let mut b = Vec::with_capacity(len);
    b.push(len as u8);
    b.push(0x03); // STRING descriptor
    for c in utf16 {
        b.push((c & 0xff) as u8);
        b.push(((c >> 8) & 0xff) as u8);
    }
    b
}

pub fn handle_control_request(setup: &[u8], is_rx: bool) -> Vec<u8> {
    if setup.len() < 8 {
        return Vec::new();
    }
    let req_type = setup[0];
    let req = setup[1];
    let value = (setup[2] as u16) | ((setup[3] as u16) << 8);
    let index = (setup[4] as u16) | ((setup[5] as u16) << 8);
    let length = (setup[6] as usize) | ((setup[7] as usize) << 8);

    let desc_type = (value >> 8) as u8;
    let desc_idx = (value & 0xFF) as u8;

    // Standard Requests
    if (req_type & 0x60) == 0x00 {
        if req == 0x06 {
            // GET_DESCRIPTOR
            if desc_type == 1 {
                // DEVICE
                let d = if is_rx { &DEV_DESC_RX } else { &DEV_DESC_TX };
                return d[..d.len().min(length)].to_vec();
            } else if desc_type == 2 {
                // CONFIGURATION
                return CONF_DESC[..CONF_DESC.len().min(length)].to_vec();
            } else if desc_type == 3 {
                // STRING
                if desc_idx == 0 {
                    let s = [4, 3, 0x09, 0x04];
                    return s[..s.len().min(length)].to_vec();
                } else if desc_idx == 1 {
                    let s = encode_utf16_desc("wch.cn");
                    return s[..s.len().min(length)].to_vec();
                } else if desc_idx == 2 {
                    let s = encode_utf16_desc(if is_rx { "SLV3RX" } else { "SLV3TX" });
                    return s[..s.len().min(length)].to_vec();
                } else {
                    return vec![2, 3];
                }
            } else if desc_type == 15 {
                // BOS
                return BOS_DESC[..BOS_DESC.len().min(length)].to_vec();
            }
        } else if req == 0x05 || req == 0x09 || req == 0x0B || req == 0x01 {
            // SET_ADDRESS, SET_CONFIGURATION, SET_INTERFACE, CLEAR_FEATURE
            return Vec::new();
        } else if req == 0x00 {
            // GET_STATUS
            return vec![0x00, 0x00];
        }
    } else if (req_type & 0x60) != 0 {
        // Vendor Request (MS OS 2.0 Descriptor)
        if req == 0x55 && index == 0x0007 {
            return MS_OS_20_SET[..MS_OS_20_SET.len().min(length)].to_vec();
        }
    }

    Vec::new()
}
