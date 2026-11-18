# HI10 slice: the IO latches (INREGMODE/OUTREGMODE=LATCH, I02)

What an IOLOGIC register does in latch mode. `FF.INREGMODE` (input register) and
`OUTREG.OUTREGMODE` (output register) are `FF` or `LATCH`, one bit each in Trellis. The open
toolchain cannot produce a latch: yosys has no cell for Lattice's IO latches (`IFS1S1*`, `FL1*`
are commented out in its ECP5 library). The emulator refused them ("IOLOGIC latch is not
implemented").

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hi10.v](hi10.v) with [hi10.lpf](hi10.lpf).
  - **Registers.** Four IO registers on the LEDs, all on one `sclk` = 25 MHz / 4 made by a
    fabric divider (high for two 25 MHz cycles, low for two). All take `SP` = `ce` and
    `CD` = `rst`, with `GSR` disabled.

    | Register | LED | Primitive | hi10_ff | hi10 |
    |---|---|---|---|---|
    | O0 | B2 | OFS1P3DX | FF | LATCH |
    | O1 | C2 | OFS1P3DX | FF | FF |
    | I0 | D1 | IFS1P3DX | FF | LATCH |
    | I1 | E2 | IFS1P3DX | FF | FF |

  - **Builds.**
    - [hi10_ff.config](hi10_ff.config) is nextpnr's output.
    - [hi10.config](hi10.config) is the same with the latch bit set on O0
      (`IOLOGICC.OUTREG.OUTREGMODE` in tile `MIB_R37C0:PICL2`, B2 = R35 PIO C) and on I0
      (`IOLOGICB.FF.INREGMODE` in `MIB_R38C0:PICL0`, D1 = R38 PIO B), by
      [hi10_latch.py](hi10_latch.py).
    - O1 and I1 stay flip-flops in both builds, as the control.
  - **Data and controls.** The output registers take `dout` and are read back through their
    `BB`. The fabric drives the input registers' pads with `din`. `din`, `dout`, `ce` (high 3/4)
    and `rst` (high 1/8) are fabric registers stepped by a 16-bit LFSR. They change only at the
    25 MHz edges between `sclk` edges, so every half period of `sclk` sees one change of each.
  - **Lines.** Sample = `{sclk, div0, ce, rst, din, dout, ob1, ob0, qi1, qi0, 0, 0}` at every
    25 MHz edge, taken before the edge. Each line `E<id> <768 hex>` is 256 samples: E00 from the
    start, E01–E07 consecutive windows, then E00 again.
- **Runner:** [run.py](run.py) `hi10_ff.bit E 3 3` and `hi10.bit E 3 3`: three loads of each,
  3 s each.
- **Model check:** [hi10_model.py](hi10_model.py), output in [model_check.txt](model_check.txt).

## Observations

[observations_ff.log](observations_ff.log) and [observations.log](observations.log): three loads
of each build, 39 distinct lines per build, the same in all three loads.

## Reading

1. **Control.** In hi10_ff all four registers follow the HI05 register model with an
   asynchronous clear: 39 396 values compared, 0 differ. The setup and the recording are sound.
2. **Latch.** In hi10, O0 and I0 are latches, and they behave alike: 39 530 values compared,
   0 differ.
   - **Transparent** while the clock is **low** and CE is high. In the ECP5 register the
     master latch is open while the clock is low, which is why a flip-flop captures at the
     rising edge. Latch mode shows the master.
   - **Holding** its output otherwise. That includes an edge where D or CE change as the
     latch closes, and an edge where `rst` falls as the latch closes: the output stays 0. The
     records hold 2 331 closing edges, 52 of them with `rst` falling.
   - **Clear.** `CD` (LSR, `SRMODE=ASYNC`) clears it at once and holds it at 0.
3. **Rejected variants.**

   | Variant | Differ |
   |---|---|
   | O0/I0 as flip-flops | 4 523 |
   | transparent while the clock is high | 3 304 |
   | transparent while the clock is low, CE ignored | 2 040 |
   | at a closing edge, D before the edge instead of the held output | 82 |
   | at a closing edge, D after the edge | 1 310 |

**Not covered:**
- `REGSET=SET` latches;
- `SRMODE=LSR_OVER_CE` and `GSR=ENABLED` with a latch;
- the tristate register in latch mode (`TSREG.OUTREGMODE`);
- inverted clocks (`CLK*MUX=INV`).

## Emulator check

The emulator now executes IO latches as above
([compile/iologic.rs](../../../../src/compile/iologic.rs), `IoPath::Latch` in
[kernel/iologic.rs](../../../../src/kernel/iologic.rs)).
- **Event kernel.** The latch is re-evaluated whenever D, CE, CLK or LSR changes, from their
  values after the change.
- **Fast mode.** The latch is one line in `asynchronous()`, which runs after every input
  change and every capture. It follows D while CLK is low and CE high, and LSR wins.
- **Refused.** `SRMODE=LSR_OVER_CE`, `GSR=ENABLED` and `REGSET=SET` latches are refused, as is
  the tristate register in latch mode. None of these was measured.

Tests:
- **[tests/io_latch.rs](../../../../tests/io_latch.rs)** checks the rules on a synthetic plan.
  It also runs fast mode against the event kernel for 30 000 random steps. In those steps a
  latch's clock, CE and D move together with another clock domain's edge.
- **[tests/io_latch_native.rs](../../../../tests/io_latch_native.rs)** runs both
  configurations here.
  - **Fast mode:** every line it prints, for hi10_ff and hi10, is a board line, except the
    first round's E01. The board prints that line before [run.py](run.py) opens the port, as
    in HI05 and HI09.
  - **Event kernel:** its E00 of hi10 is the board's.
- **Rejected shortcut.** A hold register plus multiplexers, built in the plan, was rejected:
  it has a zero-delay race. When `rst` falls with the closing edge, the output multiplexer
  sees the new `rst` and the old "open" for one wave.
