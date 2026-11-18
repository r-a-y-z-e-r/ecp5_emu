# riscv_shell: a RISC-V computer with a shell

A small computer on the ULX3S:
- the PicoRV32 RISC-V core as RV32IMC; its multiplier sits in the DSP blocks;
- 32 KB of block RAM holding a C program;
- a UART with a receive FIFO;
- the LEDs and the buttons.

The program is a shell. You type commands at 115200 baud: count primes, draw a Mandelbrot set,
evaluate expressions, peek and poke memory, sweep the LEDs. Pressing a button prints an event.

Every computing command prints the clock cycles it took. Those numbers are exact: the board prints
the same cycle counts as the emulator and the RTL simulation, which is the point of an emulator
that runs the real bitstream.

It extends [picorv32_console](../../ulx3s/picorv32_console/top.v):

| | picorv32_console | riscv_shell |
|---|---|---|
| ISA | RV32I | RV32IMC, with `mul`/`div` and compressed instructions |
| RAM | 8 KB | 32 KB |
| Receiver | one byte | 511-byte FIFO |
| Other | — | buttons, a reset register |

## Memory map

| Address | Register |
|---|---|
| `0x0000_0000` | RAM, 32 KB: 16 EBR as 4 banks × 4 byte lanes of 2048 × 8, so each byte strobe is the write enable of its own block. The core starts at 0. |
| `0x0200_0000` | UART data: a write sends a byte; a read takes the oldest received byte from the FIFO |
| `0x0200_0004` | UART status: bit 0 the transmitter is busy, bit 1 the FIFO holds a byte, bit 2 a byte was lost to a full FIFO (sticky), bit 3 a byte is arriving |
| `0x0300_0000` | LEDs, bit 7 = D7 (leftmost) |
| `0x0400_0000` | buttons, debounced for 10 ms; bit 0 PWR … bit 6 RIGHT, 1 = pressed |
| `0x0400_0004` | presses since the last read of this register (reading clears it) |
| `0x0500_0000` | writing `0x005EB007` resets the whole computer as at power-on; RAM keeps its contents |

The core:
- `ENABLE_FAST_MUL` puts the 32 × 32 multiplier in four MULT18X18D;
- `ENABLE_DIV` adds the divider;
- `BARREL_SHIFTER`, a dual-ported register file in LUTRAM, and 64-bit cycle counters (`rdcycle`/`rdcycleh`).

## Commands

```text
help                 list commands
info                 CPU and firmware details
led [hex]            read/set eight LEDs
buttons              debounced button levels
knight [n]           LED sweeps (default 3)
primes [n]           sieve below n (default 10000)
mandel               64x24 fixed-point Mandelbrot
calc <expr>          signed integer + - * / % ()
peek <addr>          read a mapped word
poke <addr> <value>  write a mapped word
dump <addr> [len]    RAM bytes (default 64)
memtest              test free RAM, destroys contents
uptime               seconds.milliseconds since reset
reboot               reset CPU and peripherals
```

Lines can be edited with Backspace. At the prompt, a button press prints `[button FIRE1]` and then
the prompt again, with what was already typed.

`reboot` first copies the bytes still waiting in the hardware FIFO into a queue in RAM that the
reset does not clear (`.persist` in [firmware/link.ld](firmware/link.ld)). A pasted script
therefore carries on after the reboot.

## Expected output

[check.json](check.json) types a script that starts with `reboot`. Part of what comes back
([expected/uart.txt](expected/uart.txt)):

```text
> reboot
RISC-V shell on ULX3S
PicoRV32 RV32IMC | 25000000 Hz | 32768 bytes RAM
firmware: 11328 bytes
> primes 10000
primes below 10000: 1229, 1305904 cycles
> calc (7+5)*3-10/4
34
> mandel
                    .........................
                ......................*-.........
             ........................::=--+.........
          ..........................::-@%=-:...........
         ........................:::-+@@@@*-:...........
…
mandel: 1025262 cycles
> memtest
memtest [0x000032cc, 0x00007b70): OK, 902055 cycles
```

`mandel` computes in fixed point with the hardware multiplier: 1 025 262 cycles, 41 ms at 25 MHz.

## Resources

nextpnr for the LFE5U-85F, seed 1:
- 4 140 LUT4 and 1 208 TRELLIS_FF;
- 32 TRELLIS_RAMW (the register file);
- 17 DP16KD: 16 for the RAM, 1 for the receive FIFO;
- 4 MULT18X18D.

