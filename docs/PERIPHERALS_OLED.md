# ULX3S OLED

The OLED model is in [src/peripherals/oled.rs](../src/peripherals/oled.rs). It models the
SSD1331 on the ULX3S: 96 by 64 pixels, with 16-bit RAM and a four-wire SPI input. It follows
Solomon Systech's [SSD1331 revision 1.2, November 2007](https://akizukidenshi.com/goodsaffix/ssd1331.pdf),
particularly sections 7.4–7.5, the command tables in section 8, and section 9. This is a digital
picture of RAM and the output mapping. It does not attempt to predict panel brightness.

## Sampling and snapshots

Create `Oled::new()`, then call `feed(time_ps, OledPins { csn, dc, clk, mosi, resn })` after
every possible pin change. Values are 0, 1, 2 for X, and 3 for Z. Identical samples do nothing.
The time is in picoseconds, in order; pulse widths are not checked. The model is independent
of the event kernel and fast mode. The package balls are constants in this module until the
board description gains these pins: CLK P4, MOSI P3, D/C P1, RES# P2, CS# N2.

With CS# low, each SCLK rising edge takes one MOSI bit, most significant bit first. The D/C
level on the eighth edge selects a command/argument byte or a data byte. Arguments use D/C
low. Raising CS# discards an incomplete SPI byte, but retains the command argument list and
any complete first byte of a multi-byte pixel. An unknown selected pin discards the partial
SPI byte and partial pixel, increments the error counter, and invalidates the current frame
CRC until the next address wrap. Unknown CS# during a selected transfer is treated the same
way. Repeated samples of the same unknown levels do not add errors.

RES# low resets the registers, address pointer, SPI shift register and command parser. RAM
survives. RAM is initially zero; display output initially is off. Diagnostic counters, the last
completed frame CRC and frame count survive controller reset. Register defaults come from
section 7.4 and the command table, including remap 40h, contrast 80h, master current 0Fh,
phase 74h, clock D0h, power save 1Ah, and precharge/VCOMH 3Eh.

`status()` renders lazily and returns `OledStatus`. Its image is an `Arc<Vec<u8>>` of
96 × 64 × 3 RGB888 bytes in row order, with red first. Cloning a snapshot shares the image.
An old snapshot remains unchanged after further writes. A version changes when RAM or a
picture register may have changed. The other fields give display state, completed frames,
last frame CRC, command opcodes, data bytes, errors, unknown commands and a short note.
`registers()` exposes the stored digital settings. `frames()` and `last_frame_crc32()` avoid
rendering when polling for completed frames.

## RAM and commands

The address window is inclusive. Data advances horizontally when remap A[0] is zero and
vertically when it is one, wrapping inside the window. Format 1 uses two big-endian RGB565
bytes per pixel. Format 2 uses three bytes: red in bits 5:1, green in 5:0, blue in 5:1, as in
table 9. The 256-colour RRRGGGBB input uses figure 11's expansion into RAM: vacant low bits
are copies of the channel's highest bit, rather than cyclic bit replication. Reserved format
11 is an error. RGB565 is expanded to RGB888 by repeating the high bits in the low bits.
A[2] exchanges red and blue at display output.

The model implements column/row windows, remap, start line, offset, multiplex, normal,
entire-on, entire-off and inverse display, and display on/off. Contrast A/B/C, master current,
master configuration, power save, phase, clock divider, second precharge A/B/C, precharge
level and VCOMH are stored without changing RGB values. Sleep output is black even if
entire-on mode is selected.

Draw line and draw rectangle use the supplied six-bit channel fields; rectangles have an
outline and an optional interior controlled by 26h A[0]. Copy takes a source rectangle and a
new origin; 26h A[4] complements the copied colours. Clear writes zero. Dim window modifies
RAM using table 15: gray levels below 16 stay unchanged, while levels 16–63 become their
integer value divided by four. Repeating dim on the same content therefore does nothing.
The last dim rectangle is retained for inspection. Graphic commands do not count as data
frames or add bytes to a frame CRC.

Scroll setup and enable/disable are stored. With scrolling enabled the status says
`scroll not rendered`. Unknown opcodes increment a counter and are ignored. Known but
unimplemented ABh, B8h and FDh consume 5, 32 and 1 arguments respectively before reporting
an unsupported command, so their arguments cannot be mistaken for opcodes. Other unknown
commands have no recoverable argument length. NOPs BCh, BDh and E3h are accepted. A data
byte interrupting a command argument list reports an error, discards the list and writes data.

A frame is one data fill of the current window, completed when the last pixel advances back
to its origin. CRC-32/ISO-HDLC covers only that frame's received data bytes, including all
pixel bytes, before any colour conversion. The reflected polynomial is EDB88320h, with
initial and final XOR FFFFFFFFh, matching zlib's crc32. The implementation is private to the
module and adds no dependency. Partial windows are frames too; drawing commands are not.

## Orientation and assumptions

