# HI07 slice: the 7:1 gearbox, cycle by cycle (I01, I03)

How ODDR71B and IDDR71B behave with CLKDIVF /3.5: which SCLK edge takes the word, which ECLK
half each bit leaves in, and which ECLK edges an IDDR71B word is made of. The emulator refused
`ODDRXN.MODE=ODDR71` and `IDDRXN.MODE=IDDR71`.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Designs:** [hi07.v](hi07.v), and [hi07b.v](hi07b.v), which also samples Q0..Q6 at 25 MHz.
  Both use [hi07.lpf](hi07.lpf), nextpnr seed 1 with `--allow-fabric-eclk`.
  - **Clocks.** As in [HI03](../hi03_gearbox/README.md): `eclk_in` = 25 MHz / 16 from the
    fabric, ECLKSYNCB, then CLKDIVF **/3.5**. Flip-flops toggle on the rising (`p`) and
    falling (`n`) edges of SCLK.
  - **ODDR71B on LED C1 (PIO A; ODDR71B needs A or C).** D0..D6 are seven 7-bit LFSRs stepped
    on SCLK rising edges. The pad is read back through its `BB`.
  - **IDDR71B on LED D2 (PIO C).** The fabric drives the pad with an LFSR that steps in the
    middle of each ECLK half period. nextpnr allows IDDR71B's D no other load, so HI07b
    records that LFSR bit (`lw[0]`) instead of the pad.
  - **Line.**
    - **O:** 384 samples of `{eclk_in, p, n, pad}` from t = 2048.
    - **Q:** HI07b only, the same samples of `{lw[0], Q0..Q6}`.
    - **I:** Q0..Q6 at 16 SCLK rising edges.
    - **N:** the SCLK rising edges before sample 0.
- **Runner:** [run.py](run.py) `BIT O= 3 1.5` (HI07) and `3 2` (HI07b).
- **Analysis:**
  - [hi07_analyse.py](hi07_analyse.py) ([analysis.txt](analysis.txt)) tries every latency,
    ECLK-half shift and lane order for ODDR71B, and every start edge and all 5 040 orders for
    IDDR71B.
  - [hi07b_check.py](hi07b_check.py) ([check_b.txt](check_b.txt)) fits IDDR71B directly from
    the 25 MHz record.

## Observations

- **HI07.** [observations.log](observations.log): three loads, one line each, the same line
  every time ([pass.txt](pass.txt)).
- **HI07b.** [observations_b.log](observations_b.log): the same three loads, one complete
  line ([pass_b.txt](pass_b.txt)). Its O and I equal HI07's, and N is 36 in both.

## Reading

Each search has a single answer: the other ODDR71B fits are the same model, shifted by a whole
word.

1. **CLKDIVF /3.5.** SCLK rises every 56 samples, 3.5 ECLK periods, alternately at a rising
   and at a falling ECLK edge. It is high for 4 ECLK halves and low for 3, as HI04 measured.
2. **ODDR71B.**
   - **Order.** The pad carries D0, D1, …, D6, one per ECLK half, from the ECLK edge after
     the one where SCLK rose.
   - **Latency.** The word is the one captured at the SCLK rising edge before that: from
     capture to D0 is one SCLK period plus one ECLK half.
   - **Changes.** The pad changes only at ECLK edges.
3. **IDDR71B.**
   - **When.** Q0..Q6 change only at SCLK rising edges. The word published at SCLK rising edge
     P is the pad at the seven consecutive ECLK edges from P−96 to P−48 samples: the six
     halves before the edge after the previous SCLK rise, and that edge.
   - **Order.** In time order those samples land in Q4, Q5, Q6, Q0, Q1, Q2, Q3.
   - **Both sides at once.** Both gearbox sides act at the ECLK edge after SCLK rises. ODDR71B
     loads its word there. IDDR71B's newest sample is taken there, and the word reaches Q0..Q6
     at the next SCLK rising edge.

**Not covered:**
- ALIGNWD on IDDR71B (nextpnr ties it, I04);
- RST;
- GSR=ENABLED (the primitives' default; nextpnr writes the IOLOGIC without it);
- the right bank;
- the pair partner's RXDATA4..6 lanes, which carried Q4..Q6 here, used by their own B/D
  register.

## Emulator check

The emulator executes both sides ([compile/eclk.rs](../../../../src/compile/eclk.rs),
[kernel/iologic.rs](../../../../src/kernel/iologic.rs)):
- **ODDR71B:** two ranks of SCLK registers, then `GearOut7`.
- **IDDR71B:** `GearIn7`, then one rank of SCLK registers on RXDATA0..6.

Two Trellis decode artefacts had to be read the right way, both from shared bits:
- **IDDR71.** Its bits include F2B2, the bit that tells `MODE=IDDRX1_ODDRX1` from `IDDRXN`,
  so the MODE decodes as IDDRX1_ODDRX1 and is taken as IDDRXN.
- **ODDR71.** Its bits also hold those of MODDRX2/MOSHX2 and MTSHX2.

Tests:
- [tests/gearbox71_native.rs](../../../../tests/gearbox71_native.rs):
  - **HI07.** `hi07.config` prints exactly the board's line in the event kernel and in fast
    mode, N included. This build started ECLKO with the fifth ECLKI edge, as the emulator does.
  - **HI07b.** `hi07b.config` prints exactly the board's line in fast mode, with its 25 MHz
    record of Q0..Q6.
- [tests/cross_domain_fast.rs](../../../../tests/cross_domain_fast.rs) checks fast mode
  against the kernel through the 7:1 gearbox: CLKDIVF /3.5 on ECLK, random data and divider
  resets, 40 000 ECLK edges.
