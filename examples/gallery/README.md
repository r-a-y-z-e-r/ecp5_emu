# Gallery

Fifteen demo designs for the ULX3S (LFE5U-85F) that explore what the emulator runs: a RISC-V computer, the
DSP blocks, block RAM, SDRAM, games, SPI OLED rendering, video on HDMI, and clock-domain crossing
with PLL and oscillator clocks, and parallel hash chains in plain RTL using LUTs and
carry chains, plus per-pixel plasma and rotozoom on HDMI. All fifteen have RTL, emulator,
board and window checks; the hardware builds use Yosys, nextpnr-ecp5 and ecppack.
The gallery checks designs three ways:

- in the emulator, in fast mode, on the command line, using the board's own configuration
  (`needs: peripherals` passes `--peripherals` to attach the design's peripheral models);
- on the board, loaded into SRAM;
- in the emulator's window, from the window's own record (`ECP5_GUI_TRACE`).

All fifteen designs' UART output was compared byte for byte between RTL simulation, the
emulator and the board. Window records check the 3D board's LEDs read back from the GPU,
console text and, where present, decoded HDMI frames, painted OLED pictures and modelled audio.

| Demo | What it shows | Blocks | Emulator | Board | Window |
|---|---|---|---|---|---|
| [riscv_shell](riscv_shell/README.md) | PicoRV32 RV32IMC running a C shell: primes, Mandelbrot, a calculator, memory commands, LEDs and buttons, with exact cycle counts | 17 EBR, 4 MULT18X18D, 4 140 LUT4 | the same 3 214 bytes as the RTL simulation | 3 of 3 loads: the same 3 214 bytes, cycle counts included | 0.85 MHz; typed commands ran, `mandel` in the board's 1 025 262 cycles; 36 LED drawings, none wrong |
| [serv_shell](serv_shell/README.md) | the same C shell on SERV, a bit-serial RV32I core: a third of PicoRV32's logic, 16 times the cycles, counted exactly | 18 EBR, 1 370 LUT4, no DSP | the same 1 304 bytes as the RTL simulation | 3 of 3 loads: the same 1 304 bytes, cycle counts included | 2.8 MHz; typed help, primes 1000 (168 primes, 1 936 733 cycles) and calc; a FIRE1 event; 3 LED drawings, none wrong |
| [riscv_terminal](riscv_terminal/README.md) | a computer you can see: riscv_shell's PicoRV32 and C shell with an 80 × 30 text screen on HDMI (original 8 × 16 font, hardware cursor and scrolling); the `screen` command prints the CRC of what the monitor shows | 20 EBR, 4 MULT18X18D, a PLL, 5 170 LUT4 | the same 2 837 bytes as the RTL simulation, both screen CRCs equal to an independent Python renderer's | 3 of 3 loads: the same 2 837 bytes, screen CRCs included | 0.30 MHz; 61 HDMI frames decoded from the pads, among them the screen whose CRC the design printed |
| [mandelbrot](mandelbrot/README.md) | a Mandelbrot explorer in hardware: three multipliers do one iteration per clock; ASCII art on the UART; the buttons pan and zoom | 3 MULT18X18D, 2 391 LUT4 | the same 24 567 bytes (16 views) as the RTL simulation | 3 of 3 loads: the same 24 567 bytes | 2.8 MHz; the tour typed, 10 views identical to the board-checked ones; 428 LED drawings, none wrong |
| [fir_scope](fir_scope/README.md) | a 31-tap FIR filter built twice (31 multipliers in parallel, and one multiplier with a hand-instantiated ALU54B accumulator), compared bit for bit on the chip every sample; an ASCII oscilloscope whose time base follows the tone, and an LED bar | 33 MULT18X18D, 1 ALU54B, 1 EBR, 4 446 LUT4 | the same 13 547 bytes (11 frames) as the RTL simulation | 3 of 3 loads: the same 13 547 bytes, `MISMATCH 0` in every frame | 0.57 MHz; the tone ladder 4–64 Hz; 299 LED drawings, none wrong |
| [video_life](video_life/README.md) | the Game of Life on an HDMI monitor: 3 072 cells in block RAM drawn as 10 × 10 squares coloured by age; the emulator decodes the TMDS pads into frames, and their CRCs match the ones the design prints | 13 EBR, a PLL, 4 ODDRX1F, 2 847 LUT4 | the same 1 748 bytes (60 frame CRCs) as the RTL simulation | 3 of 3 loads: the same 1 748 bytes | 0.38 MHz; 44 HDMI frames decoded from the pads, frames 1–14 with the board's CRCs; 25 LED drawings, none wrong |
| [hdmi_plasma](hdmi_plasma/README.md) | two original effects race the HDMI beam without a frame buffer: four-wave fixed-point plasma and a rotating, breathing cut-glass lattice; three animated palettes, buttons and RGB CRCs | eight intended DSP products, sine/palette ROMs, a PLL, 4 ODDRX1F | the same 1 389 bytes as RTL; d377f3e: 0.81 MHz on the board's configuration | 3 of 3 loads: the same 1 389 bytes; Python recomputes all 239 complete frame CRCs | 120 s, 0.65 MHz; console PASS on 1 389 bytes; all 185 decoded HDMI CRCs printed, all 182 printed CRCs decoded (matched by CRC); 13 LED drawings, 0 wrong |
| [sdram_bench](sdram_bench/README.md) | its own burst SDRAM controller writing and checking all 32 MiB at 44.5/44.8 MiB/s | 1 794 LUT4, the SDRAM, no EBR | the same bytes as the RTL simulation, from the command line with `--peripherals`; the SDRAM model reports 0 timing violations | 3 of 3 loads: the expected bytes, and all 32 MiB checked with 0 errors | 1.1 MHz; three passes, 2 and 32 MiB; 53 LED drawings, none wrong |
| [ebr_life](ebr_life/README.md) | the Game of Life with a 2 048-cell world in block RAM, swept one cell per clock: space traded for time | 5 EBR, 1 483 LUT4 | the same 80 523 bytes as the RTL simulation | 3 of 3 loads: the same 80 523 bytes | 2.4 MHz; 25 LED drawings, none wrong |
| [led_tennis](led_tennis/README.md) | tennis on eight LEDs: an FSM, timers, debounced buttons, a computer player | 1 534 LUT4 | the same 401 bytes of a full game | 3 of 3 loads: the same 401 bytes | 3.1 MHz; the ball on all 8 LEDs; 68 LED drawings, none wrong |
| [chip_synth](chip_synth/README.md) | Glass Orchard: original four-bar polyphonic RTL tune, six live notes on a spare sine voice, stereo 4-bit jack DACs and a live 96-sample OLED scope | four DDS/ADSR voices, 25 kHz samples, SPI OLED, no CPU or PLL | the same 836 bytes as RTL; d377f3e: 0.63 MHz on the board's configuration; 40 OLED frames, zero model errors | 3 of 3 loads: the same 836 bytes; 24 AUDIO CRCs (3 s) equal the model-checked loop simulation | 120 s, 1.18 MHz; console PASS on 836 bytes; 1 752 of 1 752 completed-frame OLED pictures with printed CRCs (12 startup); 110 modelled open-circuit audio records, 1.797 V peak to peak on L/R, V 0 V; 3 362 LED drawings, 0 wrong |
| [oled_bricks](oled_bricks/README.md) | Orchard, an original brick-breaker: pixels computed during SPI with no frame buffer, fixed-point physics, deterministic attract and four pulled DIP inputs | SPI renderer and fixed-point physics, no frame buffer, EBR or PLL | P27/P28: the same 4 434 bytes as RTL; d377f3e: 1.10 MHz on the board's configuration; 151 OLED frames, zero model errors | P27/P28: 3 of 3 loads PASS on 4 434 bytes; transcripts identical over 8 888 bytes | 90 s, 2.16 MHz; 392 of 392 completed-frame OLED pictures with printed CRCs (6 startup); 2 581 console bytes before scripted human play; PLAYER, SW 0100, SW 1100, PLAYER (fresh game after game-over); 413 LED drawings, 0 wrong |
| [clock_domains](clock_domains/README.md) | three concurrent textbook CDC circuits across four domains: a dual-clock FIFO, a four-phase word handshake and an acknowledged pulse toggle; PLL clocks share one VCO (6:5), OSCG is independent | 1 DP16KD, 1 PLL, OSCG, 4 505 total LUTs, 1 580 FF, no CPU | the same 356 bytes as the RTL simulation; 0.60 MHz on rserv with the board's configuration (d377f3e); the separate FREQ run gives exact PLL ratios and nominal OSCG | 3 of 3 loads: the same 356 bytes; transcripts identical over 4 979 bytes, 42–43 passes with zero errors; OSCG +0.66% | 0.13 MHz; console PASS on 356 bytes, all 11 passes equal the simulation; 9 LED drawings, none wrong; FREQ window outlasts the 60 s script |
| [sha256_chains](sha256_chains/README.md) | ten parallel iterative SHA-256 chains, standard known-answer tests and a hardware digest of all final values | 32 460 LUTs (38.8%), 13 478 FF, Fmax 44.59 MHz, no DSP, EBR, PLL or CPU | PASS on 2 780 bytes; 0.79 MHz on rserv with the board's configuration (d377f3e), one thread | 3 of 3 SRAM loads PASS; one RESTART each, identical over 33 375 bytes; hashlib confirms 381 STEP values and 12 CHAIN digests | 150 s, 0.07 MHz; console PASS on 2 780 bytes from one RESTART; 7 LED drawings, 0 wrong; hashlib confirms 47 STEP values and 1 CHAIN digest |
| [riscv_oled](riscv_oled/README.md) | PicoRV32 draws an original fixed-point 3D lantern in C; a double-buffered EBR SPI peripheral sends RGB565 frames and computes CRC32, with exact draw cycle counts | 29 EBR, 4 MULT18X18D, 4 904 TRELLIS_COMB (5%), 1 379 FF; Fmax 52.81 MHz | PASS on 1 534 bytes, cycle counts included; d377f3e: 0.11 MHz on the board's configuration; 64 OLED frames, zero model errors | 3 of 3 SRAM loads PASS; one RESTART each, identical over 12 684 bytes; 507 frames in 8 s, 63.4 fps | 90 s, 0.74 MHz; 1 066 console bytes (F0000–F0029) before LEFT changes rotation from F002A; all 1 999 completed-frame OLED pictures with printed CRCs (18 startup); 2 059 LED drawings, 0 wrong |

The board records are in [docs/inventory/LFE5U-85F/hr03_gallery](../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).
On 2026-11-18, all fifteen emulator checks passed on rserv with fast mode `d377f3e`, using
copies of the board's Mac-built configurations, hashes verified equal, without rebuilding.
The rates in the Emulator column are from that rerun; Window rates describe the recorded window runs.
Per-demo configuration hashes and
PASS/rate lines are in [emulator_board_configs.log](../../docs/inventory/LFE5U-85F/hr03_gallery/emulator_board_configs.log).
Earlier `--build --emulator` runs used rserv's own configurations and did not test the board's builds.

## A demo's folder

| File | What it is |
|---|---|
| `top.v`, `*.v`, `top.lpf`, `*.hex` | the design; the pins follow the official `ulx3s_v20.lpf` |
| `ecp5.toml` | the project file: which `.v` files, which constraints |
| `check.json` | what to send to the UART, for how many clocks to compare, and the marker the comparison starts at |
| `expected/uart.txt` | the bytes an RTL simulation of the design printed |
| `sim/` | that simulation (Verilator); `sim/run.sh` writes `expected/uart.txt` again |
| `gui.script` | a script that drives the emulator's window through the demo (`ECP5_GUI_SCRIPT`) |
| `README.md` | what it shows, its controls, its resources, and its results |

## Build, check, run

```sh
examples/ulx3s/build.sh examples/gallery/ebr_life target/gallery/ebr_life
python3 examples/gallery/check.py ebr_life --emulator    # fast mode against expected/uart.txt
python3 examples/gallery/check.py ebr_life --board       # three loads into the board's SRAM
python3 examples/gallery/gui_run.py ebr_life             # the window, driven by gui.script
```

**The whole gallery.** `all` stands for every demo:
- `check.py all --build` builds them all; `--emulator` and `--board` then check them all.
- `gui_run.py all` is a tour of about eight minutes. It opens the window on each demo in turn,
  from riscv_terminal to serv_shell, each driven by its own script, and sums up each record.

```sh
python3 examples/gallery/check.py all --build --emulator
python3 examples/gallery/gui_run.py all
```

**The tour in one window.** `gui_run.py tour` runs [tour.script](tour.script)
in about seven minutes. The tour does not include sdram_bench yet. The window starts with no
design and `open`s each demo in turn, typing and pressing as each demo's own script would.
It ends with riscv_shell's and serv_shell's
`primes 2000` back to back, showing the size/speed trade-off. Every window run requires an awake,
unlocked display and holds the main checkout's shared `out/screen.lock`; `all` holds one lock
for its whole sequence. `--no-run` only summarizes existing records and takes no lock.
Afterwards it sums up the record demo by demo.

```sh
python3 examples/gallery/gui_run.py tour
```

One run's record, summed up:
- all scripted demos were opened, and each received exactly its own commands after its
  `# design` line;
- every button the script pressed is recorded as pressed in that demo (`# button` lines): nine
  presses in six demos;
- no LED of the 3D board was drawn wrong (421 drawings);
- the monitor decoded 41 HDMI frames for riscv_terminal and 33 for video_life;
- at the end, riscv_shell printed `primes below 2000: 303, 243162 cycles` and serv_shell printed
  `primes below 2000: 303, 3957776 cycles`: the two cores' trade-off, back to back in one window.

**check.py** ([check.py](check.py)) compares a run's UART text with `expected/uart.txt` from the
**marker** on. The marker is the first thing the design prints in answer to the input of
check.json, usually a restart key.
An optional `"needs": "peripherals"` in check.json passes `--peripherals` to the emulator command.
The optional `"peripherals": ["oled"]` selects the required healthy model reports; when absent,
the required list is `["sdram"]`. OLED also requires its last completed data CRC to occur in
a frame line printed by the design during that run.

The marker matters because the board and the emulator start differently:
- On the board the serial port opens about 0.3 s after the load: the FT231X carries both JTAG
  and the UART, so nothing can listen during the load. What the design printed before that is lost.
- From the marker on, a design's output depends only on the bytes it received, never on when they
  came. Each design keeps received bytes in a FIFO and restarts from fixed values. So the board,
  the emulator and the simulation must print the same bytes from there.

`--board` does two things around the board:
- It takes the shared lock `out/board.lock` of the main checkout, so two sessions never use the
  board at once.
- It loads with `openFPGALoader -b ulx3s`, into SRAM only; the flash is never written.

**gui_run.py** ([gui_run.py](gui_run.py)) starts the window with the demo's script and sums up the
window's own record (`ECP5_GUI_TRACE`, [gui/src/trace.rs](../../gui/src/trace.rs)):
- frame times and the rate it ran at;
- whether the 3D board drew each LED as the design drove it, read back from the GPU;
- the console's text, once the window records it;
- OLED pictures, their completed-frame range and data CRCs matched to the design's printed
  frame CRCs, with startup pictures before the first completed frame counted separately
  (at most the last picture may still be waiting for its UART line);
- `SW` and `PLAYER` event lines.

## What the demos found in the emulator

Every finding is written up in the demo's README.

- **An EBR output latch that had not been read since configuration was unknown**, and fast mode
  would not start until everything it reads was known.
  - Found by riscv_shell; the repro is [riscv_shell/repro/ebr_unread](riscv_shell/repro/ebr_unread/run.sh).
  - The board then showed that such latches read 0 (HB02), and the configured startup now starts
    them at 0 (`d4f83ec` on `dev`).
- **fir_scope found two toolchain behaviours** (not emulator faults), each with a reproducer in
  [fir_scope/repro](fir_scope/repro/README.md):
  - nextpnr keeps one net per DSP block control line, so an ALU54B's own RST0 can be lost to a
    constant on its neighbouring multipliers;
  - yosys 0.70 fails on module ports declared `signed`.
- **A read of an EBR row in the clock its other port writes it can be unknown** in the emulator
  when the chip's result is undefined. Fast mode stops when logic reads it. The designs avoid
  such reads: ebr_life and riscv_shell show how. The measured `READBEFOREWRITE` case follows.
- **serv_shell found that a narrow (1/2/4-bit) READBEFOREWRITE DP16KD**, which Yosys makes of SERV's stock register file, was refused; `847a9ef` fixed this after HB11. The demo also found the same-clock cross-port collision: HB09 measured that `READBEFOREWRITE` returns the old word, fixed in the event kernel (`04268bb`) and fast mode (`8f3673e`). The repro in [serv_shell/repro/narrow_ebr](serv_shell/repro/narrow_ebr/run.sh) remains as a regression check.
