# HI03 slice: the X2 gearbox, cycle by cycle (I03, C04, C05)

How the side IOLOGIC's X2 gearbox and its edge clocks behave: ECLKSYNCB, CLKDIVF /2, ODDRX2F
and IDDRX2F. The emulator refused all four. The guides give the port lists and timing
diagrams, but no cycle rule to execute: they do not say which SCLK edge captures the word, in
which ECLK half each bit leaves, or which ECLK edges an IDDRX2F word is made of.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hi03.v](hi03.v) with [hi03.lpf](hi03.lpf), nextpnr seed 1 with
  `--allow-fabric-eclk` ([hi03.config](hi03.config)).
  - **Clocks.** `eclk_in` is bit 3 of a 4-bit counter on the 25 MHz clock: a period of 16
    cycles, high in phases 8–15. It enters ECLKSYNCB (`STOP` tied 0) from the fabric;
    CLKDIVF /2 makes SCLK from its output. The counter `t` counts 25 MHz cycles from the
    start.
  - **ODDRX2F on LED E2.** Its four inputs are four 7-bit LFSRs stepped on SCLK rising edges.
    The pad is read back through the same `BB`.
  - **IDDRX2F on LED D2.** The fabric drives the pad with a 7-bit LFSR that steps at 25 MHz
    edges in the middle of each ECLK half period, so the pad is stable at every ECLK edge.
  - **Placement.** X2 exists only on the left and right banks, and IDDRX2F needs a PIO A or C
    (the SLIP input); hence the two LEDs.
  - **Line.** `O=<144 hex> I=<32 hex> N=<2 hex>` over the FTDI UART, 115200 8N1:
    - O: 192 samples of `{eclk_in, st, pad}` at 25 MHz from t = 2048, first sample in the
      MSBs. `st` toggles on each SCLK rising edge.
    - I: `{Q0, Q1, Q2, Q3}` at 32 SCLK rising edges (64–95, counted by the design).
    - N: the SCLK rising edges before sample 0.
- **Runner:** [run.py](run.py) `hi03.bit O= 3 1.5`: three loads, 1.5 s of lines each.
- **Analysis:** [hi03_analyse.py](hi03_analyse.py) tries every latency, ECLK-half shift and lane
  order against the line, with N as recorded ([analysis.txt](analysis.txt)).

## Observations

[observations.log](observations.log): three loads, 14 lines each, all the same
([pass.txt](pass.txt)):

```text
O=000000DB6DB66DB6DBFFFFFF492492B6DB6D000000924924249249FFFFFF6DB6DBDB6DB66DB6DBB6DB6D000000B6DB6D249249DB6DB6492492DB6DB66DB6DB924924249249924924 I=F8106147916753E87126D6F634BB9957 N=3E
```

A first build, before the line had the N field, also gave one line in all three loads
([observations_first_build.log](observations_first_build.log)).

## Reading

The analysis finds one model for each path (the second fit it prints is the same model, with
the latency and the shift traded for each other).

1. **ODDRX2F.** The ECLK half periods carry the four inputs in order: D0 in a high half, D1 in
   the low half after it, then D2 and D3. A word starts at the ECLK rising edge where SCLK
   falls. It is the word captured at the SCLK rising edge two edges before the one that began
   that SCLK cycle: 2.5 SCLK cycles from capture to the first bit. The pad changes only at ECLK
   edges.
2. **IDDRX2F.** Q0..Q3 published at an SCLK rising edge E are the pad at four ECLK edges in
   order: the rising edge where SCLK fell 1.5 SCLK cycles before E, the falling edge after it,
   the rising edge at the SCLK edge before E, and the falling edge after that.
3. **ECLKSYNCB with STOP = 0** passes ECLKI unchanged.
4. **CLKDIVF /2.** SCLK rises at every other ECLK rising edge.
5. **Start-up (C05).**
   - Recorded build: SCLK rose at the ECLK rising edges at t ≡ 8 (mod 32), and N = 62. So the
     registers on SCLK saw their first SCLK rising edge at the fifth ECLK rising edge, t = 72.
   - First build: SCLK rose at t ≡ 24 (mod 32). That build had no N field, and its design text
     differed only in the line it prints.
   - [HI04](../hi04_eclk/README.md) explains both. ECLKSYNCB's STOP synchronizer starts closed
     and opens after four falling edges of ECLKI: ECLKO starts with the fifth rising edge, and
     CLKDIVF rises with ECLKO's first edge. In some builds it opens one falling edge later,
     with the sixth rising edge, which gives the first build's phase.

**Not covered** (HI04 measured the first two since):
- STOP asserted (C04);
- CLKDIVF /3.5, RST and ALIGNWD (I03/I04);
- GSR=ENABLED;
- the 7:1 gearbox (IDDR71B/ODDR71B);
- the X2 memory modes;
- the right bank.

## Emulator check

The emulator executes the four primitives; the planner is in
[compile/eclk.rs](../../../../src/compile/eclk.rs) and the ECLK stages are the `Divider`,
`GearOut` and `GearIn` paths of [kernel/iologic.rs](../../../../src/kernel/iologic.rs).
- **ODDRX2F.** Three ranks of SCLK registers, then the ECLK side loads the word at the ECLK
  rising edge where SCLK falls.
- **IDDRX2F.** The ECLK side collects four edges and hands the word over at that same ECLK
  edge; one rank of SCLK registers publishes it.
- **CLKDIVF.** It starts low and rises at the first ECLK rising edge.

[tests/gearbox_native.rs](../../../../tests/gearbox_native.rs) runs `hi03.config` in the event
kernel and in fast mode. Both print exactly the board's line, N included.
- **Before HI04.** The emulator started ECLKO with the first rising edge of ECLKI, so its SCLK
  domain ran two SCLK cycles ahead: N was 64.
- **Now.** ECLKSYNCB's STOP synchronizer starts closed, as HI04 measured.
