# HY01: synchronous user global reset (SGSR), slice FFs

Measured on the ULX3S LFE5U-85/CABGA381 on 2026-11-17, using JTAG SRAM loads;
flash was untouched. The records and measured rule are in Reading below.
[TOOLS.txt](TOOLS.txt) identifies tools and provenance, and [SHA256SUMS](SHA256SUMS)
fingerprints the delivered evidence and sources. The initial local build checks
are preserved in [validation.txt](validation.txt); the subsequent startup-counter
build checks are in [validation_boot.txt](validation_boot.txt).

The emulator's `src/compile/system.rs` executes `GSR.SYNCMODE=SYNC` using the
accepted HY01 rule below, closing the former R15/X20 refusal in
[REFUSAL_AUDIT.md](../../../REFUSAL_AUDIT.md). The public Yosys primitive
`SGSR u(.GSR(gsr_n), .CLK(clk_25mhz))` is packed into that setting. The async
user GSR already executes. Context: [SYSTEM_STARTUP.md](../../../SYSTEM_STARTUP.md),
[HD07 DSP GSR](../hd07_dsp_gsr/README.md),
[HS01 initialization](../hs01_init_slice/README.md),
[HS02 slice preload](../hs02_slice_prld/README.md),
[HI05 IO GSR](../hi05_ioreg/README.md), and
[HI09 DDR GSR](../hi09_ddr_reset/README.md). Those measurements do not establish
SGSR's assertion/release latency or its effects across clock domains.

## Experiment

[hy01_gen.py](hy01_gen.py) writes synthesizable Verilog, with a positional `s`
for the startup capture. There is exactly one SGSR, with active-low GSR_N and
the 25 MHz oscillator on its CLK pin. A PLL generates a 100 MHz observer/pulse
clock and a 600/17 MHz (35.2941176 MHz) probe clock from a 600 MHz VCO. The latter
has a different frequency and phase progression; **it is related to the other
clocks through the PLL, not an independent oscillator**. Relative phases repeat.

Six native TRELLIS_FF probes have CE tied high, LSR tied low, `CEMUX="1"`,
`LSRMODE="LSR"` (the primitive default), and `SRMODE="ASYNC"`. Local reset is
inactive, so SRMODE does not choose the global-reset behavior.

| Group | Probe names | Active clock edge | REGSET |
|---|---|---|---|
| same | `probe_0_reset`, `probe_0_set` | 25 MHz rising, also SGSR CLK | RESET, SET |
| inverted | `probe_1_reset`, `probe_1_set` | 25 MHz falling (`CLKMUX="INV"`) | RESET, SET |
| other | `probe_2_reset`, `probe_2_set` | 600/17 MHz rising | RESET, SET |

Each group has its own GSR-disabled toggle FF `phase[g]`, clocked on the same
edge as its probes. Both probes' D inputs are that toggle FF's Q. D toggles at
every local clock edge even while the probes are reset. At an ordinary capture,
the probes take the old D; after the edge, `Q = ~phase[g]`. The recorded phase
toggles are clock-edge markers. Clocks are used on clock pins, not sampled on
fabric data pins. Pair divergence to `(reset_Q,set_Q)=(0,1)` makes a held reset
visible independently of whether one FF happened already to equal its REGSET.

A 32-bit LFSR (`48593031`, feedback bits 31,21,1,0) runs continuously on the
100 MHz clock. Each fresh window chooses onset `128+lfsr[5:0]` and duration
from `lfsr[8:6]`: choices 0/1/2/3 select 1/2/3/5 oscillator cycles, and 4..7
select 20. These are exactly 4/8/12/20/80 observer periods (40..800 ns). The
registered GSR_N falls after the onset edge and rises after onset+duration.
The four possible 10 ns phase offsets relative to the 25 MHz clock exercise
between-edge timing, including bins around the falling edge. Physical phases
and CDC bin skew must be read from the markers, not inferred from a UART timestamp.
Each 512-sample window contains prehistory and both input edges, followed by
at least 240 observer ticks of posthistory. GSR_N stays high during UART output.

