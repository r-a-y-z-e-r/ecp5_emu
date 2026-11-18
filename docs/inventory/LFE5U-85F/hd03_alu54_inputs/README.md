# HD03 slice: where the ALU54B takes its operands from (D04)

What [HD02](../hd02_alu54/README.md) left open around the ALU54B: the cascade input CIN, the
feedback input CFB, the C operand (only 18 of its 54 bits arrived), the flags (they did not
move) and which clock enable the registers take.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hd03.v](hd03.v), written by [hd03_gen.py](hd03_gen.py), with
  [hd03.lpf](hd03.lpf); configuration [hd03.config](hd03.config).
  - **Six ALU54B** in three adjacent DSP blocks of row 58 (X 13, 22, 31), each with its two
    MULT18X18D as in HD02. They are chained by the dedicated cascade: CIN and SIGNEDCIN of each
    come from R and SIGNEDR of the one before.
  - **Operands.** The first ALU of a block (a0, a1, a2) takes operand set 1, the second
    (b0, b1, b2) set 2, from two LFSRs. All six take the same OP, four steps per opcode
    (8 192 steps). CE pulses once per step, as in HD02.
  - **Variants:**

    | ALU | CFB | C operand | Flags | CE0 / CE1..3 |
    |---|---|---|---|---|
    | a0 | own R | also on the MULT18 C ports | unregistered | pulse / pulse |
    | b0 | – | MULT18 C ports, `REG_INPUTC_CLK=CLK0` | unregistered | pulse / pulse |
    | a1 | – | ALU `REG_INPUTC0/1_CLK=CLK0` | registered | pulse / pulse |
    | b1 | own R | also on the MULT18 C ports | registered | pulse / high |
    | a2 | – | also on the MULT18 C ports | registered | high / pulse |
    | b2 | – | ALU pins only (as HD02) | unregistered | pulse / pulse |

  - **Line.** `T<n:4>`, the two operand sets (35 hex digits each, as in HD02), then for each ALU
    `<r:14><f:2>`.
- **Runner:** [run.py](run.py) `hd03.bit T 3 135`.
- **Model check:** [hd03_model.py](hd03_model.py) simulates the chain step by step
  ([alu54_chain.py](../hd02_alu54/alu54_chain.py)); output in
  [model_check.txt](model_check.txt).

## Observations

[observations.log](observations.log): three loads, 8 192 steps each.
- **Repeating:** a0 gave the same R in every line of a step.
- **Not repeating:** b0 and b2 differ between loads in about 1 150 steps each, and a1, b1 and
  a2 in 120 to 150. All of these are steps whose operation reads R, or reads such an R through
  the cascade.
- **Flags:** `11000000` in every line of every ALU.

## Reading

With HD02's operation table, the model below gives every value it predicts, in all three
loads ([model_check.txt](model_check.txt)).

| ALU | Equal | Not predicted |
|---|---:|---:|
| a0 | 8 192 | 0 |
| b0 | 5 376 | 2 816 |
| a1 | 7 552 | 640 |
| b1 | 7 552 | 640 |
| a2 | 5 120 | 3 072 |
| b2 | 5 120 | 3 072 |

"Not predicted" are loops (item 5), runs (item 6), and what reads them through CIN.

1. **A and B are 54 bits.** Their top 18 bits are the C input of the multiplier:
   A = {C of the first MULT18, ROB, ROA}, B = {C of the second, ROB, ROA}. With the C ports
   unconnected (a1, b2, HD02) the top bits are zero.
2. **C comes from three places:** C = {C[53:45] of the ALU, C of the second MULT18, C[26:18] of
   the ALU, C of the first MULT18}. The ALU's own pins C[17:0] and C[44:27] do nothing. So a
   design must put the low 18 bits of each half of C on the multipliers' C ports.
3. **CIN is R of the ALU before it** (OP[6:4] = 2), read as the cascade shows it at the clock
   edge. OP[6:4] = 1 takes CIN >> 18: its low 36 bits are CIN[53:18]. Its top 18 bits are all
   0 or all 1, and 1 only when CIN[53] is 1 ([HD04](../hd04_alu54_cascade/README.md) finds the
   rule). An ALU with nothing before it reads zero (a0).
4. **CFB did nothing.** a0 and b1, with CFB wired to their own R, behave like the others for
   every opcode.
5. **nextpnr unregisters part of R.** When an ALU's `REG_FLAG_CLK` is NONE (the default),
   nextpnr sets `MULT18_5.CIBOUT_BYP`, for either ALU of the block.
   - **Effect.** R[26:18] and R[53:45] of the block's second ALU (b0, b2) then leave before
     the register. The fabric and the cascade see the sum itself there, and the other 36 bits
     come from the register.
   - **Loop.** An operation that reads R then loops through those bits. The values are not
     the model's and differ between loads.
6. **A block has one CE0 for both ALUs.** a2 has CE0 high and captures at every clock. b2, in
   the same block with CE0 on the pulse in the netlist, does the same. An operation that reads
   R then runs on while the operands shift.
   - **Cause.** nextpnr gives a DSP block four CE lines for both its slices. It puts one net on
     each, taking the cells first slice first, and reconnects the cells to match
     ([HD04](../hd04_alu54_cascade/README.md), item 2). The output register always takes line
     CE0, which here carries a2's constant 1.
7. **The C registers show nothing here.** With `REG_INPUTC_CLK=CLK0` on the multipliers (b0)
   or `REG_INPUTC0/1_CLK=CLK0` on the ALU (a1), the result uses the current C, not the C of the
   step before. So these registers do not wait for the CE pulse. Whether they are registers at
   all cannot be seen with operands that hold still for many cycles.
8. **Flags** stay `11000000`, registered or not.

**Not covered:**
- the clocked behaviour of the C and opcode registers. Measured later
  ([HD08](../hd08_alu54_cop/README.md)): here each C register was asked for on one cell only
  (the MULT18 for b0, the ALU for a1), which leaves its bypass bit set, so it was off;
- resets and GSR (measured later, [HD06](../hd06_alu54_reset/README.md));
- CO and SIGNEDR, which have no route to the fabric.

## Emulator check

The emulator executes the operand rules above ([DSP_ALU.md](../../../DSP_ALU.md)). This
configuration itself is refused: the C registers of b0 and a1 are outside the executed profile.
[HD05](../hd05_alu54_chain/README.md) is the chain the emulator replays line by line.
