# HB02 slice: the EBR output latch after configuration (B02)

What an EBR port's output shows after configuration, before the port has read.

The emulator took it as unknown, so fast mode would not start until every latch it reads had
read once. This occurred in a PicoRV32 design whose upper memory banks stay
unread at start: fast mode never started and the run stayed in the event kernel.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`, under `out/board.lock`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hb02.v](hb02.v), written by [hb02_gen.py](hb02_gen.py), with [hb02.lpf](hb02.lpf);
  configuration [hb02.config](hb02.config).
  - **Memory.** Eight DP16KD at word address 5, every word all ones but the first INITVAL chunk
    (`0x000A5`), so a latch that showed the memory would read ones.
  - **Controls.** WEA = WEB = 0, RST low, OCE high, CEB low, CLKA and CLKB on the 25 MHz
    clock.

    | EBR | DATA_WIDTH_A | REGMODE | GSR | RESETMODE | CEA |
    |---|---|---|---|---|---|
    | e0 | 9 (and B 18, as yosys maps a 2048×8 memory) | NOREG | DISABLED | ASYNC | 0 |
    | e1 | 9 | NOREG | ENABLED | ASYNC | 0 |
    | e2 | 9 | OUTREG | DISABLED | ASYNC | 0 |
    | e3 | 18 | NOREG | DISABLED | ASYNC | 0 |
    | e4 | 36 (PDPW16KD) | NOREG | DISABLED | ASYNC | 0 |
    | e5 | 1 | NOREG | DISABLED | ASYNC | 0 |
    | e6 | 9 | NOREG | DISABLED | SYNC | 0 |
    | e7 | 9 | NOREG | DISABLED | ASYNC | 1 (the control: reads at every clock) |

  - **Line.** `B<448 hex>`: the first 16 samples of e0 DOA[8:0] and DOB[8:0], e1, e2, e3
    DOA[17:0], e4 DOA and DOB, e5, e6 and e7, one per clock from the first, taken before the
    edge. The window is sent again and again.
- **Runner:** [run.py](run.py) `hb02.bit B 3 6`, twice.
- **Check:** [hb02_check.py](hb02_check.py), output in [check.txt](check.txt).

## Observations

- **Records.** [observations.log](observations.log) and [observations_2.log](observations_2.log)
  hold six loads.
- **Lines.** Three gave about 150 lines each, all the same single line. Three gave none in
  their six seconds: the reader opened the port while the FTDI was still settling after the
  JTAG load.

## Reading

1. **Every unread port reads 0** from the first clock on, and stays 0 while it does not read.
   That holds in every build here:
   - every width (1, 9, 18 and 36) and both ports;
   - NOREG and OUTREG, GSR enabled and disabled, RESETMODE SYNC and ASYNC.

   It is neither the word at the address (all ones) nor word 0 (`A5`).
2. **A reading port** (e7) shows 0 before the first edge and its word from the first edge on.

**Not covered:**
- a GSR pulse or a reset before the first read (both clear to 0 anyway);
- the ready-unknown and wake-phases startups, which keep their unknowns by design.

## Emulator check

Under the `configured` startup the planner starts every read latch and output register of an
EBR at 0 ([EBR_EXECUTION.md](../../../EBR_EXECUTION.md)).

[tests/ebr_start_native.rs](../../../../tests/ebr_start_native.rs) runs
[hb02.config](hb02.config):
- **Fast mode** starts at once and prints the board's line.
- **The event kernel** prints the board's line.

The `demos` repro (riscv_shell without its reset-time reads) now starts fast mode at once and
runs its 2 000 cycles there.
