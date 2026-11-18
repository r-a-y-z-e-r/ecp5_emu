# HD07 slice: GSR, the reset line and the reset kind of the DSP output registers (D01, D05, D06)

What GSR does to the output registers of a DSP slice when it is really enabled, which line
resets each half of an ALU54B's R, which MULT18 makes each half asynchronous, and what the
output register of a MULT18 without an ALU does under RESETMODE=ASYNC and GSR.

[HD06](../hd06_alu54_reset/README.md) left all of this open:
- **GSR.** A slice has two GSR bits, shared by its ALU54B and both MULT18X18D. Any of the three
  cells at `GSR=DISABLED` sets them. HD06's multipliers asked for DISABLED, so no HD06 slice had
  GSR enabled, whatever its ALU asked for.
- **Halves.** Both MULT18 of an HD06 slice had the same RESETMODE.
- **Reset line.** Every HD06 cell left `REG_OUTPUT_RST` at nextpnr's RST0.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Designs:** [hd07.v](hd07.v) and [hd07s.v](hd07s.v), written by
  [hd07_gen.py](hd07_gen.py), with [hd07.lpf](hd07.lpf); configurations
  [hd07.config](hd07.config), [hd07s.config](hd07s.config).
  - **`GSR=ENABLED` on every ALU54B and MULT18X18D**, so every DSP slice has GSR enabled.
  - **Four ALU54B** in two DSP blocks of row 58 (X 13 and 31), each with its two MULT18X18D
    (no registers). Every ALU computes R = C at every clock (OP `01000111111`). Output and flag
    registers are on CLK0, the 25 MHz clock.

    | ALU | Slice | RESETMODE: ALU, first MULT18, second MULT18 | First MULT18 `REG_OUTPUT_RST` |
    |---|---|---|---|
    | ea | first | SYNC, ASYNC, SYNC | RST0 |
    | eb | second | SYNC, SYNC, ASYNC | RST0 |
    | ec | first | SYNC, SYNC, SYNC | RST2 |
    | ed | second | ASYNC, ASYNC, ASYNC | RST1 |

  - **Four MULT18X18D without an ALU** in the block at X 22, each with its output register on
    CLK0. A = `20001` and B = {0, c[16:1], 0}, so P[16:1] and P[33:18] are c[16:1].

    | MULT18 | Site | RESETMODE | `REG_OUTPUT_RST` |
    |---|---|---|---|
    | ma | MULT18_0 | ASYNC | RST1 |
    | mb | MULT18_1 | SYNC | (no such field) |
    | mc | MULT18_4 | SYNC | RST0 |
    | md | MULT18_5 | ASYNC | (no such field) |

  - **Controls.** The cells of a block share CE0..3 and RST0..3 (nextpnr puts one net on each
    line of a block). The block at X 13 takes `cea`/`rsta`; those at X 22 and X 31 take
    `ceb`/`rstb`.
    - Each CE bit is high 3/4 of the time and each RST bit 1/8.
    - `gsr_n`, low 1/32 of the time, drives a GSR primitive.
    - All are fabric registers, new at every clock from disjoint fields of a 64-bit LFSR.
    - C is another LFSR.
  - **Build.** As in [HI05](../hi05_ioreg/README.md), yosys `setparam -set GSR "DISABLED"
    t:TRELLIS_FF` keeps the fabric registers out of the user GSR. The configurations show
    `GSR ENABLED` for every DSP cell, and the `REG_OUTPUT_RST` and RESETMODE values of the
    tables.
  - **Line.** `M<3840 hex>`: 256 samples, one per clock, taken before the edge. A sample is:
    - `{cea, rsta, ceb, rstb, 000, gsr_n, c[50], c[30], c[20], c[0]}`;
    - R[50], R[30], R[20], R[0] of ea, eb, ec, ed: one bit of each of the four groups of R
      ([HD04](../hd04_alu54_cascade/README.md));
    - `00, c[16], c[1]`, then P[33], P[18], P[16], P[1] of ma, mb, mc, md: two bits of each
      half of P.
  - **hd07** sends the windows one after another. Its first window is on its way before the
    reader opens the port.
  - **hd07s** is the same design but records only the first window, from the first clock after
    configuration, and sends that window again and again.