`hy01` waits for synchronized PLL LOCK and 4096 further fast edges, then
captures a fresh window for each UART request. `hy01s` starts recording on the
first PLL fast edge after configuration and repeats that first window forever;
its single pulse has onset 128 and duration 80 ticks. The observer's initialized
pipeline makes the first two samples zero. They are **not** the initial probe
Q values. Before LOCK the PLL period/phase is not established; there is no
measurement of configuration-to-first-clock time. This variant records what
the running observer can see during startup, rather than establishing the
physical wake-up sequence or the probes' initial REGSET by itself.

## GSR isolation and routed checks

All six explicit probe cells request `.GSR("ENABLED")` and carry `keep=1` and
`hy01_probe=1` attributes. As in HD07/HI05, the build then applies an exhaustive
post-`synth_ecp5` override: set **every** `TRELLIS_FF` to `GSR="DISABLED"`, then
set **only** `c:probe_*` back to `ENABLED`. The six-cell name set is asserted in
the synthesis JSON. Inferred FFs' GSR defaults/initialization attributes alone
are not trusted. This covers the LFSR, pulse FF, toggle sources, both observation
stages, recorder, handshake and UART. Inferred DP16KD also explicitly receives
`GSR="DISABLED"`. Initialization still uses REGSET; disabling user GSR does not
remove configuration initialization (HS01).

[check_routed.py](check_routed.py) maps **every routed TRELLIS_FF**, using its
`NEXTPNR_BEL`, to the tile's `SLICE?.GSR` enum. It checks that all support FFs
are disabled and all six probes enabled, including the probe REGSET enums.
This matters because GSR is shared by a physical slice. It verifies probe
CLKMUX parameters, the connection of SGSR CLK to the same-clock probes, a
different clock on the other group, and disabled EBR GSR. It checks both
`GSR.SYNCMODE SYNC` and `GSR.GSRMODE ACTIVE_LOW`; these live in different tiles
on this device. Checking just a global grep count is insufficient.

## Wire format and timing

UART: 115200 8N1, actually 25 MHz / 217 = 115207.37 baud. Uppercase hex:

```text
Y1 V QQQQQQQQ SSSSSSSS [512 * WWWW] XXXX CR LF
```

Spaces/brackets are explanatory and are not sent. `V=R` is recurring capture,
`V=S` is startup replay; Q is the UART sequence number modulo 2^32 (it advances
also on startup replay); S is the LFSR at sample zero, informational rather than
a promise to reconstruct clock phase. Samples are chronological, most significant
nibble first. X is the 16-bit XOR of all samples. Each frame is 2073 bytes with
CR/LF, 2071 without. RAM/header/checksum stay frozen until a complete LF has
left the UART; a two-stage toggle handshake crosses between clock domains.

| Sample bits | Meaning |
|---|---|
| 15 | observed PLL LOCK |
| 14 | observed GSR_N |
| 13:11 | phase[2:0], toggles on other / inverted / same clock edges |
| 10:5 | Q[5:0]: other SET, other RESET, inverted SET, inverted RESET, same SET, same RESET |
| 4:0 | zero, reserved |

GSR_N, the three markers, six probe Qs, and LOCK all pass through identical
two-stage observation pipelines before RAM capture. Sample k is the pre-edge
value of stage two at capture edge k. Nominally it represents physical events
in `(k-3,k-2]` fast-clock periods. A generator change made just after edge j
first appears in sample j+3. The common delay cancels in relative timing, but
independent input paths, probe propagation, and metastability can move channels
into adjacent bins or occasionally further. The two Q bits of a pair are not
an atomic CDC bus. Default analysis permits +/-1 bin, reports exact residuals
as well, and uses at least two equal held/not-held bins to reject isolated
pair skew. This is a 10 ns observation experiment, not an analog timing test.

## Exact build commands (no board access)

Run from the repository root. The helper writes all build products to external scratch
and refuses a scratch path inside the repository. On rserv first set
`export TMPDIR="$HOME/work/tmp"`; its RAM-backed temporary filesystems are full:

```sh
series_dir="$(pwd)/docs/inventory/LFE5U-85F/hy01_sgsr"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/hy01.XXXXXX")"
python3 -B "$series_dir/build.py" "$work_dir"
python3 -B "$series_dir/check_capture.py" "$work_dir"
python3 -B "$series_dir/hy01_analyse.py" --selftest
```

