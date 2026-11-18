# HI09 slice: RST and GSR on the X1 DDR paths (I02)

How ODDRX1F and IDDRX1F take their reset (`RST`) and the global reset.
[HI02b](../hi02_iologic/README.md) measured `RST` with `GSR=DISABLED` around one reset pulse: an
asynchronous clear of every DDR stage. The emulator refused `GSR=ENABLED` ("IOLOGIC GSR=ENABLED
is not measured for MODE=IDDRX1_ODDRX1"). nextpnr writes `GSR=ENABLED` when the primitive's `GSR`
parameter says so; Lattice's own default for these primitives is `ENABLED`. With no parameter,
nextpnr writes `DISABLED`.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hi09.v](hi09.v) with [hi09.lpf](hi09.lpf); configuration
  [hi09.config](hi09.config).
  - **Paths.** Four X1 DDR paths on the LEDs, all on one `sclk` = 25 MHz / 4 made by a fabric
    divider: high for two 25 MHz cycles, low for two.

    | Path | LED | Primitive | GSR | IOLOGIC MODE |
    |---|---|---|---|---|
    | O0 | B2 | ODDRX1F | ENABLED | IDDRX1_ODDRX1 |
    | O1 | C2 | ODDRX1F | DISABLED | IDDRX1_ODDRX1 |
    | I0 | D1 | IDDRX1F | ENABLED | IDDRX1_ODDRX1 |
    | I1 | E2 | IDDRX1F | DISABLED | IDDRX1_ODDRX1 |

    nextpnr writes `LSRMUX=LSR`, `SRMODE` left at `ASYNC`, and `LSRIMUX=LSRMUX` on the inputs.
  - **Data.** The ODDRs take D0/D1 from two 7-bit LFSRs stepped at `sclk` rises; each pad is
    read back through its `BB`. The fabric drives both IDDR pads with one bit `din`.
  - **Controls.**
    - `rst` drives the `RST` of all four paths.
    - `gsr_n` drives a `GSR` primitive.
    - Both are fabric registers stepped by a 16-bit LFSR, as is `din`: `rst` is high 1/8 of
      the time, `gsr_n` low 1/32.
    - All three change only at the 25 MHz edges between `sclk` edges, so a change that shows
      in the next sample acted at once.
    - Everything is idle for the first 16 cycles.
  - **Build.** As in [HI05](../hi05_ioreg/README.md), yosys `setparam -set GSR "DISABLED"
    t:TRELLIS_FF a:syn_useioff %d` keeps the divider, the LFSRs, the recorder and the UART out
    of the user GSR.
  - **Lines.** Sample = `{sclk, rst, gsr_n, din, d0, d1, ob1, ob0, q0e, q1e, q0d, q1d}` at every
    25 MHz edge, taken before the edge (`ob` is a `[1:0]` vector, so O1 comes first). Each line
    `E<id> <768 hex>` is 256 samples: E00 from the start, E01–E07 consecutive windows, then E00
    again.
- **Runner:** [run.py](run.py) `hi09.bit E 3 3`: three loads, 3 s each.
- **Model check:** [hi09_model.py](hi09_model.py), output in [model_check.txt](model_check.txt).

## Observations

[observations.log](observations.log): three loads, 44 or 45 lines each. All three loads gave
the same 39 distinct lines: one E00 and 38 windows.
- **Pulses.** The lines hold 467 `rst` pulses and 105 GSR pulses.
- **GSR mattered.** In 483 samples the GSR=ENABLED pair (O0, I0) differs from the GSR=DISABLED
  pair (O1, I1).

## Reading

The model below gives every compared sample of every line: 58 484 values, 0 differ. E00 is
compared from the configured state (every stage 0). Each window starts unknown and is compared
from the moment the model knows the value.

1. **Data paths.** They are HI02's ([kernel/iologic.rs](../../../../src/kernel/iologic.rs)):
   - ODDRX1F: at a `sclk` rise, s0 ← D0, s1 ← D1, s2 ← s0, s3 ← s1, s4 ← s3 and Q ← s2; at a
     fall, Q ← s4.
   - IDDRX1F: at a rise, Q0 ← s0, Q1 ← s1 and s0 ← D; at a fall, s1 ← D.
2. **RST** clears every stage and the outputs at once, and holds them clear while high. The
   next `sclk` edge after it falls starts from zero. This confirms HI02b over 467 pulses at
   every phase of `sclk`.
3. **GSR** (user GSR low) does the same on the `GSR=ENABLED` paths only. A `GSR=DISABLED` path
   ignores it.

**Rejected variants.**

| Variant | Differ |
|---|---|
| reset at the next `sclk` edge, not at once | 739 |
| reset clears the outputs only, not the stages | 5 812 |
| GSR also on the `GSR=DISABLED` paths | 560 |

**Not covered:**
- the X2 and 7:1 gearboxes with `GSR=ENABLED` (their `RST` comes from CLKDIVF, HI03/HI07);
- a clock enable and `SRMODE=LSR_OVER_CE` on DDR paths: ODDRX1F/IDDRX1F have no such
  parameter or port, so the open toolchain cannot produce them;
- the top/bottom IOLOGIC (only side IOLOGIC on the LED bank was run).

## Emulator check

The emulator now executes `GSR=ENABLED` on the X1 DDR paths. The GSR acts like the path's
`RST`: an asynchronous clear of every stage and output
([compile/iologic.rs](../../../../src/compile/iologic.rs),
[kernel/iologic.rs](../../../../src/kernel/iologic.rs)). `RST` itself was already executed so,
from HI02b.
[tests/ddr_reset_native.rs](../../../../tests/ddr_reset_native.rs) runs
[hi09.config](hi09.config):
- **Fast mode** runs two rounds of eight lines. Every line it prints is a board line, except
  the first round's E01. The board prints that line about 0.13 s after the load, before
  [run.py](run.py) opens the port, as in HI05.
- **The event kernel** runs the start, and its E00 is the board's.