- **Runner:** [run.py](run.py) `hd07.bit M 3 20` and `hd07s.bit M 3 8`.
- **Analysis:** [hd07_analyse.py](hd07_analyse.py) tries, for each cell and each recorded bit,
  180 models:
  - every enable (none, CE0..3);
  - every reset (none, RST0..3, synchronous or asynchronous);
  - four kinds of GSR:
    - ignored;
    - asynchronous, holding the register through an edge it is active before;
    - asynchronous without that hold;
    - synchronous.

  Output in [analysis.txt](analysis.txt) and [analysis_s.txt](analysis_s.txt).

## Observations

- [observations.log](observations.log): three loads of hd07. Each gave the same 59 windows,
  15 104 samples.
- [observations_s.log](observations_s.log): three loads of hd07s. Each gave the same single
  window.

## Reading

Exactly one model gives every sample of hd07, for each of the 32 bits (8 cells, 4 bits each).
The same models fit hd07s's first window, which is too short to exclude every other.

1. **GSR clears every DSP output register.** This holds for the ALU's R and for a MULT18's P,
   once the slice has GSR enabled:
   - it clears at once;
   - the register stays 0 through every clock edge while GSR is active;
   - it captures again at the first edge after GSR is released.

   With the slice's GSR disabled it does nothing (HD06).
2. **Enable:** the block's CE0 line, in every cell (as HD01 and HD06 found).
3. **Reset line of an ALU54B:**

   | Bits | Line |
   |---|---|
   | R[17:0], R[44:27] | the one the first MULT18's `REG_OUTPUT_RST` selects (RST2 in ec, RST1 in ed) |
   | R[26:18], R[53:45] | RST0, always |

   - The first MULT18's `REG_OUTPUT_RST` is the same two bit pairs as the ALU's
     `REG_OUTPUT0_RST` and `REG_OUTPUT1_RST`, and nextpnr always writes it (RST0 by default).
   - Which of the two ALU fields drives which of R[17:0] and R[44:27] is not separated here:
     nextpnr sets both together.
4. **Which MULT18 makes which half asynchronous:**
   - R[17:0] and R[44:27] are asynchronous when the first MULT18's RESETMODE is ASYNC (ea), or
     the ALU's (HD06);
   - R[26:18] and R[53:45] when the second MULT18's is (eb).

   This confirms HD06's reading, which rested on the shared configuration bits.
5. **A MULT18 without an ALU:**
   - **Reset line.** All of P takes the line `REG_OUTPUT_RST` selects on MULT18_0 and MULT18_4
     (RST1 in ma), and RST0 on MULT18_1 and MULT18_5, which have no such field (as HD01 found).
   - **Reset kind.** `RESETMODE=ASYNC` makes the reset of P asynchronous (ma, md): it clears at
     once and holds through the edges while high. SYNC clears at a clock edge (mb, mc).
6. **After configuration** (hd07s, sample 0): every R and every P read 0 before the first clock
   edge. That is with GSR enabled. A MULT18 output register with GSR disabled was not seen at
   this moment.

**Not covered:**
- the flag register under a reset or GSR (the flags are constants,
  [HD04](../hd04_alu54_cascade/README.md));
- `REG_OUTPUT0_RST` and `REG_OUTPUT1_RST` set to different lines;
- the start value of a MULT18 output register with GSR disabled;
- the C and opcode registers.

## Emulator check

The emulator executes all of this ([DSP_ALU.md](../../../DSP_ALU.md),
[DSP_OUTPUT.md](../../../DSP_OUTPUT.md)):
- **ALU54B.** R[17:0] and R[44:27] take the line REG_OUTPUT0/1_RST selects, and R[26:18],
  R[53:45] take RST0; an enabled GSR clears R at once.
- **MULT18.** The output stage takes an enabled GSR the same way. Under the `configured`
  startup it starts at 0 where the slice has GSR enabled, and stays unknown until it captures
  where GSR is disabled.

[tests/alu54_native.rs](../../../../tests/alu54_native.rs) runs both configurations:
- **Fast mode, hd07s:** it prints the board's first window, from the first clock.
- **Fast mode, hd07:** it prints eight windows. The first is hd07s's, and the other seven are
  among the board's.
- **Event kernel, hd07s:** the first window, from the first clock.
