// No library or OS: volatile loads/stores are the peripheral bus transactions.
#include <stdint.h>
#include <stddef.h>
#define REG(a) (*(volatile uint32_t *)(uintptr_t)(a))
#define UART_DATA REG(0x02000000)
#define UART_STATUS REG(0x02000004)
#define LEDS REG(0x03000000)
#define BUTTONS REG(0x04000000)
#define EVENTS REG(0x04000004)
#define RESET REG(0x05000000)
#define CYCLE_LO REG(0x06000000)
#define CYCLE_HI REG(0x06000004)
#define CLOCK 25000000u
#define BUILD_DATE "2026-11-16"
extern char _bss_end[], _image_end[];
static char line[128], picture[24][64];
static const char *const names[] = {"PWR", "FIRE1", "FIRE2", "UP", "DOWN", "LEFT", "RIGHT"};

// A reset cannot reload RAM. This deliberately retained queue carries bytes
// already read from the UART across a reboot, including a whole pasted script.
static volatile struct {
    uint32_t count, next;
    uint8_t bytes[512];
} pending __attribute__((section(".persist"))) = {0, 0, {0}};

static inline uint32_t cycle(void) {
    return CYCLE_LO;
}
static inline uintptr_t stack(void) {
    uintptr_t p;
    __asm__ volatile("mv %0, sp" : "=r"(p));
    return p;
}
static void putc_uart(char c) {
    while (UART_STATUS & 1) {}
    UART_DATA = (uint8_t)c;
}
static void puts_uart(const char *s) { while (*s) putc_uart(*s++); }
static void decimal(uint32_t n) {
    char b[10]; unsigned k = 0;
    do { b[k++] = '0' + n % 10; n /= 10; } while (n);
    while (k) putc_uart(b[--k]);
}
static void signed_decimal(int32_t n) {
    if (n < 0) { putc_uart('-'); decimal(0u - (uint32_t)n); }
    else decimal((uint32_t)n);
}
static void hex(uint32_t n, unsigned digits) {
    while (digits) putc_uart("0123456789abcdef"[(n >> (--digits * 4)) & 15]);
}
static int equal(const char *a, const char *b) {
    while (*a && *a == *b) { a++; b++; }
    return *a == *b;
}
static void spaces(const char **s) { while (**s == ' ' || **s == '\t') ++*s; }
static int digit(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}
static int number(const char **s, uint32_t *n, unsigned base) {
    spaces(s);
    if ((*s)[0] == '0' && ((*s)[1] == 'x' || (*s)[1] == 'X')) { *s += 2; base = 16; }
    const char *start = *s; *n = 0;
    int d;
    while ((d = digit(**s)) >= 0 && (unsigned)d < base) { *n = *n * base + d; ++*s; }
    return *s != start;
}
static int end(const char *s) { spaces(&s); return *s == 0; }
static void spent(uint32_t n) { decimal(n); puts_uart(" cycles\r\n"); }
static void banner(void) {
    puts_uart("RISC-V shell on ULX3S\r\nSERV RV32I | 25000000 Hz | 32768 bytes RAM\r\nfirmware: ");
    decimal((uintptr_t)_image_end); puts_uart(" bytes\r\n> ");
}

// Recursive descent follows precedence: sum -> product -> unary/parentheses.
// Unsigned intermediate arithmetic gives explicit modulo-2^32 overflow.
static const char *expr;
static const char *expr_error;
static unsigned depth;
static uint32_t sum(void);
static uint32_t atom(void) {
    spaces(&expr);
    if (++depth > 24) { expr_error = "expression too deep"; --depth; return 0; }
    uint32_t v = 0;
    if (*expr == '+' || *expr == '-') {
        char op = *expr++; v = atom(); if (op == '-') v = 0u - v;
    } else if (*expr == '(') {
        expr++; v = sum(); spaces(&expr);
        if (*expr == ')') expr++; else expr_error = "syntax error";
    } else if (!number(&expr, &v, 10)) expr_error = "syntax error";
    --depth;
    return v;
}
static uint32_t product(void) {
    uint32_t a = atom();
    for (;;) {
        spaces(&expr); char op = *expr;
        if (op != '*' && op != '/' && op != '%') return a;
        expr++; uint32_t b = atom();
        if (op == '*') a *= b;
        else if (!b) { expr_error = "division by zero"; a = 0; }
        // RISC-V defines this signed overflow; avoid undefined C division.
        else if (a == 0x80000000u && b == 0xffffffffu) a = op == '/' ? a : 0;
        else if (op == '/') a = (uint32_t)((int32_t)a / (int32_t)b);
        else a = (uint32_t)((int32_t)a % (int32_t)b);
    }
}
static uint32_t sum(void) {
    uint32_t a = product();
    for (;;) {
        spaces(&expr); char op = *expr;
        if (op != '+' && op != '-') return a;
        expr++; uint32_t b = product(); a = op == '+' ? a + b : a - b;
    }
}

