// Lian Li RF Master on ESP32-C3 (ESPeranto radio): serial <-> RF bridge.
//
// The Rust bridge emulates the USB/IP dongle for L-Connect and forwards each 64-byte Bulk OUT packet
// over the C3's USB-serial-JTAG; this firmware reassembles and transmits the on-air frames and
// reports what it hears. Protocol: see README.md (newline-terminated ASCII lines).
//
// On-air format (WCH RF_PHY basic mode = BLE-like PDU): AA 0x71764129, then [type][len][payload]
// [CRC-24 init 0xA0F955], whitened with the BLE channel index as seed, 2 Mbps. Lian Li "channel" n
// is a BLE data channel index: f = 2404 + 2n (n <= 10).
// In ESPeranto's DTM-RX format the hardware still de-whitens, seeded by the HOPCTRL channel index, and
// parses the real length byte; the RX buffer holds type,len,payload without the CRC. DTM TX neither
// whitens nor appends a valid CRC, so rf_tx does both in software. The control structure keeps the
// access address and CRC init little-endian, and ESPeranto's configure_sync_word writes MSB first, so
// the sync word passed to it is the byte-reversed AA (0x29417671 for 0x71764129).
#include <string.h>
#include <stdlib.h>
#include <stdint.h>
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/semphr.h"
#include "esp_timer.h"
#include "esp_system.h"
#include "esp_log.h"
#include "nvs_flash.h"
#include "driver/usb_serial_jtag.h"
#include "esperanto.h"
#include "helpers.h"
#include "lianli.h"

extern radio_config_t radio_config;
extern uint8_t esp_ch_idx;
extern int esp_dewhiten_ch;
struct em_buf_tx_desc *get_em_tx_desc(void);

static uint8_t MAC[6] = {0xAA, 0xBB, 0xCC, 0x11, 0x22, 0x99}; // emulated master identity (MASTER cmd)
#define AA_CS       0x29417671u // LIANLI_AA byte-reversed for configure_sync_word
#define RX_CH       0           // listen on Lian Li channel 0 (2404 MHz), where the fans report
static uint32_t tx_ms = 20;             // how long DTM keeps repeating each PDU (every 625 us+)
// One radio does both jobs (the dongle has a TX chip and an RX chip): after each TX keep listening at
// least this long, or back-to-back L-Connect frames starve fan telemetry (FAN lines ~35/s -> ~0.5/s).
// Repeats queued meanwhile are coalesced (see enqueue), so the air rate just drops to ~1/(rx_hold+60ms).
static uint32_t rx_hold = 100, rx_since;

static uint8_t FRAME[LIANLI_FRAME];
static uint8_t BEACON[64];
static uint8_t UREPLY[64];
static uint8_t frame_count;
static uint32_t tx_count;

static uint32_t clk(void) { return (uint32_t)(esp_timer_get_time() / 1000) & 0x7FFFFFFF; }
static uint32_t chfreq(int n) {  // BLE channel index -> MHz
    return n == 37 ? 2402 : n == 38 ? 2426 : n == 39 ? 2480 : n <= 10 ? 2404 + 2*n : 2406 + 2*n;
}

// ---- serial line I/O over USB-serial-JTAG (the only USB on the board) ----
// Lines come from two tasks (main loop + ESPeranto RX task): one mutex keeps them whole.
static SemaphoreHandle_t out_mx;
static void sput(const char *s) {
    xSemaphoreTake(out_mx, portMAX_DELAY);
    usb_serial_jtag_write_bytes((const uint8_t *)s, strlen(s), portMAX_DELAY);
    xSemaphoreGive(out_mx);
}
static void hexline(const char *tag, const uint8_t *d, int n) {
    static char buf[24 + 2*270 + 2];
    xSemaphoreTake(out_mx, portMAX_DELAY);
    int o = 0; for (const char *p = tag; *p; p++) buf[o++] = *p; buf[o++] = ' ';
    static const char H[] = "0123456789abcdef";
    for (int i = 0; i < n; i++) { buf[o++] = H[d[i]>>4]; buf[o++] = H[d[i]&0xf]; }
    buf[o++] = '\n';
    usb_serial_jtag_write_bytes((const uint8_t *)buf, o, portMAX_DELAY);
    xSemaphoreGive(out_mx);
}
static int hexval(char c){ if(c>='0'&&c<='9')return c-'0'; if(c>='a'&&c<='f')return c-'a'+10; if(c>='A'&&c<='F')return c-'A'+10; return -1; }

