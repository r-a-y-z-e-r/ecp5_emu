# video_life: the Game of Life on an HDMI monitor

Life on a monitor: 3 072 cells (a 64 × 48 torus) in block RAM, and one engine that is idle
99% of each frame. Every cell is a 10 × 10 pixel square of a 640 × 480 DVI picture, so the
world fills the whole screen: live cells drawn 9 × 9 on a 1-pixel dark grid, coloured by age —
bright yellow-green when newborn, then green, blue, and violet after a few generations; dead
cells near-black. This demo is the gallery's video showcase.

The lesson is where the time goes. A 640×480@60 frame is 420 000 pixel clocks; the sweep of
[ebr_life](../ebr_life/README.md) computes a whole generation (48 rows × 74 clocks, about
3 600 clocks) into the *other* world, inside the vertical blank, and the worlds swap only
there. So the engine works 0.9% of the frame and a frame never shows half of two generations.
Everything — engine, display, UART — runs in the 25 MHz pixel clock domain; only the TMDS
shift uses the PLL's 125 MHz.

## What the receiver checks

The video pipeline (PLL, TMDS encoders, `ODDRX1F` lanes, 800×525 timing) is
[hdmi_moving](../../ulx3s/hdmi_moving/)'s, reused through symlinks. Like hdmi_moving, the
design feeds every active pixel's RGB into a parallel CRC-32/ISO-HDLC and, after each frame,
prints one line on the UART:

```text
F0000 G0000 P0033 C472719D7
```

frame number, generation, population, and the CRC of the pixels sent in that frame. The
emulator's HDMI receiver decodes the TMDS pads on its own and computes the same CRC of every
frame it receives; the window records `# monitor frame N WxH crc XXXXXXXX`, which must equal
the design's line for frame N. That equality is the demo's proof that the video on the pads
is right. (The CRC function is copied from hdmi_moving's top.v; the receiver's is zlib's
`crc32`.)

## Controls

| Button | UART key | Action |
|---|---|---|
| FIRE1 | `s` | fill the world with random soup (from an LFSR) |
| FIRE2 | `p` | pause / run |
| UP / DOWN | `+` / `-` | faster / slower: one generation every N frames, N ∈ {1, 2, 4, 8, 16}, default 4 (about 15 generations a second) |
| LEFT or RIGHT | `n` | one generation, while paused |
| — | `g` | load the start pattern again |
| — | `r` | restart at the exact power-on state; prints `RESTART`, then restarts at a frame boundary, frame counter included |

Keys wait in a 256-byte FIFO and are acted on in order, one per frame boundary. The LEDs show
the population divided by 16, saturated. The start pattern ([seed.hex](seed.hex), written by
[make_seed.py](make_seed.py), 51 cells) is Gosper's glider gun, two gliders and an R-pentomino,
re-placed for 64 × 48; a world whose population is unchanged for 32 generations is sown again.

## Resources

nextpnr for the LFE5U-85F, seed 1:
- 2 847 LUT4 (TRELLIS_COMB) and 1 296 TRELLIS_FF;
- 13 DP16KD (PDPW16KD in yosys): the two worlds, the two age arrays, the start pattern;
- 32 TRELLIS_RAMW: the UART receive FIFO of io.v, shared with [ebr_life](../ebr_life/);
- 1 EHXPLLL (125 MHz shift + 25 MHz pixel) and 4 SIOLOGIC (the `ODDRX1F` lanes).

yosys.log maps `world0`, `world1`, `age0`, `age1` and `seed` via `$__PDPW16KD_`: the worlds
are block RAM, not LUTs. Fmax is 271 MHz for the 125 MHz shift clock and 101 MHz for the
25 MHz pixel clock (both PASS).

## Build and check

```sh
examples/ulx3s/build.sh examples/gallery/video_life target/gallery/video_life
python3 examples/gallery/check.py video_life --emulator
```

[check.json](check.json) sends `r`: a restart. From the marker on, the output is deterministic
— the frame counter, generation, population, LFSR and speed all restart from fixed values —
so the board, the emulator and the RTL simulation print the same bytes. The expected text
([expected/uart.txt](expected/uart.txt), 61 lines, 1 748 bytes: the marker plus 60 frame
lines) comes from a Verilator simulation of the RTL: [sim/run.sh](sim/run.sh). The testbench
simulates the same top.v; only the PLL and the TMDS output path are replaced
([sim/stubs.v](sim/stubs.v)), because the UART bytes depend solely on the 25 MHz pixel
domain, whose clock on the chip is the board clock in phase. The frame CRC for the start
pattern was also checked against an independent Python renderer of the picture (same bytes,
same CRC).

```text
RESTART
F0000 G0000 P0033 C472719D7
F0001 G0001 P0037 C535ACA12
F0002 G0001 P0037 C535ACA12
F0003 G0001 P0037 C535ACA12
F0004 G0001 P0037 C535ACA12
F0005 G0002 P003C C8216714C
…
```

Generations advance every 4 frames, so a repeated line means a living world pausing between
generations, not a stuck demo. `cycles` is 26.2 million (about 60 frames after the restart);
the emulator check ran 29.0 million cycles in 71.1 s of wall time — 0.56 MHz in fast mode —
and printed PASS: the same 1 748 bytes from the marker on as the RTL simulation.

## In the emulator's window

```sh
ECP5_GUI_SCRIPT=examples/gallery/video_life/gui.script \
  target/release/ecp5_gui target/gallery/video_life/top.config
```

The Monitor window opens by itself when the design first drives the GPDI pads.
[gui.script](gui.script) lets the start pattern run, sows a soup with FIRE1, pauses with
FIRE2, steps once with RIGHT, runs again, and goes faster with UP. Buttons are held for a
second, because the design debounces them for 10 ms of board time. For its first 15 s nothing is
pressed, so the frames decoded then are the power-on start pattern's.

The window's record of one run (`ECP5_GUI_TRACE`, summed up by
[gui_run.py](../gui_run.py)):
- 50 s at 60 frames per second (median frame 16.7 ms);
- 0.38 MHz with the HDMI receiver attached, 19.1 million clocks;
- the receiver decoded 44 frames from the pads, all 640 × 480, and the monitor painted them.

The decoded frames 1 to 14 have the CRCs the design printed for frames 1 to 14 of the start
pattern: the board printed them after a restart. From frame 15 on they differ, because the
script sowed a soup there. So the picture on the pads, decoded as a monitor would, is the
picture the logic meant to send.

25 drawings of the 3D board were read back, none with an LED drawn wrong.

## On the board

```sh
openFPGALoader -b ulx3s target/gallery/video_life/top.bit   # SRAM only
```

Then watch the monitor, and the UART at 115200 baud. `python3 examples/gallery/check.py
video_life --board` loads the bitstream three times, sends `r`, and compares the frame lines
with the expected text.

**Board:** on 2026-11-16, 3 of 3 loads printed the expected 1 748 bytes from `RESTART` on: 60
frame lines, each with its frame CRC. The board's HDMI output itself was not captured, since that
needs a TMDS capture or a camera on a monitor. Its frames carry the CRCs that the emulator's
receiver decoded from the emulated pads. Records are in
[docs/inventory/LFE5U-85F/hr03_gallery](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).
