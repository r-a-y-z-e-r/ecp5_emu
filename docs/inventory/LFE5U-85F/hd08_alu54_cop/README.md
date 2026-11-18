# HD08 slice: the C registers and the opcode registers of the ALU54B (D01, D05)

What the ALU54B's C registers (`REG_INPUTC0/1_CLK`) and opcode registers (`REG_OPCODE*`) do:
- **Bits.** Which bits each register holds.
- **Clocking.** On which clock edges it captures.
- **Control.** Which enable and reset it takes, and what GSR does to it.

Until now the emulator refused all of them. [HD03](../hd03_alu54_inputs/README.md) had asked
for C registers, but on one cell of each pair only, which (as found here) leaves them off.

Along the way this series found what Z = 7 reads. It also found how nextpnr ties constant
DSP inputs, and which of its routes reach the chip.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Designs:** written by [hd08_gen.py](hd08_gen.py) (`hd08.v`, and the variants `s`, `b`, `c`)
  and [hd08z_gen.py](hd08z_gen.py), with [hd08.lpf](hd08.lpf). Each `.config` is next to its
  `.v`.
- **hd08** ([hd08.v](hd08.v), [hd08.config](hd08.config)): eleven ALU54B in six DSP blocks of
  row 58 (X 13 to 58), each with its two MULT18X18D.
  - Output and flag registers are on CLK0, the 25 MHz clock. Every cell has `GSR=ENABLED`.
  - **Controls.** CE0..3 and RST0..3 are shared by every block, and a GSR primitive is on
    `gsr_n`. All are random fabric registers: each CE high 3/4 of the time, each RST 1/8,
    `gsr_n` low 1/32.
  - **Seven opcode ALUs** (o1..o7) have constant operands and an OP that is new at every
    clock.
    - Each has one opcode register on, on lines other than CE0/RST0. o7 has all six on.
    - OP[3:2] and OP[10:7] are random. The other bits are random only in windows where no
      mix of old and new bits can select R or CIN; that mode moves on with each window.
    - The record has R[15:0] of each.
  - **Four C ALUs** (c1..c4) take C new at every clock, on the ALU's pins and on the MULT18 C
    ports. They record four bits of R each:

    | ALU | Operation | ALU REG_INPUTC0/1_CLK | MULT18 REG_INPUTC_CLK | RESETMODE |
    |---|---|---|---|---|
    | c1 | R = C | CLK0, CLK0 | CLK0, CLK0 | SYNC |
    | c2 | R = A (top 18 bits: the first MULT18's C) | CLK0, CLK0 | CLK0, CLK0 | ASYNC |
    | c3 | R = C | CLK0, NONE | CLK0, NONE | SYNC |
    | c4 | R = C | CLK0, CLK0 | NONE, NONE | SYNC |

  - **Line.** `M<5120 hex>`: 128 samples of 160 bits, one per clock, taken before the edge;
    the layout is in [hd08_gen.py](hd08_gen.py).
- **hd08s** ([hd08s.v](hd08s.v), [hd08s.config](hd08s.config)): hd08 recording only the first
  window, from the first clock, sent again and again.
- **hd08b** ([hd08b.v](hd08b.v), [hd08b.config](hd08b.config)): hd08 without IN_1.
  - o4 has the three first stages (IN_0, OP0_0, OP1_0).
  - o7 has IN_0 and both stages of OP0 and OP1.
- **hd08bs** ([hd08bs.v](hd08bs.v), [hd08bs.config](hd08bs.config)): hd08b recording only its
  first window, as hd08s does, for the event kernel's check.
- **hd08c** ([hd08c.v](hd08c.v), [hd08c.config](hd08c.config)): hd08b with the DSP data
  constants on nets.
  - Each constant is xored with z = &r, which the control LFSR never reaches.
  - The ALU's C[17:0] and C[44:27] take the nets of the MULT18 C inputs.
- **hd08z** ([hd08z.v](hd08z.v), [hd08z.config](hd08z.config)): eight ALUs computing R = Z
  with Z = 7, with different RNDPAT and GSR. Line `Z<n><14 hex>` per ALU.
- **Runner:** [run.py](run.py): `hd08.bit M 3 25`, `hd08b.bit M 3 25`, `hd08c.bit M 3 25`,
  `hd08s.bit M 3 8`, `hd08bs.bit M 3 8`, `hd08z.bit Z 3 3`.
- **Analysis:**
  - [hd08_analyse.py](hd08_analyse.py) simulates each ALU from the record with the measured
    output register (HD06, HD07). For each register it tries every model:
    - every set of OP fields it holds;
    - a clock on every edge or on every other edge (a divider, reset by GSR or not);
    - every enable and reset, sync or async, and GSR.

    The ALUs with several registers are checked with the stages found. Output in
    [analysis.txt](analysis.txt), [analysis_s.txt](analysis_s.txt),
    [analysis_b.txt](analysis_b.txt), [analysis_bs.txt](analysis_bs.txt) and
    [analysis_c.txt](analysis_c.txt).
  - [hd08z_model.py](hd08z_model.py) checks hd08z, in [model_check_z.txt](model_check_z.txt).

## Observations

Three loads of each build gave the same lines, every one:
- hd08, hd08b, hd08c: 55 windows, 7 040 samples each;
- hd08s, hd08bs: one window each;
- hd08z: eight lines.

The records are [observations.log](observations.log), [observations_s.log](observations_s.log),
[observations_b.log](observations_b.log), [observations_bs.log](observations_bs.log),
[observations_c.log](observations_c.log) and [observations_z.log](observations_z.log).

## Reading

1. **Z = 7 reads ~RNDPAT & (RNDPAT − 1)**, the mask of RNDPAT's trailing zeros (hd08z, all
   eight):

   | RNDPAT | Z = 7 |
   |---|---|
   | 0 (GSR enabled and disabled) | 3FFFFFFFFFFFFF |
   | 0123456789ABC5 | 0 |
   | 10, 100, 2 | F, FF, 1 |
   | 20000000000000 | 1FFFFFFFFFFFFF |
   | 3FFFFFFFFFFFF0 | F |

   HD02..HD05 had an odd RNDPAT, so Z = 7 read zero there.
