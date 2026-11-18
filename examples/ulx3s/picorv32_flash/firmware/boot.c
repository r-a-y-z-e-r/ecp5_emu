// Validate before jumping. No CPU register can issue an arbitrary SPI command.
#include "io.h"
static void hex(uint32_t n, unsigned digits) {
    static const char chars[] = "0123456789ABCDEF";
    while (digits) put_char(chars[(n >> (--digits * 4)) & 15]);
}
int main(void) {
    put_string("PicoRV32 flash ID ");
    hex(*(volatile uint32_t *)0x04000000, 6);
    put_string("\r\n");
    volatile const uint32_t *header = (volatile const uint32_t *)0x10200000;
    volatile const uint8_t *payload = (volatile const uint8_t *)0x1020000c;
    uint32_t length = header[1];
    if (header[0] == 0x57465652 && length >= 4 && length <= 65536 && !(length & 3)) {
        uint32_t crc = ~0u;
        for (uint32_t i = 0; i < length; i++) {
            crc ^= payload[i];
            for (unsigned b = 0; b < 8; b++) crc = (crc >> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
        }
        if (~crc == header[2]) ((void (*)(void))0x1020000c)();
    }
    for (;;) {
        uint32_t start = cycles();
        put_string("no firmware at 200000: ");
        volatile const uint8_t *bytes = (volatile const uint8_t *)header;
        for (unsigned i = 0; i < 16; i++) hex(bytes[i], 2);
        put_string("\r\n");
        while ((uint32_t)(cycles() - start) < 25000000) {}
    }
}
