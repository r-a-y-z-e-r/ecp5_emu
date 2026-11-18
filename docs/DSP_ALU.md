# ALU54B execution

Added on 2026-11-12. The emulator executes the ALU54B of a DSP slice, in the event kernel and
in fast mode, as seven board runs on the ULX3S measured it:
[HD02](inventory/LFE5U-85F/hd02_alu54/README.md) (the operation table),
[HD03](inventory/LFE5U-85F/hd03_alu54_inputs/README.md) (the operands),
[HD04](inventory/LFE5U-85F/hd04_alu54_cascade/README.md) (cascade sign, flags, clock enable,
unregistered outputs), [HD05](inventory/LFE5U-85F/hd05_alu54_chain/README.md) (a chain that
the emulator replays line by line),
[HD06](inventory/LFE5U-85F/hd06_alu54_reset/README.md) (clock enable and reset) and
[HD07](inventory/LFE5U-85F/hd07_dsp_gsr/README.md) (GSR, the reset line and which MULT18 makes
which half asynchronous) and [HD08](inventory/LFE5U-85F/hd08_alu54_cop/README.md) (the C and
opcode registers, and what Z = 7 reads). [HD09](inventory/LFE5U-85F/hd09_dsp_clkdiv/README.md) adds CLK_DIV=DISABLED in both the event kernel and fast mode. Before
this the ALU was not executed at all; only the operands of the multipliers beside it could be
observed ([DSP_ALU_INPUTS.md](DSP_ALU_INPUTS.md)).

No public document gives the operation table, so everything below is from the board. The
builds are nextpnr's: where nextpnr configures the block in a way that changes what the chip
does, that is part of the behaviour described here.

## The model

### Sources and operation

Three 54-bit sources, chosen by OP[6:0], and an operation, chosen by OP[10:7]:

| Field | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
|---|---|---|---|---|---|---|---|---|
| X = OP[1:0] | R | MA | A | zero | | | | |
| Y = OP[3:2] | MB << 18 | MB | B | zero | | | | |
| Z = OP[6:4] | zero | CIN >> 18 | CIN | C | A | R | RNDPAT | ~RNDPAT & (RNDPAT − 1) |

- **Z = 7** reads the mask of RNDPAT's trailing zeros: all ones for RNDPAT 0, F for 10, 0
  for an odd RNDPAT (HD08z). HD02..HD05 had an odd RNDPAT, so it read zero there.
- **OP[10:9] = 1:** R = ±Z + X ± Y modulo 2^54. OP7 subtracts Y, OP8 subtracts Z.
- **OP[10:9] = 0, 2, 3:** bit by bit, without X. With Y' = Y inverted when OP7 and Z' = Z
  inverted when OP8: NAND, AND and XNOR of Y' and Z'.

### Operands

- **MA, MB:** the 36-bit products of the slice's two MULT18X18D, sign-extended to 54 bits when
  the multiplier's SIGNEDP is 1.
- **A, B:** 54 bits. A = {C input of the first MULT18, ROB, ROA}, B the same of the second.
- **C:** {C[53:45] of the ALU, C input of the second MULT18, C[26:18] of the ALU, C input of
  the first MULT18}. The ALU's own pins C[17:0] and C[44:27] do nothing on the chip.
- **CIN:** R of the ALU before this one, by the dedicated cascade; zero when there is none.
  CIN >> 18 has CIN[53:18] in its low 36 bits and CIN[53] & SIGNEDIB in its top 18.
- **R:** the output register.
- **Not operands:** CFB (wiring it to R changed nothing) and SIGNEDCIN.

### Registers

- **Output register.** It is 0 after configuration (HD04, and HD07s with GSR enabled) and
  captures the result at a rising clock edge while the enable is high.
- **Enable.** The registers take the block's CE0 line, which is the ALU's CE0 pin as routed.
  There is no setting for it: `REG_OUTPUT0_CE` and its like do not exist in the bitstream.
