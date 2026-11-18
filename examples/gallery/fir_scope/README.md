# fir_scope: one FIR filter, two hardware implementations, compared on the chip

A 31-tap low-pass FIR filter is implemented twice, in two different parts of the ECP5's
DSP hardware: **filter 1** computes all 31 products in parallel on 31 MULT18X18D
multipliers plus a fabric adder tree, and **filter 2** time-shares ONE MULT18X18D with a
hand-instantiated ALU54B accumulator over 31 clocks. On every one of the 2 000 samples
per second the chip subtracts the two results and counts the disagreements in the
footer: `MISMATCH 0x00000000` means the parallel and the serial filter agree bit for
bit — on the board, in the emulator and in the RTL simulation alike.

The point for a student: a filter is a specification, not a circuit. The same
coefficients, the same samples, two circuits with nothing in common but the arithmetic
— and the chip itself proves them equal, every sample, forever. And because the signal
is a tunable tone with noise added, an ASCII oscilloscope on the UART shows the filter
doing its job: above the cutoff the `#` trace visibly shrinks while the input does not.

## The scope

Every frame is a fresh, complete capture of 64 columns. The time base follows the tone:
the 64 columns are always **exactly two periods** — one column is 1/32 of a period,
strobed off the tone's own DDS phase, so the trace cannot drift or scroll. At 64 and
128 Hz a column would be shorter than one sample, so every sample becomes a column and
the header says how many whole periods the window holds (`COL 1 SAMPLE`). After a
restart or any key the next frame waits for 64 new columns; there is never a
half-empty history, and sampling never pauses while a frame is printed.

The input (tone + noise) is drawn as `.`, the displayed filter's output as `#` (where
they coincide, `#`). Sixteen rows span ±65536, so a full-scale sine fills the screen.
The second header line gives the peak-to-peak amplitudes over the captured window in
decimal: `IN` is the input, `OUT` the selected filter's output (also when the raw
input is on screen). Their ratio is the filter's visible effect. Both numbers describe
the plotted samples; at high tones `OUT` mostly shows the noise's low-frequency
residue, which varies from window to window — the pure-tone response is in the table
below.

```text
FRAME 0x00000002 FIR 1 LED F TONE 004Hz NOISE 2 SQ 0 FS 2000
IN 184502 OUT 131511 WINDOW 2 PERIODS COL PERIOD/32
          #####..                        .#####
      .  #     #                       ..#     #
        #     . #                       #      .#
    .. #..       #                     #         #
                                                .
      #           #                 . #           #
   . #             #              .  #            .#
  . #            ...             .    .              .
                    #.              #            .  #
 . #                 #.         .  #                 #
  #                   #.                           ..
.                                 #                   #
 #                     #.        #                     #
#                   .   #.     .#                      .#      .
                         #.    #                         #.  ..#
                          #####                       . ..#####
MISMATCH 0x00000000
```

A noisy 4 Hz input (a band of `.`) around a clean `#` sine: the noise is above the
cutoff, the tone below it. `IN` is 184 502, `OUT` 131 511 — the filter threw the noise
away and kept the tone (a full-scale sine is 131 072 peak to peak).

## Controls

| Button | UART key | Action |
|---|---|---|
| FIRE1 | `f` | filtered / raw on the LEDs and the scope |
| FIRE2 | `n` | noise level: off, low, high |
| UP / DOWN | `u` / `j` | tone: 1, 2, 4, 8, 16, 32, 64, 128 Hz |
| LEFT / RIGHT | `1` / `2` | the scope (and `FIR` line) shows filter 1 / filter 2 |
| — | `q` | add a square wave at the tone frequency |
| — | `r` | restart at the power-on state; prints `KEY r` |

Buttons are debounced for about 10 ms and inject the same key bytes as the UART. Keys
wait in a 256-byte FIFO and are acted on one per frame boundary, in arrival order (the
first key after configuration is taken at once, so an early restart is not stuck
behind a power-on capture). Each key prints its `KEY <c>` line and forces a fresh
capture. The LEDs are an 8-level bar of the displayed signal, D7 leftmost; unfiltered,
they jitter with the noise.

