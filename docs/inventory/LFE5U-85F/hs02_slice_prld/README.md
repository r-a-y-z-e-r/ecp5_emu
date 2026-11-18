# HS02: slice flip-flop preload

Question: with `LSRMODE=PRLD`, does LSR load M, or the REGSET constant? Does it
happen synchronously or asynchronously, and does LSR override CE? This directory
contains the board-tested generators, eight-setting probes, records and analysis.
Three loads of each design gave identical complete windows.

`hs02.v` free-runs and records fresh 128-clock windows. `hs02s.v` records only the
first window from configuration and resends it. Both instantiate 32 TRELLIS_FF
cells directly on the 25 MHz clock:

| FF indices | LSRMODE | Settings in index order |
|---|---|---|
| 00..07 | PRLD | Eight combinations below |
| 08..15 | PRLD | Second copy |
| 16..23 | PRLD | Third copy |
| 24..31 | LSR | Controls |

Within each block of eight, the settings are:

| Offset | SRMODE | REGSET | CEMUX |
|---|---|---|---|
| 0 | LSR_OVER_CE | RESET | CE |
| 1 | LSR_OVER_CE | RESET | 1 |
| 2 | LSR_OVER_CE | SET | CE |
| 3 | LSR_OVER_CE | SET | 1 |
| 4 | ASYNC | RESET | CE |
| 5 | ASYNC | RESET | 1 |
| 6 | ASYNC | SET | CE |
| 7 | ASYNC | SET | 1 |

GSR is ENABLED, CLKMUX is CLK, and LSRMUX is LSR on every probe. There is no
user GSR primitive. Do not apply HD07/08's blanket fabric GSR override here.

## Traffic and alignment

The 64-bit LFSR has polynomial x^64+x^63+x^61+x^60+1 and initial state
`1D2CACE15EED0001`. For FF i, take seven low bits after 7*i single-bit shifts:
DI is bit 0, M is bit 1, CE is `(bits[3:2] != 0)`, and LSR is
`(bits[6:4] == 0)`. Thus DI/M vary, CE is high about 3/4 and LSR about 1/8.
The LFSR advances **224 single-bit shifts per clock**. All four decoded fields
are registered and initialized from the seed; fields after clock k come from the
new LFSR state. CE/LSR decoding precedes their registers, avoiding decode glitches
on the asynchronous control pins. Traffic runs while the UART sends.

Each DI goes through a kept identity `TRELLIS_COMB` LUT. This matters: nextpnr's
`pack_ffs` requires DI to be driven by a LUT F output to pair the FF and use SD=1.
Otherwise it tries to rename DI to M for general routing, conflicting with the
already connected preload input. The explicit LUT keeps DI and M separate. The
synthesis checker verifies the LUT F to FF DI connection and the distinct M net.

The identity LUT must **omit its unused M connection**. Its M pin shares the
physical M wire with the paired FF, even when the LUT's OFX output is unused.
The original `.M(1'b0)` forced a GND route onto the same wire as the FF's preload
traffic, causing the two-arcs-with-same-sink error. Removing that unused LUT port
fixes routing without changing any FF setting, traffic or placement constraint.
The packer's `pack_ffs` in `ecp5/pack.cc` pairs DI with LUT F; slice checks in
`ecp5/arch_place.cc` track M usage for active mux outputs, but the explicit unused
LUT M connection still generated a conflicting route. No companion FF is needed.

Line: `P<16 hex win_lfsr><1024 hex samples>` followed by CR LF, at 115200 8N1.
Each sample is Q[31:0] (eight hex digits), with FF 0 at bit zero. The first sample
is at the left. `win_lfsr` is the actual LFSR state at sample zero, so every window
is self-contained even when the UART reader missed previous lines.

Sample k is taken **before rising edge k**. A synchronous update at that edge uses
DI/M/CE/LSR for sample k. Sample k+1 can also reflect the asynchronous response to
new inputs for sample k+1. The analysis regenerates both input states. Sample zero
is not assumed to equal REGSET or M; it is reported separately for seed windows.

