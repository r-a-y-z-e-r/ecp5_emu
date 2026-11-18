# led_tennis: tennis on eight LEDs

A one-dimensional tennis game for the ULX3S. The ball is one lit LED running along D7…D0, and the
players hit it back with the buttons at the two ends of the row.

The whole game is a state machine of about 1 500 LUT4 and 370 flip-flops. It contains:
- the rules;
- a timer that speeds the ball up after every return;
- a computer opponent whose returns are drawn from an LFSR;
- a serial log of every event.

It is a small, complete example of what students build first: an FSM, timers, debounced buttons
and a UART, all in one clock domain.

## Rules

- D7 is the leftmost LED on the board and D0 the rightmost. The left player owns D7…D4 and hits with
  **FIRE1** (the button under D7/D6) or **LEFT**. The right player owns D3…D0 and hits with
  **FIRE2** (under D1) or **RIGHT**.
- A hit counts only while the ball is on your end lamp: D7 for the left player, D0 for the right.
  Pressing while the ball is anywhere else on your half is a **fault**, and the point goes to the
  other player, so mashing the button does not work.
- The ball starts at 200 ms per lamp. Every return shortens the step by about 8 %, down to 40 ms.
- A ball that leaves the row is a point for the other player: all LEDs flash. The serve alternates.
- The first to 5 points wins. The winner's half blinks, then the game goes back to attract mode.

**Attract mode.** At power-on, after a restart and after each game, the FPGA plays both sides.
- It starts at 80 ms per lamp, so it is lively on the board and still watchable in the emulator.
- A return succeeds with a chance that falls as the ball speeds up: 242/256 at 200 ms, 194/256 at
  80 ms, 178/256 at 40 ms. The draw comes from a 32-bit LFSR, so rallies end by themselves.
- Everything restarts from fixed values, so attract mode is the same game every time, on the
  board, in the emulator and in the RTL simulation. That is what the checks compare.

Any button press in attract mode starts a game: you on the left, the computer on the right.

## Controls

| Button | UART key | Action |
|---|---|---|
| FIRE1 or LEFT | `a` | left player hits (in attract mode: start a game against the computer) |
| FIRE2 or RIGHT | `l` | right player hits, in a two-player game |
| — | `n` | new game: you on the left against the computer |
| — | `2` | new game for two players |
| — | `r` | restart into attract mode at the exact power-on state |

## The log

At 115200 baud, one line per event:

```text
RESTART
ATTRACT
SERVE L
HIT R 1
HIT L 2
HIT R 3
HIT L 4
HIT R 5
HIT L 6
POINT L 1:0
SERVE R
POINT R 1:1
…
HIT L 11
POINT L 5:1
GAME L 5:1
ATTRACT
```

This is the start of [expected/uart.txt](expected/uart.txt): one full attract-mode game and the
start of the next. `HIT R 3` is the right player's return, the third of the rally. `POINT L 1:0`
gives the score left:right.

## Resources

nextpnr for the LFE5U-85F, seed 1: 1 534 LUT4, 367 TRELLIS_FF, 32 TRELLIS_RAMW (the 256-byte
UART receive FIFO of [io.v](io.v), in LUTRAM), no EBR, no DSP. Fmax 31.2 MHz for the 25 MHz clock.
[io.v](io.v) (UART, receive FIFO, button synchronisers and 10 ms debouncers) is shared with
[ebr_life](../ebr_life/).

## Build and check

```sh
examples/ulx3s/build.sh examples/gallery/led_tennis target/gallery/led_tennis
python3 examples/gallery/check.py led_tennis --emulator
```

[check.json](check.json) sends `r` and compares 450 000 000 clocks (18 s of the board) of the log.
- The expected text comes from a Verilator simulation of the RTL: [sim/run.sh](sim/run.sh).
- The emulator printed the same 401 bytes after the marker, at 2.3 MHz in fast mode on a busy
  M3 Max.

## In the emulator's window

```sh
ECP5_GUI_SCRIPT=examples/gallery/led_tennis/gui.script \
  target/release/ecp5_gui target/gallery/led_tennis/top.config
```

The window runs the game at about 3 MHz, an eighth of the board's 25 MHz, so the ball moves at an
eighth of its real speed. Hold a button for about a second: the design debounces for 10 ms of board
time, and the window takes that long to run it.

[gui.script](gui.script) does the following:
- watches attract mode;
- starts a game with FIRE1;
- swings at the ball a few times. It cannot see the ball, so some swings are faults.

The window's record of one run (`ECP5_GUI_TRACE`):
- 55 s at 60 frames per second (median frame 16.7 ms);
- 3.07 MHz;
- 68 drawings of the 3D board read back from the GPU, none with an LED drawn wrong;
- the ball seen on every one of the eight lamps, plus the all-on flash of a point.

## On the board

```sh
openFPGALoader -b ulx3s target/gallery/led_tennis/top.bit   # SRAM only
```

Then play with FIRE1 and FIRE2, and watch the log at 115200 baud.

`python3 examples/gallery/check.py led_tennis --board` loads the bitstream three times. Each time it
sends `r` and compares 19.5 s of the log with the expected text.

**Board:** 3 of 3 loads printed the expected log, all 401 bytes after the marker, on 2026-11-16.
Records are in [docs/inventory/LFE5U-85F/hr03_gallery](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).