`build.py` runs the following exact synthesis/place/route/packing flow for
both variants. These are also complete manual commands:

```sh
python3 -B "$series_dir/hy01_gen.py" > "$work_dir/hy01.v"
python3 -B "$series_dir/hy01_gen.py" s > "$work_dir/hy01s.v"
cp "$series_dir/hy01.lpf" "$work_dir/hy01.lpf"
cd "$work_dir"
for design in hy01 hy01s; do
  yosys -Q -q -l "$design.yosys.log" -p "read_verilog $design.v; synth_ecp5 -top top; setparam -set GSR \"DISABLED\" t:TRELLIS_FF; setparam -set GSR \"ENABLED\" c:probe_*; setparam -set GSR \"DISABLED\" t:DP16KD; write_json $design.json"
  nextpnr-ecp5 --85k --package CABGA381 --seed 1 --json "$design.json" --lpf hy01.lpf --freq 25 --textcfg "$design.config" --write "$design.routed.json" -l "$design.nextpnr.log" --quiet
  python3 -B "$series_dir/check_routed.py" "$design.routed.json" "$design.config"
  rg -n '^enum: GSR\.(SYNCMODE|GSRMODE) ' "$design.config"
  rg '^enum: (SLICE[A-D]\.GSR|SLICE[A-D]\.REG[01]\.REGSET|EBR[0-9]+\.GSR) ' "$design.config" > "$design.gsr-enums.txt"
  ecppack "$design.config" "$design.bit"
done
```

The 25 MHz input is constrained in [hy01.lpf](hy01.lpf). nextpnr derives the
100 MHz and 600/17 MHz PLL constraints; no timing-allow-fail flag is used.
Inspect the post-route frequency report, not only the placement estimate.
`check_capture.py` needs Icarus (`iverilog`, `vvp`) and uses simplified clock/FF
fixtures that deliberately ignore GSR. It decodes the serial **wire**, verifies
RAM/hex framing/XOR and all five widths/four phases, and checks that startup
replays one frozen payload. Only its UART divider is accelerated. This validates
the recorder's plumbing, rather than measuring silicon SGSR behavior.

<a id="planning-session-load-collect-and-analyse"></a>

## Load, collect and analyse

The commands below reproduce the board collection. They need
`openFPGALoader`, Python `pyserial`, and the board's actual serial device.
No board or serial access was performed during the subsequent residual analysis
or startup-counter development.
[run.py](run.py) loads SRAM first and checks successful DONE, **then** opens
serial and clears its input buffer, following HB11's ordering. Flash is not
written. It prints load metadata as `#` lines and raw candidate frames as
individual lines. The analyser rejects malformed/truncated/checksum-failing
frames and deduplicates identical payloads across replays and loads.

```sh
python3 -B "$series_dir/run.py" "$work_dir/hy01.bit" --port /dev/cu.usbserial-<FTDI-serial> --loads 3 --seconds 20 > "$work_dir/observations.log"
python3 -B "$series_dir/run.py" "$work_dir/hy01s.bit" --port /dev/cu.usbserial-<FTDI-serial> --loads 3 --seconds 8 > "$work_dir/observations_s.log"
python3 -B "$series_dir/hy01_analyse.py" "$work_dir/observations.log" --events-csv "$work_dir/events.csv" > "$work_dir/analysis.txt"
python3 -B "$series_dir/hy01_analyse.py" "$work_dir/observations_s.log" --events-csv "$work_dir/events_s.csv" > "$work_dir/analysis_s.txt"
python3 -B "$series_dir/hy01_analyse.py" "$work_dir/observations.log" --tolerance 0 --max-k 4 > "$work_dir/analysis_exact.txt"
```

## Analysis and interpretation

[hy01_analyse.py](hy01_analyse.py) uses only the Python standard library.
It reports measured pulse widths and assertion/release phase coverage; missing
widths or fewer than three observed phase bins are flagged as incomplete.
It excludes initial/unlocked data and enough following bins to flush the fitted
synchronizer history. For each group and REGSET it fits complete Q traces to
fully asynchronous reset, synchronous assertion **and** release, asynchronous
assertion with synchronous release, and an ignored-reset negative control.
It also reports a common six-FF fit. Both reference-edge sampling orders and
both reset-release/capture orders within a bin are tried. All ties are retained.

