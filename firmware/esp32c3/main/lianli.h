// Lian Li dongle protocol: pure builders shared by the firmware and the host test (test/).
#ifndef LIANLI_H
#define LIANLI_H

#include <stdbool.h>
#include <stdint.h>

#define LIANLI_AA       0x71764129u // access address
#define LIANLI_CRCINIT  0xA0F955u   // CRC-24 init as WCH's RF_Config takes it (not reflected)
#define LIANLI_FRAME    242         // on-air frame rebuilt from 4 Usb_SendRf chunks (+2 trailer bytes)
#define LIANLI_PIP_HDR  6           // packet-in-packet: [55 55][AA LE] before the Lian Li PDU
#define LIANLI_PIP_MAX  244         // largest payload that fits one 255-byte DTM packet

void lianli_be32(uint8_t *p, uint32_t v);

// Feed one 64-byte Usb_SendRf (cmd 0x10) packet. Chunks must arrive as idx 0,1,2,3; a missing chunk
// drops the frame. Returns true when `frame` holds a complete frame. `frame` persists between frames
// on purpose: bytes 0xf0/0xf1 are only rewritten by SyncRgb (0x20) frames, the others inherit them.
bool lianli_frame_feed(uint8_t frame[LIANLI_FRAME], uint8_t *count, const uint8_t pkt[64]);

// Stamp the clock into a 242-byte frame at air time, like the dongle (0x14 at [2..6], 0x20 part 0 at [0x1c..0x20]).
void lianli_stamp_clock(uint8_t *frame, int len, uint32_t clock);

// 64-byte reply to USB_GetMac.
void lianli_getmac_reply(uint8_t r[64], const uint8_t mac[6], uint32_t clock);

// 64-byte presence beacon the dongle sends on channel 0 after each GetMac (CRC-16/XMODEM at 0x3e).
void lianli_beacon(uint8_t b[64], const uint8_t mac[6], uint8_t channel, uint32_t clock);

// Seal a packet-in-packet buffer in place. The caller has put the PDU [type][len][payload] at
// w + LIANLI_PIP_HDR; this appends CRC-24, whitens with BLE channel index `ch` and writes preamble +
// AA in front. Returns the DTM payload length (len + 11). Requires len <= LIANLI_PIP_MAX.
int lianli_pip_seal(uint8_t *w, int len, int ch);

#endif
