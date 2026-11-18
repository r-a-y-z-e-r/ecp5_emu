# serv_shell: a bit-serial RISC-V computer with a shell

The C shell of [riscv_shell](../riscv_shell/README.md) running on [SERV](serv/README.md), Olof Kindgren's bit-serial RV32I core (`W=1`), using its stock `serv_rf_top` and default 2-bit register file, instead of PicoRV32 (RV32IMC). Both designs run the same interactive shell on the ULX3S at 25 MHz with 32 KB of block RAM, a 511-byte UART RX FIFO, LEDs, and debounced buttons.

The point for students is the size/speed trade-off. SERV processes one bit of the 32-bit datapath per clock cycle and has no hardware multiplier or divider (`MDU=0`, `COMPRESSED=0`), so it uses roughly a third of the whole SoC's LUTs (and zero DSP blocks) while taking ~16× more cycles on `primes 2000` and ~426× more cycles on fixed-point `mandel`.

## PicoRV32 vs SERV

| | [riscv_shell](../riscv_shell/README.md) | serv_shell |
|---|---|---|
| Core & ISA | PicoRV32 (RV32IMC) | SERV (RV32I, `W=1`) |
| LUT4 (`TRELLIS_COMB`) | 4 140 | 1 370 |
| `TRELLIS_FF` | 1 208 | 522 |
| `DP16KD` | 17 | 18 |
| `MULT18X18D` | 4 | 0 |
| `TRELLIS_RAMW` | 32 (register file) | 0 (register file in 1 `DP16KD`) |
| Routed Fmax (`clk_25mhz`) | 58.5 MHz | 80.39 MHz |
| `primes 2000` | 303 primes, 243 162 cycles | 303 primes, 3 957 776 cycles |
| `mandel` | 1 025 262 cycles | 437 300 476 cycles (software `*`/`/`, omitted from `check.json`) |

## Memory map

SERV implements no hardware `cycle` CSR (`rdcycle`), so a free-running 64-bit cycle counter is mapped at `0x0600_0000`. Software reads `high`, `low`, `high` again until the two high reads match.

| Address | Register |
|---|---|
| `0x0000_0000` | RAM, 32 KB: 16 EBR as 4 banks × 4 byte lanes of 2048 × 8. The core starts at 0. |
| `0x0200_0000` | UART data: a write sends a byte; a read takes the oldest received byte from the FIFO |
| `0x0200_0004` | UART status: bit 0 TX busy, bit 1 RX FIFO non-empty, bit 2 RX FIFO overflow (sticky), bit 3 RX active |
| `0x0300_0000` | LEDs, bit 7 = D7 (leftmost) |
| `0x0400_0000` | buttons, debounced for 10 ms; bit 0 PWR … bit 6 RIGHT, 1 = pressed |
| `0x0400_0004` | presses since the last read of this register (reading clears it) |
| `0x0500_0000` | writing `0x005EB007` resets the whole computer as at power-on; RAM keeps its contents |
| `0x0600_0000` | 64-bit cycle counter, low 32 bits (read-only) |
| `0x0600_0004` | 64-bit cycle counter, high 32 bits (read-only) |

## Controls

- **UART** (`115200 8N1`): `help`, `info`, `led [hex]`, `buttons`, `knight [n]`, `primes [n]`, `mandel`, `calc <expr>`, `peek <addr>`, `poke <addr> <value>`, `dump <addr> [len]`, `memtest`, `uptime`, `reboot`.
- **Buttons**: `btn[1..6]` (FIRE1, FIRE2, UP, DOWN, LEFT, RIGHT), debounced for 10 ms (250 000 clocks). Pressing a button at the prompt prints `[button <name>]` and restores the current input line. `btn[0]` (PWR) is active-low and left unused for user actions.

## Expected output

Excerpt from [expected/uart.txt](expected/uart.txt) (produced by [sim/run.sh](sim/run.sh) over 12 000 000 cycles):