For synchronous reset, assertion `ka` and release `kr` independently range
from 1 to `--max-k` (default 4); mixed reset has asynchronous assertion and
`kr=1..max-k`. The first reference edge that samples the new input counts as
edge 1. Equal ka/kr is a shift pipeline that propagates a sampled short pulse;
unequal values model separate assertion/release transition transport. If their
delays reorder a narrow pulse's edges, that candidate can leave a held reset
and should fail the observed posthistory, rather than silently dropping the
pulse. Mixed mode asynchronously fills its reset pipeline and shifts zeros
after release. Each candidate is tried with the SGSR CLK, inverted clock, and
other clock as reference, to test which edges explain the latency. These are
explicit hypotheses; failure of every candidate calls for a wider model or
better observations, not an emulator rule inferred from a best nonzero score.

Every Q prediction uses the recorded local edge markers and D toggles, with
reset taking the requested REGSET and normal capture taking the old D. The
assumption is a global reset distributed to probes after the candidate
synchronizer. A different winning reference per group or a failing joint fit
may indicate local-clock behavior or CDC uncertainty; it does not identify
undocumented internal circuitry. Exact errors are shown alongside symmetric
errors allowing +/-1 bin. A tolerant zero is a resolution-limited fit; especially
at 35 MHz, a one-bin allowance can hide real distinctions. Inspect exact
residuals and use multiple phase offsets before selecting a model.

Without `--events-csv`, every assertion/release's six rows print to stdout.
CSV rows include the input tick, first stable pair hold / resumed-capture tick,
latency in observer ticks, and number of intervening SGSR rising edges, with
an uncertainty interval. Per-REGSET Q transition/departure ticks, latencies and
edge counts are separate columns. An assertion when Q was already at REGSET
is explicitly censored. A release becomes visible only when a local capture
occurs, and one REGSET may need another edge to depart; the pair's first
equality bounds release rather than locating its invisible physical instant.
Delayed responses to short pulses are associated even after the input release.
Missed/overlapping responses and startup events remain marked unresolved.

`--selftest` uses an independent queued-event fixture to identify async,
sync(2,2), sync(1,3), mixed(2), and ignored behavior across all groups and both
REGSET values, including coincident/between-edge phases, five pulse widths,
delayed short pulses, censoring, damaged frames and observation tolerance.
The original fitter is retained for comparison; `hy01_residuals.py --candidate`
checks the accepted raw-active-OR-sync2 rule and asserts the two-tick timing
hazard bound for every exact residual. Its selftest checks literal stage-output
timing, both REGSET values, all clocks, startup offsets and the hazard bound.

## Reading

[observations.log](observations.log) contains three
20-second loads with 108/95/110 candidate lines: 313 complete checksum-valid
frames, 310 distinct windows after payload deduplication. There are no rejected
candidate frames. The saved [hy01.config](hy01.config) and
[hy01s.config](hy01s.config) match the SHA256 values in the original local
build record; the board used the bitstreams that `build.py` produced in scratch.
The local routed FF checks therefore apply to these configurations.
The supplied [analysis.txt](analysis.txt) and [analysis_s.txt](analysis_s.txt)
are preserved unchanged.

**Assertion is asynchronous to REGSET.** Every enabled probe group reaches
the RESET/SET pair in the same observer bin as GSR_N falls: zero relative ticks
in all 310 assertions, including the 35 one-cycle pulses and all four phase
bins. This establishes assertion without waiting for SGSR CLK, at the 10 ns
observation resolution. Naturally equal individual Q values are not themselves
proof of a reset; the differing REGSET pair supplies the evidence.

**The regular release follows a two-stage synchronizer on SGSR CLK rising
edges.** When GSR_N rises, the first rising edge that samples it high advances
the first stage. The second such edge releases the global reset after that
edge; FFs clocked on SGSR CLK still see reset at that second edge and first
capture at the third. The inverted-clock and other-clock FFs are released at
that same global instant, then capture on their next eligible local edge;
they do not wait for two edges of their own clocks. This is the measured
functional rule for these loads, with the short-pulse and boundary exceptions
below; it does not identify the internal gate topology or wake-up state.