2. **Opcode registers**, one model each (hd08 o1, o2, o3, o5, o6; hd08b and hd08c o4, o7):

   | Register | Holds | CE, RST | Reset kind |
   |---|---|---|---|
   | IN_0 | OP[6:0] | its own fields | RESETMODE of the ALU |
   | OP0_0, OP0_1 | OP7 | their own fields | the same |
   | OP1_0, OP1_1 | OP[10:8] | OP0_0's, OP0_1's fields | the same |

   - **Rate and GSR.** Each captures at every edge of its clock, and GSR clears it.
   - **Stages.** The second stage follows the first (o7 in hd08b, hd08c: 0 of 6 225 clocks
     differ), and the three first stages together work as one (o4: 0 of 6 208).
   - **Shared select bits.** OP1 has no CE or RST field of its own, and it shares its clock
     select bits with OP0. With OP1_0 on and OP0_0 at NONE, OP7 still came straight through
     (o3).
   - **IN_1 is not understood.** Alone (o4 of hd08) and after IN_0 (o7), its ALU's R follows
     no model in 5–6% of the clocks: 336 of 5 871 and 237 of 5 298. The bits whose change
     would explain them are mostly Y = OP[3:2], sometimes OP1. On quiet clocks (every CE high,
     no reset) the same happens.
3. **C registers** (c1..c4):
   - **Contents.** C0 holds C[26:0] and the top 18 bits of A, the first MULT18's C (c2, c3).
     C1 holds C[53:27].
   - **Every other edge.** A divider toggles at each edge of the register's clock, and the
     register captures only at the edges before which the divider is 0. GSR holds the divider
     at 0. The phase after configuration did not show in hd08s: CE0 was low and RST0 high in
     its first clocks, and GSR came at the tenth.
   - **Controls.** The enable is CE0 and the reset RST0. The reset is synchronous with
     RESETMODE SYNC (c1, c3) and asynchronous with ASYNC on the ALU and both MULT18 (c2).
     GSR clears it.
   - **On only where both ask.** The ALU's `REG_INPUTCn_CLK` and the MULT18's
     `REG_INPUTC_CLK` are the same select bits, with a bypass bit set by NONE. With the ALU at
     CLK0 and the MULT18 at NONE the register is off (c4), and so it was in HD03.
4. **After configuration** every R and every C register reads 0 (hd08s).
5. **What nextpnr does with constant DSP inputs:**
   - **Shared wires.** The ALU's C[17:0] and C[44:27] are the same CIB wires as the MULT18 C
     inputs (the DSP C lines JDSPCn feed both). Tied to two different constants, nextpnr
     writes the wire's constant mux once for each value, ecppack sets the bits of both, and
     the chip takes 0. The opcode ALUs of hd08 and hd08b saw C[15:0] = CM0 & CALU (`4291`
     instead of `C6B1`); hd08c, with nets, saw the MULT18 constant.
   - **A floating pin.** In hd08b nextpnr left C18 of o7, a live C pin, with neither a route nor
     a constant (`R58C65:JC6`).
   - **The JROC route.** nextpnr may route an ALU C pin through `JROC`. That sets one bit that
     Trellis lists for every C line of the tile. With it set, the MULT18 C lines of o5 kept
     their direct value (hd08c).

**Not covered:**
- IN_1 (refused by the emulator);
- B[53:36] through C1 (A was recorded, B by symmetry);
- the divider's state after configuration;
- a C register whose ALU and MULT18 differ in RESETMODE;
- `CLKn_DIV=DISABLED`;
- the ALU's own C pins through `JROC` (bits above R[15:0] were not recorded).

## Emulator check

The emulator executes all of this but IN_1 ([DSP_ALU.md](../../../DSP_ALU.md)).
[tests/alu54_native.rs](../../../../tests/alu54_native.rs):
- **Fast mode, hd08c:** four windows; the second to the fourth are among the board's.
- **Event kernel, hd08bs:** the 128 clocks of its first window, read at the R pins of the
  eleven ALUs before every edge, are the board's: 16 384 bits. (Through the UART a window
  takes 11 M clocks, hours in the kernel; a replay of hd08b's second window that way was
  stopped before it ended and is gone.)
- **Event kernel, hd08z:** the eight lines are the board's.
- **Planner, hd08:** it refuses IN_1.
- **Fast mode, hd08b:** it does not start, because of the floating C18.

[tests/alu54.rs](../../../../tests/alu54.rs) runs fast mode against the kernel with every
register here on random traffic.