// A bit per integer leaves room for a larger sieve without stealing the stack.
// Volatile scratch accesses also prevent the compiler moving work past cycle().
static __attribute__((noinline)) uint32_t sieve(uint32_t n) {
    volatile uint8_t *bits = (volatile uint8_t *)_bss_end;
    for (uint32_t i = 0; i < (n + 7) / 8; i++) bits[i] = 0;
    uint32_t count = 0;
    for (uint32_t i = 2; i < n; i++) {
        if (bits[i >> 3] & (1u << (i & 7))) continue;
        count++;
        if (i * i < n)
            for (uint32_t j = i * i; j < n; j += i) bits[j >> 3] |= 1u << (j & 7);
    }
    return count;
}
static void primes(const char *args) {
    uint32_t n = 10000;
    uintptr_t ceiling = (stack() - 1024) & ~3u;
    // i*i remains in range up to this cap; the available RAM is the tighter bound.
    uint32_t max = (ceiling - (uintptr_t)_bss_end) * 8;
    if (max > 65535) max = 65535;
    if ((!end(args) && !number(&args, &n, 10)) || !end(args) || n < 2 || n > max) {
        puts_uart("primes: range 2.."); decimal(max); puts_uart("\r\n"); return;
    }
    uint32_t start = cycle();
    uint32_t count = sieve(n);
    uint32_t elapsed = cycle() - start;
    puts_uart("primes below "); decimal(n); puts_uart(": "); decimal(count); puts_uart(", "); spent(elapsed);
}

