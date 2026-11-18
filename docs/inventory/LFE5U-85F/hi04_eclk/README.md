# HI04 slice: the edge-clock controls (C04, C05, I03)

What the controls of the edge-clock blocks do, cycle by cycle:
- ECLKSYNCB's STOP;
- CLKDIVF's RST and ALIGNWD;
- CLKDIVF /3.5;
- where both start after configuration.

[HI03](../hi03_gearbox/README.md) ran these blocks with STOP and RST tied low, ALIGNWD tied low
and /2 only. Its emulated line came out two SCLK cycles early. The guides describe STOP as
stopping on the fourth falling edge and resuming on the fourth rising edge. They give no rule
for RST, ALIGNWD or the /3.5 waveform.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Designs:** [hi04.v](hi04.v) and [hi04b.v](hi04b.v), with the same circuit:
  - **ECLKI.** `eclk_in` is bit 2 of a counter on the 25 MHz clock: period 8, high in phases
    4–7. nextpnr runs with `--allow-fabric-eclk`.
  - **Edge clocks.** `eclk_in` drives two ECLKSYNCB with one STOP. A CLKDIVF takes only one
    edge clock of a bank (CLKDIV0 ECLK0, CLKDIV1 ECLK1), so each ECLKSYNCB feeds its own divider:
    `cd2` (/2) and `cd35` (/3.5).
  - **Observation.**
    - Flip-flops toggle on the rising and on the falling edges of each CDIVX.
    - An ODDRX2F on `cd2` with data 1, 0, 1, 0 puts ECLKO itself on LED E2, read back through
      its `BB`.
    - Sample = `{eclk_in, pad, p2, n2, p35, n35}` at 25 MHz.
  - **Lines.** Each line `E<id> <384 hex>` is 256 samples:
    - E00: samples 0–255 after the start.
    - Every other line: a window that starts in phase 0 of `eclk_in`. A control goes high at
      sample `a` and low `dur` samples later, never at an edge of `eclk_in`.
  - **Experiments.**
    - [hi04.v](hi04.v): STOP for 64 samples at six phases; RST and ALIGNWD of each divider at
      two phases, for 1 and for 24 samples.
    - [hi04b.v](hi04b.v): STOP for 8 and 16 samples; RST of each divider at six phases;
      ALIGNWD after a reset of that divider at sample 10. For /2 every position over four ECLK
      periods, for /3.5 every ECLK half of a CDIVX cycle, each for 8 and for 16 samples.
- **Configurations:** [hi04.config](hi04.config), and [hi04b.config](hi04b.config) (nextpnr
  seed 1). The same HI04b design was also built with seeds 2 and 3
  ([hi04b_seed3.config](hi04b_seed3.config); seed 2 printed what seed 1 printed).
- **Runner:** [run.py](run.py) `BIT E LOADS 3|4`: three loads for HI04 and for HI04b seed 1,
  one load for seeds 2 and 3.
- **Tools:**
  - [hi04_show.py](hi04_show.py) lists the edges in a line.
  - [hi04_model.py](hi04_model.py) checks every line against the model below. For each line
    it searches the free state at the window start (the dividers' phases and the /2 ALIGNWD
    count) and accepts the line only if one state gives all 256 samples
    ([model_check.txt](model_check.txt)).

## Observations

| Log | Loads | Lines a load | Distinct lines |
|---|---:|---:|---:|
| [observations_hi04.log](observations_hi04.log) | 3 | 80 | 54 for 23 experiments, the same in every load |
| [observations_hi04b_seed1.log](observations_hi04b_seed1.log) | 3 | 112–113 | 85 for 39 experiments, the same in every load |
| [observations_hi04b_seed2.log](observations_hi04b_seed2.log) | 1 | 114 | the 85 of seed 1 |
| [observations_hi04b_seed3.log](observations_hi04b_seed3.log) | 1 | 113 | 77: 76 of the 85, and another E00 |

An experiment gives two or three lines because the dividers run freely between windows: each
pass starts them from another phase. Every line shows the state before its event.

## Reading

Every recorded line fits the following model, with no exception:
- 54 lines of HI04;
- 85 lines of HI04b seed 1 and seed 2;
- 77 lines of HI04b seed 3.

Times below are in ECLKI edges.

