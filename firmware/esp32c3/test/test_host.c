// Host unit test for the pure radio/protocol helpers (no ESP-IDF). Run: make -C test
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include "helpers.h"
#include "lianli.h"

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)

static void unhex(uint8_t *out, const char *h) {
    for (; h[0] && h[1]; h += 2) sscanf(h, "%2hhx", out++);
}

static void test_crc24(void) {
    const uint8_t s[] = "123456789";
    CHECK(rev24(0x555555) == 0xAAAAAA);
    CHECK(ble_crc24(s, 9, rev24(0x555555)) == 0xC25A56);   // reveng CRC-24/BLE check value
    CHECK(rev24(LIANLI_CRCINIT) == 0xAA9F05);
}

static void test_crc16(void) {
    uint8_t s[] = "123456789";
    CHECK(crc16(s, 9, 0) == 0x31C3);                     // CRC-16/XMODEM check value
    // Fan status packet (real layout, MACs replaced): CRC-16/XMODEM of [0..0x3c) at 0x3e.
    uint8_t p[64];
    unhex(p, "12020000000001020000000099080100012c0500030334ed9d1818180003ce03d703cb00007c7c7c7c"
             "00000000000000000000000000000000000000000052db");
    CHECK(crc16(p, 0x3c, 0) == (p[0x3e] << 8 | p[0x3f]));
}

static void test_whitening(void) {
    uint8_t a[64], b[64];
    for (int i = 0; i < 64; i++) a[i] = b[i] = (uint8_t)(i * 37 + 1);
    dewhiten_ble(b, 64, 8);
    CHECK(memcmp(a, b, 64) != 0);
    dewhiten_ble(b, 64, 8);
    CHECK(memcmp(a, b, 64) == 0);                        // self-inverse
    uint8_t h[2] = {0x00, 0xF2};
    dewhiten_ble(h, 2, 8);
    CHECK(h[1] == 0xD6);                                 // length 242 whitens to 214 on channel 8
}

static void test_frame(void) {
    uint8_t frame[LIANLI_FRAME] = {0}, cnt = 0, pkt[64];
    for (int idx = 0; idx < 4; idx++) {
        memset(pkt, 0, 64);
        pkt[0] = 0x10; pkt[1] = idx; pkt[2] = 8; pkt[3] = 1;
        for (int i = 0; i < 60; i++) pkt[4 + i] = (uint8_t)(idx * 60 + i);
        if (idx == 0) { pkt[5] = 0x20; pkt[4 + 0x0e] = 0xAB; pkt[4 + 0x0f] = 0xCD; }
        CHECK(lianli_frame_feed(frame, &cnt, pkt) == (idx == 3));
    }
    CHECK(frame[1] == 0x20 && frame[100] == 100 && frame[239] == 239);
    CHECK(frame[0xf0] == 0xAB && frame[0xf1] == 0xCD);   // SyncRgb copies [0x0e..0x10) to the trailer

    // A missing chunk drops the frame; the next idx 0 starts over.
    memset(pkt, 0, 64); pkt[0] = 0x10;
    pkt[1] = 0; CHECK(!lianli_frame_feed(frame, &cnt, pkt));
    pkt[1] = 2; CHECK(!lianli_frame_feed(frame, &cnt, pkt));
    pkt[1] = 3; CHECK(!lianli_frame_feed(frame, &cnt, pkt));
    for (int idx = 0; idx < 4; idx++) { pkt[1] = idx; CHECK(lianli_frame_feed(frame, &cnt, pkt) == (idx == 3)); }

    uint8_t f[LIANLI_FRAME] = {0};
    f[1] = 0x14; lianli_stamp_clock(f, LIANLI_FRAME, 0x01020304);
    CHECK(f[2] == 1 && f[3] == 2 && f[4] == 3 && f[5] == 4);
    memset(f, 0, sizeof f); f[1] = 0x20; lianli_stamp_clock(f, LIANLI_FRAME, 0x01020304);
    CHECK(f[0x1c] == 1 && f[0x1f] == 4);
    memset(f, 0, sizeof f); f[1] = 0x20; f[0x12] = 1; lianli_stamp_clock(f, LIANLI_FRAME, 0x01020304);
    CHECK(f[0x1c] == 0);                                 // only part 0 of a SyncRgb carries the clock
}

static void test_getmac_beacon(void) {
    const uint8_t mac[6] = {0xAA, 0xBB, 0xCC, 0x11, 0x22, 0x99};
    uint8_t r[64], b[64];
    lianli_getmac_reply(r, mac, 0x7F000001);
    CHECK(r[0] == 0x11 && !memcmp(r + 1, mac, 6) && r[7] == 0x7F && r[10] == 0x01 && r[11] == 0 && r[12] == 9);
    lianli_beacon(b, mac, 8, 0x11223344);
    CHECK(b[0] == 0x12 && !memcmp(b + 1, mac, 6) && b[0x0d] == 8 && b[0x0e] == 0xff && b[0x13] == 0xff);
    CHECK(b[0x0f] == 0x11 && b[0x12] == 0x44);
    CHECK(crc16(b, 0x3c, 0) == (b[0x3e] << 8 | b[0x3f]));
}

static void test_pip(void) {
    uint8_t w[LIANLI_PIP_HDR + 2 + 255 + 3], pl[LIANLI_FRAME];
    for (int i = 0; i < LIANLI_FRAME; i++) pl[i] = (uint8_t)(i ^ 0x5A);
    uint8_t *p = w + LIANLI_PIP_HDR;
    p[0] = 0x01; p[1] = LIANLI_FRAME; memcpy(p + 2, pl, LIANLI_FRAME);
    int n = lianli_pip_seal(w, LIANLI_FRAME, 8);
    CHECK(n == LIANLI_FRAME + 11 && n <= 255);
    CHECK(w[0] == 0x55 && w[1] == 0x55 && w[2] == 0x29 && w[3] == 0x41 && w[4] == 0x76 && w[5] == 0x71);
    dewhiten_ble(p, LIANLI_FRAME + 5, 8);                // what the fan's receiver does
    CHECK(p[0] == 0x01 && p[1] == LIANLI_FRAME && !memcmp(p + 2, pl, LIANLI_FRAME));
    uint32_t c = ble_crc24(p, LIANLI_FRAME + 2, 0xAA9F05);
    CHECK(p[LIANLI_FRAME + 2] == (c & 0xff) && p[LIANLI_FRAME + 3] == ((c >> 8) & 0xff) && p[LIANLI_FRAME + 4] == c >> 16);
}

int main(void) {
    test_crc24();
    test_crc16();
    test_whitening();
    test_frame();
    test_getmac_beacon();
    test_pip();
    printf(fails ? "%d FAILED\n" : "all tests passed\n", fails);
    return fails != 0;
}