- **Reset.** It wins over the enable (HD06). R has two halves for it, one per MULT18 of the
  slice (HD07):

  | Bits | Line | Asynchronous when RESETMODE=ASYNC on |
  |---|---|---|
  | R[17:0], R[44:27] | the one `REG_OUTPUT0_RST`/`REG_OUTPUT1_RST` select | the ALU or the first MULT18 |
  | R[26:18], R[53:45] | RST0, always | the second MULT18 |

  - The two ALU fields are the same bits as the first MULT18's `REG_OUTPUT_RST`, which nextpnr
    always writes (RST0 by default).
  - An ALU set to ASYNC beside multipliers left at SYNC resets half of R at once and the other
    half at the next clock edge.
- **GSR.** The slice's two GSR bits are shared by the ALU54B and both MULT18X18D, and any of
  the three at `GSR=DISABLED` sets them.
  - **Enabled** (all three ENABLED): GSR clears R at once and holds it at 0 through the clock
    edges while it is active (HD07).
  - **Disabled:** it does nothing (HD06).
- **Input registers** (HD08), each 0 after configuration, cleared by an enabled GSR, its
  reset winning over its enable:

  | Register | Holds | Clock | Enable, reset | Reset kind |
  |---|---|---|---|---|
  | opcode IN_0 | OP[6:0] | `REG_OPCODEIN_0_CLK`, every edge | `REG_OPCODEIN_0_CE`, `_RST` | RESETMODE of the ALU |
  | opcode OP0_0, OP0_1 | OP7 | `REG_OPCODEOP0_n_CLK` | `REG_OPCODEOP0_n_CE`, `_RST` | the same |
  | opcode OP1_0, OP1_1 | OP[10:8] | `REG_OPCODEOP1_n_CLK` | OP0_n's fields | the same |
  | C0 | C[26:0] and A[53:36] | `REG_INPUTC0_CLK`, every other edge | CE0, RST0 | RESETMODE |
  | C1 | C[53:27] and B[53:36] | `REG_INPUTC1_CLK`, every other edge | CE0, RST0 | RESETMODE |

  - **Opcode stages.** The second stage (`_1`) of a group takes the first's output where the
    first is on, else OP.
  - **IN_1 is refused.** With IN_1 on, 5–6% of the clocks follow no register model (HD08 o4,
    o7), mostly through Y = OP[3:2].
  - **C registers.** A register is on only where the ALU's field and its MULT18's
    `REG_INPUTC_CLK` (the same select bits) both ask for a clock. With one of them at NONE its
    bypass bit is set and the register is off (HD08 c4; that is why HD03 saw no delay).
  - **Every other edge with CLK_DIV=ENABLED.** A divider toggles at each rising edge of the C register's clock, and the
    register takes only the edges before which it is 0. An enabled GSR holds the divider at 0.
    Its state after configuration did not show; the emulator takes 0.
  - **CLK_DIV=DISABLED (HD09).** R and opcode stages capture on both edges. C0/C1 (including MULT18 INPUTC) capture on every rising edge without the divider. CE, reset kind and priority remain as above. GSR active at a rise holds through the following fall; it releases for captures at the next rise. Mixed active register clocks/cells with ENABLED and DISABLED, and ALU/MULT disagreement within a slice, are refused. Flags were not recorded by HD09.
  - **Shared select bits.** OP1 shares its clock select bits with OP0, as C does with the
    MULT18, and each has its own bypass bit. With one on and the other at NONE the decoder
    sees both CLKn and NONE for the second; it is off (HD08 o3).
- **Groups.** R has four groups of bits. Each is unregistered when its MULT18's `CIBOUT_BYP`
  is on. Otherwise it is a register on its clock, and with that clock at NONE it stays 0.

  | Bits | Clock | Bypass |
  |---|---|---|
  | R[17:0] | `REG_OUTPUT0_CLK` | the slice's first MULT18 |
  | R[26:18] | `REG_OUTPUT0_CLK` | the second MULT18 |
  | R[44:27] | `REG_OUTPUT1_CLK` | the first MULT18 |
  | R[53:45] | `REG_OUTPUT0_CLK` | the second MULT18 |