// Q12 fixed point: |z| <= 2 before each iteration keeps all products in int32.
// Rendering into RAM excludes every UART wait from the measured computation.
static __attribute__((noinline)) void render(void) {
    static const char shades[] = " .:-=+*#%@";
    for (int y = 0; y < 24; y++) {
        int32_t ci = (y - 12) * 410;
        for (int x = 0; x < 64; x++) {
            int32_t cr = (x - 42) * 205, zr = 0, zi = 0;
            unsigned k = 0;
            for (; k < 24; k++) {
                int32_t rr = zr * zr, ii = zi * zi;
                if (rr + ii > (4 << 24)) break;
                zi = ((zr * zi) >> 11) + ci;
                zr = ((rr - ii) >> 12) + cr;
            }
            picture[y][x] = k == 24 ? '@' : shades[k * 9 / 24];
        }
    }
}
static void mandel(void) {
    uint32_t start = cycle(); render(); uint32_t elapsed = cycle() - start;
    for (unsigned y = 0; y < 24; y++) {
        for (unsigned x = 0; x < 64; x++) putc_uart(picture[y][x]);
        puts_uart("\r\n");
    }
    puts_uart("mandel: "); spent(elapsed);
}
static void knight(const char *args) {
    uint32_t n = 3;
    if ((!end(args) && !number(&args, &n, 10)) || !end(args) || n > 100) {
        puts_uart("knight: range 0..100\r\n"); return;
    }
    uint32_t saved = LEDS;
    for (uint32_t sweep = 0; sweep < n; sweep++)
        for (unsigned step = 0; step < 14; step++) {
            // D7 is physically leftmost: begin there and return from D0.
            unsigned bit = step < 8 ? 7 - step : step - 7;
            LEDS = 1u << bit;
            uint32_t start = cycle();
            while (cycle() - start < CLOCK / 100) {}
        }
    LEDS = saved; puts_uart("done\r\n");
}
static void buttons(void) {
    uint32_t levels = BUTTONS;
    for (unsigned b = 0; b < 7; b++) {
        if (b) putc_uart(' ');
        puts_uart(names[b]); putc_uart('='); putc_uart('0' + ((levels >> b) & 1));
    }
    puts_uart("\r\n");
}
static int mapped(uint32_t a) {
    return a < 32768 || a == 0x02000000 || a == 0x02000004 || a == 0x03000000 ||
           a == 0x04000000 || a == 0x04000004 || a == 0x05000000 ||
           a == 0x06000000 || a == 0x06000004;
}
static void memory(const char *cmd, const char *args) {
    uint32_t a, v = 64;
    if (!number(&args, &a, 10)) goto usage;
    if (equal(cmd, "dump")) {
        if ((!end(args) && !number(&args, &v, 10)) || !end(args) ||
            a >= 32768 || v > 1024 || v > 32768 - a) goto usage;
        for (uint32_t i = 0; i < v; i++) {
            if (!(i & 15)) { hex(a + i, 8); putc_uart(':'); }
            putc_uart(' '); hex(*(volatile uint8_t *)(uintptr_t)(a + i), 2);
            if ((i & 15) == 15 || i == v - 1) puts_uart("\r\n");
        }
        return;
    }
    if ((a & 3) || !mapped(a)) goto usage;
    if (equal(cmd, "poke")) {
        if (!number(&args, &v, 10) || !end(args)) goto usage;
        REG(a) = v;
    } else if (!end(args)) goto usage;
    hex(a, 8); puts_uart(": "); hex(REG(a), 8); puts_uart("\r\n"); return;
usage:
    puts_uart("memory: aligned mapped word, or dump RAM [len <= 1024]\r\n");
}
static __attribute__((noinline)) uintptr_t test_ram(uintptr_t lo, uintptr_t hi) {
    for (unsigned pass = 0; pass < 4; pass++) {
        uint32_t mask = pass == 0 ? 0 : pass == 1 ? 0xffffffffu : pass == 2 ? 0x55555555u : 0xaaaaaaaau;
        for (uintptr_t a = lo; a < hi; a += 4) REG(a) = (uint32_t)a ^ mask;
        for (uintptr_t a = lo; a < hi; a += 4) if (REG(a) != ((uint32_t)a ^ mask)) return a;
    }
    return 0;
}
static void memtest(void) {
    uintptr_t lo = (uintptr_t)_bss_end, hi = (stack() - 1024) & ~3u;
    uint32_t start = cycle(); uintptr_t bad = test_ram(lo, hi); uint32_t elapsed = cycle() - start;
    puts_uart("memtest [0x"); hex(lo, 8); puts_uart(", 0x"); hex(hi, 8); puts_uart("): ");
    if (bad) { puts_uart("FAIL at "); hex(bad, 8); } else puts_uart("OK");
    puts_uart(", "); spent(elapsed);
}
static void uptime(void) {
    // High-low-high read keeps the 64-bit snapshot consistent across rollovers.
    uint32_t h, l, again;
    do {
        h = CYCLE_HI;
        l = CYCLE_LO;
        again = CYCLE_HI;
    } while (h != again);
    // Long division in base 2 avoids requiring a 64-bit compiler runtime.
    uint64_t ticks = ((uint64_t)h << 32) | l;
    uint64_t ms = 0; uint32_t rem = 0;
    for (int bit = 63; bit >= 0; bit--) {
        rem = (rem << 1) | ((ticks >> bit) & 1);
        if (rem >= CLOCK / 1000) { rem -= CLOCK / 1000; ms |= (uint64_t)1 << bit; }
    }
    // Seconds wrap after 49 days; the millisecond conversion uses all 64 bits.
    uint32_t seconds = 0; rem = 0;
    for (int bit = 63; bit >= 0; bit--) {
        rem = (rem << 1) | ((ms >> bit) & 1);
        if (rem >= 1000) { rem -= 1000; if (bit < 32) seconds |= 1u << bit; }
    }
    decimal(seconds); putc_uart('.'); putc_uart('0' + rem / 100);
    putc_uart('0' + rem / 10 % 10); putc_uart('0' + rem % 10); puts_uart(" s\r\n");
}
static void reboot(void) {
    // Compact unread retained input, then drain the finite hardware queue.
    uint32_t n = pending.count - pending.next;
    for (uint32_t i = 0; i < n; i++) pending.bytes[i] = pending.bytes[pending.next + i];
    pending.next = 0;
    uint32_t quiet = cycle();
    for (;;) {
        uint32_t status = UART_STATUS;
        if ((status & 2) && n < sizeof(pending.bytes)) {
            pending.bytes[n++] = UART_DATA; quiet = cycle();
        }
        if (status & 8) quiet = cycle();
        if (!(status & 3) && cycle() - quiet > 4340) break;
    }
    pending.count = n;
    RESET = 0x5eb007;
    for (;;) {}
}
static void command(char *s) {
    while (*s == ' ' || *s == '\t') s++;
    char *args = s;
    while (*args && *args != ' ' && *args != '\t') args++;
    if (*args) *args++ = 0;
    if (!*s) return;
    if (equal(s, "help")) puts_uart(
        "help                 list commands\r\n"
        "info                 CPU and firmware details\r\n"
        "led [hex]            read/set eight LEDs\r\n"
        "buttons              debounced button levels\r\n"
        "knight [n]           LED sweeps (default 3)\r\n"
        "primes [n]           sieve below n (default 10000)\r\n"
        "mandel               64x24 fixed-point Mandelbrot\r\n"
        "calc <expr>          signed integer + - * / % ()\r\n"
        "peek <addr>          read a mapped word\r\n"
        "poke <addr> <value>  write a mapped word\r\n"
        "dump <addr> [len]    RAM bytes (default 64)\r\n"
        "memtest              test free RAM, destroys contents\r\n"
        "uptime               seconds.milliseconds since reset\r\n"
        "reboot               reset CPU and peripherals\r\n");
    else if (equal(s, "info")) {
        puts_uart("CPU: SERV\r\nISA: RV32I\r\nclock: 25000000 Hz\r\nRAM: 32768 bytes\r\nfirmware: ");
        decimal((uintptr_t)_image_end); puts_uart(" bytes\r\nstack pointer: 0x"); hex(stack(), 8);
        puts_uart("\r\nbuild: " BUILD_DATE "\r\n");
    } else if (equal(s, "led")) {
        uint32_t n;
        if (!end(args)) {
            const char *p = args;
            if (!number(&p, &n, 16) || !end(p) || n > 255) { puts_uart("led: hex 00..ff\r\n"); return; }
            LEDS = n;
        }
        puts_uart("led: "); hex(LEDS, 2); puts_uart("\r\n");
    } else if (equal(s, "buttons")) buttons();
    else if (equal(s, "knight")) knight(args);
    else if (equal(s, "primes")) primes(args);
    else if (equal(s, "mandel")) mandel();
    else if (equal(s, "calc")) {
        expr = args; expr_error = 0; depth = 0;
        uint32_t result = sum();
        if (!end(expr) && !expr_error) expr_error = "syntax error";
        if (expr_error) puts_uart(expr_error); else signed_decimal((int32_t)result);
        puts_uart("\r\n");
    } else if (equal(s, "peek") || equal(s, "poke") || equal(s, "dump")) memory(s, args);
    else if (equal(s, "memtest")) memtest();
    else if (equal(s, "uptime")) uptime();
    else if (equal(s, "reboot")) reboot();
    else { puts_uart("unknown command: "); puts_uart(s); puts_uart(" (try help)\r\n"); }
}
int main(void) {
    banner();
    unsigned length = 0; int last_cr = 0;
    for (;;) {
        uint32_t events = EVENTS;
        for (unsigned b = 0; b < 7; b++) if (events & (1u << b)) {
            puts_uart("\r\n[button "); puts_uart(names[b]); puts_uart("]\r\n> ");
            for (unsigned i = 0; i < length; i++) putc_uart(line[i]);
        }
        int c;
        if (pending.next < pending.count) c = pending.bytes[pending.next++];
        else if (UART_STATUS & 2) c = UART_DATA & 255;
        else continue;
        if (c == '\n' && last_cr) { last_cr = 0; continue; }
        last_cr = c == '\r';
        if (c == '\r' || c == '\n') {
            puts_uart("\r\n"); line[length] = 0; command(line); length = 0; puts_uart("> ");
        } else if (c == 8 || c == 127) {
            if (length) { length--; puts_uart("\b \b"); }
        } else if (c >= 32 && c <= 126 && length < sizeof(line) - 1) {
            line[length++] = (char)c; putc_uart((char)c);
        }
    }
}
