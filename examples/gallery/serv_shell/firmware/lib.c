// Software multiply and divide helpers required by RV32I under -nostdlib.
#include <stdint.h>

uint32_t __mulsi3(uint32_t a, uint32_t b) {
    uint32_t r = 0;
    while (b) {
        if (b & 1u) r += a;
        a <<= 1;
        b >>= 1;
    }
    return r;
}

// Restoring shift-and-subtract division over 32 bits.
static uint32_t udivmod(uint32_t n, uint32_t d, uint32_t *rem_out) {
    if (d == 0) {
        if (rem_out) *rem_out = n;
        return 0xffffffffu;
    }
    uint32_t q = 0, r = 0;
    for (int i = 31; i >= 0; i--) {
        r = (r << 1) | ((n >> i) & 1u);
        if (r >= d) {
            r -= d;
            q |= 1u << i;
        }
    }
    if (rem_out) *rem_out = r;
    return q;
}

uint32_t __udivsi3(uint32_t n, uint32_t d) {
    return udivmod(n, d, 0);
}

uint32_t __umodsi3(uint32_t n, uint32_t d) {
    uint32_t r;
    udivmod(n, d, &r);
    return r;
}

int32_t __divsi3(int32_t a, int32_t b) {
    uint32_t ua = a < 0 ? 0u - (uint32_t)a : (uint32_t)a;
    uint32_t ub = b < 0 ? 0u - (uint32_t)b : (uint32_t)b;
    uint32_t uq = udivmod(ua, ub, 0);
    return ((a < 0) ^ (b < 0)) ? (int32_t)(0u - uq) : (int32_t)uq;
}

int32_t __modsi3(int32_t a, int32_t b) {
    uint32_t ua = a < 0 ? 0u - (uint32_t)a : (uint32_t)a;
    uint32_t ub = b < 0 ? 0u - (uint32_t)b : (uint32_t)b;
    uint32_t ur;
    udivmod(ua, ub, &ur);
    return a < 0 ? (int32_t)(0u - ur) : (int32_t)ur;
}
