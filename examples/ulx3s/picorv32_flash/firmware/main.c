// Code and constants execute from flash; the sieve and stack live in EBR.
#include "io.h"
#define LIMIT 2000
static uint8_t composite[LIMIT];
static void decimal(uint32_t n) {
    char digits[10]; unsigned count = 0;
    do { digits[count++] = '0' + n % 10; n /= 10; } while (n);
    while (count) put_char(digits[--count]);
}
int main(void) {
    for (uint32_t pass = 0;; pass++) {
        for (unsigned i = 0; i < LIMIT; i++) composite[i] = 0;
        uint32_t start = cycles(), count = 0;
        for (unsigned i = 2; i < LIMIT; i++) {
            if (composite[i]) continue;
            count++;
            for (unsigned j = i + i; j < LIMIT; j += i) composite[j] = 1;
        }
        uint32_t spent = cycles() - start;
        put_string("pass "); decimal(pass);
        put_string(": primes below 2000: "); decimal(count);
        put_string(", "); decimal(spent); put_string(" cycles\r\n");
    }
}
