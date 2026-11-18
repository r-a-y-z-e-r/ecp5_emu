// Runs entirely from SDRAM: a sieve followed by a 256 KB LFSR memtest.
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
    put_string("PicoRV32 from SDRAM\r\n");
    for (uint32_t pass = 0;; pass++) {
        for (uint32_t i = 0; i < LIMIT; i++)
            composite[i] = 0;
        uint32_t start = cycles();
        uint32_t primes = count_primes();
        uint32_t spent = cycles() - start;
        put_string("pass ");
        put_decimal(pass);
        put_string(": primes below 2000: ");
        put_decimal(primes);
        put_string(", ");
        put_decimal(spent);
        put_string(" cycles\r\n");
        volatile uint32_t *ram = (volatile uint32_t *)0x40100000;
        uint32_t seed = 0x12345678u ^ (pass * 0x9e3779b9u);
        if (seed == 0)
            seed = 1;
        uint32_t pattern = seed;
        for (uint32_t i = 0; i < 65536; i++) {
            ram[i] = pattern;
            pattern = (pattern << 1) ^ ((pattern & 0x80000000) ? 0x04c11db7 : 0);
        }
        uint32_t errors = 0;
        pattern = seed;
        for (uint32_t i = 0; i < 65536; i++) {
            if (ram[i] != pattern)
                errors++;
            pattern = (pattern << 1) ^ ((pattern & 0x80000000) ? 0x04c11db7 : 0);
        }
        put_string("pass ");
        put_decimal(pass);
        put_string(": memtest 256 KB: ");
        put_decimal(errors);
        put_string(" errors\r\n");
        LEDS = errors ? 128 : 2;
    }
}
