# riscv_oled: Kinetic seed lantern

A soft CPU drives a real peripheral from C. PicoRV32 draws an original faceted
lantern above a perspective garden floor on the ULX3S's 96 × 64 SSD1331 OLED.
The little four-digit counter, staggered ribs, asymmetric crown, stars and palette
were made for this demo. No assets or code were downloaded.

![Frame F0020 decoded from the SPI pads, enlarged four times](frame.png)

This is [riscv_shell](../riscv_shell/README.md)'s computer: PicoRV32 RV32IMC,
32 KB RAM in sixteen EBR byte lanes loaded from `fw_*.hex`, buffered UART RX,
LEDs, synchronised buttons with 10 ms debounce, and software reset. It runs
directly from 25 MHz without a PLL. The PicoRV32 source is the same symlink;
the firmware retains the shell's `start.S`, `link.ld`, Homebrew LLVM RV32IMC
clang/lld build, and sixteen memory images. It never reads DIP switches.

## Driver split and double buffering

[firmware/main.c](firmware/main.c) controls RES#, sends the common 37 init bytes
through a byte command register, draws every pixel, and decides when to submit a
frame. Init includes `A0 72`: RAM column 0, row 0 is the upright panel's top left.
RES# stays low for at least 25,000 clocks (1 ms), then high for another 1 ms before
init. The driver polls busy before sending each init byte.

[oled_dma.v](oled_dma.v) owns SPI mode 0 timing, byte order, address windows,
bulk transfer and CRC. A submission latches a buffer number and emits
`15 00 5F 75 00 3F`, then exactly 12,288 data bytes, high byte of each RGB565
pixel first. Both address-window commands restart the SSD1331's fill origin.
CS# stays low throughout the six commands and complete data fill; D/C changes
on a falling SPI edge before the first data bit. Individual init commands are
separate CS# transactions.

Two independent 12 KB EBR arrays have CPU byte-masked word write ports and
synchronous 32-bit streamer read ports. The streamer prefetches the word for
the next byte during the current byte's 32 clocks. It reads only the submitted
buffer, only while a frame is in flight, and never reads beyond its last word.
CPU stores to that buffer are rejected and flagged; stores to the other buffer
continue normally. Thus even halfword drawing stores cannot collide with a
streamer read of the same physical EBR row (see
[EBR execution](../../../docs/EBR_EXECUTION.md)). RAM contents survive reset;
the CPU completely redraws each buffer before its first submission.

The software pipeline primes buffer 0, then draws frame 1 in buffer 1 while
frame 0 transfers. It waits for frame 0, captures its CRC, starts buffer 1,
and prints frame 0's report while frame 1 transfers. Next it draws frame 2
in buffer 0. Neither printing nor waiting is included in the draw measurement.
This overlap is the hardware/software split of a small DMA driver.

## Memory map

Registers are volatile 32-bit accesses. The shell's addresses and semantics
are unchanged. No interrupts, cache, operating system or C library is involved.

| Address | Access | Meaning |
|---|---|---|
| `00000000–00007FFF` | R/W | 32 KB firmware, data and stack RAM |
| `02000000` | R/W | UART RX FIFO pop / TX byte; poll status before TX |
| `02000004` | R | bit 0 TX busy, 1 RX available, 2 sticky RX overflow, 3 RX receiving |
| `03000000` | R/W | eight LEDs, low byte |
| `04000000` | R | debounced button levels, PWR/FIRE1/FIRE2/UP/DOWN/LEFT/RIGHT in bits 0–6 |
| `04000004` | R | press events; reading clears them, retaining simultaneous new presses |
| `05000000` | W | full-word `005EB007` reboots after the last UART stop bit |
| `06000000–06002FFF` | W | buffer 0: 6,144 native little-endian RGB565 halfwords |
| `06004000–06006FFF` | W | buffer 1, same layout; 16 KB address stride |
| `06008000` | W | command byte in bits 7–0, accepted only when idle |
| `06008004` | R/W | bit 0 RES# level; write bit 1 submits a frame, bit 2 selects buffer; write bit 3 clears rejection flag; read returns RES# |
| `06008008` | R | bit 0 busy, bit 1 latched buffer, bit 2 last operation was a frame, bit 3 sticky protected-store rejection |
| `0600800C` | R | CRC32 of the last fully sent frame; unchanged during the next transfer |
| `06008010` | R | completed-frame counter, reset to zero by reboot |

Frame buffers accept byte, halfword and word write masks; drawing uses word
stores for pairs of background pixels and halfword stores for lines and glyphs.
They are write-only CPU windows (loads return zero). Holes return zero and ignore
writes. Command/submission writes while busy are ignored; software polls first.
Writing control also updates RES#, so every frame submission keeps bit 0 high.
The streamer contains no panel-init ROM: all 37 init bytes really come from C.