## Generate and validate

From this directory:

```sh
python3 -B hs02_gen.py > hs02.v
python3 -B hs02_gen.py s > hs02s.v
python3 -B check.py
```

`check.py` compares the files with the generator and attempts the exact
`yosys -q -p 'read_verilog X.v; synth_ecp5 -top top -json .check.json'` for **both**.
On the installed Yosys 0.70+post, that command fails for ASYNC+PRLD:
`Failed to get a constant init value for \Q: \M`. ABC9 processes the native
simulation whitebox, whose `initial Q = M` is not a constant. This is a synthesis
tool limitation; the hardware settings have not been weakened to pass it.

The checker then uses the tested `synth.py` workaround and verifies every named
FF's four experimental parameters, all traffic connections, and the identity LUT
data path. It runs analysis selftests afterward. Temporary JSON stays here and
is removed. `validation.txt` records the default-command failures and the
successful alternate flow with cell counts. **The exact plain command
does not pass for HS02 on this Yosys version.**

For the routing JSON, use:

```sh
python3 -B synth.py
python3 -B synth.py s
```

The helper performs normal synthesis through `map_ffs`, deletes only the library
TRELLIS_FF simulation definition, reads `hs02_ff_bb.vh` (the same native primitive's
blackbox parameter/port interface), and resumes at `map_luts`. This prevents ABC9
from elaborating the invalid dynamic initial value while preserving all 32 native
FFs, including their real PRLD/ASYNC parameters and separate DI/M wiring. No
installed library is changed. The output is `hs02.json` or `hs02s.json`.
The accepted legacy `-abc2`/`-noabc9` options are ignored by this Yosys version and
are not a workaround ([Yosys synthesis source](https://github.com/YosysHQ/yosys/blob/main/techlibs/lattice/synth_lattice.cc)).

Local Yosys 0.70's `ecp5/cells_sim.v` includes `common_sim.vh`, where TRELLIS_FF
(around lines 249..294) has CLK, LSR, CE, DI, **M**, Q and the experimental parameters.
Its `srval` is M for PRLD and the REGSET constant for LSR. Both synchronous and
asynchronous behavioral branches give LSR priority over CE. The asynchronous
branch is sensitive to positive CLK and positive LSR, not changes in M while LSR
is held high; the analysis therefore also tries an assertion-only async model.
These simulation definitions are hypotheses for this experiment, not measurements.

The local nextpnr `ecp5/bitstream.cc`, `write_ff` around lines 798..809, writes
`SLICE?.REG?.LSRMODE` directly from the FF parameter, along with REGSET, CEMUX and
LSR0/1.SRMODE. `pack_ffs` around 230..279 establishes the LUT/FF path described
above. Both variants now route and pass ecppack with nextpnr-ecp5
0.11.1-77, CABGA381, seed 1. `check_routed.py` verifies all 32 probe placements,
parameters, DI/M connections and corresponding configuration fields. Each routed
config contains exactly **24 REGx.LSRMODE PRLD** enums; see `validation.txt`.
That check proved the enum placement, not the consistency of CEMUX=1 with a
concurrently routed CE pin; the board records expose that conflict below.

Alternate-flow synthesis counts (full mux/carry counts are in `validation.txt`):

| Design | TRELLIS_FF | Of these, probe FFs | Identity TRELLIS_COMB | LUT4 |
|---|---:|---:|---:|---:|
| hs02 | 4424 | 32 | 32 | 4906 |
| hs02s | 4424 | 32 | 32 | 4588 |

<a id="planning-session-handoff"></a>

## Board build and collection

Use `hs02.lpf` for both designs and the reference flow's 25 MHz clock constraint.
Copy the two generated Verilog files, `hs02_ff_bb.vh` and `hs02.lpf` to a scratch
directory outside the repository. The tested commands for each variant are:

```sh
series_dir="~/IDE Workspaces/RustroverProjects/ecp5_emu/docs/inventory/LFE5U-85F/hs02_slice_prld"
yosys -q -l yosys_hs02.log -p 'read_verilog hs02.v; synth_ecp5 -top top -run begin:map_luts; delete =TRELLIS_FF; read_verilog -lib hs02_ff_bb.vh; synth_ecp5 -top top -run map_luts: -json hs02.json'
nextpnr-ecp5 --85k --package CABGA381 --seed 1 --json hs02.json --lpf hs02.lpf --textcfg hs02.config --write hs02.routed.json -l nextpnr_hs02.log --quiet
python3 -B "$series_dir/check_routed.py" hs02.routed.json hs02.config
ecppack hs02.config hs02.bit
```

Here `series_dir` is the absolute path to this directory; repeat with hs02s. These
match the supplied build script, adding `--write` for verification. The supplied
flow uses nextpnr's default 12 MHz target; add `--freq 25` for board builds.
Before packing or loading, check each routed `.config`:

```sh
rg -c '^enum: SLICE[A-D]\.REG[01]\.LSRMODE PRLD$' hs02.config
rg '^enum: SLICE[A-D]\.REG[01]\.(LSRMODE|REGSET|SD) ' hs02.config
rg '^enum: (SLICE[A-D]\.CEMUX|LSR[01]\.SRMODE) ' hs02.config
```

There should be **24 PRLD** enums in each configuration. There are more than eight
LSR enums overall because the recorder and traffic generator also use fabric FFs.
Use the routed cell placements for `ff00`..`ff31` to map the exact tile, slice and
register to this design table; confirm the eight LSR controls, REGSET, CEMUX,
LSR0/1.SRMODE, and SD=1 for every probe. Sharing is per slice for CEMUX and per LSR
line for SRMODE, so the FF placement mapping is necessary. Preserve tile context.

`run.py` and `hs02.lpf` are copied from HB11. Board runner examples
(not executed during preparation):

```sh
python3 run.py hs02.bit P 3 12 > observations.log
python3 run.py hs02s.bit P 3 8 > observations_s.log
python3 -B hs02_analyse.py observations.log
python3 -B hs02_analyse.py observations_s.log
python3 -B hs02_analyse.py --selftest
```

## Reading

### Records

There are 132 complete free-running windows, plus one fragment per load (the
runner reported 133 lines). Each s load contains the same configuration window.
All three loads agree on the complete windows. Analyses deduplicate reloads.
Each FF therefore has 16,896 samples and **16,764 checked transitions** in the
free-running record. A three-FF group has 50,292 transitions; this does not count
the repeated board loads three times. `analysis_inputs.sha256` fingerprints the
unchanged Verilog, configs and records.

Results are in `analysis.txt` and `analysis_s.txt`. The first configuration Q is
`CC94ACFE`, exactly the word predicted by initial M on all PRLD probes and REGSET
on the eight LSR controls (0/32 differing bits). This single observation is
consistent with that initialization hypothesis; it does not identify its physical
configuration mechanism.

### PRLD synchronous preload

With LSRMODE=PRLD, SRMODE=LSR_OVER_CE, CEMUX=CE:

```text
if LSR(a): Q(b) = M(a)
else if CE(a): Q(b) = DI(a)
else: Q(b) = Q(a)
```

Both REGSET groups fit **0/50,292** transitions each. LSR overrides CE and REGSET
does not choose the active preload value. All four plain LSR, CEMUX=CE controls
also fit their intended synchronous/asynchronous REGSET models with
0/16,764 mismatches per FF.

### CEMUX=1 conflicted with the routed CE input

This was a design error, not evidence that a clean constant-enable FF has new
semantics. Every probe retained a random CE input even when it requested
CEMUX=1. Trellis implements the constant choice using **CE routing-mux bits**.
For example, PLC2 SLICEA.CEMUX=1 sets `F48B1 F54B0`; those are also bits of the
CE0 source mux. The text config asks for the constant tie and a routed CE source
at the same time. Combining both requests creates a multi-selection rather
than a clean forced-high enable.

The overlap is visible in the installed PLC2 bits database and the actual board
configs. `routing_evidence.json` records the bit footprints, probe locations,
intended CE sources and additional source patterns contained in the union. It
uses saved routing metadata only after verifying that its config is **byte-for-byte
identical** to each board config; no design was rebuilt or rerouted.

The records confirm route-dependent enable behavior:

- Free-running ff01, ff05, ff07, ff11, ff13, ff15, ff21, ff25 and ff29 use the
  random **CE pin**, not constant 1, with zero mismatches after preload-release
  timing is accounted for.
- ff03 is exactly explained by effective `CE_pin OR ff27.DI`: 0/16,764. This is
  a record-specific routing-contamination model, not an intrinsic FF operation.
- ff23 does fit constant enable 1 after release timing is accounted for.
- The other five CEMUX=1 FFs still lack exact models. Their best extended counts
  are shown below; the added mux selections are identified, but their complete
  electrical behavior has not been reconstructed.

| Free-running FF | Best mismatches / 16,764 |
|---|---:|
| ff09 | 1,816 |
| ff17 | 1,857 |
| ff19 | 916 |
| ff27 | 868 |
| ff31 | 1,589 |

In the startup design, whose placement differs, the residuals are ff07 **2/127**,
ff19 **13/127**, and ff31 **3/127**. Every other FF has an exact fitted model.
Even the plain LSR controls exhibit the enable problem, so it cannot be attributed
to PRLD's M input or its preload timing.

For a future clean CEMUX=1 experiment, omit the CE connection or tie CE high so
nextpnr removes the random CE route. The board-tested files here have **not** been
changed or regenerated. Do not implement the above enable overrides as universal
CEMUX=1 emulator semantics.

### ASYNC PRLD follows M, with a release-timing race

For the six PRLD/ASYNC/CEMUX=CE FFs, every mismatch of the old async-level model
occurs at **LSR 1->0**, and none at 0->0, 0->1 or 1->1. The mismatch counts are:

| FF | Old level model errors | Fitted release value | Extended errors |
|---|---:|---|---:|
| ff04 | 892 | new M | 0/16,764 |
| ff06 | 932 | new M | 0/16,764 |
| ff12 | 452 | old M AND new M | 0/16,764 |
| ff14 | 927 | new M | 0/16,764 |
| ff20 | 960 | new M | 0/16,764 |
| ff22 | 973 | new M | 0/16,764 |

In these formulas, a is the previous sample and b the new sample. The fitted
level-preload model, with LSR over CE, is:

```text
if LSR(b): Q(b) = M(b)
else if LSR(a): Q(b) = release(M(a), M(b))
else if effective_CE(a): Q(b) = DI(a)
else: Q(b) = Q(a)
```

For ff04/06/14/20/22 in the free-running design, `release` selects M(b). For ff12
it selects M(a)&M(b). These are exact empirical sample rules, **not** a claim that
PRLD implements an AND gate on release. M and LSR change together after the same
clock edge; differing propagation delays can choose old M, new M, or different
outcomes for rising and falling M. Placement-dependent skew is the explanation
supported by these observations; no analog timing measurement was made.

The s design supports new M for ff04/06/14, AND for ff12/20, and old M for ff22.
Thus ff20 and ff22 change their release rule with placement. The short ff22 s
record also ties with assertion-only models, while the long record excludes them.
There is no single release rule valid for every location or setting group.

M is not merely latched at LSR assertion: while LSR remains high and M changes,
all six valid-enable probes follow new M with **0/773** total mismatches. The
assertion-only latch and clocked-M alternatives are tested numerically in the
reports. For example ff04 gives:

| Timing hypothesis, CE pin | Mismatches |
|---|---:|
| Synchronous preload | 1,868 / 16,764 |
| Async assertion capture | 1,007 / 16,764 |
| M latched on LSR assertion, held | 1,002 / 16,748 |
| Clocked-M async load | 1,896 / 16,764 |
| Async level, new M on release | 0 / 16,764 |

The latched-M model has 16 unknown initial predictions, explicitly excluded.
All M changes occur near clock edges in this stimulus, so arbitrary-time M
transparency versus an equivalent clock-phase-sensitive behavior remains
unidentified. The full causal behavior of the malformed CEMUX=1 routes also
remains unexplained. These limits matter when translating the record into an
emulator; the exact fitted release truth tables are not universal timing rules.

### Reproduce

Python 3 and NumPy (2.5.2 in this run) are required for the extended search.
The scripts read only; none writes or regenerates the designs/configs.

```sh
python3 -B hs02_analyse.py observations_s.log > analysis_s.txt
python3 -B hs02_analyse.py observations.log > analysis.txt
python3 -B hs02_analyse.py --selftest
```

`--legacy` retains the original nominal-enable models. `hs02_models.py` adds
explicit enable hypotheses, assertion-latched/clocked-M models and old/new/AND/OR
release alternatives. Reports show each FF, every setting group's best common
model, and the sum of per-FF best models. A common model may fail even when each
FF has an exact release model; the report preserves that distinction. The
record-specific ff03 enable expression does not resolve the remaining malformed
CE routes. Selftests serialize independent known-model windows and verify fitting;
results are in `analysis_checks.txt`.

Only analysis code, analysis reports and documentation were changed for this
board-record investigation. No board, synthesis, routing, cargo, rserv or commit
operation was performed; all board-tested Verilog and configurations are intact.

## Emulator check

The planner now decodes PRLD's M from the physical FF's M0/M1 slice input,
independently of ordinary SD data selection. `FlipFlop.preload: Option<Input>`
is None for LSR and Some(M) for PRLD (including constant M). In the event kernel,
synchronous LSR loads M over CE; ASYNC follows M while LSR is high, holds after
release, and makes Q X when M changes in the same instant as falling LSR. The
race check spans capture waves at that timestamp. REGSET does not choose the
preload value. Under configured startup, settled initial M initializes Q when
known, otherwise X. This startup rule rests on the single HS02s observation.
CEMUX behavior has not changed.

`tests/hd09_hs02_native.rs` replays hs02s through its routed fabric and reads the
physical FF Q pins: 1,992/1,992 known bits match. It excludes 2,048 CEMUX=1 bits
and 56 async-release race/X bits. The first complete free-running window is
identified by its LFSR header `P721C4B458E5EC737` (window 1, after the incomplete startup UART line), then replayed as regenerated pin traffic at its placed FFs, seeding Q from its first
sample: 1,980/1,980 predicted bits match; 2,048 CEMUX=1 bits, 52 race/X bits and
16 seed bits are excluded. It does not claim to run the intervening UART clocks.
`hs02_probes.json` and `hs02s_probes.json` preserve FF placements from the saved
routing JSON, whose configs were verified byte-identical to the board configs;
the tests check each config's hash. The free replay is a pin-stimulus test; the
startup replay also checks routed M decoding and startup initialization.

`tests/event_kernel.rs` checks synchronous priority, async M following,
release races, CE hold, unknown preload data and configured startup. Fast mode
now runs PRLD (`ed624e7`): synchronous preload wins over CE, ASYNC follows live M
while LSR is active, and startup uses initial M (`StartsUnknown` if unknown).
On simultaneous asynchronous LSR release and M change it stops with error code 5:
`asynchronous PRLD register ... released LSR while M changed in the same instant;
the event kernel makes Q unknown`. The saved pre-capture M/controls and race
flags span every wave at that timestamp. `tests/fast_reset_order.rs` covers
these paths; the fast pin replays in `tests/hd09_hs02_native.rs` match the same
1,992 and 1,980 known bits above. None retains existing LSR/REGSET behavior.
The older refusal recorded in `emulator_checks.txt` describes the preceding
implementation and is superseded by `ed624e7`.

Validation commands and results are in `emulator_checks.txt`. `TOOLS.txt` records
the actual split synthesis flow and board build commands. No board-tested
Verilog/config was regenerated, and `analysis_inputs.sha256` still verifies it.