- **Unregistered bits.** The fabric, the cascade and the ALU's own R operand see the result
  itself. An operation that reads R through such a bit is a loop on the chip. The board gave
  values that differ between loads.
- **Flags.** EQZ, EQZM, EQOM, EQPAT, EQPATB, OVER, UNDER and OVERUNDER never follow R. They are
  what a zero word gives against the patterns:
  - EQZ and EQZM are 1, EQOM is 0;
  - EQPAT is 1 when every unmasked bit of MCPAT is 0, EQPATB when every unmasked bit is 1 (a
    MASKPAT bit of 1 masks);
  - OVER, UNDER and OVERUNDER are 0.

  A flag register (`REG_FLAG_CLK` on a clock) is 0 after configuration and takes these
  constants at its first enabled clock edge.

### What nextpnr does

- **`REG_FLAG_CLK=NONE`** (the default) makes nextpnr set `MULT18_5.CIBOUT_BYP`, for either
  ALU of the block. R[26:18] and R[53:45] of the block's second ALU are then unregistered. A
  MAC in the second slice with default flags therefore loops on the chip. Give its flags a
  clock to avoid it.
- **`REG_OUTPUT0_CLK=NONE`** makes it set the bypass of the slice's first MULT18. The bypass
  of MULT18_1 is never set. So an ALU54_3 with both output registers off has R[26:18] and
  R[53:45] stuck at 0.
- **Constant resets** on the ALU are left unrouted.
- **Constant C pins** (HD08). The ALU's C[17:0] and C[44:27] are the same CIB wires as the
  MULT18 C inputs. Tied to two different constants, the wire's constant mux is written once for
  each, and the chip takes 0 where either is 0. nextpnr may also leave a live C pin of the ALU
  floating (hd08b, C18 of o7). Drive constants through logic to avoid both.
- **The JROC route** (HD08). nextpnr may route an ALU C pin through `JROC`, which sets a bit
  that Trellis lists for every C line of the tile. The MULT18 C lines kept their direct route
  on the board (hd08c); the planner follows that, and gives the ALU's own C pins the `JROC`
  route nextpnr chose (not measured).
- **One net per control line.** A DSP block has four CLK, four CE and four RST lines for both
  its slices. nextpnr (`remap_dsp_blocks`) puts one net on each, taking the cells first slice
  first, reconnects every cell's pins to match and renames the parameters that pick a line.
  The ALU's output and flag registers always take line CE0. So a second ALU whose netlist CE0
  is another net than the first ALU's runs on the first's: with the first's CE0 tied high it
  captures at every clock (HD03 b2), and with its own tied high it still waits for the first's
  (HD04 rb). The emulator follows the routed pins, so it needs no rule for this.

## What is executed

- **Planner** ([compile/dsp.rs](../src/compile/dsp.rs), `execute_alu`):
  - It reads the settings and assembles A, B and C from the ALU's and the multipliers' ports.
  - It decides each group of R from `REG_OUTPUT0/1_CLK` and the two `CIBOUT_BYP` selectors.
  - In a slice whose ALU it can execute, the multipliers get their products too (MA, MB), and
    the output bank is the ALU's.
  - An inactive ALU reads zero through the cascade.
- **Event kernel** ([kernel/alu.rs](../src/kernel/alu.rs)): four-valued.
  - A sum with unknown bits is unknown only from the lowest unknown bit up, and a bitwise
    result only in those bits.
  - R read through unregistered bits is unknown.