Fmax is 58.5 MHz for the 25 MHz clock.

Two details of the RTL come from how block RAM behaves:
- **The FIFO is never read in the clock its write port writes the same row.** On the chip such a
  read returns undefined data, and the emulator makes it unknown. The FIFO holds 511 bytes, so
  its read and write addresses meet only while it is empty, and the read skips that case.
- **Every RAM bank is read once during the reset.** This design found an emulator gap: an EBR
  output latch not yet read since configuration was unknown, and fast mode, which starts only once
  everything it reads is known, never started ([repro/ebr_unread/run.sh](repro/ebr_unread/run.sh)).
  - The board answered the question: every EBR latch and output register reads 0 until its port
    reads ([HB02](../../../docs/inventory/LFE5U-85F/hb02_ebr_start)).
  - The configured startup now starts them at 0. That is on `dev` since commit `d4f83ec`.
  - The read during reset is no longer needed, and is kept: it does no harm, and it keeps the
    design starting in fast mode under the `ready-unknown` startup too.

## Build and check

```sh
examples/ulx3s/build.sh examples/gallery/riscv_shell target/gallery/riscv_shell
python3 examples/gallery/check.py riscv_shell --emulator
```

**The firmware.**
- [firmware/build.sh](firmware/build.sh) compiles [firmware/main.c](firmware/main.c) for `rv32imc`
  with Homebrew's clang and writes the 16 images `fw_<bank>_<lane>.hex`.
- They are committed, so the design builds without a RISC-V compiler.
- After a change to the firmware, [swap.sh](../../ulx3s/swap.sh) puts it into the built bitstream in
  seconds, without place and route:

```sh
examples/gallery/riscv_shell/firmware/build.sh
examples/ulx3s/swap.sh examples/gallery/riscv_shell target/gallery/riscv_shell
```

**The expected text** comes from a Verilator simulation of [top.v](top.v) with the same firmware
images: [sim/run.sh](sim/run.sh), 16 000 000 clocks. The emulator prints the same 3 214 bytes from
`reboot` on, cycle counts included. Its fast mode ran at 0.2–0.4 MHz on the command line while the
machine was busy with other builds (load average 50), and at 0.85 MHz in the window.

## In the emulator's window

```sh
ECP5_GUI_SCRIPT=examples/gallery/riscv_shell/gui.script \
  target/release/ecp5_gui target/gallery/riscv_shell/top.config
```

Type into the console under the board. [gui.script](gui.script) does the following:
- types `help`, `led 5a`, `calc 6*7` and `mandel`;
- starts typing `calc 1+`, then presses FIRE1 and RIGHT;
- finishes the line;
- runs `knight 2` and `primes 2000`.

The window's record of one run (`ECP5_GUI_TRACE`, summed up by [gui_run.py](../gui_run.py)):
- 60 s at 60 frames per second (median frame 16.7 ms);
- 0.85 MHz on three threads;
- 36 drawings of the 3D board read back from the GPU, none with an LED drawn wrong. They show
  `led 5a` (01011010) and the `knight` sweep.

The record's `# uart` lines hold the console's text, 2 555 bytes. Among them:

```text
> mandel
…
mandel: 1025262 cycles
> calc 1+
[button FIRE1]
> calc 1+
[button RIGHT]
> calc 1+2
3
> knight 2
done
> primes 2000
primes below 2000: 303, 243162 cycles
```

`mandel` took 1 025 262 cycles in the window, the same count as on the board and in the
simulation. Each button event comes in the middle of a typed line, and the line is printed again
with what was already typed.

## On the board

```sh
openFPGALoader -b ulx3s target/gallery/riscv_shell/top.bit   # SRAM only
```

Then open a terminal at 115200 baud on `/dev/cu.usbserial-…` and press Enter. The banner is printed
within 10 ms of the load, before a terminal can be open (the FT231X carries both JTAG and the UART),
so type `reboot` to see it.

`python3 examples/gallery/check.py riscv_shell --board` loads the bitstream three times, types the
script of check.json each time, and compares what comes back.

**Board:** on 2026-11-16, 3 of 3 loads printed the expected 3 214 bytes, the cycle counts of
`primes`, `mandel` and `memtest` included. Records are in
[docs/inventory/LFE5U-85F/hr03_gallery](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).
