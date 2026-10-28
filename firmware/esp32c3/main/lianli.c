#include <string.h>
#include "lianli.h"
#include "helpers.h"

void lianli_be32(uint8_t *p, uint32_t v) { p[0]=v>>24; p[1]=v>>16; p[2]=v>>8; p[3]=v; }

bool lianli_frame_feed(uint8_t frame[LIANLI_FRAME], uint8_t *count, const uint8_t pkt[64]) {
    uint8_t idx = pkt[1];
    if (idx == 0) *count = 0;
    if (idx == *count && idx < 4) { memcpy(frame + (uint32_t)idx * 60, pkt + 4, 60); (*count)++; }
    else *count = 0xff;
    if (*count != 4) return false;
    *count = 0;
    if (frame[1] == 0x20) { frame[0xf0] = frame[0x0e]; frame[0xf1] = frame[0x0f]; }
    return true;
}

void lianli_stamp_clock(uint8_t *f, int len, uint32_t t) {
    if (len != LIANLI_FRAME) return;
    if (f[1] == 0x14) lianli_be32(f + 2, t);
    else if (f[1] == 0x20 && f[0x12] == 0) lianli_be32(f + 0x1c, t);
}

void lianli_getmac_reply(uint8_t r[64], const uint8_t mac[6], uint32_t t) {
    memset(r, 0, 64);
    r[0] = 0x11; memcpy(r + 1, mac, 6);
    lianli_be32(r + 7, t); r[11] = 0x00; r[12] = 0x09;
}

void lianli_beacon(uint8_t b[64], const uint8_t mac[6], uint8_t channel, uint32_t t) {
    memset(b, 0, 64);
    b[0] = 0x12;
    memcpy(b + 1, mac, 6);
    b[0x0d] = channel;
    b[0x0e] = 0xff;
    lianli_be32(b + 0x0f, t);
    b[0x13] = 0xff;
    uint16_t c = crc16(b, 0x3c, 0);
    b[0x3e] = c >> 8; b[0x3f] = c;
}

// The DTM hardware sends its 2 header bytes raw and then exactly <header length> bytes, so a whitened
// length byte smaller than the frame (242 on ch 8 whitens to 214) would cut it. Instead the whole Lian
// Li packet rides INSIDE the DTM payload; the hardware packet itself uses AA 0x55555555 + header 55,<n>,
// which the fans see as a long preamble before the embedded AA.
int lianli_pip_seal(uint8_t *w, int len, int ch) {
    uint8_t *p = w + LIANLI_PIP_HDR;
    uint32_t c = ble_crc24(p, len + 2, rev24(LIANLI_CRCINIT));
    p[len+2] = c; p[len+3] = c >> 8; p[len+4] = c >> 16;
    dewhiten_ble(p, len + 5, ch);
    w[0] = w[1] = 0x55;                            // preamble (AA LSB is 1 -> ...0101 then AA)
    for (int i = 0; i < 4; i++) w[2 + i] = (uint8_t)(LIANLI_AA >> 8*i);   // AA, LE
    return len + 11;
}
