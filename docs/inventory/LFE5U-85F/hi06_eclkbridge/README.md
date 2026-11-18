# HI06 slice: the edge-clock bridge ECLKBRIDGECS and BRGECLKSYNC

What the edge-clock bridge does: ECLKBRIDGECS selects one of two edge clocks with SEL, and
BRGECLKSYNC (an ECLKSYNCB placed after it) synchronizes the result. The guide calls the
bridge "a 2:1 mux with no pulse guarantee". The emulator refused both.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hi06.v](hi06.v) with [hi06.lpf](hi06.lpf), nextpnr seed 1 with
  `--allow-fabric-eclk`.
  - **Clocks.** Two edge clocks from the fabric: `c0` = bit 2 and `c1` = bit 3 of a counter
    on the 25 MHz clock, periods 8 and 16.
  - **Path.** The bridge (`SEL`), an ECLKSYNCB with `STOP` (nextpnr places it on
    BRGECLKSYNC0), CLKDIVF /2, and an ODDRX2F with data 1, 0, 1, 0 that puts ECLKO itself on
    LED E2.
  - **Placement.** nextpnr places the bridge on the right side; its output reaches the left
    bank 7 ECLK0 across the device.
  - **Sample.** Sample = `{c0, c1, sel, pad, p2, n2}` at 25 MHz, where p2/n2 toggle on the
    rising/falling edges of CDIVX. Lines as in [HI04](../hi04_eclk/README.md):
    - E00 is the start;
    - E01–E04 raise SEL at sample 34, 38, 42 or 46 for 68 samples, so it switches in each
      of the four (c0, c1) states and back in another;
    - E05–E07 pulse STOP for 8, 64 and 16 samples.
- **Two configurations:**
  - [hi06.config](hi06.config) as nextpnr writes it. nextpnr 0.11.1 never writes
    `ECLKBRIDGECS.MODE`: `bitstream.cc` sets it only for a cell with a STOP port, a test
    copied from ECLKSYNCB, and the bridge has none.
  - [hi06_mode.config](hi06_mode.config), the same plus
    `enum: ECLKBRIDGECS0.MODE ECLKBRIDGECS` (Trellis bit F48B0 of ECLK_R).
- **Runner:** [run.py](run.py) `BIT E 3 3`, three loads of each.
- **Model check:** [hi06_model.py](hi06_model.py), output in [model_check.txt](model_check.txt).

## Observations

- **nextpnr's build.** [observations_nextpnr.log](observations_nextpnr.log): 29 distinct lines
  a load, the same in all three loads. Each experiment has four lines: the divider runs
  freely between windows.
- **Build with the mode.** [observations_mode.log](observations_mode.log): 8 lines, the same
  in every load and pass.

## Reading

1. **The mode bit decides whether the bridge switches.**
   - **MODE=ECLKBRIDGECS.** ECSOUT = SEL ? CLK1 : CLK0, from the moment SEL changes: a plain
     mux.
     - **Cut pulse.** E02 switches from `c0` (high) to `c1` (low) mid-pulse: ECLKO falls
       two samples early.
     - **Added pulse.** E01 switches back to `c0` while `c0` is high and `c1` low: ECLKO rises
       at once for a two-sample pulse.
   - **MODE=NONE.** ECSOUT is CLK0 whatever SEL is. A design built by nextpnr 0.11.1 with
     ECLKBRIDGECS therefore never switches on the chip.
2. **BRGECLKSYNC** is an ECLKSYNCB: every STOP line fits the four-stage synchronizer that HI04
   measured.
3. **Start.** Both builds started ECLKO with the sixth rising edge of ECLKI, the other of the
   two starts HI04 saw. Four of the eight builds so far started this way.
4. **Fit.**
   - Every line of both logs fits the model, with the mode taken as written (E00 with the
     sixth-edge start).
   - Each log fails against the other mode: 4 lines of the mode build and 16 of nextpnr's.

**Not covered:**
- SEL changing at the same instant as a clock edge;
- clocks from pins or PLLs (nextpnr's dedicated routing; only fabric clocks were used);
- the left-side bridge;
- bridge outputs into both banks at once.

## Emulator check

- **The two blocks.** The emulator executes the bridge
  ([compile/eclk.rs](../../../../src/compile/eclk.rs)) as a mux with MODE=ECLKBRIDGECS and as
  CLK0 with MODE=NONE. BRGECLKSYNC is planned as an ECLKSYNCB. Both are built from parts fast
  mode already runs.
- **Tests.** [tests/eclk_bridge_native.rs](../../../../tests/eclk_bridge_native.rs) runs both
  configurations in fast mode for two passes:
  - **nextpnr's build.** Every experiment line is a board line.
  - **Build with the mode.** In every experiment the columns that do not depend on the
    divider's phase equal the board's: `c0`, `c1`, SEL, and ECLKO on the pad with its cut and
    added pulses.
- **Divider phase.** It differs by one ECLK period, because the emulator starts ECLKO with the
  fifth edge and this build started with the sixth (C05). In that build every window repeats
  the divider's phase, so the whole line differs.
- **Start.** E00 differs in both builds for the same reason.
