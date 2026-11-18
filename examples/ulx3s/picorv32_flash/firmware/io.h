#include <stdint.h>
static void put_char(char c) {
    while (*(volatile uint32_t *)0x02000004 & 1) {}
    *(volatile uint32_t *)0x02000000 = (uint8_t)c;
}
static void put_string(const char *s) { while (*s) put_char(*s++); }
static uint32_t cycles(void) {
    uint32_t value;
    __asm__ volatile("rdcycle %0" : "=r"(value));
    return value;
}
