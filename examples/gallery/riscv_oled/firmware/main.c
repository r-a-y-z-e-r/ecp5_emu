// Kinetic seed lantern: all geometry, pixels and panel policy live in C.
#include <stdint.h>
#include <stddef.h>
#define REG(a) (*(volatile uint32_t *)(uintptr_t)(a))
#define UART_DATA REG(0x02000000)
#define UART_STATUS REG(0x02000004)
#define LEDS REG(0x03000000)
#define BUTTONS REG(0x04000000)
#define EVENTS REG(0x04000004)
#define RESET REG(0x05000000)
#define OLED_COMMAND REG(0x06008000)
#define OLED_CONTROL REG(0x06008004)
#define OLED_STATUS REG(0x06008008)
#define OLED_CRC REG(0x0600800c)

// This is the ONLY retained mutable state. start.S clears .bss on every boot;
// no mutable .data initializer is used (physical RAM survives soft reset).
static volatile uint32_t reboot_cookie __attribute__((section(".persist"))) = 0;
static const int16_t sine[64] = {
    0,25,50,74,98,121,142,162,181,198,213,226,237,245,251,255,
    256,255,251,245,237,226,213,198,181,162,142,121,98,74,50,25,
    0,-25,-50,-74,-98,-121,-142,-162,-181,-198,-213,-226,-237,-245,-251,-255,
    -256,-255,-251,-245,-237,-226,-213,-198,-181,-162,-142,-121,-98,-74,-50,-25
};
// Original 3x5 hexadecimal glyphs, top row in bits 14..12.
static const uint16_t glyph[16] = {
    0x7b6f,0x2492,0x73e7,0x73cf,0x5bc9,0x79cf,0x79ef,0x7249,
    0x7bef,0x7bcf,0x7bed,0x6bae,0x7927,0x6b6e,0x79e7,0x79e4
};
static const uint8_t init[] = {
    0xae,0xa0,0x72,0xa1,0,0xa2,0,0xa4,0xa8,0x3f,0xad,0x8e,
    0xb0,0x0b,0xb1,0x31,0xb3,0xf0,0x8a,0x64,0x8b,0x78,0x8c,0x64,
    0xbb,0x3a,0xbe,0x3e,0x87,6,0x81,0x91,0x82,0x50,0x83,0x7d,0xaf
};
static inline uint32_t cycle(void) {
    uint32_t n;
    __asm__ volatile("rdcycle %0" : "=r"(n) :: "memory");
    return n;
}
static void delay(uint32_t clocks) { uint32_t t=cycle(); while (cycle()-t<clocks) {} }
static void putc_uart(char c) {
    while (UART_STATUS & 1) {}
    UART_DATA=(uint8_t)c;
}
static void puts_uart(const char *s) { while (*s) putc_uart(*s++); }
static void hex(uint32_t n, unsigned digits) {
    while (digits) putc_uart("0123456789ABCDEF"[(n >> (--digits*4)) & 15]);
}
static void decimal(uint32_t n) {
    char b[10]; unsigned k=0;
    do { b[k++]='0'+n%10; n/=10; } while (n);
    while (k) putc_uart(b[--k]);
}
static void wait_oled(void) { while (OLED_STATUS & 1) {} }
static void panel_init(void) {
    OLED_CONTROL=0; delay(25000);  // RES# low for at least 1 ms
    OLED_CONTROL=1; delay(25000);
    for (unsigned i=0;i<sizeof(init);i++) { wait_oled(); OLED_COMMAND=init[i]; }
    wait_oled();
}
static void pixel(volatile uint16_t *fb, int x, int y, uint16_t c) {
    if ((unsigned)x<96 && (unsigned)y<64) fb[y*96+x]=c;
}
static void line(volatile uint16_t *fb, int x, int y, int x1, int y1, uint16_t c) {
    int dx=x1>x ? x1-x : x-x1, sx=x<x1 ? 1 : -1;
    int dy=y1>y ? y-y1 : y1-y, sy=y<y1 ? 1 : -1, e=dx+dy;
    for (;;) {
        pixel(fb,x,y,c);
        if (x==x1 && y==y1) break;
        int twice=e*2;
        if (twice>=dy) { e+=dy; x+=sx; }
        if (twice<=dx) { e+=dx; y+=sy; }
    }
}
static __attribute__((noinline)) void draw(unsigned buffer, unsigned frame, unsigned angle) {
    volatile uint16_t *fb=(volatile uint16_t *)(uintptr_t)(0x06000000+buffer*0x4000);
    // Two pixels per word for the background, halfword stores for the lines.
    volatile uint32_t *words=(volatile uint32_t *)fb;
    for (unsigned y=0;y<64;y++) {
        uint32_t c=0x0806+((y>>4)<<5);
        uint32_t pair=c|(c<<16);
        for (unsigned x=0;x<48;x++) words[y*48+x]=pair;
    }
    static const uint8_t stars[][2]={{8,14},{23,25},{78,17},{87,32},{11,39},{69,9}};
    for (unsigned i=0;i<sizeof(stars)/sizeof(stars[0]);i++) {
        int x=stars[i][0], y=stars[i][1];
        pixel(fb,x,y,0x7c95); pixel(fb,x-1,y,0x320e); pixel(fb,x,y+1,0x320e);
    }
    for (int x=-48;x<=144;x+=24) line(fb,48,44,x,63,0x196b);
    for (int y=49;y<64;y+=6) line(fb,0,y,95,y,0x196b);
    line(fb,39,58,57,58,0x52b0); line(fb,44,60,52,60,0x320d);

    // Three staggered hexagonal ribs, plus an off-axis crown. Coordinates
    // are integer model units; sine/cosine and both rotations use signed Q8.
    int px[19], py[19];
    int s=sine[angle&63], c=sine[(angle+16)&63];
    int tilt=sine[(angle+8)&63]/4, tc=256-(tilt*tilt>>9);
    for (unsigned i=0;i<19;i++) {
        int x,y,z;
        if (i==18) { x=-4; y=-25; z=2; }
        else {
            unsigned ring=i/6, a=((i%6)*64/6+ring*5)&63;
            int radius=ring==0 ? 15 : ring==1 ? 24 : 10;
            x=(sine[(a+16)&63]*radius)>>8;
            z=(sine[a]*radius)>>8;
            y=ring==0 ? -14 : ring==1 ? 1 : 17;
        }
        int rx=(x*c-z*s)>>8, rz=(x*s+z*c)>>8;
        int ry=(y*tc-rz*tilt)>>8;
        rz=(y*tilt+rz*tc)>>8;
        px[i]=48+rx*100/(110+rz); py[i]=32+ry*100/(110+rz);
    }
    static const uint16_t ink[3]={0xff50,0x4ef9,0xfae9};
    for (unsigned ring=0;ring<3;ring++) for (unsigned j=0;j<6;j++) {
        unsigned a=ring*6+j, b=ring*6+(j+1)%6;
        line(fb,px[a],py[a],px[b],py[b],ink[ring]);
        if (ring<2) line(fb,px[a],py[a],px[a+6],py[a+6],0x8c57);
        if (!ring) line(fb,px[a],py[a],px[18],py[18],0xcdd1);
    }
    // Four hex digits, then a 16-step phase ruler at the other end of the bar.
    for (unsigned digit=0;digit<4;digit++) {
        uint16_t bits=glyph[(frame>>((3-digit)*4))&15];
        for (int y=0;y<5;y++) for (int x=0;x<3;x++)
            if (bits & (1u<<(14-y*3-x))) pixel(fb,3+digit*4+x,2+y,0xded7);
    }
    for (unsigned i=0;i<16;i++) pixel(fb,62+i*2,4,i==(angle>>2) ? 0xff50 : 0x426f);
}
static void controls(int *direction) {
    unsigned events=EVENTS;
    int restart=(events&2)!=0; // FIRE1; PWR is unused
    while (UART_STATUS&2) if ((UART_DATA&255)=='r') restart=1;
    if (restart) {
        reboot_cookie=0x72656274;
        RESET=0x005eb007;  // hardware waits for the final UART stop bit
        for (;;) {}
    }
    unsigned held=BUTTONS;
    if (held&(1u<<5)) *direction=-1;
    if (held&(1u<<6)) *direction=1;
}
int main(void) {
    unsigned restarted=reboot_cookie==0x72656274;
    reboot_cookie=0;
    if (restarted) puts_uart("RESTART\r\n");
    panel_init();
    unsigned frame=0, buffer=0, angle=0;
    int direction=1;
    uint32_t t=cycle(); draw(buffer,frame,angle); uint32_t drawn=cycle()-t;
    OLED_CONTROL=3; // RES# high, send buffer 0
    for (;;) {
        unsigned back=buffer^1, next_angle=(angle+direction)&63;
        t=cycle(); draw(back,frame+1,next_angle); uint32_t next_drawn=cycle()-t;
        wait_oled();
        uint32_t crc=OLED_CRC; // capture BEFORE starting the next transfer
        OLED_CONTROL=3|(back<<2);
        LEDS=frame;
        putc_uart('F'); hex(frame&65535,4); puts_uart(" C"); hex(crc,8);
        puts_uart(" T"); decimal(drawn); puts_uart("\r\n");
        controls(&direction);
        frame++; buffer=back; angle=next_angle; drawn=next_drawn;
    }
}