// ---- RF ----
static void rx_cb(uint8_t *packet, size_t size, int8_t rssi, int frequency) {
    // packet = [sync 4][type][len][payload...]
    (void)frequency;
    if (size < 8) return;
    char tag[24]; snprintf(tag, sizeof tag, "RF %d %d", rssi, (int)size - 6);
    hexline(tag, packet + 4, size - 4 > 262 ? 262 : (int)size - 4);
    const uint8_t *p = packet + 6;   // payload
    if (size >= 6 + 64 && p[0] == 0x12 && crc16((uint8_t *)p, 0x3c, 0) == (uint16_t)(p[0x3e] << 8 | p[0x3f])) {
        snprintf(tag, sizeof tag, "FAN %d", rssi);
        hexline(tag, p, 64);
    }
}

// Wait for `n` HCI command-complete events after `start` (the controller rejects a command sent
// while the previous one is still running: "command disallowed"). Reports failures as LOG lines.
static void hci_wait(uint32_t start, uint32_t n, const char *what) {
    for (int i = 0; i < 50 && hci_cc_count - start < n; i++) vTaskDelay(pdMS_TO_TICKS(2));
    if (hci_last_fail) {
        char m[64]; snprintf(m, sizeof m, "LOG hci fail %s op=%02x status=%02x\n", what, (unsigned)(hci_last_fail >> 8), (unsigned)(hci_last_fail & 0xff));
        hci_last_fail = 0;
        sput(m);
    }
}

// ESPeranto turns the scan event into a DTM-RX format event, which the hardware only ends on
// RFTEST_ABORT (RWBLECNTL bit 26), not on the SCAN_ABORT (bit 24) the stack raises on scan disable:
// without this the event never ends and every later HCI command is "command disallowed" (0x0C).
#define RWBLECNTL (*(volatile uint32_t *)0x60031000)
static void radio_off(void) {
    uint32_t s = hci_cc_count; bool was = radio_config.enabled; bool rx = radio_config.mode == MODE_RX;
    stop_radio();
    if (was && rx) RWBLECNTL |= 1u << 26;
    if (was) { hci_wait(s, 1, rx ? "scan-off" : "test-end"); vTaskDelay(pdMS_TO_TICKS(30)); }   // let the activity really end
}

static void radio_rx(void) {
    bool from_rx = radio_config.mode == MODE_RX;
    radio_off();
    uint32_t s = hci_cc_count;
    // RX->RX restarts hit "command disallowed" without a reset; after a TX test no reset is needed,
    // and resetting there leaks a controller buffer per TX (assert ble_util_buf.c 291 after ~460).
    if (from_rx) { hci_reset(); hci_wait(s, 1, "reset"); }
    esp_ch_idx = RX_CH; esp_dewhiten_ch = -1;   // the hardware already de-whitened (HOPCTRL seed)
    set_sync_word(AA_CS); set_frequency(chfreq(RX_CH)); set_data_rate(DATARATE_2M);
    s = hci_cc_count;
    start_radio(MODE_RX); hci_wait(s, 3, "scan-on");   // event mask + scan params + scan enable
}

