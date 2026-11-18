# HR06: the SDRAM on the board, against the emulator

Two reference designs that use the ULX3S's 32 MB SDRAM (MT48LC16M16A2), as
[docs/PERIPHERALS_SDRAM.md](../../../PERIPHERALS_SDRAM.md) describes. Both ran on the board on
2026-11-16:
- [sdram_test](../../../../examples/ulx3s/sdram_test/top.v): a plain-Verilog controller at 25 MHz,
  CL 2. It writes an LFSR pattern over 64 KB, reads it back, and prints `SDRAM W<words> E<errors>`.
- [picorv32_sdram](../../../../examples/ulx3s/picorv32_sdram/top.v): PicoRV32 runs its firmware
  from SDRAM, after a boot ROM copies it there. Each pass prints the primes below 2000 with the
  cycles they took, then the result of a 256 KB memtest.

The cycle count is the strict check here. It depends on every SDRAM access, its latency, and when
the controller's refresh counter interrupts the CPU. The emulator must model the controller, the
pads and the chip well enough to land on the same cycle.

## Setup

- **Board:** ULX3S "85K v3.0.8" (LFE5U-85), SRAM load with `openFPGALoader -b ulx3s`, under the
  shared board lock.
- **Runner:** [../hr04_hdmi_oled/run.py](../hr04_hdmi_oled/run.py) `BITSTREAM PREFIX 3 SECONDS`:
  `SDRAM` for 3 s and `pass` for 8 s.
- **Configurations:** [sdram_test.config](sdram_test.config) and
  [picorv32_sdram.config](picorv32_sdram.config), built on the Mac with `examples/ulx3s/build.sh`
  (nextpnr seed 1).
- **Records:** [TOOLS.txt](TOOLS.txt), [SHA256SUMS](SHA256SUMS),
  [observations.log](observations.log).
- **Prediction:** fast mode with the SDRAM model (tests/sdram.rs) printed:
  - [emulator_sdram_test.txt](emulator_sdram_test.txt);
  - [emulator_picorv32_sdram.txt](emulator_picorv32_sdram.txt), passes 0–3, 120 million cycles at
    0.29 MHz on rserv, with 0 timing violations and no refresh shortfall.

## Observations

| Design | Loads | Board | Emulator |
|---|---:|---|---|
| sdram_test | 3 | `SDRAM W8000 E0000`, 108 lines a load | `SDRAM W8000 E0000` |
| picorv32_sdram, pass 1 | 3 | `primes below 2000: 303, 854997 cycles` | `303, 854997 cycles` |
| picorv32_sdram, passes 2–6 | 3 | `303, 855452 cycles` each | passes 2–3: `303, 855452 cycles` |
| picorv32_sdram, memtest | 3 | `0 errors`, every pass | `0 errors`, every pass |

- Pass 0's primes line and the banner come before the serial port opens after a load. The
  emulator's pass 0 took 855498 cycles.
- The passes differ by the phase of the controller's free-running refresh counter when each
  starts. The board shows the same pattern as the emulator, cycle for cycle.
- `tests/sdram.rs` `fast_picorv32_executes_from_sdram` now asserts pass 1 = 854997, the board's
  count. `SDRAM_PASSES=4` runs the passes compared here.
- The window ran picorv32_sdram for 40 s (docs/PERIPHERALS.md): the SDRAM initialised, about a
  million reads, 0 timing violations.
