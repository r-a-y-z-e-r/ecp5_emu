# HP02 slice: pseudo-differential LVCMOS33D output (prepared, not yet run)

A slice of the planned series **HP02** (differential pairs, question P03 in
[PIO_BANKS.md](../../../PIO_BANKS.md)). The design is ready; it has **not been run on the board
yet**.

## Why it needs wires

The FPGA cannot read back either pad of an `LVCMOS33D` pair:
- nextpnr accepts a top-bank `LVCMOS33D` only as an `OUTPUT` on pad A;
- the configuration leaves the input buffer of the complement pad B off.

Two wires on the ULX3S J1 header bring both pads to two input pins instead.

## Design

[hp02.v](hp02.v) with [hp02.lpf](hp02.lpf), built into [hp02.config](hp02.config):
- **Output.** An `ODDRX1F` drives `gp[3]` (ball B9, R0C58 PIOA) with `IO_TYPE=LVCMOS33D DRIVE=4`,
  as ULX3S HDMI designs do. Its complement is `gn[3]` (C10, R0C58 PIOB). The DDR data, the LFSRs
  and `sclk` are those of [HI02](../hi02_iologic/README.md), so pad A must repeat the HI02 board
  samples.
- **Tristate.** A fabric tristate releases the pair for samples `t = 1156..1211`.
- **Inputs.** `gp[4]` (A7) and `gn[4]` (A8), with `PULLMODE=DOWN`. Without the wires they read 0.
- **Report.** Over the FTDI UART: `P=` followed by 256 samples of `{gp4, gn4}` for
  `t = 1024..1279`.

**Wires, per the ULX3S schematic `gpio.sch`.** On J1, each pair index sits at one position across
the two rows: `gpN` in one row, `gnN` in the other. Counted from the pin-1 end, the positions
are 3.3 V, GND, pair 0, pair 1, pair 2, pair 3, pair 4. The two wires join positions "pair 3" and
"pair 4" along each row. A wire across the two rows would short the two complementary outputs.

[run.py](run.py) loads the bitstream into SRAM and reads the lines. [hp02_analyse.py](hp02_analyse.py)
checks:
- pad A against the HI02 ODDR model;
- pad B against the inverse of pad A;
- both pads released during the tristate window.

On the emulator's own line, with the wires modelled as board wires, the checks pass 200 of 200.