CRC-32/ISO-HDLC covers only the received frame data bytes, before any colour
conversion, exactly as the [OLED model](../../../docs/PERIPHERALS_OLED.md).
It starts at FFFFFFFF, consumes each byte LSB first using reflected polynomial
EDB88320, and complements at the end. Eight fabric clocks do this while the
same byte shifts MSB first on SPI. This is Python/zlib `crc32`; neither C nor
the CPU computes a substitute CRC.

## Drawing and timing

The lantern has three staggered hexagonal ribs and an off-axis crown. A hand
written 64-entry signed Q8 sine table supplies yaw and a slow tilt. Rotation
products shift right eight bits; the tilt cosine uses `256 - tilt²/512`.
Perspective projection divides integer coordinates by `110 + rotated_z`, which
stays positive for every vertex. Bresenham lines connect ribs and the crown;
all stores clip to 96 × 64. A four-digit 3 × 5 hex font and a phase ruler finish
the picture. Everything uses bounded integer arithmetic, without floating point.

Each `T` is the exact difference between two `rdcycle` reads around the drawing
call, including call/return and measurement overhead. It belongs to the **same
completed frame** as `F` and `C`, although the following frame has already started
transferring when the line prints. There is no dummy first CRC or shifted numbering.
The first 61 checked frames take **138,159–141,140 clocks** to draw, **5.52636–5.64560 ms**.

SPI uses four 25 MHz clocks per bit: **6.25 MHz**, with two clocks high and two
low. There are no gaps between bytes of a frame. The data costs **393,216 clocks**;
the six window bytes add 192, for **393,408 clocks (15.73632 ms)** per submission.
The independent decoder measures 393,212 clocks from first to last data rising
edge (393,213 inclusive); the final falling edge is two clocks later. In the
checked run consecutive fills are **393,437–393,452 clocks** apart, including
the CPU's status-poll and resubmit overhead: **15.73748–15.73808 ms, about 63.54 fps**.
Drawing uses at most 35.88% of a transfer; UART reports overlap the next transfer.

An excerpt of [expected/uart.txt](expected/uart.txt), with CRLF endings:

```text
RESTART
F0000 C72336258 T140213
F0001 CF0B1531D T139282
F0002 CC577AE0D T139969
```

## Controls, reset and LEDs

- UART `r` or FIRE1 reboots the computer through `05000000`.
- LEFT held selects negative rotation; RIGHT selects positive rotation. The
  selected direction continues on release, starting after the already drawn frame.
- LED0–LED7 show the completed frame number's low eight bits in binary.
- PWR, FIRE2, UP, DOWN and the DIP switches are unused by the firmware.

UART is 115200 8N1 (217 clocks/bit) on L4 out / M1 in. OLED pins are CLK P4,
MOSI P3, D/C P1, RES# P2 and CS# N2, using the gallery's constraints. Buttons
retain the shell's two synchronizers, 250,000-clock debounce and event registers.
The shell's 511-byte-capacity EBR RX FIFO is retained unchanged.

Soft reset preserves physical RAM. `start.S` clears `.bss`; every drawing and
animation variable is initialized explicitly on entry. There are no initialized
mutable `.data` statics to restore. A single `.persist` word carries a deliberate
reboot cookie, cleared immediately on entry. Only that cookie causes `RESTART`
to print as the first output after reboot; power-on has no marker. The current
UART line finishes before the firmware requests reset; hardware waits for its
last stop bit. An in-flight next OLED fill may be aborted, then RES# and the
full init sequence run again. Frame 0 and every draw cycle count repeat exactly.

## Reproducible RTL checks

```sh
nice -n 15 sh examples/gallery/riscv_oled/firmware/build.sh
nice -n 15 sh examples/gallery/riscv_oled/sim/run.sh
```

[sim/run.sh](sim/run.sh) compiles Verilator with **`make -j 1`** and works in
paths containing spaces. All generated C++, firmware binaries, logs, raw RGB565
and timing CSVs go under `target/gallery/riscv_oled/`. The source firmware,
sixteen reproducible `fw_*.hex` files, golden UART and PNG stay in this folder.
[ecpbram.txt](ecpbram.txt) is the shell's sixteen-image list for the existing
placeholder-and-swap build flow.

[sim/uart_tb.cpp](sim/uart_tb.cpp) drives released buttons and UART `r` at clock
1,200,000, after power-on frames have started. Independent pad decoders check
UART framing, exactly one marker, both 37-byte init sequences, every six-byte
window, 12,288-byte fills, every SPI bit/pulse and transaction duration, frame
numbers, and each fill's zlib CRC against the printed `C`. They never inspect the
CPU bus, frame memory or hardware CRC. A completed fill without a complete UART
report at the end fails loudly. A reset-aborted partial fill is discarded.