| GSR_N release phase, ticks since CLK rising marker | Windows | Regular same-clock first capture, ticks after release |
|---|---:|---:|
| 0 (same bin as CLK rising) | 87 | 8 |
| 1 | 75 | 11 |
| 2 | 73 | 10 |
| 3 | 75 | 9 |

Every regular same-clock pair capture follows this table. In the original
fitter (which lacks raw-active-OR-sync2), the joint best hypothesis is `async-assert/sync-release(kr=2,ref=same,old-reset,new-input)`. Its **154
mismatching channel-bins** are out of 151,590 compared bins **per FF**, or
909,540 across all six FFs; the individual counts are:

| Group | REGSET RESET | REGSET SET |
|---|---:|---:|
| same | 19 | 16 |
| inverted | 21 | 33 |
| other | 39 | 26 |

[hy01_residuals.py](hy01_residuals.py) enumerates every residual, including
zero-based unique-window ID, all source load/sequence/line occurrences, input
edge ticks, phase, pulse width, group, REGSET, every mismatching sample and its
offset in ticks/clock edges. Reproduce [residuals.txt](residuals.txt) with:

```sh
python3 -B "$series_dir/hy01_residuals.py" "$series_dir/observations.log" "$series_dir/observations_s.log" > "$series_dir/residuals.txt"
```

There are two distinct patterns, affecting 50 windows in total (two windows
contain both patterns). **The simple two-stage model is not bit-exact for short
pulses.** All 35 one-cycle pulses show a brief Q departure after raw release,
followed by return to REGSET, before the regular synchronized release. These
are 120 residual bins: 35 on the same clock, 54 on the inverted clock, and 31
on the other clock. The same-clock excursion is one bin at the first CLK rise
at/after raw release: +0/+3/+2/+1 ticks for phases 0/1/2/3. The inverted group
has three-bin excursions in the 18 windows at phases 1 and 2, starting at
+1/+0 ticks respectively. The other group has 1..3-bin early excursions in
18 windows (20 runs), starting +1..3 ticks after release. The early residual bins by
phase are 6/51/49/14. They occur at **all four phases**, not just when input
release shares a CLK-rising bin, and do not occur for the 2/3/5/20-cycle pulses.
They cannot be dismissed as only a same-bin synchronizer sampling race or as
only the other-clock CDC. The recorded excursion/reassertion is established;
its internal cause is not determined by this experiment.

The remaining 34 bins are 17 two-bin runs **only on the other clock**, across
all five pulse widths and all four phase bins (6/6/8/14 bins by phase).
In each case Q stays at REGSET through one predicted non-REGSET capture.
That predicted capture is exactly one observer tick after the model's global
release. The next actual departure is five ticks later, spanning two local
clock edges and one SGSR CLK edge. Because D alternates, that five-tick gap
does **not** measure a five-tick reset-release delay: the intervening capture
can equal REGSET. This is a reset-release/local-capture boundary discrepancy
consistent with clock-domain/observation skew; these bins do not prove a
particular metastability or distribution-delay mechanism. There are no
residuals elsewhere and no delayed assertion bins. In particular, the evidence
does not support classifying all 154 bins as phase-0 sampling races.

[observations_s.log](observations_s.log) contains 43 valid lines from load 1
and 43 from load 2, giving two distinct payloads and 84 duplicate replays.
Both unique windows fit the common kr=2 rule with zero errors across all six
FFs (974 compared bins per FF). Their pulse widths are both 20 cycles and
their phase bins are 0 and 2, so this is a smaller confirmation, not complete
width/phase coverage. Sample zero is the observer's initialized pipeline;
the first usable data already has PLL LOCK high. **There is no information
about synchronizer state at early wake-up before PLL lock.**

For hy01s load 3, the log says `loader_done=True`, `candidate_lines=0` and
`discarded_fragments=755`. There are no valid HY01S frames from that load to
analyse. The runner discarded nonmatching newline-delimited material and/or
a trailing partial buffer, but did not preserve those bytes. A reported DONE
does not establish successful UART operation. The cause of this serial
collection failure is unknown; no reset/startup mechanism is inferred from it.

## Startup-counter follow-up: hy01b and hy01c

