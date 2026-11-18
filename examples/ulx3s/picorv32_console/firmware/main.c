// Firmware for picorv32_console. Counts the primes below LIMIT with a sieve,
// prints a banner with the count and how many cycles the sieve took, then
// echoes what arrives on the UART in capitals; the LEDs show the last byte
// received.
#include <stdint.h>

#define UART_DATA (*(volatile uint32_t *)0x02000000)
#define UART_STATUS (*(volatile uint32_t *)0x02000004)
#define LEDS (*(volatile uint32_t *)0x03000000)
#define TX_BUSY 1
#define RX_READY 2

#define LIMIT 2000

static uint8_t composite[LIMIT];

static void put_char(char c) {
    while (UART_STATUS & TX_BUSY) {
    }
    UART_DATA = (uint8_t)c;
}

static void put_string(const char *s) {
    while (*s)
        put_char(*s++);
}

static void put_decimal(uint32_t n) {
    char digits[10];
    int count = 0;
    do {
        digits[count++] = '0' + n % 10;
        n /= 10;
    } while (n != 0);
    while (count != 0)
        put_char(digits[--count]);
}

static int get_char(void) {
    while (!(UART_STATUS & RX_READY)) {
    }
    return UART_DATA & 0xff;
}

static uint32_t cycles(void) {
    uint32_t value;
    __asm__ volatile("rdcycle %0" : "=r"(value));
    return value;
}

// Additions only: rv32i has no multiplier.
static uint32_t count_primes(void) {
    uint32_t count = 0;
    for (uint32_t i = 2; i < LIMIT; i++) {
        if (composite[i])
            continue;
        count++;
        for (uint32_t j = i + i; j < LIMIT; j += i)
            composite[j] = 1;
    }
    return count;
}

int main(void) {
    LEDS = 1;
    uint32_t start = cycles();
    uint32_t primes = count_primes();
    uint32_t spent = cycles() - start;
    put_string("PicoRV32 on ECP5\r\nprimes below ");
    put_decimal(LIMIT);
    put_string(": ");
    put_decimal(primes);
    put_string(", ");
    put_decimal(spent);
    put_string(" cycles\r\n> ");
    for (;;) {
        int c = get_char();
        LEDS = c;
        if (c == '\r') {
            put_string("\r\n> ");
            continue;
        }
        if (c >= 'a' && c <= 'z')
            c -= 'a' - 'A';
        put_char(c);
    }
}
