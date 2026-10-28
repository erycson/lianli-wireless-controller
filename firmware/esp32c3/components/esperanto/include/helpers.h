#ifndef HELPERS_H
#define HELPERS_H

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <stdbool.h>



uint32_t switch_endianness(uint32_t val);
uint8_t swap(uint8_t val);
void dewhiten_ble(uint8_t *data, int len, int channel);

// CRC16 function
unsigned short crc16(uint8_t *ptr, int count, int init);

// BLE PHY CRC-24 (poly 0x00065B, reflected). Pass the CRCInit register value bit-reversed over
// 24 bits (helper rev24). Validated against the canonical check 0xC25A56 ("123456789", init
// rev24(0x555555)=0xAAAAAA). For the Lian Li network use init rev24(0xA0F955)=0xAA9F05.
uint32_t rev24(uint32_t v);
uint32_t ble_crc24(const uint8_t *data, int len, uint32_t crc_init_reversed);

// Dot15d4 related functions
int hamming(uint8_t *demod_buffer,uint8_t *pattern);
void shift_buffer(uint8_t *demod_buffer,int size);
void msk_to_oqsk(uint8_t *oqpsk_buffer, uint32_t *oqpsk_buffer_length, uint8_t *msk_buffer);
void oqpsk_to_msk(uint8_t *msk_buffer, size_t* msk_size, uint8_t *oqpsk_buffer, size_t oqpsk_buffer_size);

uint16_t update_fcs_dot15d4(uint8_t byte, uint16_t fcs);
uint16_t calculate_fcs_dot15d4(uint8_t *data, int size);
bool check_fcs_dot15d4(uint8_t *data,int size);

#endif