The footer `MISMATCH 0x00000000` counts, since the restart, the samples where filter 1
and filter 2 disagreed — including the samples of frames you did not select for display.

## Frequency response, measured

[sim/response.py](sim/response.py) feeds the RTL (iverilog; `filter.v` and the real
`dsp_mac.v`) a pure full-scale tone at each ladder frequency and measures the settled
output over the input:

| tone | out/in, filter 1 | out/in, filter 2 (ALU54B) | mismatches |
|---|---|---|---|
| 1 Hz | 1.000 | 1.000 | 0 |
| 2 Hz | 0.999 | 0.999 | 0 |
| 4 Hz | 0.997 | 0.997 | 0 |
| 8 Hz | 0.991 | 0.991 | 0 |
| 16 Hz | 0.965 | 0.965 | 0 |
| 32 Hz | 0.864 | 0.864 | 0 |
| 64 Hz | 0.548 | 0.548 | 0 |
| 128 Hz | 0.067 | 0.067 | 0 |

The coefficients are a Hamming-windowed sinc with a 50 Hz cutoff at fs = 2 kHz
([generate.py](generate.py) writes [coeff.hex](coeff.hex) and the sine table; no
numerical packages). The table is the honest lesson of a 31-tap filter: the knee is
soft. At 64 Hz half of the tone still passes; real attenuation only arrives well above
the cutoff (0.067 at 128 Hz). The two filters match on every tone and every sample.

fs is 2 kHz, not the sketch's 1 kHz: fast mode runs this DSP-dense design at about
0.2–0.4 MHz (below), and at 1 kHz the check's frames would take twice the wall time.
Every per-second figure doubles; the normalized cutoff is unchanged.

## Resources

nextpnr for the LFE5U-85F, seed 1:
- 4 446 LUT4 (1 850 logic, 2 308 carry, 192+96 LUTRAM — the scope's capture and
  picture buffers, the key FIFO) and 3 748 TRELLIS_FF;
- 1 DP16KD — the 256-entry 18-bit sine table;
- 33 MULT18X18D — 31 for filter 1, plus the two of the serial MAC;
- 1 ALU54B — the serial accumulator.

Fmax is 63.29 MHz after routing (58.01 MHz after placement) for the 25 MHz clock.

On the die map the DSP cells light in two rows: 27 of the parallel multipliers lie
along row Y34 (X4–X59), and row Y58 holds the serial MAC — pinned to
X13/Y58/MULT18_0, X14/Y58/MULT18_1 and X16/Y58/ALU54_3, the sites
docs/inventory/LFE5U-85F/hd05 validated — with the four remaining parallel
multipliers next to it (X18–X32).

## Build and check

```sh
examples/ulx3s/build.sh examples/gallery/fir_scope target/gallery/fir_scope
python3 examples/gallery/check.py fir_scope --build --emulator
```

[check.json](check.json) sends `rnuuuuu2f`: restart; noise high (a noisy 4 Hz,
filtered); tone up through 8, 16, 32, 64 and 128 Hz — the `OUT` number shrinks from
~131 000 to ~10 000 while `IN` does not; filter 2, the same output; raw. The expected
text ([expected/uart.txt](expected/uart.txt), 13 547 bytes, 11 frames) comes from a
Verilator simulation of the demo's own RTL: [sim/run.sh](sim/run.sh).

The emulator prints the same bytes in fast mode — but at **0.20 MHz** (the 68.3
million clocks in 345 s on the Linux check machine; 0.37 MHz on a MacBook), far below
the ~2 MHz the smaller demos reach: 33 multipliers and the adder tree are a lot of
compiled gates. The check's nine-frame story cannot shrink either — two 4 Hz windows
alone are 25 million board clocks — so this demo's check takes minutes, not seconds.
`board_seconds` is 4.22 (= cycles/25e6 + 1.5).

