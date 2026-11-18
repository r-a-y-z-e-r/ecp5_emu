// The runtime helpers clang calls for rv32i, which has no divider.
#include <stdint.h>

uint32_t __udivsi3(uint32_t n, uint32_t d) {
    uint32_t quotient = 0, remainder = 0;
    for (int bit = 31; bit >= 0; bit--) {
        remainder = (remainder << 1) | ((n >> bit) & 1);
        if (remainder >= d) {
            remainder -= d;
            quotient |= 1u << bit;
        }
    }
    return quotient;
}

uint32_t __umodsi3(uint32_t n, uint32_t d) {
    uint32_t remainder = 0;
    for (int bit = 31; bit >= 0; bit--) {
        remainder = (remainder << 1) | ((n >> bit) & 1);
        if (remainder >= d)
            remainder -= d;
    }
    return remainder;
}