- **Fast mode** ([kernel/fast_dsp.rs](../src/kernel/fast_dsp.rs)):
  - `alu54` in the C prelude gives the 54-bit result. Each registered bit is a capture of its
    bit of it, and each unregistered bit a line.
  - A source that the known bits of OP cannot select is not read, so a constant OP reads only
    what it uses.
  - A product that comes straight from a multiplier is computed inside the expression, so the
    36 product nets are not kept up at every cycle (2.1 → 4.1 MHz on HD02, compile included).
  - DISABLED R and opcode captures declare both clock edges. C stays rising-only
    and has no divider; ENABLED keeps its existing divider and generated C.
    Per-register GSR samples carry the rise's hold through the fall, including
    handover from the kernel. Flags retain the previous rising-edge model;
    HD09 did not record them.
  - A shared clock group also settles its slice FFs on the added falling edge;
    the extra settle is idempotent and affects only DISABLED designs.
  - It refuses a program whose OP can read R through unregistered bits.

**Refused, not measured:**
- `REG_OPCODEIN_1_CLK` on a clock (see above);
- a C register whose ALU and MULT18 have different RESETMODE;
- output registers on two different clocks;
- `MCPAT_SOURCE`/`MASKPAT_SOURCE=DYNAMIC`, `LEGACY=ENABLED`,
  `FORCE_ZERO_BARREL_SHIFT=ENABLED`, a nonstandard clock divider;
- registered flags beside a bypass of their half (when a flag output is used);
- the outputs CO and SIGNEDR (SIGNEDR is driven unknown);
- `REG_OUTPUT0_RST` and `REG_OUTPUT1_RST` on different lines (nextpnr always sets both to one).

**Assumed, not measured:**
- the flag register is left alone by the reset and by GSR. Its content is a constant either
  way, so only the moments right after a reset could differ;
- B[53:36] passes C1, as A[53:36] passes C0 (HD08 recorded A only);
- the C registers' divider is 0 after configuration;
- the `JROC` route of an ALU's own C pins.

## Checks

- **[tests/alu54.rs](../tests/alu54.rs)**, on a synthetic plan:
  - the operation against an independent reference for every code;
  - the kernel and fast mode against that reference on random traffic, with registered,
    clockless and unregistered groups, a flag register, a reset line per half and GSR;
  - fast mode against the kernel with opcode and C registers, every kind of reset and GSR,
    and an even RNDPAT, for ENABLED and DISABLED, with default and forced four
    threads; GSR hold/release and handover are checked explicitly; Z = 7 in the kernel;
  - fast mode's refusal of a loop.
- **[tests/alu54_native.rs](../tests/alu54_native.rs)**, on the board's configurations:
  - HD02: fast mode prints all 16 384 lines of the board's pass; the kernel prints the first
    four.
  - HD05: fast mode prints the board's lines for the `I` line and a whole pass of eight
    chained ALUs; the kernel prints the start.
  - HD04: the kernel prints the `I` line and the first steps of twelve ALUs, and fast mode
    refuses the configuration (three of its ALUs can loop).
  - HD06, HD06b: fast mode prints the board's windows of four ALUs under random CE, RST and
    GSR, with every RESETMODE combination; the kernel prints HD06b's second window.
  - HD08: hd08c (every register the planner runs, constants on nets) in fast mode, hd08bs's
    first window in the kernel (read at the ALU R pins), hd08z (Z = 7) in the kernel; hd08
    (IN_1) is refused.
  - HD07, HD07s: GSR enabled, every reset line and RESETMODE combination, and four MULT18
    with registered outputs. Fast mode prints HD07s's first window from the first clock, and
    eight HD07 windows; the kernel prints HD07s's first window.
- **[tests/hd09_hs02_native.rs](../tests/hd09_hs02_native.rs)** checks DISABLED:
  hd09s/ms/ps at the DSP pins against board samples and fast against the kernel;
  hd09m/p free UART windows in fast mode against complete board lines. Startup
  sign bits remain unknown: hd09ps and hd09p use the first kernel edge to load
  them before fast handover, while sample zero is still checked by the kernel.
- **[tests/dsp_alu_operands_native.rs](../tests/dsp_alu_operands_native.rs)** still checks the
  operand view. Of its two ALUs the right one, with unregistered outputs, is now one the
  planner can execute, so its multipliers have products.