The golden **26,000,000-clock** window contains three power-on frame lines and
**61 checked lines, F0000–F003C**, after one `RESTART`: 1,609 bytes in total,
**1,534 bytes from the marker**. The last fill ends at clock **25,640,119** and
its UART LF is sampled at **25,695,051**, leaving **304,949 clocks** to the
chosen boundary. An extended RTL run places the next completed fill at
**26,033,558**, **33,558 clocks after** that boundary. `margin_cycles: 0` preserves
that boundary and keeps the model's last completed CRC among printed CRCs.

The script also runs check.py's immediate-`r` schedule: 63 checked frames, last
fill at 25,640,106 and complete report at 25,695,036, leaving 304,964 clocks.
A debounced FIRE1 run produces the same 61-frame checked transcript as late `r`.
Both restart paths have byte-identical checked prefixes. A full rerun regenerates
byte-identical `expected/uart.txt` and `frame.png`.

Frame F0020 is saved from the decoded SPI stream. [sim/png.py](sim/png.py)
uses Python's standard library (`struct` and `zlib`) to expand RGB565 channels
by repeating their high bits, matching the model's RGB888 conversion, and
enlarges the picture ×4 without interpolation. `check.json` requires only
`peripherals: ["oled"]`; the 8 s board listen is over seven times the 1.04 s
checked board-time window.

## Resources and window script

Yosys / nextpnr-ecp5 on this Mac, LFE5U-85F CABGA381, seed 1:

- **4 904 TRELLIS_COMB (5%)** and **1 379 TRELLIS_FF**;
- **29 DP16KD (13%)**: sixteen firmware RAM EBRs, one UART RX FIFO and twelve
  for the two 12 KB frame buffers;
- **4 MULT18X18D**, **32 TRELLIS_RAMW** and **23 TRELLIS_IO**;
- routed **Fmax 52.81 MHz**, passing the 25 MHz requirement.

No PLL is instantiated. [ecp5.toml](ecp5.toml) lists every synthesized `.v` and
[top.lpf](top.lpf). [gui.script](gui.script) is a 90 s run, UART `r`, drawing,
then repeated LEFT/RIGHT holds.

## Checks

Records are in [HR03](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md);
board checks ran on 2026-11-17, the emulator rerun and window on 2026-11-18.

**RTL:** PASS: firmware and single-job Verilator, late/early UART restart,
FIRE1, SPI timing, every frame CRC and byte-identical rerun;
[sim/run.sh](sim/run.sh), [expected/uart.txt](expected/uart.txt).

**Emulator:** the board's own configuration (`54f65b7f`),
[riscv_oled.config.gz](../../../docs/inventory/LFE5U-85F/hr03_gallery/riscv_oled.config.gz),
passed on rserv without rebuilding: **1 534 bytes from RESTART**, draw cycle counts
included. Fast mode `d377f3e`: **26 000 000 cycles in 230.385 s, 0.11 MHz**
(earlier: 269.054 s, 0.10 MHz). OLED: 64 frames, zero errors, last completed CRC
`36E1F3DE` among the printed CRCs;
[emulator_board_configs.log](../../../docs/inventory/LFE5U-85F/hr03_gallery/emulator_board_configs.log).

**Board:** **3 of 3 SRAM loads PASS**, each on 1 534 bytes with one RESTART;
identical over **12 684 bytes**, **507 frames in 8 s, 63.4 fps**.
`top.bit` SHA-256:
`c9793c55ad76e9ee1159e4c0477baa702af9f6c3b296aca8b099ac3a05015266`.
Records: [observations.log](../../../docs/inventory/LFE5U-85F/hr03_gallery/observations.log),
[SHA256SUMS](../../../docs/inventory/LFE5U-85F/hr03_gallery/SHA256SUMS),
[riscv_oled.board_uart_1.txt.gz](../../../docs/inventory/LFE5U-85F/hr03_gallery/riscv_oled.board_uart_1.txt.gz).
This checks the board's UART; the physical panel was not observed.

**Window:** 90 s under `out/screen.lock`, display awake and unlocked at both ends:

- **5 383 window frames**, median 16.7 ms, p95 17.4 ms;
- **65 218 465 cycles** (2.609 s of board time), **0.74 MHz**;
- **2 059 LED drawings, 0 wrong**; LEFT ×3, RIGHT ×3;
- console matched **1 066 bytes from RESTART**, F0000–F0029, until the first
  scripted LEFT at cycle 17 811 361 changed rotation from F002A;
- **2 017 OLED pictures**: 18 before the first completed frame; all **1 999**
  others carry data CRCs printed by the design.

Picture evidence: [riscv_oled.window_oled.txt](../../../docs/inventory/LFE5U-85F/hr03_gallery/riscv_oled.window_oled.txt).
