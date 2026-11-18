# HD04 slice: the ALU54B cascade sign, flags, clock enable and unregistered outputs (D01, D04)

Four things [HD03](../hd03_alu54_inputs/README.md) left open:
- what fills the top of CIN >> 18;
- why the flags never move;
- which CE0 the second ALU of a block runs on;
- what R is when the output registers are off.

It also shows the state right after configuration.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hd04.v](hd04.v), written by [hd04_gen.py](hd04_gen.py), with
  [hd04.lpf](hd04.lpf); configuration [hd04.config](hd04.config).
  - **Twelve ALU54B** in five DSP blocks of row 58, each with its two MULT18X18D and the C
    operand also on the MULT18 C ports. The second ALU of a block takes CIN from the first.
    Blocks p, q, r are adjacent and chained: pa → pb → qa → qb → ra → rb.
  - **OP** is one of:
    - `opa` = n[12:2], four steps per opcode;
    - `opb` = `opa` × 1237 + 77 modulo 2048, the same opcodes in another order;
    - `read` = `01000011111`, R = CIN >> 18.
  - **Variants:**

    | ALU | X | Set | OP | Outputs 0 / 1, flags | CE0 | Patterns |
    |---|---|---|---|---|---|---|
    | pa | 13 | 1 | opa | CLK0 / CLK0, CLK0 | pulse | A |
    | pb | 13 | 2 | read | CLK0 / CLK0, CLK0 | pulse | A |
    | qa | 22 | 2 | opb | CLK0 / CLK0, CLK0 | pulse | A |
    | qb | 22 | 1 | read | CLK0 / CLK0, CLK0 | pulse | A |
    | ra | 31 | 1 | read | CLK0 / CLK0, CLK0 | pulse | A |
    | rb | 31 | 2 | opa | CLK0 / CLK0, CLK0 | **high** | A |
    | sa | 49 | 1 | opa | CLK0 / CLK0, CLK0 | pulse | **Z** |
    | sb | 49 | 2 | opa | CLK0 / CLK0, CLK0 | pulse | **O** |
    | ta | 69 | 1 | opa | **NONE / NONE**, CLK0 | pulse | A |
    | tb | 69 | 2 | opa | **NONE / NONE, NONE** | pulse | A |
    | ua | 87 | 1 | opa | CLK0 / **NONE**, CLK0 | pulse | A |
    | ub | 87 | 2 | opa | **NONE** / CLK0, CLK0 | pulse | A |

    Patterns A are HD02's. Z: MCPAT, MASKPAT and MASK01 all zero. O: all three
    `3FFFFFFFFFFFFF`. CE1..3 are the pulse everywhere.
  - **Lines.** The first line, `I...`, comes 2^24 cycles after configuration and before any
    pulse. Then `T<n:4>`, the two operand sets and `<r:14><f:2>` for each ALU, as in HD03.
- **Runner:** [run.py](run.py) `hd04.bit TI 3 200`.
- **Model check:** [hd04_model.py](hd04_model.py) simulates the chain step by step
  ([alu54_chain.py](../hd02_alu54/alu54_chain.py)); output in
  [model_check.txt](model_check.txt).

## Observations

[observations.log](observations.log): three loads, the `I` line and 8 192 steps each.
- **Repeating:** pa, qa, rb, sa, sb and ua gave the same R in every line of a step. pb, qb and
  ra differ in one or two steps, the first of the second pass, whose predecessor is the end of
  a pass instead of the configured state.
- **Not repeating:** ta, tb and ub differ between loads in about 1 100 steps each, all with
  operations that read R.

## Reading

The model gives every value it predicts, in all three loads: 8 192 of 8 192 steps for each of
pa, pb, qa, qb, ra, rb, sa, sb and ua. For ta, tb and ub it gives 5 376, and the other 2 816
are loops (item 4). The flags and the `I` line fit as well.

