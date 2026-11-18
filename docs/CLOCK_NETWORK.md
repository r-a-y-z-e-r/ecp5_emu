# Global clocks, DCC/DCS, ECLK, PLL, and OSC

Snapshot: October 22, 2026, LFE5U-85F, updated Trellis and nextpnr from [TOOLCHAIN.md](TOOLCHAIN.md). This is collected information for a future model; the executable kernel and the main decoder were not extended by this study.

The main source of behavior is [sysCLOCK PLL/DLL, FPGA-TN-02200-1.4](https://www.latticesemi.com/view_document?document_id=50465), September 2025. The placement metadata were checked against the native library. The nextpnr rules confirm how the software chain is built and are kept separate from observations on silicon.

## Full primary clock network

Each of the four quadrants has 16 channels. The 60 source connections are formed by 56 peripheral DCC and four central inputs from the fabric. This is not 60 channels in each quadrant. All 12 192 positions of the 96×127 grid were checked through Trellis `global_data`: the quadrant, the tap side/column, and the spine coordinates matched an independent parse of `globals.json`.

| Part | Exported |
|---|---:|
| Quadrants | UL/UR: rows 0–46; LL/LR: 47–95, column split 67/68 |
| Tap columns | 13, 31, 58, 78, 96, 114 |
| SPINE | 12 tiles; rows 22/70, column tap−1 |
| Directed SPINE buffers | 192, 16 per tile |
| TAP_DRIVE | 564 tiles, rows 1–94 |
| Directed TAP buffers | 18 048, two sides ×16 per tile |
| Raw wire → segment relations | 197 408 |
| Route configuration group patterns checked against the native API | 7 007 |

The metadata also cover the edge rows 0/95, where there are no ordinary TAP_DRIVE. This is kept as a boundary of the description, not as a buffer drawn in.

In `RoutingGraph.cpp`, some names with `G_` intentionally stay local. nextpnr adds special relations in `globals.cc`/`trellis_import.py`:

| Source name at position (row,col) | Segment representative |
|---|---|
| `G_HPBXnn00` | the same row, the tap column, `L_HPBXnn00` or `R_HPBXnn00` |
| `G_VPTXnn00` | the corresponding spine row/column, the same name |
| `G_HPRXnn00` | (0,0), `G_{quadrant}PCLK{n}` |

These relations do not allow merging all wires of a channel across the whole chip. SPINE/TAP contain programmable directed buffers, each with its own bits. The structure keeps the local names, the segment relations, and the separate switches.

Channel availability inside a PLC is uneven: CLK0/1 receive 0…15; LSR0/1 receive 4,5,6,7,8,14,15; CE0…3 receive 9…15. Channels 0…7 have additional outputs into local routing, listed in `PLC_global_eligibility.csv`. A global channel cannot be connected to an arbitrary control port without an allowed path.

## Controlled clocks

DCCA stops the output at zero without a shortened pulse; the configuration allows a permanent bypass of the CE control. The waveform on p. 24 shows a dependence on phase. A plain `CLKI & CE` does not reproduce it.

**Measured on 2026-11-11 ([HC01](inventory/LFE5U-85F/hc01_dcca/README.md)):** CE is taken at each rising edge of CLKI, and that high phase passes whole or not at all. There is no shortened pulse at a stop and no runt pulse at a start. The emulator executes `MODE=DCCA` this way, in the event kernel and in fast mode. Still open from C02: CE changing together with an edge of CLKI, the state after configuration, and the centre DCCs.

DCSC has two inputs, SEL1:0 and MODESEL, at DCS0 (R22C66) and DCS1
(R70C66). The event kernel executes it through a normal component driver
([src/kernel/dcs.rs](../src/kernel/dcs.rs)); the planner reads the decoded
DCSMODE **configuration bits**. MODESEL=1 is an immediate one-hot mux: SEL=01
passes CLK0, SEL=10 passes CLK1, and 00/11 output 0. Entry can truncate phases.

**Measured HC02, 2026-11-17 ([Reading](inventory/LFE5U-85F/hc02_dcs/README.md#reading)):**
with MODESEL=0, POS follows the old source through its eligible rising edge,
holds 1 through **two subsequent rising edges of the new source**, and then
follows the new source (first visible edge: its next fall). NEG uses falling
edges and holds 0 (first visible edge: the next rise). This adds one whole
new-source cycle to the first-new-edge sequence in the guide. It fits all
113 switches per DCS in both directions, with no observed truncated pulse.
A coincident new edge does not consume the stage launched by the old edge.
The kernel waits for strictly later physical timestamps; it does not infer an
analog SEL setup/hold margin from HC02's 5 ns observer.

The ten-mode static table is exported in `clock_system_research/DCS_steady_selection.csv`.
POS/NEG at SEL=00/11 hold 1/0; CLK0_LOW/HIGH, CLK1_LOW/HIGH, CLK0/CLK1 and
LOW/HIGH have their documented source/constant selections. HC02 measured every
SEL/MODESEL entry for POS/NEG, and SEL=01/10/11 at both MODESEL values for
CLK0_LOW and CLK1_HIGH. **SEL=00 for those two static modes, and all selections
of CLK0_HIGH, CLK1_LOW, CLK0, CLK1, LOW and HIGH, are documented-only.** The
same documented MODESEL override applies to every mode. Trellis `NONE` has
exactly the NEG raw bit pattern; it therefore executes NEG, without inventing
a distinct NONE behavior.

Policies beyond the measured one-hot switch: startup immediately applies the
selected table entry, without invented initial clock edges. A protected
source-to-00/11 transition finishes at the old active edge, then holds; a
00/11-to-source transition holds through two new active edges. Static-mode
selection changes apply their table immediately; their exact glitchless gating
timing was not established by HC02. MODESEL changes cancel pending handshakes:
bypass takes effect instantly; returning to protection uses the currently
followed source. Retargeting while holding restarts the two-new-edge wait;
returning to the old source before its stop cancels the switch. Unknown controls
produce X unless every possible table selection names the same target; an
uncertain handshake edge produces X until a definite new request. These are
explicit digital policies, not additional silicon measurements.

If the new source does not run, the kernel stays at the documented hold level
after the old active edge, indefinitely. **No timeout is invented; HC02 did not
measure stopped sources.** An old source that stops before acknowledging SEL
continues to hold its last output. Exact setup/hold, startup silicon behavior,
rapid retargeting, sub-resolution glitches and the anomalous static observer
record remain open. Fast mode implements the same table and handshakes in
per-simulation slots, capturing inputs within its derived-clock wave loop.
Its same-timestamp guard excludes coincident new edges, including later waves
and DTR host pauses. Unknown DCSOUT or an uncertain handshake is refused with
a DCSC diagnostic because fast mode is two-valued; irrelevant unknown constants
are allowed when every possible interpretation names the same target.

The guide's p. 21 figure incorrectly labels four inputs/SEL bits; p. 22 and the
physical ports define two. Three tool probes also found that nextpnr's writer
reads DCSMODE from the **attribute**, while its declaration/timing constraints
read the parameter. HC02 sets both and confirms POS/NEG and CLK0_LOW/CLK1_HIGH
in the routed configs. Default router1 fully routed both HC02 builds; the
historical router1 assertion was not reproduced. The emulator follows mode bits,
so the attribute/parameter quirk has no effect on execution.

ECLKBRIDGECS is a 2:1 mux with no pulse guarantee; ECLKSYNCB is needed after it. The bridge occupies ECLK1 of both banks on the left or ECLK0 of both banks on the right. The four CLKDIVF support 2 and 3.5, an asynchronous active-high RST, and ALIGNWD. The exact sequence of the half periods of 3.5 and the phase after reset remained C05 (both measured since, in HI04 below).

For ECLKSYNCB, p. 33 directly specifies a stop on the fourth falling edge after STOP and a resume on the fourth rising edge after STOP is released. The text and the diagram were inspected. HI13 below qualifies the initial state by source history; coinciding events remain unmeasured. The eight fixed ECLKI→ECLKO links of the database cannot be executed unconditionally in parallel with the STOP model: the bypass question C04 was kept (HI04 below measured STOP, and the emulator treats the link as the block).

**Measured on 2026-11-11 ([HI03](inventory/LFE5U-85F/hi03_gearbox/README.md), [HI04](inventory/LFE5U-85F/hi04_eclk/README.md)):** edge clocks made in the fabric (nextpnr `--allow-fabric-eclk`) through ECLKSYNCB into CLKDIVF /2 and /3.5, with an ODDRX2F and an IDDRX2F on them (HI03) and every control of the blocks exercised (HI04). Every recorded line fits the model below. The emulator executes it, in the event kernel and in fast mode ([compile/eclk.rs](../src/compile/eclk.rs), [kernel/iologic.rs](../src/kernel/iologic.rs)):
- **ECLKSYNCB (C04).**
  - **Synchronizer.** STOP passes four stages clocked by the falling edges of ECLKI, and ECLKO = ECLKI while the last stage is 0.
  - **Stop and resume.** ECLKO stops low right after the fourth falling edge that sees STOP high, with no runt pulse. It restarts with the first rising edge after the fourth falling edge that sees STOP low. A short STOP removes one rising edge for each falling edge that saw it.
  - **Start (C05), fabric-generated ECLKI.** HI03/HI04/HI06 observe the fourth or fifth falling edge before the first passed rising edge; the emulator initializes the stages to 1 and takes the fifth rising edge. HI12/HI13 find no such deficit with a continuously running primary pin clock; its stages start cleared. HI13's delayed DCCA retains the four-edge deficit. See the source rule below.
  - **Database link.** The fixed ECLKI→ECLKO link is the block itself. Its unique owner and MODE/STOP profile are interpreted before execution; only qualified NONE may take the buffer fallback.
- **ECLKBRIDGECS ([HI06](inventory/LFE5U-85F/hi06_eclkbridge/README.md)).**
  - **MODE=ECLKBRIDGECS.** ECSOUT = SEL ? CLK1 : CLK0 from the moment SEL changes: a plain mux,
    so a switch can cut a pulse short or add a short one.
  - **MODE=NONE.** ECSOUT is CLK0 whatever SEL is.
  - **nextpnr.** nextpnr 0.11.1 never writes the mode: it tests the cell for a STOP port, as
    for ECLKSYNCB, and the bridge has none. So its bridge designs do not switch on the chip,
    and the emulator, which follows the bits, does not switch them either.
  - **BRGECLKSYNC.** The synchronizer after the bridge is an ECLKSYNCB (`MODE ECLKSYNCB`),
    with the same STOP behaviour.
- **TRELLIS_ECLKBUF.** A buffer onto the bank's edge clock `G_BANK<bank>ECLK<n>`. It reaches every IO tile of the bank as `BNK_ECLK<n>` (the bank of the tile's PIOs). A CLKDIVF takes one edge clock: CLKDIV0 ECLK0 and CLKDIV1 ECLK1 of bank 6 or 7 on the left. An ECLKSYNCB of one bank reaches the other bank's edge clock through `JNEIGHBORECLK`.
- **CLKDIVF.**
  - **Ratios.** /2 toggles CDIVX at each rising edge of CLKI. /3.5 counts both edges: CDIVX is high for 4 half periods of CLKI and low for 3. In HI04's newly started fabric-clock chain both start low and rise with the first passed edge. HI12's running pin clock leaves a free divider phase at the start of fabric counting.
  - **RST.** RST clears CDIVX at once. After its release the first rising edge is consumed, and CDIVX rises with the second.
  - **ALIGNWD.**
    - **Sampling.** ALIGNWD is sampled at rising edges, and only its rise counts.
    - **Slip.** Two rising edges after the first sample that sees it high, the divider ignores that rising edge and the falling edge after it: one CLKI period of slip.
    - **Which assertions slip.** /3.5 slips on every ALIGNWD. /2 slips on every second one since the last RST (I03).
  - **Rejected.** GSR=ENABLED.

## HI12: MODE NONE and pin-clock startup

[HI12 run_2](inventory/LFE5U-85F/hi12_eclksync_none/README.md#reading) closes R09:
MODE NONE on the used bank-7 lane-1 ECLKI -> ECLKO path acts as a plain buffer.
Default/explicit /2 count every input edge from the start of fabric counting:
I=P+F at both snapshots. /3.5 has a 14-sample period at the 100 MHz observer,
high for 8 samples and low for 6; both snapshots fit rounded 2I/7 counts with
one common initial divider phase. /3.5 does not uniquely identify a sub-period
startup offset from those cumulative counts alone. No additional gate or startup
deficit is needed for NONE. Execution remains qualified to unrouted STOP; a
connected logic/constant STOP is refused. Other banks/lanes and BRGECLKSYNC NONE
inherit this implementation, with no direct HI12 measurement of those placements.

<a id="eclksync-startup"></a>

### HI13: enabled ECLKSYNCB start relative to fabric wake-up

[HI13 run_1](inventory/LFE5U-85F/hi13_eclksync_start/README.md#reading)
closes the enabled-start accuracy item recorded with R09. Three identical SRAM
loads compare two STOP=0 chains. Lane P's primary pin clock runs through
configuration: (P_r,P_f)=(262144,262144) at I=524288 and (524288,524288) at
I=1048576, with zero deficit, as in HI12 sync_2. Lane G's DCCA CE rises after
64 pin cycles: G_in=I-64, and its /2 counts are (262110,262110), then
(524254,524254), a constant four-ECLKI-period deficit. G_in=A at both snapshots
independently qualifies the DCCA input count.

The planner keeps HI04's four falling-edge STOP stages and chooses their
configuration initial values from the selected ECLKI source:

- **0:** a primary clock input pin through routes/buffers, including DCC bypass
  and DCCA with CE tied to constant 1. This represents a clock already running
  through configuration; enabled ECLKSYNCB passes the first observed edge.
- **1:** fabric logic or DCC/DCCA with CE other than constant 1. Once ECLKI
  begins toggling with STOP=0, four rising edges are swallowed and the fifth
  passes, preserving HI04 and HI13 G.
- **1, unmeasured:** PLL or OSCG outputs. Their configuration-time activity was
  not measured by HI04, HI12 or HI13; the emulator deliberately retains HI04's
  start rather than inferring activity from their reference input.

All four stages, including the output gate, use that initial value. Runtime
STOP behavior is unchanged. Fast mode copies the kernel's initial stages;
`tests/eclksync_start.rs` replays packed HI13 and HI12 sync_2 images with full
per-signal event/fast trace equality and both board-count snapshots.
`tests/eclksync_none.rs` covers pin, constant/dynamic DCCA, fabric, PLL and OSCG
source classification. HI04 native board-line tests retain the fabric rule.

This is a source-history initialization model, not a simulation of configuration,
PUR or a measured activation timeline. An externally withheld primary pin clock
is not distinguished from a pre-running pin. HI04's occasional extra edge,
/3.5's free board boot phase, and PLL/OSCG startup history remain unmeasured or
unmodelled; none changes the four-stage runtime STOP pipeline. Historical HI12
replay files retain the earlier four-edge pin prediction.

## Four PLLs

The corners each have one PLL with four outputs and a shared state. The top settings are split between PLL0 and PLL1; the bottom ones between PLL0 and BANKREF8/BANKREF4. The bank fields do not become PLL fields. The research catalog now contains PLL/DCC/DCS; the main runtime catalog still lacks these fields.

The nextpnr writer encodes CLKI_DIV, CLKFB_DIV, and the four output DIV as the logical value minus one. CPHASE/FPHASE are written directly. Two circuits obtained through ecppll confirmed these rules for external and internal feedback. The choice of feedback, the source of the output divider, REFCLK-bypass, the static enables, the dynamic ENCLK, RESET/STDBY, and the PHASE ports must remain separate settings.

For correct periodic feedback through the output with divider Dfb:

`Fvco = Fin × CLKFB_DIV × Dfb / CLKI_DIV`, `Fout = Fvco / Dout`.

This is the frequency relation used by nextpnr; an arbitrary external CLKFB is not described by it. Page 42 also allows REFCLK-bypass with or without the use of a divider. The bypass path requires a separate reconstruction by fields, not a substitution of the VCO frequency.

Collected from pp. 40–44: RESET zeroes the ordinary outputs and the dividers, bypass is not reset; STDBY switches off the VCO while keeping the programming; LOCK has sticky/unsticky modes. PHASESEL 00/01/10/11 selects OS/OS2/OS3/OP; PHASEDIR 0/1 selects delay/advance. PHASESTEP is captured on the falling edge, the minimum step corresponds to T_VCO/8; PHASELOADREG refers to the post-divider. Dynamic shifting of the feedback output is prohibited by the document because of the loss of lock.

**Update 2026-10-29:** after configuration with a running reference, the board's PLL is already locked when user logic starts (HC05, [hc05_pll_lock](inventory/LFE5U-85F/hc05_pll_lock/README.md)), so for start-up the conflict below does not matter. The emulator executes EHXPLLL as described in [PLL_EXECUTION.md](PLL_EXECUTION.md). Relock after RST/STDBY stays open.

The guide has a conflict in the LOCK time: pp. 40/42 give 16 **ms**, p. 44 after standby gives 16 **us**. There are also ambiguities in the PHASELOADREG formula and in the figure captions. Therefore the LOCK time, the wrap/load of the phases, and the transient pulses remain C06/C07, with an explicit future digital profile and the HC05 experiments. A full analog loop is not required for this.

## OSC

**Update 2026-10-29:** the board slice HC06 ([hc06_osc](inventory/LFE5U-85F/hc06_osc/README.md)) measured the divider of all 63 `OSC.DIV` codes and the base frequency of one chip (311.8–311.97 MHz). The divider is exact up to 31, even up to 64 and a multiple of 4 up to 128, so the aliases below are resolved by the chip itself. The emulator executes OSCG as described in [OSC_EXECUTION.md](OSC_EXECUTION.md).

One physical oscillator serves the user OSCG, the configuration controller, and SED through different dividers. 310 MHz is the nominal value; the frequency of a specific instance is set by an environment parameter or by a measurement.

For OSC.DIV the database contains 126 names 2…127, but the joint patterns of the two configuration fragments give only **63 different codes**. There are groups of several values, not just a simple pairwise match. For example, 32/33 and 95/96/97/98 are indistinguishable by the corresponding groups of bits. The original HDL divider cannot be recovered from them unambiguously. The full table is kept in `OSC_code_aliases.csv`.

nextpnr additionally replaces a request of DIV=128 by the label 127. The probes DIV=2/3/127/128 confirm the write. The bit F508B1135, shared with CCLK.MODE, is kept for both owners: the writer uses `_NONE_` for CCLK so as not to overwrite OSC. The actual ratios of all raw codes require HY02.

## Data, checks, and boundaries

[clock_system_research](inventory/LFE5U-85F/clock_system_research/evidence_report.json) contains 108 resources, 384 ports of the selected part, 337 composite fields/343 fragments, and 1 331 verified native groups of their encoding. This is a subset of the general catalog, without DQS/DDRDLL. The 60 DCC include four central ones, which the ordinary BEL constructor of the full chip does not create.

Nine software probes were obtained after placement with `--no-route`: they check the transfer of the settings into `.config`, not a full routing or the behavior of the chip. The first attempt at a full DCS routing ended with a router1 assertion; the log is kept in `target/research/clock_system/probes/initial_dcs_router1.log`. This is not masked by the successful placed probes.

```sh
python3 tools/collect_clock_system.py
```

Questions C01–C10 and Y01–Y09, rules CS01–CS27, and the future HC/HY series are kept in the [machine contract](inventory/LFE5U-85F/clock_system_research/clock_system_contract.json). [DQS_CONTROL.md](DQS_CONTROL.md) supplements this section with the DLL/DDRDLL, DDRDEL, and FIFO.

Pinned primary sources: [Trellis RoutingGraph.cpp](https://github.com/YosysHQ/prjtrellis/blob/a31ff7042e712c0e2ad217054116ce2c4995f5c7/libtrellis/src/RoutingGraph.cpp), [nextpnr globals.cc](https://github.com/YosysHQ/nextpnr/blob/6b438a1c9ab83cfb872fea21d9545d7d0bb8573f/ecp5/globals.cc), [nextpnr bitstream.cc](https://github.com/YosysHQ/nextpnr/blob/6b438a1c9ab83cfb872fea21d9545d7d0bb8573f/ecp5/bitstream.cc). The hashes of the specific local files are in the collection report.