[hy01_boot_gen.py](hy01_boot_gen.py) writes a small experiment that needs no
PLL or fast observer. It runs four 16-bit binary counters from configuration,
with no local reset, enable gating, or later restart. On the 25 MHz rising
edge, A has **every FF GSR-enabled** and B has **every FF GSR-disabled**; a
second A/B pair uses the falling edge. All 64 counter FFs explicitly request
`REGSET="RESET"`, CE high, LSR low. Each bit is a kept native TRELLIS_FF.

`hy01b` adds one kept `SGSR` with GSR_N tied high and CLK on the 25 MHz pin.
It measures any extra enabled-counter hold imposed by the synchronizer's
configuration-time state even though no user reset is ever asserted.
`hy01c` has the same counters, delay, snapshots and UART, **without any SGSR
or GSR primitive**. Its line identifier is C instead of B. It measures the
baseline enabled/disabled difference caused by configuration GSR release and
clock startup. Compare distributions over repeated SRAM loads; do not assume
that the control difference must be zero or that one load establishes startup.
Placement is independently routed for each build with seed 1.

After a GSR-disabled 20-bit delay (2^20 rising edges, about 41.94 ms), support
FFs snapshot each pair on the same edge of its own clock. A timed half-cycle
request/acknowledge handshake transfers the falling pair. Counters keep running;
the single snapshot and its differences remain frozen and are sent repeatedly,
so a serial port opened after loading can still recover the early count offset.
All support FFs, including delay, snapshots, handshake and UART, have user GSR
disabled. The build first disables every FF, then enables only `count_a_*`.

Each 33-byte frame at 115200 8N1 is:

```text
Y1B AAAA BBBB DDDD EEEE FFFF GGGG XXXX CR LF
Y1C AAAA BBBB DDDD EEEE FFFF GGGG XXXX CR LF
```

There are no spaces on the wire. Six 16-bit uppercase hex words are rising A,
rising B, rising `(B-A) mod 65536`, falling A, falling B, falling `(B-A) mod
65536`. X is their 16-bit XOR. The within-pair snapshots are simultaneous;
rising and falling snapshots occur at different edges and are not compared
against each other. B-A is the enabled counter's modular edge deficit relative
to its disabled control, provided both run normally after startup. Compare b's
deficit with c's baseline for each polarity. This tests startup holding rather
than treating later absolute counts as a configuration timestamp. Persistent
non-small or varying offsets require investigation rather than an assumed
small signed edge count.

The exact new build/check commands are:

```sh
series_dir="$(pwd)/docs/inventory/LFE5U-85F/hy01_sgsr"
boot_dir="$(mktemp -d "${TMPDIR:-/tmp}/hy01-boot.XXXXXX")"
python3 -B "$series_dir/build_boot.py" "$boot_dir"
python3 -B "$series_dir/check_boot_capture.py" "$boot_dir"
for variant in b c; do
  python3 -B "$series_dir/check_boot_routed.py" "$variant" "$boot_dir/hy01$variant.routed.json" "$boot_dir/hy01$variant.config"
  rg -n '^enum: GSR\.(SYNCMODE|GSRMODE) ' "$boot_dir/hy01$variant.config" || test "$variant" = c
  rg '^enum: (SLICE[A-D]\.GSR|SLICE[A-D]\.REG[01]\.REGSET) ' "$boot_dir/hy01$variant.config" > "$boot_dir/hy01$variant.gsr-enums.txt"
done
```

`build_boot.py` runs `yosys -Q -q -l DESIGN.yosys.log -p 'read_verilog
DESIGN.v; synth_ecp5 -top top; setparam -set GSR "DISABLED" t:TRELLIS_FF;
setparam -set GSR "ENABLED" c:count_a_*; write_json DESIGN.json'`, then
`nextpnr-ecp5 --85k --package CABGA381 --seed 1 --json DESIGN.json --lpf
hy01.lpf --freq 25 --textcfg DESIGN.config --write DESIGN.routed.json -l
DESIGN.nextpnr.log --quiet`, the checker, and `ecppack DESIGN.config DESIGN.bit`.
It writes only external scratch. Both designs pass timing, routing and packing
on this Mac; [validation_boot.txt](validation_boot.txt) retains all 64 counter
FF mappings per design. b emits SYNC/ACTIVE_LOW with a constant-high GSR input;
c has no GSR resource and emits neither SYNCMODE nor GSRMODE. Each design has
32 enabled counter FFs, 32 disabled counter FFs and 324 disabled support FFs.

