# HD01 slice: the CE and RST of the MULT18X18D output register (D01, D05, D06)

Which clock enable and which reset drive the output register of a MULT18X18D.
- **Primitive.** It has CE0..3 and RST0..3, and the parameters `REG_OUTPUT_CE` and
  `REG_OUTPUT_RST` choose among them.
- **Trellis.** It has no `REG_OUTPUT_CE` field at all, and `REG_OUTPUT_RST` only on MULT18 0
  and 4 of each DSP block ([DSP_OUTPUT.md](../../../DSP_OUTPUT.md)).
- **Emulator.** It therefore ran the output stage only with all CE high and all RST low, and
  stopped otherwise.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hd01.v](hd01.v) with [hd01.lpf](hd01.lpf), nextpnr seed 1
  ([hd01.config](hd01.config)).
  - **Multipliers.** Two MULT18X18D with only the output register (`REG_OUTPUT_CLK=CLK0`,
    25 MHz), `RESETMODE=SYNC`, `GSR=DISABLED`:
    - m0 asks for `REG_OUTPUT_CE=CE2`, `REG_OUTPUT_RST=RST3`;
    - m1 asks for `CE1` and `RST2`.
  - **Controls.** CE0..3 and RST0..3 are shared, driven independently at random by fabric
    registers: CE high 3/4 of the time, RST 1/8.
  - **Operands.** A and B are 4-bit LFSR values, new every cycle; m1 multiplies A by B+1.
  - **Placement.** nextpnr puts m1 on MULT18_0 and m0 on MULT18_1 of one DSP block. It writes
    `MULT18_0.REG_OUTPUT_RST RST2` and nothing for m0's reset, because MULT18_1 has no such
    field.
  - **Line.** Each line `M=<1536 hex>` is 256 samples of `{ce, rst, a, b, p0[3:0], p1[3:0]}`,
    one per cycle; windows follow one another.
- **Runner:** [run.py](run.py) `hd01.bit M= 3 2`.
- **Analysis:** [hd01_analyse.py](hd01_analyse.py) tries every CE (none, CE0..3) and every RST
  (none, RST0..3) with a synchronous reset ([analysis.txt](analysis.txt)).

## Observations

[observations.log](observations.log): three loads with 14 distinct windows of 256 samples
between them.

## Reading

Of the 25 choices, exactly one fits every sample of every window, for each multiplier:

| Multiplier | Asked for | Placed on | CE that acts | RST that acts |
|---|---|---|---|---|
| m0 | CE2, RST3 | MULT18_1 | **CE0** | **RST0** |
| m1 | CE1, RST2 | MULT18_0 | **CE0** | **RST2** (its REG_OUTPUT_RST) |

1. **CE.** The output register captures under **CE0**, whatever `REG_OUTPUT_CE` says. There
   is no field to choose another.
2. **Reset on MULT18_0.** It follows its `REG_OUTPUT_RST` field.
3. **Reset on MULT18_1.** It has no such field and uses **RST0**.
4. **Reset kind.** It is synchronous (`RESETMODE=SYNC`): P becomes 0 at the edge after RST,
   whether CE is high or not.
5. **nextpnr.** It accepts other CE/RST choices on the primitive without a word, so a design
   can ask for CE2 and get CE0.

**Not covered:**
- `RESETMODE=ASYNC` on the output stage and GSR (measured later,
  [HD07](../hd07_dsp_gsr/README.md): both clear it at once);
- MULT18 4/5 (the other half of the block, assumed the same as 0/1);
- the other output clocks;
- the ALU and its flags on the same bank.

## Emulator check

The emulator now runs the output stage like the other stages:
- **Planner** ([compile/dsp.rs](../../../../src/compile/dsp.rs)): CE0, the `REG_OUTPUT_RST`
  reset (RST0 where the field is missing), and RESETMODE.
- **Kernel** ([kernel/dsp.rs](../../../../src/kernel/dsp.rs)) and **fast mode**
  ([kernel/fast_dsp.rs](../../../../src/kernel/fast_dsp.rs)).

The runtime stop is gone. Since HD07 the output stage also takes GSR and starts at 0 where the
slice has GSR enabled.
- [tests/dsp.rs](../../../../tests/dsp.rs):
  - **Kernel.** The output stage's enable, its synchronous reset at the edge and unknown
    controls.
  - **Fast mode against the kernel.** Driven CE and reset over 4 000 random steps.
  - **Random stage test.** The output stage now takes random CE/RST too.
- **HD01 itself.** It is not yet compared window by window ([hd01_check.txt](hd01_check.txt)).
  - **Start.** The output bank starts X, the first window records that X, and fast mode can
    start only after the first UART line.
  - **Link failure.** There, fast mode's C program failed to link (`_piece0`), a
    reported code-splitting defect.
  - **Small variant.** The same two multipliers without the recorder give byte-identical VCD
    files in both executors.