1. **ECLKSYNCB STOP (C04).**
   - **Synchronizer.** STOP passes four stages clocked by the falling edges of ECLKI.
     ECLKO = ECLKI while the last stage is 0.
   - **Stop.** ECLKO stops low right after the fourth falling edge that sees STOP high. The
     rising edge just before that fourth falling edge still passes, as a whole pulse.
   - **Resume.** ECLKO restarts with the first rising edge after the fourth falling edge that
     sees STOP low.
   - **Short pulses.** A STOP pulse that one falling edge sees removes exactly one rising edge
     of ECLKO, the one after the fourth falling edge; a pulse that two falling edges see
     removes two. So the four stages are a shift register, not a counter.
   - **Coinciding edges.** STOP changing with an edge of ECLKI was not tried.
2. **Start (C05).** The synchronizer starts as if STOP had been high, so ECLKO starts after the
   fourth falling edge of ECLKI, with its fifth rising edge. Both dividers start low and rise
   with ECLKO's first edge.
   - **The other start.** In HI04b seed 3 the gate opened one falling edge later, with the
     sixth rising edge. That build differs from seed 1 only in placement and routing.
   - **HI03.** The two HI03 builds also differed. The recorded one started with the fifth
     edge. The first had the other SCLK phase, which the sixth edge gives (as would the
     fourth).
   - **Count.** At least four of six builds started with the fifth edge. What decides it is
     open.
3. **CLKDIVF RST.**
   - **While high.** RST clears CDIVX at once (asynchronously), whatever the phase, and holds it
     low while high.
   - **After release.** The first rising edge of ECLKO is consumed, and CDIVX rises with the
     second. The pattern then continues from that rise, for /2 and for /3.5 alike, at all six
     release phases.
4. **CLKDIVF /3.5.** CDIVX counts both edges of ECLKO: high for 4 ECLK half periods, low for 3.
   So its rising edges alternate between rising and falling edges of ECLKO, and every cycle
   is the same.
5. **ALIGNWD (I03).**
   - **Sampling.** ALIGNWD is sampled at rising edges of ECLKO, and only its rise counts. A
     pulse that one rising edge sees acts as one that three see; a pulse that no rising edge
     sees does nothing.
   - **Slip.** Two rising edges after the first sample that sees it high, the divider ignores
     that rising edge and the falling edge after it. Everything after the slip is one ECLK
     period later.
   - **/3.5.** Every ALIGNWD slips.
   - **/2.** Only every second ALIGNWD since the last RST slips; the first after a reset never
     does, at any position. With one ECLK period per slip, the /2 word boundary moves by two
     bits every second ALIGNWD. That would fit the one-bit shift per ALIGNWD of the primitive
     tables, if the gearbox moves the other bit itself; HI04 did not observe the gearbox.

**Not covered:**
- controls changing together with an ECLKI edge;
- RST or STOP while ALIGNWD is pending;
- ALIGNWD held across a reset;
- the /2 ALIGNWD count at start-up (only after a reset);
- GSR=ENABLED on CLKDIVF;
- the right bank;
- ECLKBRIDGECS;
- the effect of ALIGNWD on an IDDRX2F word.

## Emulator check

The emulator executes the model:
- ECLKSYNCB as `IoPath::StopSync` plus a mux;
- CLKDIVF as `IoPath::Divider`, /2 and /3.5 ([kernel/iologic.rs](../../../../src/kernel/iologic.rs),
  [compile/eclk.rs](../../../../src/compile/eclk.rs)).

It starts as most builds did: ECLKO with the fifth rising edge of ECLKI.

- **Event kernel.** [tests/edge_clock.rs](../../../../tests/edge_clock.rs) checks the rules
  above on a synthetic plan: the start, STOP, RST, /3.5 and ALIGNWD. It also checks fast mode
  against the kernel over 40 000 units of random STOP, RST and ALIGNWD.
- **Fast mode, HI04 designs.**
  [tests/edge_clock_native.rs](../../../../tests/edge_clock_native.rs) runs `hi04.config` and
  `hi04b.config` for two passes. Every line is a board line, except E01 and E02 of the first
  pass. The board prints those within 70 ms of the load, before the reader opens the port, so
  no load recorded them. Over four passes the emulator printed every line the board recorded.
- **HI03.** With this start, `hi03.config` prints exactly the line of the board, N included
  ([tests/gearbox_native.rs](../../../../tests/gearbox_native.rs)).