// Transmit one PDU [type][len][payload] on channel `ch`, like WCH's RF_Tx in basic mode, as a
// packet-in-packet (see lianli_pip_seal). Limit: payload <= 244 bytes (the dongle's largest is 242).
static void rf_tx(const uint8_t *payload, int len, int ch, uint8_t type) {
    static uint8_t w[LIANLI_PIP_HDR + 2 + 255 + 3];
    uint8_t *p = w + LIANLI_PIP_HDR;
    p[0] = type; p[1] = len; memcpy(p + 2, payload, len);
    if (len > LIANLI_PIP_MAX) { sput("LOG tx skipped: payload > 244 bytes\n"); return; }
    char tag[24]; snprintf(tag, sizeof tag, "TX %d %d", ch, len + 2);
    hexline(tag, p, len + 2);
    int n = lianli_pip_seal(w, len, ch);

    radio_off();
    // Once a scan has run, this controller answers every LE test command with "command disallowed"
    // (0x0C) until an HCI reset; measured on this board, so reset before each TX.
    tx_count++;
    uint32_t s = hci_cc_count;
    hci_reset(); hci_wait(s, 1, "reset");
    esp_ch_idx = 39;   // DTM runs on its channel 39; the hook retunes that table entry (no HW whitening in TX)
    set_frequency(chfreq(ch)); set_data_rate(DATARATE_2M);
    hci_tx_len = n;
    s = hci_cc_count;
    start_radio(MODE_TX); hci_wait(s, 1, "tx-test");
    for (int i = 0; i < 25 && !radio_config.enabled; i++) vTaskDelay(pdMS_TO_TICKS(2));
    static int wedged;   // last-resort safety net: the bridge RTS-resets the board on "(reset me)"
    if (!radio_config.enabled) { sput(++wedged >= 5 ? "LOG tx hook did not fire 5x (reset me)\n" : "LOG tx hook did not fire\n"); }
    else {
        wedged = 0;
        vTaskDelay(pdMS_TO_TICKS(2));                  // let the LLD finish its own test-PDU setup
        struct em_buf_tx_desc *d = get_em_tx_desc();
        uint8_t *buf = r_emi_get_mem_addr_by_offset(d->txdataptr);
        uint8_t *cs = r_emi_get_mem_addr_by_offset(0x400);
        memset(cs + 12, 0x55, 4);                      // hardware AA 0x55555555 (pure preamble)
        for (int i = 0; i < 3; i++) cs[16 + i] = (uint8_t)(LIANLI_CRCINIT >> 8*i);   // CRCINIT, LE
        memcpy(buf, w, n);                             // the DTM buffer holds exactly n bytes
        d->txheader = 0x55 | (n << 8);
        vTaskDelay(pdMS_TO_TICKS(tx_ms));              // DTM repeats the PDU until stopped
    }
    radio_rx();
    rx_since = clk();
}

// TX queue with coalescing. One TX here costs ~100 ms (stop scan, HCI reset, DTM start/stop, rescan)
// while L-Connect sends ~23 frames/s, mostly the same SyncRgb/SyncPwm frames repeated. Sending them
// in order would fall behind forever (and back-pressure the bridge), so a frame that is newer for the
// same target replaces the queued one in place. Key = [0x12, sub, MAC(6)] + frame[0x12] (SyncRgb part
// index) + channel/txType: distinct parts and targets are all kept, only stale repeats are dropped.
#define NQ 16
static struct { bool used; uint8_t ch, type, len; uint32_t seq; uint8_t d[LIANLI_FRAME]; } q[NQ];
static uint32_t qseq;
static uint32_t q_dropped;   // repeats replaced before going on air (STAT)

static bool tx_next(void) {
    int b = -1;
    for (int i = 0; i < NQ; i++) if (q[i].used && (b < 0 || q[i].seq < q[b].seq)) b = i;
    if (b < 0) return false;
    lianli_stamp_clock(q[b].d, q[b].len, clk());     // at air time, like the dongle
    q[b].used = false;
    rf_tx(q[b].d, q[b].len, q[b].ch, q[b].type);
    return true;
}

static void enqueue(const uint8_t *d, int len, uint8_t ch, uint8_t type) {
    int slot = -1;
    for (int i = 0; i < NQ && slot < 0; i++)
        if (q[i].used && q[i].ch == ch && q[i].type == type && q[i].len == len && !memcmp(q[i].d, d, 8)
            && (len < 0x13 || q[i].d[0x12] == d[0x12])) { slot = i; q_dropped++; }
    while (slot < 0) {
        for (int i = 0; i < NQ && slot < 0; i++) if (!q[i].used) { slot = i; q[i].seq = qseq++; }
        if (slot < 0) tx_next();                        // full: drain the oldest first
    }
    q[slot].used = true; q[slot].ch = ch; q[slot].type = type; q[slot].len = len;
    memcpy(q[slot].d, d, len);
}

// One 64-byte Bulk OUT packet from L-Connect (via the bridge).
static void process_usb(const uint8_t *pkt) {
    uint8_t cmd = pkt[0];
    if (cmd == 0x10) {                 // Usb_SendRf chunk: 4 x 60 bytes -> 240-byte frame
        if (lianli_frame_feed(FRAME, &frame_count, pkt))
            enqueue(FRAME, LIANLI_FRAME, pkt[2], pkt[3]);   // channel, txType
        memset(UREPLY, 0, 64); UREPLY[0] = 0x10; hexline("IN", UREPLY, 64);   // ack back to L-Connect
    } else if (cmd == 0x11) {          // USB_GetMac -> reply MAC + send our presence beacon
        lianli_getmac_reply(UREPLY, MAC, clk());
        hexline("IN", UREPLY, 64);
        lianli_beacon(BEACON, MAC, pkt[1], clk());
        enqueue(BEACON, 64, 0, 0xFF);  // beacon rides on channel 0, txType 0xFF
    }
}