[check_boot_capture.py](check_boot_capture.py) wire-decodes repeated frames in
Icarus, checks XOR and modulo differences, and tests a deliberately artificial
two-edge hold against a zero-hold control. Its baud and delay are accelerated.
These checks validate measurement/transport logic, not silicon wake-up state.

The b/c board records were subsequently collected with these
commands (no hardware access was performed during emulator implementation):

```sh
python3 -B "$series_dir/run.py" "$boot_dir/hy01b.bit" --port /dev/cu.usbserial-<FTDI-serial> --loads 10 --seconds 3 > "$boot_dir/observations_b.log"
python3 -B "$series_dir/run.py" "$boot_dir/hy01c.bit" --port /dev/cu.usbserial-<FTDI-serial> --loads 10 --seconds 3 > "$boot_dir/observations_c.log"
```

The extended runner accepts Y1B/Y1C as well as the original Y1R/Y1S and still
loads SRAM successfully **before opening serial**. The original window analyser
is for R/S; B/C's six words are directly readable, and
`check_boot_capture.decode(line)` validates their checksum and differences.
The records include [observations_b.log](observations_b.log),
[observations_c.log](observations_c.log), [hy01b.config](hy01b.config) and
[hy01c.config](hy01c.config). Each experiment had ten successful JTAG SRAM loads,
with one frozen snapshot per load and approximately 1,050 identical replay
frames. There are 10,497 valid B frames and 10,505 valid C frames; none is
rejected, and no within-load snapshot changes.

| Load | hy01b rising B-A | hy01b falling B-A | hy01c rising B-A | hy01c falling B-A |
|---|---:|---:|---:|---:|
| 1 | 2 | 1 | 0 | 0 |
| 2 | 2 | 1 | 0 | 0 |
| 3 | 2 | 2 | 0 | 0 |
| 4 | 2 | 1 | 0 | 0 |
| 5 | 2 | 2 | 0 | 0 |
| 6 | 2 | 2 | 0 | 0 |
| 7 | 2 | 2 | 0 | 0 |
| 8 | 2 | 2 | 0 | 0 |
| 9 | 2 | 2 | 0 | 0 |
| 10 | 2 | 1 | 0 | 0 |

Thus the SGSR synchronizer is in its reset state after configuration. With
GSR_N high, enabled FFs on SGSR CLK hold REGSET through rising edges 1 and 2,
and first capture at edge 3. There are one or two held falling edges, depending
on where wake-up falls in the clock period. The C control's zero offsets rule
out an enabled/disabled startup deficit without SGSR in these loads. The frozen
counters do not record wake phase; the falling result tests membership in
{1,2}, rather than fitting an unobserved phase.

## Accepted functional rule and timing hazards

The accepted rule uses zero propagation delay:

```text
s1 = s2 = 1 after configuration
on rising SGSR CLK: (s1, s2) <- (GSR_active, old s1)
user_GSR = GSR_active OR s2
```

Both stages are plain reset-free FFs: no asynchronous preset, local reset or
global reset. GSR_active is the inversion of the measured ACTIVE_LOW GSR pin.
The effective user GSR acts asynchronously on every already-supported,
GSR-enabled resource, through the same path used for asynchronous user GSR.
Captures sample old s2; subsequent s2 changes immediately apply asynchronous
reset at the same logical instant. Long pulses give the fitted two-edge
synchronized release; short pulses can release reset before s2 asserts. A
same-clock capture at the s2 assertion edge is immediately reset again in the
settled emulator result.

[candidate_residuals.txt](candidate_residuals.txt) checks the rule against all
four logs. In R there are 120 exact residual channel-bins out of 909,540:

| Residual class | same | inverted | other | Total | Nearest modeled reset transition / distance |
|---|---:|---:|---:|---:|---|
| early excursion still visible on board | 35 | 18 | 18 | 71 | s2 rise / 0 ticks |
| predicted early capture absent on board | 0 | 0 | 15 | 15 | GSR_N rise / 0,1,2 ticks: 8,6,1 bins |
| held through predicted capture | 0 | 0 | 34 | 34 | s2 fall / 1,2 ticks: 17,17 bins |