## In the emulator's window

```sh
ECP5_GUI_SCRIPT=examples/gallery/fir_scope/gui.script \
  target/release/ecp5_gui target/gallery/fir_scope/top.config
```

The window runs this design at about 0.5 MHz, so a frame of two 4 Hz periods (12.5
million clocks) takes about 23 s. Keys and buttons queue in the design's FIFO and act
one per frame.

[gui.script](gui.script) presses the buttons at the start, one second each (the design
debounces 10 ms of board time): FIRE2 for noise, UP four times for 8 to 64 Hz, RIGHT for
filter 2. It then lets the frames walk up the ladder for 85 s. It does not use `type`,
which the window does not have yet.

The window's record of one run (`ECP5_GUI_TRACE`):
- 85 s at 60 frames per second (median frame 16.7 ms);
- 0.57 MHz on four threads, 47.2 million clocks (1.9 s of the board), enough for the ladder
  from 4 Hz to 64 Hz;
- 299 drawings of the 3D board read back from the GPU, none with an LED drawn wrong;
- the LED bar seen at every level from 0 to 6.

## On the board

```sh
openFPGALoader -b ulx3s target/gallery/fir_scope/top.bit   # SRAM only
```

Then watch the scope at 115200 baud, and the LED bar.

`python3 examples/gallery/check.py fir_scope --board` loads the bitstream three times, sends
`rnuuuuu2f` each time, and compares 4.2 s of output.

**Board:** on 2026-11-16, 3 of 3 loads printed the expected 13 547 bytes from `KEY r` on. In the
4.2 s each load listened, 25 or 26 frames arrived, every one with `MISMATCH 0x00000000`. The ALU54B filter equals the
31-multiplier filter on every sample, on the chip. An earlier build with the first scope also
passed 3 of 3. Records are in
[docs/inventory/LFE5U-85F/hr03_gallery](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).

## What building it taught

Two toolchain behaviours cost real debugging time; both have minimal reproducers in
[repro/](repro).

- **nextpnr keeps one net per DSP block control line.** The ALU54B's accumulator clear
  was wired to its RST0 pin while the two MULT18X18Ds of the same DSP block had RST0
  tied to 0. The RTL sim passed, but in the emulator, which runs the configuration nextpnr wrote,
  the accumulator never cleared. The CE/RST lines are shared by the block's cells, and
  nextpnr keeps only one net per line: the multipliers' constant won. That variant never
  went to the board. The fix, as in hd05: every cell of the
  block gets the same controls; the multipliers ignore theirs (no register stages
  selected). [repro/README.md](repro/README.md) isolates the MAC and drives it through
  seven phases.
- **yosys 0.70 re-elaborates signed-port modules from their AST at the final hierarchy
  check**, after `flatten` has deleted the sibling modules, and the run dies with
  `Module '\gallery_io' referenced in module '\top' ... is not part of the design`.
  The only trigger is the `signed` keyword on `dsp_mac`'s ports; the demo declares no
  signed ports anywhere and puts the sign on the wires inside instead.
  [repro/yosys_signed_ports/run.sh](repro/yosys_signed_ports/run.sh) shows both
  variants, passing and failing.

## Files

- [top.v](top.v) — signal source (DDS + LFSR noise + optional square), the scope
  (phase-locked capture, printer), controls, LEDs;
- [filter.v](filter.v) — the two filters and the mismatch counter;
- [dsp_mac.v](dsp_mac.v) — the serial MAC, generated by [generate.py](generate.py):
  one MULT18X18D feeding a hand-instantiated ALU54B;
- [io.v](io.v) — UART RX/TX at 115200 8N1, button debounce, the 256-byte key FIFO;
- [sim/](sim) — the Verilator reference simulation ([run.sh](sim/run.sh)),
  behavioural DSP models, [response.py](sim/response.py);
- [repro/](repro) — the two toolchain reproducers above.
