# HI05 slice: IO registers with CE, set/reset and GSR (I02)

How the side IOLOGIC's IO registers handle the controls that [HI02](../hi02_iologic/README.md)
left out: a clock enable, a synchronous set/reset, and `GSR=ENABLED` with a user GSR.
yosys maps Lattice's IO register primitives `IFS1P3{B,D,I,J}X` and `OFS1P3{B,D,I,J}X` to IO
registers with exactly these settings. LiteX uses them for its SDR IO, which drives the SDRAM
of the ULX3S:
- `GSR=ENABLED` by default;
- CE from `SP`;
- `B`/`D` an asynchronous preset/clear;
- `I`/`J` a synchronous clear/preset (`SRMODE=LSR_OVER_CE`).

The emulator refused all of them ("IOLOGIC GSR=ENABLED is not measured").

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hi05.v](hi05.v) with [hi05.lpf](hi05.lpf); configuration
  [hi05.config](hi05.config).
  - **Registers.** Eight IO registers on the LEDs, all on one 25 MHz clock:

    | Register | LED | Primitive | REGSET | SRMODE | GSR |
    |---|---|---|---|---|---|
    | O0 | B2 | OFS1P3BX | SET | ASYNC | ENABLED |
    | O1 | C2 | OFS1P3DX | RESET | ASYNC | DISABLED |
    | O2 | C1 | OFS1P3IX | RESET | LSR_OVER_CE | ENABLED |
    | O3 | D2 | OFS1P3JX | SET | LSR_OVER_CE | DISABLED |
    | I0 | D1 | IFS1P3BX | SET | ASYNC | DISABLED |
    | I1 | E2 | IFS1P3DX | RESET | ASYNC | ENABLED |
    | I2 | E1 | IFS1P3IX | RESET | LSR_OVER_CE | DISABLED |
    | I3 | H3 | IFS1P3JX | SET | LSR_OVER_CE | ENABLED |

    nextpnr packs all eight into the IOLOGIC (`CEOMUX`/`CEIMUX=CEMUX`, `CEMUX=CE`,
    `LSRMUX=LSR`).
  - **Controls.** All registers share one data bit `d`, one enable `ce` and one set/reset `sr`.
    These are fabric registers stepped by a 16-bit LFSR: `ce` is high 3/4 of the time and `sr`
    1/8. `gsr_n`, low 1/32 of the time, drives a `GSR` primitive. Everything is idle for the
    first 16 cycles.
  - **Data paths.** The output registers drive their pads and are read back through their
    `BB`. The fabric drives the input registers' pads with `d`.
  - **Build.** With a `GSR` primitive in the design, nextpnr puts every fabric register under
    GSR too. So the build runs yosys `setparam -set GSR "DISABLED" t:TRELLIS_FF a:syn_useioff
    %d` after `synth_ecp5`: the recorder and the UART stay out of GSR, and the IO registers keep
    their own setting. A first build without it printed nothing: the GSR pulses kept resetting
    the UART.
  - **Lines.** Sample = `{d, ce, sr, gsr_n, ob3..ob0, qi3..qi0}` at every 25 MHz edge. Each line
    `E<id> <768 hex>` is 256 samples: E00 from the start, E01–E07 consecutive windows, then E00
    again.
- **Runner:** [run.py](run.py) `hi05.bit E 3 3`: three loads, 3 s each.
- **Model check:** [hi05_model.py](hi05_model.py) predicts every register from the recorded
  controls and compares all 256 samples of every line ([model_check.txt](model_check.txt)).

## Observations

[observations.log](observations.log): three loads, 44 lines each.
- **Windows.** The same 38 window lines in every load.
- **Start.** One E00 per load, in two variants. In load 1 the first sample (t = 0) is 0 in
  every bit, fabric registers included (`gsr_n`'s register starts at 1). In loads 2 and 3 it
  already holds the start values. From sample 1 on, the two variants are equal.

The records exercise:
- 298 samples with `sr` high while `ce` is low;
- 147 GSR pulses;
- every combination of `d`, `ce` and `sr` around the edges.

## Reading

All 40 lines fit one model ([model_check.txt](model_check.txt)). Sample k is the state after
edge k, and R is REGSET.

1. **Clock enable.** A register captures `D` at a rising edge only when CE is high, as a PLC
   flip-flop does.
2. **Synchronous set/reset (`LSR_OVER_CE`, I/J).** `Q(k) = sr(k-1) ? R : ce(k-1) ? D(k-1) :
   Q(k-1)`, so set/reset wins over a low CE.
3. **Asynchronous set/reset (`ASYNC`, B/D).**
   - **While high.** `sr` sets R at once and holds it.
   - **After release.** The edge right after a release still sees `sr` high (it was released
     by a register at that edge), so capture resumes at the edge after.
4. **GSR.**
   - **ENABLED.** A register with `GSR=ENABLED` takes R while the user GSR is active, at once
     (asynchronously), whatever CE and LSR do.
   - **DISABLED.** A register with `GSR=DISABLED` ignores it.
5. **Start.** Every register starts at REGSET, with GSR enabled and disabled alike (as HI02
   found for `GSR=DISABLED`).
6. **Alternatives the data rule out.** In register-lines that differ from the record (8 per
   line, 320 in all):
   - GSR acting at the next edge instead of at once: 131;
   - a synchronous reset that needs CE: 129;
   - ASYNC and LSR_OVER_CE swapped: 320.

**Not covered:**
- controls changing at the same instant as the clock edge (they change one register delay
  after it here);
- the first edge after the start, which in one load of three read every register as 0;
- CE or LSR inverted (`CEMUX=INV`, `LSRMUX=INV`);
- the top and bottom SIOLOGIC (HI02c found them equal to the side IOLOGIC for the plain
  registers);
- the DDR and gearbox paths with CE, GSR or a synchronous reset.

## Emulator check

The emulator executes these IO registers:
- `IoPath::Register` with `enable`, `sync_reset` and `global_reset` in
  [kernel/iologic.rs](../../../../src/kernel/iologic.rs);
- the planner in [compile/iologic.rs](../../../../src/compile/iologic.rs).

The DDR and gearbox paths still require `GSR=DISABLED`, `ASYNC` and no CE.

- **Synthetic plan.** [tests/io_register.rs](../../../../tests/io_register.rs) checks the
  rules above, and fast mode against the event kernel on 30 000 steps of random clock, data,
  CE, set/reset and GSR.
- **HI05 design.** [tests/io_register_native.rs](../../../../tests/io_register_native.rs) runs
  `hi05.config` in fast mode for two rounds. Every line is a board line, E00 and every window
  bit for bit, except the first round's E01. The board prints that line within 0.15 s of the
  load, before the reader opens the port.
- **LiteX-style design.** A small design with `IFS1P3BX`/`OFS1P3BX` the way LiteX instantiates
  them now runs in both executors, with byte-identical VCD files.