The orientation convention is that common init `A0 72` puts RAM column 0, row 0 at the
upright panel's top-left. Clearing A[1] mirrors columns. Clearing A[4] flips rows at 64 MUX.
Changing COM split A[5] interleaves the rows relative to this wiring. A[3] exchanges the two
COM pad banks. For an explicit description, the baseline row y has logical COM 63−y and
physical pad `COM/2 + (COM%2)*32`. The selected remap decodes that pad using either split
or sequential numbering, after the bank exchange. Subtract offset modulo 64, blank scan
positions beyond the multiplex ratio, reverse the active scan if A[4] is set, and add start
line modulo 64. This also defines the model for unusual combinations of remap and MUX.

The following choices are assumptions, rather than measurements of this panel:

- The fixed panel wiring is inferred from the required 72h convention and the datasheet's
  COM pad arrangement; the alternative maps and reduced MUX have not been checked on hardware.
- A remap command restarts at the current window origin, dropping any partial pixel and CRC.
  Each address-window command also starts a fresh fill at the origin of both axes.
- A line uses integer Bresenham with inclusive endpoints; the datasheet does not specify
  which pixel wins when the ideal line is exactly between two pixels.
- Copy snapshots its source before writing, including overlaps. Destinations beyond RAM
  are clipped. Invalid source rectangles and invalid address windows are rejected as errors.
- Dim treats five-bit red/blue as six-bit gray levels with a zero low bit, then truncates
  the result back to five bits. This digital quantisation has not been measured on hardware.
- Scroll setup has no specified reset value in the table, so the stored reset bytes are zero.

## Reference design and checks

[oled_pattern/top.v](../examples/ulx3s/oled_pattern/top.v) uses only plain fabric logic on
25 MHz, with no PLL, block RAM or DSP. It holds RES# low for 25,000 clocks (1 ms), sends the
common 37-byte init from a small case ROM, and sends one byte per CS# transfer. SCLK periods
within a byte are four board clocks (6.25 MHz); there are idle gaps between bytes. It then
repeats full-window writes. A gradient moves with the 16-bit frame number and a white 16×16
box moves over it. LEDs show the low eight frame bits. The design folds each data byte into
a parallel CRC function and prints `O%04X C%08X\r\n` at 115200 8N1 after each frame, beginning
at frame 0000. UART transmission finishes before the next picture starts.

For pixel (x,y) in frame f, the background RGB565 fields are `(x+f)&31`, `(y+f)&63`, and
`(x^y^f)&31`. The white box covers `(f&63)..(f&63)+15` in x and `(f&31)..(f&31)+15` in y.
This formula is independently evaluated in Rust for the image comparison.

Build and check on the Mac:

```sh
examples/ulx3s/build.sh examples/ulx3s/oled_pattern target/reference/oled_pattern
cargo build --release
cargo test --release --lib peripherals
RUST_TEST_NOCAPTURE=1 cargo test --release --test oled -- --ignored --test-threads=1
```

The unit suite passed all seven tests: full common init and an exact 96×64 image with a
zlib-checked CRC, 256-colour and format 2, both increment directions and window wrap,
remap/display modes, graphics and dim, interrupted SPI, reset, unknown pins and unsupported
command synchronisation. The integration suite passed its one test. It rebuilds the design,
compiles all five OLED balls and L4, retries fast-mode startup through event-kernel cycles
only on `StartsUnknown`, and samples both clock edges with `run_edges_sampled` at 20,000 ps
per edge. This build needed zero warm-start cycles. L4 is decoded with `uart::Receiver`.

Measured on 2026-11-16, model and UART agreed for all three frames, with exact RGB888 images:

| Frame | Model CRC | UART CRC | Wall seconds per frame |
|---|---|---|---|
| 0000 | F558384E | F558384E | 0.039 |
| 0001 | 00E9D106 | 00E9D106 | 0.043 |
| 0002 | 45901B6E | 45901B6E | 0.032 |

These times cover fast execution, sampling and model feeding, including rendering/checking
at frame boundaries. The first includes reset and init; later intervals include the preceding
UART report. They exclude synthesis, model loading and C compilation. The model runs at
UI cadence only when a snapshot is requested; SPI capture itself allocates no image.

nextpnr, seed 1, reported these placement and routed timing lines:

```text
Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 110.79 MHz (PASS at 25.00 MHz)
Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 126.20 MHz (PASS at 25.00 MHz)
```

Resource counts were 624 LUT4s, 726 TRELLIS_COMB, 186 TRELLIS_FF, 15 TRELLIS_IO and one
DCCA; DP16KD, MULT18X18D, ALU54B, EHXPLLL and IOLOGIC were all zero. No emulator feature was
refused by this design.

## Limits

Scrolling movement, scan timing, SPI timing limits and accelerator execution delays are not
modelled. Commands execute as soon as their last argument is received. The model does not
simulate power, supply sequencing, analogue current, luminance, custom gray-scale tables,
dim display mode or command lock.

The live runner and the window show the panel (docs/PERIPHERALS.md, *In the window*): an OLED
window, and the picture on the 3D board's module.

On the board ([HR04](inventory/LFE5U-85F/hr04_hdmi_oled/README.md)), `oled_pattern` printed the
emulator's line for every frame it reported: frames 3–219, three loads, 651 lines. That checks the
design's SPI sequence and CRC on the board. The panel itself was not observed.