RESET/SET residual totals are same=19/16, inverted=7/11, other=36/31.
All 71 early-excursion bins are exactly at the CLK edge where s2 rises; six
also share the raw-release bin. The synchronizer reset reaches the board FFs
after capture. The 15 other-clock captures occur before raw asynchronous
release has reached the board FFs. The final 34 bins are the corresponding
synchronized-release boundary. **Every residual is at most two observer ticks
(20 ns) after a modeled reset transition.** The analyzer tests raw GSR_N rise
and fall and s2 rise and fall separately, chooses the nearest preceding
transition, records its identity and distance for each residual sample, and
asserts the bound. The S replay has zero residuals in 5,844 channel-bins; B/C
have zero rising/falling offset residuals over ten loads each.

These are reset-network propagation delays. The emulator gives propagation
delays zero time, as elsewhere in the emulator. FFs clocked within approximately
20 ns of a reset transition are a timing hazard, not an additional data rule.
The functional comparison is exact outside those windows. This observation
bounds this experiment's discrepancies at 10 ns resolution; it does not
measure an analog delay or a universal setup/recovery guarantee.

Reproduce the report from the repository root:

```sh
series_dir="docs/inventory/LFE5U-85F/hy01_sgsr"
python3 -B "$series_dir/hy01_residuals.py" --selftest --candidate \
  "$series_dir/observations.log" "$series_dir/observations_s.log" \
  "$series_dir/observations_b.log" "$series_dir/observations_c.log" \
  > "$series_dir/candidate_residuals.txt"
```

## Emulator verification

`src/compile/system.rs` binds the selected GSR pin and CLK as execution roots,
builds two reset-free internal kernel FFs initialized to one and an OR using
the existing mux builder. The common kernel register machinery also serves
physical slice FFs; `src/kernel/mod.rs` adds internal FF binding without a
physical slice node and `src/kernel/cache.rs` preserves that distinction in
cached plans. No fast-mode implementation change is required. ASYNC/NONE keep
the existing behavior. SYNC requires measured ACTIVE_LOW polarity and a routed
CLK; other/ambiguous modes and missing clocks are refused explicitly.

`tests/sgsr_native.rs` builds small native designs in removable scratch under
TMPDIR. It compares settled event/fast traces for six probes (same, inverted,
other clocks; RESET and SET), one/two/three/five/twenty-cycle pulses at four
phases, all 312 distinct R/S board windows, startup counters with both wake
phases and a no-SGSR control, and the exact X20 HDL in text/packed forms. It
compares the board's 915,384 channel-bins with the functional rule, requiring
all 120 discrepancies to lie inside the two-tick reset hazard windows, and
checks all twenty B/C frozen snapshots. Tool-dependent tests are ignored by
default. Shared SGSR audit/startup/settings sections now describe this rule.
No commit, board, serial-port or flash access occurred during implementation.


[validation_execution.txt](validation_execution.txt) records the rserv commands
and tool versions: 58 targeted tests passed with ignored tests included; the
earlier package-only suite reported 343 passes/233 ignores from raw summaries;
that figure was not a complete-workspace or unique-test count. The reconciled
follow-up below supersedes that census.
Formatting, clippy with warnings denied, the four-log audit, checksum checks
and scratch cleanup passed. All four new SGSR checks depend on external tools
and are ignored by default.

[validation_followup.txt](validation_followup.txt) records the ignored
`automatic_compile` polarity-check correction, the test/doc/tool expectation
audit, and the explicit full-workspace test recount (including `ecp5_gui`).

The documentation validation passed all **50 targeted tests**, including
the formerly stale ignored `automatic_compile` fixture. The complete workspace
passed **499 tests, 245 ignored**: emulator 358/235 and GUI 141/10. Four nested
child-process summaries are excluded; executable inventories agree. See
[validation_followup.txt](validation_followup.txt) and the per-binary
[workspace_tests.tsv](workspace_tests.tsv). Workspace clippy with warnings denied,
formatting and the home-path checker passed on rserv.
