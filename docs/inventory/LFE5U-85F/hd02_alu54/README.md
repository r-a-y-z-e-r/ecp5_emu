# HD02 slice: the ALU54B operation table (D04)

What an ALU54B computes for each of its 2 048 operation codes. The primitive takes an 11-bit
`OP`; no public document gives its full table, and the emulator executed nothing of the ALU
(it only let the operands of the multipliers beside it be observed,
[DSP_ALU_INPUTS.md](../../../DSP_ALU_INPUTS.md)).

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hd02.v](hd02.v), written by [hd02_gen.py](hd02_gen.py) (the primitives have a
  port per bit), with [hd02.lpf](hd02.lpf); configuration [hd02.config](hd02.config).
  - **Primitives.** Two MULT18X18D with every register bypassed feed one ALU54B the way
    nextpnr requires:
    - A = {ROB, ROA} of the first multiplier, B the same of the second;
    - MA and MB = their products P;
    - SIGNEDIA and SIGNEDIB = their SIGNEDP.

    C and OP come from the fabric. CIN, CFB and SIGNEDCIN are not connected. nextpnr placed
    the three in the first DSP block of row 58 (MULT18_0, MULT18_1, ALU54_3).
  - **Registers.** Only the ALU's output and flag registers are on (`REG_OUTPUT0_CLK`,
    `REG_OUTPUT1_CLK`, `REG_FLAG_CLK` = CLK0, the 25 MHz clock).
  - **Program.** 16 384 steps: every opcode in turn, eight steps each.
    - Each step shifts a 168-bit LFSR 168 times, so all operand bits are new: four 18-bit
      multiplier operands, a 54-bit C and four sign bits.
    - Everything then holds still, and CE0..CE3 pulse together for one clock cycle. The output
      register captures once per step.
    - The steps of one opcode follow one another, so an operation that reads R back uses the
      R of the line before.
  - **Patterns.** MCPAT `1555555555555A`, MASKPAT `3FFFFFFFFFFFF0`, MASK01 `3FFFFFFFFFFF0F`,
    RNDPAT `0123456789ABC5`.
  - **Line.** `T<n:4><sg:1><a0:5><b0:5><a1:5><b1:5><c:14><r:14><f:3>`, hex: the step, the
    signs, the operands, R and the flags after the pulse. The LFSR restarts with n = 0, so the
    pass repeats.
- **Runner:** [run.py](run.py) `hd02.bit T 3 95`: three loads, 95 s each (a pass takes 83 s).
- **Model check:** [hd02_model.py](hd02_model.py), output in [model_check.txt](model_check.txt).

## Observations

[observations.log](observations.log): three loads. Each gave all 16 384 steps, and the three
sets of lines are the same.

## Reading

One model gives R of all 16 384 steps, 0 differ.

1. **Three sources**, each 54 bits, chosen by the low seven bits of OP:

   | Field | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
   |---|---|---|---|---|---|---|---|---|
   | X = OP[1:0] | R | MA | A | zero | | | | |
   | Y = OP[3:2] | MB << 18 | MB | B | zero | | | | |
   | Z = OP[6:4] | zero | (zero here) | (zero here) | C | A | R | RNDPAT | zero |

   R is the output register itself. Z = 1 and 2 read zero in this design; HD03 and HD04 show
   they are the cascade input (CIN >> 18 and CIN). Z = 7 reads zero because RNDPAT is odd
   here: it is the mask of RNDPAT's trailing zeros ([HD08](../hd08_alu54_cop/README.md)).
2. **The operation**, by the top four bits:
   - **OP[10:9] = 1:** a sum modulo 2^54, R = ±Z + X ± Y. OP7 subtracts Y, OP8 subtracts Z.
   - **OP[10:9] = 0, 2, 3:** bit by bit, and X is not used. With Y' = Y inverted when OP7 and
     Z' = Z inverted when OP8: NAND, AND and XNOR of Y' and Z'. That gives all the usual pairs:
     OR is 0011, NOR 1011, XOR 1101 and 1110.
3. **Signs.** MA and MB are the 36-bit products. Each is sign-extended to 54 bits when its
   multiplier's SIGNEDP (SIGNEDA | SIGNEDB) is 1, zero-extended otherwise. A and B are not
   sign-extended.
4. **Feedback.** An operation that reads R (X = R or Z = R) reads the register as the previous
   pulse left it: exactly one capture per pulse.
5. **Not explained here, measured later:**
   - **C.** Only C[26:18] and C[53:45] reach the result through the ALU's own pins. The rest
     comes through the multipliers' C inputs (HD03).
   - **Flags.** They are `11000000` (EQZ and EQZM high) in every line, also when R is not zero
     (HD04: they never follow R).

## Emulator check

The emulator executes the ALU54B with this table ([DSP_ALU.md](../../../DSP_ALU.md)).
[tests/alu54_native.rs](../../../../tests/alu54_native.rs) runs [hd02.config](hd02.config):
- **Fast mode** runs the whole pass. All 16 384 lines it prints are the board's.
- **The event kernel** runs the first four steps, and its lines are the board's.
