# ebr_life: the Game of Life in block RAM

Conway's Game of Life on a 64 × 32 torus (2 048 cells) whose world lives in block RAM. Each
generation is printed on the UART as text.

The lesson is the space/time trade-off. [life](../../ulx3s/life/top.v) gives each cell a
flip-flop and its own neighbour counter: 729 cells, all of them updated every clock, in about
10 850 LUT4. This design keeps three times as many cells in five EBR, and one small circuit
visits them one at a time. It does a generation in about 2 300 clocks (92 µs) with 1 480 LUT4,
most of which are the UART, the buttons and the decimal printer.

## How it sweeps

The two worlds are `reg [63:0] world0[0:31]` and `world1[0:31]`: one 64-bit word per row, read 36
bits at a time from PDPW16KD blocks.

**One generation:**
1. For each row it fetches the rows above, at and below it into three 64-bit shift registers. Each
   fetch is an address clock, a wait clock and a capture clock.
2. It rotates the three rows by one bit per clock for 64 clocks. Every clock it counts the 8
   neighbours of bit 0 and shifts the new cell into a result register.
3. It writes the finished row to the other world.

After 32 rows the worlds swap roles. The torus wraps by truncation: row −1 is row 31, column 64
is column 0.

**Rules for the block RAM:**
- **No read of a world in a clock that writes it.** While a start pattern is copied in, the read
  address is the row being written. An EBR row read in the clock its other port writes it returns
  undefined data on the chip, and the emulator makes it unknown. No state uses such a read, so
  the reads are switched off while a row is written.
- **A settled world is sown again.** After 32 generations in a row with an unchanged population,
  the world has settled into still lifes and blinkers, and a new random soup is sown. The board
  never stops changing.

**The start pattern** is in [seed.hex](seed.hex), written by [make_seed.py](make_seed.py):
- Gosper's glider gun;
- two gliders;
- an R-pentomino.

On this small torus the gun's gliders come round and wreck it after about 180 generations.

## Output

Every generation by default, or every 8th, the design prints a header and the world (`#` alive):

```text
RESTART
GEN 00000 POP 0051
................................................................
................................................................
..........................#.....................................
........................#.#.....................................
..............##......##............##..........................
.............#...#....##............##..........................
..##........#.....#...##........................................
..##........#...#.##....#.#.....................................
............#.....#.......#.....................................
.............#...#..............................................
..............##................................................
…
```

A frame is about 2.2 KB: 0.19 s at 115200 baud. The engine waits for the printer, so no
generation is skipped unless every 8th is asked for. The LEDs show the population divided by 32.

## Controls

| Button | UART key | Action |
|---|---|---|
| FIRE1 | `s` | fill the world with random soup (from an LFSR) |
| FIRE2 | `p` | pause / run |
| LEFT or RIGHT | `n` | one generation, while paused |
| UP / DOWN | `f` (toggles) | print every generation / every 8th |
| — | `g` | load the start pattern again |
| — | `r` | restart at the exact power-on state; prints `RESTART` |

Keys wait in a 256-byte FIFO and are acted on in order, at the end of a generation.

## Resources

nextpnr for the LFE5U-85F, seed 1:
- 1 483 LUT4 and 635 TRELLIS_FF;
- 5 DP16KD: the two worlds and the start pattern;
- 32 TRELLIS_RAMW: the UART receive FIFO of [io.v](io.v), shared with [led_tennis](../led_tennis/).

Fmax is 106 MHz for the 25 MHz clock.

## Build and check

```sh
examples/ulx3s/build.sh examples/gallery/ebr_life target/gallery/ebr_life
python3 examples/gallery/check.py ebr_life --emulator
```

[check.json](check.json) sends `rfpnns gp`. In order, that means:
1. restart;
2. print every 8th generation;
3. pause;
4. two single steps;
5. soup;
6. a key that is ignored;
7. the start pattern again;
8. run.

The expected text ([expected/uart.txt](expected/uart.txt), 82 KB) runs past the moment the world
settles, and through the soup sown after it. It comes from a Verilator simulation of the RTL:
[sim/run.sh](sim/run.sh). The emulator prints the same text in fast mode, at about 2 MHz.

## In the emulator's window

```sh
ECP5_GUI_SCRIPT=examples/gallery/ebr_life/gui.script \
  target/release/ecp5_gui target/gallery/ebr_life/top.config
```

[gui.script](gui.script) does the following:
- lets the start pattern run;
- sows a soup with FIRE1;
- pauses with FIRE2;
- steps twice with RIGHT;
- runs again.

Buttons are held for a second, because the design debounces for 10 ms of board time.

The window's record of one run:
- 48 s at 60 frames per second (median frame 16.7 ms, 95th percentile 17.3 ms);
- 2.4 MHz, 114.6 million clocks, 4.6 s of the board;
- 25 drawings of the 3D board read back from the GPU, none with an LED drawn wrong;
- 12 different LED patterns shown, the population bar moving with the world.

## On the board

```sh
openFPGALoader -b ulx3s target/gallery/ebr_life/top.bit   # SRAM only
```

Then watch it at 115200 baud. `python3 examples/gallery/check.py ebr_life --board` loads the
bitstream three times, sends the keys of check.json, and compares 8.7 s of output with the
expected text.

**Board:** on 2026-11-16, 3 of 3 loads printed the expected 80 523 bytes from the marker on. An
earlier build without the reseeding also passed 3 of 3, with 64 462 bytes over 145 million clocks.
Records are in [docs/inventory/LFE5U-85F/hr03_gallery](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).