```text
> reboot
RISC-V shell on ULX3S
SERV RV32I | 25000000 Hz | 32768 bytes RAM
firmware: 16828 bytes
> info
CPU: SERV
ISA: RV32I
clock: 25000000 Hz
RAM: 32768 bytes
firmware: 16828 bytes
stack pointer: 0x00007f60
build: 2026-11-16
> led a5
led: a5
> calc (7+5)*3-10/4
34
> calc -17%5
-2
> calc 1/0
division by zero
> primes 2000
primes below 2000: 303, 3957776 cycles
```

## Vendoring and emulator note

- **Third-party code**: [serv/](serv/) contains the unmodified RTL and ISC `LICENSE` of `olofk/serv` tag `1.4.0` (commit `7d9cde4e6ca4f4c84d7512a752a4c187537dd373`). Top-level `serv_*.v` symlinks expose the modules needed by [top.v](top.v) to [build.sh](../../ulx3s/build.sh).
- **Register-file history**: This demo exposed the narrow `READBEFOREWRITE` EBR gap, fixed in `847a9ef` after board series HB11. The stock register file then exposed a same-clock cross-port collision that stopped fast mode at cycle 32. Board series HB09 measured that reading the word the other port writes returns the old word under `READBEFOREWRITE`; `04268bb` fixed the event kernel and `8f3673e` fixed fast mode. The width-8 workaround is now removed, and the demo uses SERV's stock `serv_rf_top`. [repro/narrow_ebr](repro/narrow_ebr/run.sh) remains as a regression check.

## Build, check, run

```sh
examples/ulx3s/build.sh examples/gallery/serv_shell target/gallery/serv_shell
python3 examples/gallery/check.py serv_shell --emulator
```

**Changing the firmware:** [firmware/build.sh](firmware/build.sh) compiles [firmware/start.S](firmware/start.S), [firmware/lib.c](firmware/lib.c) (software `__mulsi3`/`__divsi3`/`__modsi3`/`__udivsi3`/`__umodsi3`), and [firmware/main.c](firmware/main.c) for `-march=rv32i -mabi=ilp32`, then writes the 16 `fw_<bank>_<lane>.hex` files. They are committed and swappable into a built bitstream without place and route:

```sh
examples/gallery/serv_shell/firmware/build.sh
examples/ulx3s/swap.sh examples/gallery/serv_shell target/gallery/serv_shell
```

**In the emulator's window:**

```sh
ECP5_GUI_SCRIPT=examples/gallery/serv_shell/gui.script \
  target/release/ecp5_gui target/gallery/serv_shell/top.config
```

**On the board:**

```sh
openFPGALoader -b ulx3s target/gallery/serv_shell/top.bit   # SRAM only
```

Emulator speed: 1.29 MHz in fast mode on the remote Linux check (current branch with `8f3673e`): PASS, 1 304 bytes from the marker on, 13.4 million cycles; 2.79 MHz in the window.

**Board:** on 2026-11-17 at 07:30–07:31 UTC, 3 of 3 loads printed the expected 1 304 bytes, `primes below 2000: 303, 3957776 cycles` included: the board counts the same cycles as the emulator and the simulation. The same bitstream (SHA256 `4cb26b746fabdecb39acdee81fbf448a73602533de3debcebb14ddadf9b33a5b`) had also passed 3 of 3 loads at 02:18 UTC, before the emulator fixes. Both runs are in `target/gallery/serv_shell/board_runs.log` and archived in [docs/inventory/LFE5U-85F/hr03_gallery](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).

The window's record of one run (`ECP5_GUI_TRACE`, GUI rebuilt on the branch, summed up by [gui_run.py](../gui_run.py) under `out/screen.lock`):

- 58 s at 60 frames per second (median frame 16.7 ms);
- 2.79 MHz, 150.0 million clocks;
- 3 drawings of the 3D board read back, none with an LED drawn wrong.

The record's `# uart` lines hold the console's text, 846 bytes: the typed `help`, `primes below 1000: 168, 1936733 cycles`, `calc 6*7` → `42` and `[button FIRE1]`.
