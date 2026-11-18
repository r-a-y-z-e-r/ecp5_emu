# HD06 slice: the clock enable, the reset and GSR of the ALU54B output register (D01, D05, D06)

Which CE and which RST the ALU54B output register takes, whether its reset is synchronous, and
what a user GSR does to it. The primitive has CE0..3, RST0..3 and parameters that choose among
them. The bitstream has no CE choice for this register, and nextpnr does not write its reset
choice. Until now the emulator required every ALU reset to stay low.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Designs:** [hd06.v](hd06.v) and [hd06b.v](hd06b.v), written by
  [hd06_gen.py](hd06_gen.py), with [hd06.lpf](hd06.lpf); configurations
  [hd06.config](hd06.config), [hd06b.config](hd06b.config).
  - **Four ALU54B** in two DSP blocks of row 58 (X 13 and 31), each with its two MULT18X18D.
    Every ALU computes R = C at every clock (OP `01000111111`); output and flag registers on
    CLK0, the 25 MHz clock.
  - **Controls.** A block's two ALUs share CE0..3 and RST0..3: a DSP block has four lines of
    each for both slices, and nextpnr refuses more than four distinct nets. The first block
    takes `cea`/`rsta`, the second `ceb`/`rstb`.
    - Each CE bit is high 3/4 of the time and each RST bit 1/8.
    - `gsr_n`, low 1/32 of the time, drives a GSR primitive.
    - All are fabric registers, new at every clock from disjoint fields of a 64-bit LFSR.
    - C is another LFSR.
  - **Variants** (RESETMODE of the ALU, GSR of the ALU, RESETMODE of its two MULT18):

    | ALU | Slice | HD06 | HD06b |
    |---|---|---|---|
    | ea | first | SYNC, DISABLED, SYNC | ASYNC, DISABLED, SYNC |
    | eb | second | ASYNC, DISABLED, SYNC | SYNC, DISABLED, SYNC |
    | ec | first | SYNC, ENABLED, SYNC | SYNC, DISABLED, ASYNC |
    | ed | second | ASYNC, ENABLED, SYNC | ASYNC, DISABLED, ASYNC |

  - **Build.** As in [HI05](../hi05_ioreg/README.md), yosys `setparam -set GSR "DISABLED"
    t:TRELLIS_FF` keeps the fabric registers out of the user GSR.
  - **Line.** `M<2560 hex>`: 256 samples, one per clock, taken before the edge. A sample is
    `{cea, rsta, ceb, rstb, 000, gsr_n, c[50], c[30], c[20], c[0]}` and R[50], R[30], R[20],
    R[0] of ea, eb, ec, ed: one bit of each of the four groups of R
    ([HD04](../hd04_alu54_cascade/README.md)). Windows follow one another.
- **Runner:** [run.py](run.py) `hd06.bit M 3 12` and `hd06b.bit M 3 12`.
- **Analysis:** [hd06_analyse.py](hd06_analyse.py) tries, for each ALU and bit, every enable
  (none, CE0..3), every reset (none, RST0..3, synchronous or asynchronous) and GSR clearing or
  not: 90 models. Output in [analysis.txt](analysis.txt) and [analysis_b.txt](analysis_b.txt).

## Observations

[observations.log](observations.log) and [observations_b.log](observations_b.log): three loads
of each build. Each build gave the same 53 windows in all three loads, 13 568 samples.

## Reading

Exactly one model gives every sample, for each of the 32 bits (4 ALUs, 4 groups, 2 builds).

1. **Enable:** the block's CE0 line, always.
2. **Reset:** the block's RST0 line, always. It wins over the enable.
3. **Synchronous or asynchronous**, per half of the output bank:

   | Bits | Asynchronous when |
   |---|---|
   | R[17:0], R[44:27] | RESETMODE of the ALU **or** of the slice's first MULT18 is ASYNC |
   | R[26:18], R[53:45] | RESETMODE of the slice's second MULT18 is ASYNC |

   - **Pitfall.** An ALU with `RESETMODE=ASYNC` beside multipliers left at SYNC resets half of
     R at once and the other half at the next clock edge (eb in HD06, ea in HD06b).
   - **Synchronous:** R is cleared at a clock edge with RST0 high, whatever CE0 is.
   - **Asynchronous:** R is cleared as soon as RST0 is high and stays 0 through the edges while
     it is.
4. **GSR did nothing** to the output register in these builds: 416 samples with the user GSR
   low changed nothing in any ALU.
   - **Correction (2026-11-12): this says nothing about `GSR=ENABLED`.** The two GSR bits of a
     slice are shared by its ALU54B and both MULT18X18D, and any of the three at DISABLED sets
     them. The multipliers here ask for `GSR=DISABLED`, so ec and ed had the bits at DISABLED
     on the chip, whatever the ALU asked for. With GSR really enabled it clears R at once
     ([HD07](../hd07_dsp_gsr/README.md)).
5. **Not separated here:** both MULT18 of a slice had the same RESETMODE in every build, so which
   of the two makes R[26:18] and R[53:45] asynchronous was read from the shared configuration
   bits. [HD07](../hd07_dsp_gsr/README.md) measured it: the second MULT18, as read here.
6. **Reset line:** RST0 in every build here, where nextpnr left `REG_OUTPUT_RST` at RST0. With
   another selection R[17:0] and R[44:27] follow it ([HD07](../hd07_dsp_gsr/README.md)).

**Not covered:**
- the reset of the flag register (the flags are constants, [HD04](../hd04_alu54_cascade/README.md));
- the C and opcode registers;
- a reset line other than RST0 and GSR really enabled: both are in [HD07](../hd07_dsp_gsr/README.md).

## Emulator check

The emulator executes this reset ([DSP_ALU.md](../../../DSP_ALU.md)).
[tests/alu54_native.rs](../../../../tests/alu54_native.rs) runs both configurations in fast
mode: the windows it prints are the board's.