static void command(char *line) {
    if (!strncmp(line, "OUT", 3)) {
        uint8_t pkt[64];
        const char *h = line + 3; while (*h == ' ') h++;
        for (int b = 0; b < 64; b++) {
            int hi = hexval(h[2*b]), lo = hexval(h[2*b+1]);
            if (hi < 0 || lo < 0) { sput("LOG bad OUT hex\n"); return; }
            pkt[b] = (hi << 4) | lo;
        }
        process_usb(pkt);
    } else if (!strncmp(line, "STAT", 4)) {
        // The bridge detects this firmware by the "LOG stat cfg=" prefix: keep it.
        extern volatile uint32_t g_rx_cb_entries, g_cfg_entries;
        char m[200]; snprintf(m, sizeof m, "LOG stat cfg=%u rxcb=%u hci_cc=%u fail=%04x enabled=%d mode=%d tx=%u heap=%u minheap=%u qdrop=%u\n",
            (unsigned)g_cfg_entries, (unsigned)g_rx_cb_entries, (unsigned)hci_cc_count, (unsigned)hci_last_fail,
            (int)radio_config.enabled, (int)radio_config.mode, (unsigned)tx_count,
            (unsigned)esp_get_free_heap_size(), (unsigned)esp_get_minimum_free_heap_size(), (unsigned)q_dropped);
        sput(m);
    } else if (!strncmp(line, "MAC", 3)) {           // self-test: a GetMac (reply + beacon)
        uint8_t mac_cmd[64] = {0x11, 8}; process_usb(mac_cmd);
    } else if (!strncmp(line, "MASTER", 6)) {        // impersonate a master MAC (e.g. the fans' bound one)
        const char *h = line + 6; while (*h == ' ') h++;
        for (int b = 0; b < 6; b++) MAC[b] = hexval(h[2*b]) << 4 | hexval(h[2*b+1]);
    } else if (!strncmp(line, "RXHOLD", 6) || !strncmp(line, "TXMS", 4)) {
        if (line[0] == 'R') rx_hold = strtoul(line + 6, NULL, 10);
        else tx_ms = strtoul(line + 4, NULL, 10);
        char m[64]; snprintf(m, sizeof m, "LOG ok rxhold=%ums txms=%ums\n", (unsigned)rx_hold, (unsigned)tx_ms);
        sput(m);
    }
}

void app_main(void)
{
    out_mx = xSemaphoreCreateMutex();
    esp_log_level_set("*", ESP_LOG_NONE);   // keep the serial stream clean; HCI failures come as LOG
    ESP_ERROR_CHECK(nvs_flash_init());
    // The IDF 4.4 driver silently DROPS input when this ring is full (no USB back-pressure), and the
    // main loop is busy ~60-160 ms per TX while L-Connect streams ~2 KB/s: keep seconds of headroom.
    usb_serial_jtag_driver_config_t ucfg = {.rx_buffer_size = 16384, .tx_buffer_size = 4096};
    usb_serial_jtag_driver_install(&ucfg);

    init_radio(rx_cb);
    vTaskDelay(pdMS_TO_TICKS(200));         // HCI reset completes
    radio_rx();
    sput("LOG master up: RX ch0 (2404 MHz), awaiting OUT/MAC over serial\n");

    static char line[600];
    int li = 0;
    uint8_t rxbuf[128];
    for (;;) {
        bool pending = false;
        for (int i = 0; i < NQ; i++) pending |= q[i].used;
        uint32_t held = clk() - rx_since;
        bool can_tx = pending && held >= rx_hold;
        int n = usb_serial_jtag_read_bytes(rxbuf, sizeof rxbuf,
                    can_tx ? 0 : pdMS_TO_TICKS(pending ? (rx_hold - held < 20 ? rx_hold - held + 1 : 20) : 20));
        if (n <= 0 && can_tx) { tx_next(); continue; }   // input drained, RX window done: oldest frame on air
        for (int i = 0; i < n; i++) {
            char ch = rxbuf[i];
            if (ch == '\n' || ch == '\r') {
                line[li] = 0;
                if (li) command(line);
                li = 0;
            } else if (li < (int)sizeof(line) - 1) {
                line[li++] = ch;
            }
        }
    }
}