1. **The fill of CIN >> 18.** Its top 18 bits are CIN[53] & SIGNEDIB, the reading ALU's own
   second sign input. pb, qb and ra read it at every step; this holds in all 4 049, 4 044 and
   3 034 steps where CIN[53] was 1. SIGNEDCIN plays no part, and nor do the signs or the opcode
   of the ALU that drives CIN.
2. **A block has one CE0 line, and nextpnr decides what is on it.** rb has CE0 high in the
   netlist, and still captures exactly once per step, with ra's pulse. In HD03, b2 captured at
   every clock beside a2's high CE0.
   - **Cause.** A DSP block has four CLK, four CE and four RST lines for both its slices.
     nextpnr (`remap_dsp_blocks`) puts one net on each line, taking the cells in order, first
     slice first. It reconnects every cell's CLK/CE/RST pins to match and renames the
     parameters that pick a line. Its log says `Connection to CE0 remapped to CE1` for rb.
   - **Effect.** The ALU's output and flag registers have no such parameter: they take line
     CE0. So a second ALU whose netlist CE0 is another net than the first ALU's runs on the
     first's. On the chip each ALU simply follows its own CE0 pin, which carries the block's
     CE0 net.
3. **The flags are those of a zero word.** They never follow R. Against the patterns:
   - EQZ and EQZM are 1;
   - EQOM is 0, even with every bit masked (sb);
   - EQPAT is 1 when every unmasked bit of MCPAT is 0 (sa, sb);
   - EQPATB is 1 when every unmasked bit of MCPAT is 1 (sb);
   - OVER, UNDER and OVERUNDER are 0.

   A MASKPAT bit of 1 masks. So A gives `11000000`, Z `11010000` and O `11011000`. A flag
   register starts at 0 and takes these at the first pulse; unregistered flags (tb) show them
   at once.
4. **Output registers, group by group.** R has four groups. A group is unregistered when its
   MULT18's `CIBOUT_BYP` is on. Otherwise it is a register on its clock, and with that clock
   at NONE it stays 0.

   | Bits | Clock | Bypass |
   |---|---|---|
   | R[17:0] | `REG_OUTPUT0_CLK` | the slice's first MULT18 |
   | R[26:18] | `REG_OUTPUT0_CLK` | the second MULT18 |
   | R[44:27] | `REG_OUTPUT1_CLK` | the first MULT18 |
   | R[53:45] | `REG_OUTPUT0_CLK` | the second MULT18 |

   - **What nextpnr sets.** The first MULT18's bypass when `REG_OUTPUT0_CLK` is NONE, and
     `MULT18_5.CIBOUT_BYP` when `REG_FLAG_CLK` is NONE (HD03). It never sets the bypass of
     MULT18_1.
   - **ta** (both NONE, first ALU of its block): R[17:0] and R[44:27] show the result, the
     other 18 bits stay 0.
   - **tb** (all NONE, second ALU): every bit shows the result.
   - **ua** (`REG_OUTPUT1_CLK` NONE): R[44:27] stays 0, the rest is registered.
   - **ub** (`REG_OUTPUT0_CLK` NONE, second ALU): R[17:0] and R[44:27] show the result, the
     rest stays 0.
   - **Loops.** An operation that reads R through unregistered bits loops, as in HD03.
5. **After configuration** every output register and every flag register is 0 (`I` line).

**Not covered:**
- output registers on two different clocks, and NONE with the selected-by-default clock
  running (CLK1..3 are low here);
- registered flags beside a bypass of their half;
- resets and GSR (measured later, [HD06](../hd06_alu54_reset/README.md));
- the C and opcode registers;
- CO and SIGNEDR.

## Emulator check

The emulator executes all of the above ([DSP_ALU.md](../../../DSP_ALU.md)).
- **Event kernel.** [tests/alu54_native.rs](../../../../tests/alu54_native.rs) runs
  [hd04.config](hd04.config) through the `I` line and the first steps. Its lines are the
  board's.
- **Fast mode** refuses this configuration, because ta, tb and ub can read R through
  unregistered bits. [HD05](../hd05_alu54_chain/README.md) is the same chain without them.
