# HC02: DCSC dynamic clock selection

Board experiment for C03 in [CLOCK_NETWORK.md](../../../CLOCK_NETWORK.md), with
supplied ULX3S records analysed on 2026-11-17. The measured source switch holds
through **two eligible edges of the new clock**, one whole new-clock period longer
than the original documented-sequence fit. Both executors now run DCS
from its configured bits; fast mode requires two-valued results (see Emulator check).

## Files and variants

- `hc02_gen.py`: emits Verilog on stdout; its docstring specifies the complete UART
  format, observer alignment, startup and stimulus.
- `hc02_analyse.py`: reads raw UART lines or the existing runners' `distinct=[...]`
  logs, reconstructs timelines, fits switching sequences, and reports departures,
  pulse findings and observed settled outputs by MODESEL/SEL. Standard library only.
- `hc02_timing.py`: nextpnr pre-pack clock constraints; use with the existing hb11 LPF.
- `dynamic.v`, `static.v`, and their `.config` files: the supplied artifacts that
  ran on the board. These, both observation logs, the generator and timing hook
  were left byte-for-byte unchanged during analysis.
- `analysis_dynamic.txt` and `analysis_static.txt`: per-record fits, complete
  binned timelines, steady tables, pulse diagnostics and candidate-rule counts.

| Variant | DCS0, R22C66 | DCS1, R70C66 | UART variant digit |
|---|---|---|---|
| `dynamic` (default) | POS | NEG | 0 |
| `static` | CLK0_LOW | CLK1_HIGH | 1 |

Both sites are explicitly constrained by BEL. Both the primitive parameter and
the **cell attribute** carry DCSMODE: the attribute controls this nextpnr version's
bitstream writer. The static rebuild reuses both sites because the part has only
two DCS blocks. It tests a low-holding and a high-holding static mode together.

## Clocks, stimulus and observation

The G2 25 MHz input drives one EHXPLLL, with a 600 MHz VCO, feedback through
CLKOP, CLKI_DIV=1 and CLKFB_DIV=8. Its outputs are:

| Output | Divider | Frequency | CPHASE / FPHASE | Use |
|---|---:|---:|---|---|
| CLKOP | 3 | 200 MHz | 1 / 0 | Fast sampling |
| CLKOS | 96 | 6.25 MHz | 11 / 0 | DCS CLK0 |
| CLKOS2 | 128 | 4.6875 MHz | 27 / 0 | DCS CLK1 |

These are different-period, different-phase clocks from a common VCO, **not
independent oscillators**. Their relative phase repeats after 640 ns. Their high
and low phases are nominally 80 ns and 106.667 ns. LFSR-selected trigger positions
vary the control write relative to both clocks.

Each clock drives its own rising- and falling-edge four-bit Gray counters.
Clock nets are never sampled as data. Two synchronizer stages, a registered Gray
decode, and registered subtraction preserve per-bin edge counts. The nominal
resolution is **5 ns**, with four-to-five-fast-cycle observation latency. Per-channel
skew/metastability can move an event by another bin. The analyser defaults to a
one-tick relative matching tolerance; it reports edge-order alternatives near
boundaries rather than declaring exact sub-tick ordering.

The observer anchors a level at every unambiguous edge. It invalidates level when
both polarities appear in one bin, and reanchors at the next unambiguous edge.
Startup waits for synchronized PLL LOCK, then runs SEL=01 and SEL=10 for 32768
fast cycles each, establishing levels even for the static variant. LOCK is also
recorded throughout each window. A LOCK loss makes the observation unqualified.

One nonzero 32-bit LFSR runs continuously in the fast domain. Three quarters of
control proposals alternate one-hot SEL at MODESEL=0; the rest draw all three
control bits, including 00, 11, bypass, and MODESEL-only changes. Identical proposals
are replaced by a MODESEL toggle. Each accepted request settles for at least 256
ticks before capture, writes the controls once, then holds them throughout the
rest of capture and the entire UART transmission. There is no control queue or
capture overwrite while sending.

Each window contains 512 consecutive samples (2.56 us). The first sample with new
controls is T=128..255. The physical register write is at fast edge T-1. The tail
check begins at T+128, leaving at least 640 ns after the write for a switch to
settle. A record takes approximately 0.537 s to transmit at 115200 8N1; allow
several minutes per build for coverage of less frequent control combinations.

## UART format and analysis

No spaces are transmitted:

```text
D2 V QQQQQQQQ TTT SSSSSSSS [512 * WWWWWWWWWWWW] XXXXXXXXXXXX CR LF
```

The line is 6180 bytes including CR/LF: version `D2`, variant digit, eight hex
sequence digits, three hex T digits, eight hex LFSR digits, 512 twelve-hex-digit
samples in chronological order, then the 48-bit XOR of those samples. The checksum
detects many transport faults but is not a CRC. The header is validated structurally
and against the recorded control change; it is not covered by the XOR.

Each 48-bit sample is `{4'b0, LOCK, MODESEL, SEL[1:0], valid[3:0], level[3:0],
DCS1_fall_delta, DCS1_rise_delta, DCS0_fall_delta, DCS0_rise_delta,
CLK1_fall_delta, CLK1_rise_delta, CLK0_fall_delta, CLK0_rise_delta}`.
Every delta is four bits; validity/level bit order is DCS1, DCS0, CLK1, CLK0.
Levels describe the state **after the deltas in the same sample**. Sample k is
recorded immediately before fast edge k; controls written at T-1 first appear in
sample T. A physical edge in `(k-5,k-4]` appears in sample k, absent extra
metastability delay. The analyser estimates its physical time as `k-4.5`, then
subtracts `T-1` when printing times relative to the control write.

```sh
python3 "$HC02/hc02_analyse.py" --selftest
python3 "$HC02/hc02_analyse.py" "$BUILD/observations_dynamic.log" --timelines
python3 "$HC02/hc02_analyse.py" "$BUILD/observations_static.log" --summary-only
python3 "$HC02/hc02_analyse.py" "$BUILD/observations_dynamic.log" --json > "$BUILD/analysis.json"
```

For a MODESEL=0 one-hot source switch, the default measured fit follows the old
source through its first eligible rising edge (POS) or falling edge (NEG), holds
high or low respectively through the **second** corresponding new-source edge,
then follows the new source. Every expected edge is matched once; missing and extra edges are
listed and counted. The report gives the terminating old-source/output edge, held
level and measured duration, inferred new handshake edge and first **visible**
new-source/output edge. The release handshake edge itself leaves the held level
unchanged; its time is inferred from the source timeline, not directly observed
as an output edge. `--rule documented` restores the original first-new-edge
candidate. `--compare-rules` compares 48 fixed polarity/index/delay variants.

Both pre-change and settled post-change regions are checked against the static
table. The summary reports expected and **observed** 0, 1, CLK0, CLK1 or other edges
for each observed MODESEL/SEL combination. POS/NEG 00/11 transitions, transitions
of static modes and MODESEL changes get steady checks and pulse reports without
an invented transition rule. With MODESEL=1 before and after the write, the
ordinary mux sequence is also fit outside the unresolved control-edge boundary.
Runts in bypass are reported but are not themselves specification departures.

Runts are opposite-polarity output edges in **separate single-edge bins**, closer
than the minimum observed source phase of that level minus twice the matching
tolerance. Same-bin opposite edges and multi-count bins are unordered packets;
the analyser assigns them no pulse width. It retains the old flattened counts as
`raw_runts` and `raw_repeated_polarity` for comparison, and checks cumulative
rises-minus-falls against a binary clock's possible levels. An edge pair inside
one 5 ns bin can be detected, but its width/order cannot be resolved.
An extremely narrow pulse may fail to clock the observer, and 16 same-polarity
edges in one bin can alias in the modulo-16 counter. This test cannot certify
analog pulse widths or exact setup/hold boundaries. Frozen-source handshakes and
configuration-time DCS state are outside this experiment.

Exit status is 0 for no departures, 1 for departures/unqualified/unresolved observations,
and 2 for malformed input or no records. Unrelated log lines are ignored, but D2
lines with bad checksums, metadata or lengths are errors. The existing runners
can include an incomplete record in `distinct`; preserve that diagnostic or exclude
only that known cut record before interpreting the fit's exit status. Variant and
sequence are retained; records are not deduplicated across loads.

## Reading

### Records and checks

Sequence numbers in this Reading section are decimal; the reports print Q as
eight hexadecimal digits (for example, Q=36 here is `Q=00000024` in the report).

[observations_dynamic.log](observations_dynamic.log) contains three 30 s loads:
55 complete records per load, **165 complete records** in total. Each load also
contains one cut record. [observations_static.log](observations_static.log)
contains three 15 s loads: 27, 27 and 26 complete records, **80 complete records**
in total, plus one cut record per load. All complete records pass XOR, control
alignment, observer metadata, LOCK and source-period checks. The six cut records
remain in the original logs and are reported as `bad_records=3` per file.

The analysis commands, run in this directory without rebuilding anything, are:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 hc02_analyse.py observations_dynamic.log \
  --timelines --compare-rules > analysis_dynamic.txt 2>&1
PYTHONDONTWRITEBYTECODE=1 python3 hc02_analyse.py observations_static.log \
  --timelines --compare-rules > analysis_static.txt 2>&1
```

Both return 2 because the cut records are retained. The report summaries separate
input errors from switching mismatches and unresolved observations. With the
measured rule there are **zero switching/steady-table departures**. The static
file additionally has **one unresolved observer record**, discussed below; zero
departures does not mean every static transient has been modelled.

### Order, held level and pulse completion

There are **113 MODESEL=0 one-hot source switches per DCS**: 61 CLK0 to CLK1 and
52 CLK1 to CLK0. Let A mean rising for POS and falling for NEG. One waveform rule
fits all 113 switches of each block, with **zero missing or extra edges** at the
one-tick (5 ns) matching tolerance:

1. Continue following the old source through its first eligible A edge, `o`.
2. Hold the resulting level: **1 for POS, 0 for NEG**.
3. The next eligible A edge of the new source is `n1`. Keep holding through the
   following A edge, `n2 = n1 + Tnew`.
4. After `n2`, follow the new source. Because `n2` has the held polarity, the first
   **visible output edge** is the opposite edge after `n2`: falling for POS,
   rising for NEG. The first resumed pulse then has the new source's full width.

Thus the measured held interval, from the terminating output edge to its first
opposite edge, is approximately
`H = (n1 - o) + Tnew + Tnew/2`. The inferred handoff at `n2` is invisible at the
output; it is the new-source timeline and subsequent output edges that identify
it. This is a waveform rule, not proof of a particular two-flip-flop internal
circuit or the exact instant of an invisible handoff while its level is unchanged.

| Mode | Direction | Switches fitting | Held level | Observed hold, min / median / max |
|---|---|---:|---:|---|
| POS | CLK0 to CLK1 | 61/61 | 1 | 345 / 400 / 510 ns |
| POS | CLK1 to CLK0 | 52/52 | 1 | 265 / 370 / 375 ns |
| NEG | CLK0 to CLK1 | 61/61 | 0 | 375 / 480 / 535 ns |
| NEG | CLK1 to CLK0 | 52/52 | 0 | 290 / 345 / 400 ns |

**No truncated pulse is observed in these protected one-hot switches.** The phase
ending at `o` is a full old-source phase: 80 ns for CLK0 or 105/110 ns as sampled
for CLK1. The first phase after resumption is likewise 80 ns or 105/110 ns,
matching the new source. POS stretches the high phase; NEG stretches the low
phase. Neither has detected runts, unordered packets, repeated polarities or
multi-count bins in these 113 switches.

For example, dynamic load 1, Q=1, CLK0 to CLK1, relative to the control write:
POS ends at old rising `+6.5t`, sees new rising `+11.5t` and `+54.5t`, then falls
at `+75.5t`: high hold 69 ticks = 345 ns. NEG ends at old falling `+22.5t`, sees
new falling `+33.5t` and `+75.5t`, then rises at `+97.5t`: low hold 75 ticks =
375 ns. The documented first-new-edge fit instead predicts POS falling at
`+33.5t` and rising at `+54.5t`, and NEG rising at `+54.5t` and falling at
`+75.5t`. Those are exactly the two missing edges per output.

### Candidate rules, direction and phase

The analyser compares **48 fixed rules per mode**. A/O choose active/opposite
old and new polarities; the old index shifts by -1, 0 or +1 same-polarity periods;
the new rule skips 0, 1, 2 or 3 same-polarity edges after the first eligible edge.
The old source runs through the selected stop, its resulting level holds, and
the new source takes over at release. A release of the opposite polarity creates
an immediate visible edge. Only boundary ordering within the matching tolerance
can vary per record. The rule's polarity, old index and new delay are fixed over
all records. Complete counts for all candidates are in `analysis_dynamic.txt`.

| Candidate | POS: failing events; missing / extra edges | NEG: failing events; missing / extra edges |
|---|---|---|
| Documented: old A, first new A | 113; 226 / 0 | 113; 226 / 0 |
| **Measured: old A, second new A** | **0; 0 / 0** | **0; 0 / 0** |
| Old A, third new A | 113; 0 / 226 | 78; 0 / 156 |
| Old A, fourth new A | 113; 0 / 452 | 113; 0 / 378 |
| Previous old A, second new A | 113; 238 / 226 | 113; 228 / 226 |
| Following old A, second new A | 106; 212 / 206 | 107; 214 / 152 |
| Other polarity at both ends, second new edge | 113; 118 / 108 | 113; 136 / 102 |
| Old A, second new opposite edge | 55; 110 / 0 | 71; 142 / 0 |
| Old A, third new opposite edge | 49; 0 / 98 | 30; 0 / 60 |

The measured rule is the **only candidate with zero mismatches over all 113
records of each mode**. It needs no direction-specific delay and no additional
phase-dependent number of new-clock cycles. Direction changes the new clock's
period; phase changes the wait from `o` to `n1`, so the hold durations vary.

Eligibility at a boundary stays limited by the observer:

- Seven POS and six NEG switches have an old A edge within one tick of the fabric
  SEL write. The fit uses the following old A edge. Three POS and two NEG cases
  estimate that near edge at `+0.5t`; the others estimate it at `-0.5t`.
  These observations do not establish a setup/hold threshold or the exact arrival
  time of SEL inside DCS. In particular, "first eligible old edge" must not be
  replaced by an exact comparison against the fabric register's write timestamp.
- In 35 NEG switches, a new A edge and the old stop are within one tick. The fit
  skips that near-coincident edge and uses the next two new A edges. Counting the
  near-coincident edge as an eligible first edge makes a nominal third-new-edge
  candidate indistinguishable in those individual records. It fails the other
  78. This does not require changing the measured rule with phase; the order and
  setup margin of the near-coincident edges cannot be resolved at 5 ns.
- The clocks share one PLL and repeat relative phase every 640 ns. This record
  set does not sweep arbitrary independent clock phases, stopped sources or
  configuration-time behavior. Sub-tick truncations cannot be excluded.

### Short phases and observer limits

The dynamic log's original **8 runt reports per DCS** are positive-width short
phases in separate, alternating single-edge bins. All occur on **MODESEL 0 to 1
changes with SEL also changing**, outside the protected one-hot rule. Both DCS
outputs show the same spliced timelines. They are evidence of real shortened
clock phases on entry to bypass, rather than zero-width artefacts manufactured
by ordering two events in one bin:

| Dynamic record | Control before/after (MODESEL:SEL) | Short phases per DCS |
|---|---|---|
| Load 1 Q=36 | 0:01 to 1:10 | high 65 ns |
| Load 3 Q=20 | 0:10 to 1:01 | high 20 ns, low 60 ns |
| Load 3 Q=40 | 0:01 to 1:10 | high 45 ns, low 35 ns |
| Load 3 Q=45 | 0:10 to 1:01 | low 25 ns |
| Load 3 Q=52 | 0:01 to 1:10 | low 10 ns, high 45 ns |

Widths are sampled estimates: nominal width uncertainty is +/-5 ns; allowing an
additional relative observation bin gives a conservative +/-10 ns at the default
tolerance. The 10 ns phase is especially close to the resolution limit. The
records support shortened phases, but do not determine their analog widths or
exclude additional sub-5 ns glitches.

The static log has two similarly resolved short phases on MODESEL changes:
load 1 Q=8, CLK0_LOW, high 65 ns (`1:10` to `0:11`), and load 1 Q=15,
CLK1_HIGH, low 25 ns (`1:01` to `0:10`). Neither is a pure MODESEL=0 static-mode
SEL transition.

**Static load 3 Q=6 remains unresolved.** On `0:10` to `1:01`, both outputs report
one rise and one fall in the bin centred at `+1.5t`. CLK0_LOW also reports two
rises at `+21.5t`. The old analyser arbitrarily ordered rises before falls,
creating a zero-width runt and two repeated-polarity counts per output. Those
are not measurements of a physical zero-width pulse or a known edge order.

There is a stronger problem on CLK0_LOW: after anchoring the initial level low,
the reported rises-minus-falls imply level **-1 for 16 samples**, `+5.5t` through
`+20.5t`, until the double rise restores the count. No within-bin reordering can
turn that into a binary-clock waveform. Thus these counter observations are not
a faithful edge timeline, even though the eventual totals close and steady
outputs recover. A very short switching glitch disturbing the counter is a
possible explanation; the records cannot distinguish the DCS waveform from
counter/timing failure or determine a physical glitch count/width. CLK1_HIGH's
same-bin pair could be a short low excursion (fall then rise), but its width and
origin also remain unresolved. The updated analyser reports unordered packets
and the count-balance violation explicitly, rather than treating them as resolved
runts or silently repairing the data. Each static output retains **one resolved
short phase**; the original two-per-output counts remain available as `raw_runts`.

### Steady selection and scope

All complete records fit both their pre-change and settled post-change tables:
POS/NEG each have 165/165 matches; CLK0_LOW/CLK1_HIGH each have 80/80. The dynamic
build covers all four SEL values at both MODESEL values. The static build covers
SEL 01, 10, 11 at both MODESEL values; **SEL 00 was not recorded in the static
build**. The reports list the actual counts and observed source/constant for
every combination present.

The measured handshake rule covers protected one-hot source switches only.
Transitions involving 00/11, MODESEL changes and the static modes still receive
steady checks and pulse diagnostics; a complete transient state machine for
those transitions has not been established here. The unresolved static record,
exact SEL setup/hold behavior, near-coincident edge ordering, analog minimum
pulse widths and unseen/sub-resolution glitches remain open.

## Emulator check

`src/compile/dcs.rs` dispatches both physical DCS resources and reads DCSMODE
from the decoded CRAM field. `src/kernel/dcs.rs` implements a DCSC component
with a normal internal driver on DCSOUT and listeners on CLK0, CLK1, SEL0,
SEL1 and MODESEL. The driver participates in the usual simultaneous capture,
routing, resolution and downstream generated-clock waves. Component state is
excluded from the construction cache, and its changed signal schema bumps the
plan-cache version. Fast mode now mirrors the table and handshake in
`src/kernel/fast_dcs.rs`, with eight state slots per DCS and simultaneous
wave captures. The event-kernel semantics below are unchanged.

POS/NEG implement the measured rule: follow the old source to its active edge,
hold 1/0 through two later new-source active edges, then follow the new source.
A new edge at the old stop's physical timestamp cannot consume a stage, even
if routing delivers it in another delta wave. Edges of a stimulus batch see that
batch's controls; a previously consumed edge cannot retroactively acknowledge a
later control capture at the same timestamp. There is no invented 5 ns setup
threshold: the emulator uses exact rational time and the Reading section retains
the physical observer's boundary uncertainty.

MODESEL=1 is the immediate one-hot mux, including short phases on entry.
The static table is implemented for all ten documented modes. `NONE` is the same
zero-bit pattern as NEG in Trellis and therefore executes NEG. All POS/NEG table
entries and SEL=01/10/11 of CLK0_LOW/CLK1_HIGH at both MODESEL levels are supported
by HC02. **SEL=00 for those two static modes and all entries of the other six
static modes are documented-only.** MODESEL's documented override applies to
those remaining modes too.

Policies beyond the measured protected switches are explicit:

- Startup applies the selected table entry immediately, without an invented
  initial clock edge. This is a digital startup assumption.
- POS/NEG source-to-00/11 finishes at the old active edge; 00/11-to-source waits
  through two new active edges. Exact hardware behavior of these transitions
  was not established by HC02.
- Static-mode selection applies the table immediately. This does not claim its
  unmeasured glitchless gate timing or reproduce the anomalous observer packet.
- Bypass cancels pending handshakes. Returning to protection uses the source
  currently followed. Retargeting while held restarts the two-edge wait; returning
  to the old source before its stop cancels the switch.
- If the new clock is stopped, the documented hold level persists after the old
  stop edge until two new active edges actually occur. **There is no timeout**;
  this is the unmeasured stopped-clock policy. A stopped old source
  cannot acknowledge the change and keeps its last output.
- Unknown controls produce X unless every possible table selection identifies
  the same target. An uncertain handshake edge makes the handshake unknown
  until a definite new request. Analog metastability and sub-resolution glitches
  are not synthesized by this digital component.

The original blanket fast-mode refusal has been replaced by DCSC execution.
`tests/dcs_native.rs` now compares both saved configs through the same 350 us,
including DCSOUT and the physical rising/falling edge-counter FFs downstream.
`tests/event_kernel.rs` adds fast twins for the two-valued table, exact phases,
bypass, stopped sources, coincidence, constant selections and retargeting, plus
derived-source waves, threads, warm handoffs, snapshots, tuner swaps and DTR.
X/Z input transitions and uncertain clock edges cannot be driven through the
two-valued fast input API. Their event-kernel checks remain; fast mode refuses
unknown DCSOUT/handshake state and ambiguous runtime targets. Unknown constants
that cannot affect the selected target remain executable.

The saved configs also exposed legitimate default-plus-selected input arcs at
one DCS mux sink. The legacy routing map rejected those while extracting text
service metadata, before native decoding. `src/image.rs` now ignores routing
lines in that metadata-only view; the complete original source still goes to
libtrellis for CRAM decoding and is retained byte-for-byte. A configuration-state
regression checks both source preservation and service directives.

Original event-kernel implementation validation on rserv (before fast support):

- Six targeted `dcs_` kernel tests pass. They cover all 80 mode/SEL/MODESEL table
  entries (400 clock-value samples), 96 exact-edge phase/direction scenarios,
  bypass truncation/cancellation, stopped-source recovery, coincident edges,
  unknown controls and fast refusal.
- `hc02_dynamic_port_edges_follow_two_new_active_edges`: pass, two switches at
  each DCS, exact source/output edge fits. It probes all six ports directly at
  each physical site, with no UART interpretation.
- `hc02_static_ports_match_the_table_at_every_settled_time`: pass, 6,565 table
  samples and two control changes per site.
- Each native replay is bounded to **350 us** of design time, covering the startup
  SEL change and the first capture change in both directions; each took about
  **3.5 s** on rserv. It does not run intervening UART transfers, replay all 113
  board windows, or reach later bypass/00/11 traffic. Those table/control cases
  are checked separately by the kernel unit tests.
- `hc02_dynamic_is_clearly_refused_by_fast_mode`: pass, checks the exact DCSC
  diagnostic without requiring a C compilation.
- `configuration_state --include-ignored`: 10 passed, including the unchanged
  native text/bit payload check and the DCS-arc metadata regression.

The include-ignored area runs passed: event_kernel 28, fast_mode 19 and
fast_reset_order 7. The full workspace suite was invoked exactly once:
364 passed, 195 ignored, no failures across 76 result groups. `cargo fmt --all
-- --check` and `cargo clippy --release --workspace --all-targets -- -D warnings`
passed on rserv; only `cargo check --tests` ran on this Mac.

Exact validation commands and final suite/format/lint results are in `TOOLS.txt`.
All board-tested `.v`, `.config` and observation logs, the generator and timing
hook remain byte-for-byte unchanged. No board, serial port, branch switch, stash,
reset or commit was used for this implementation.

<a id="exact-build-commands-for-the-board-owning-planning-session"></a>

## Exact board build commands

All generated HDL, netlists, configs, bitstreams and logs go to scratch space.
The LPF is the existing G2/L4 constraint file. Keep the pre-pack hook:
`--freq 200` alone incorrectly constrains the 25 MHz UART domain to 200 MHz.

```sh
set -eu
cd '~/IDE Workspaces/RustroverProjects/ecp5_emu'
HC02="$PWD/docs/inventory/LFE5U-85F/hc02_dcs"
HC02_LPF="$PWD/docs/inventory/LFE5U-85F/hb11_ebr_narrow_rbw/hb11.lpf"
BUILD="$(mktemp -d /tmp/hc02-dcs.XXXXXX)"
export PYTHONDONTWRITEBYTECODE=1
python3 "$HC02/hc02_analyse.py" --selftest
cd "$BUILD"
for variant in dynamic static; do
  python3 "$HC02/hc02_gen.py" --variant "$variant" > "$variant.v"
  yosys -Q -T -p "read_verilog $variant.v; synth_ecp5 -top top -json $variant.json; check" \
    > "$variant.yosys.log" 2>&1 || exit 1
  nextpnr-ecp5 --85k --package CABGA381 --seed 1 --freq 200 \
    --json "$variant.json" --lpf "$HC02_LPF" \
    --pre-pack "$HC02/hc02_timing.py" --textcfg "$variant.config" \
    > "$variant.nextpnr.log" 2>&1 || exit 1
  if grep -q 'ERROR' "$variant.nextpnr.log"; then
    cat "$variant.nextpnr.log"
    exit 1
  fi
  grep -E 'enum: DCS[01]\.DCSMODE' "$variant.config"
  ecppack --compress "$variant.config" "$variant.bit" \
    > "$variant.ecppack.log" 2>&1 || exit 1
done
```

Require POS/NEG for dynamic and CLK0_LOW/CLK1_HIGH for static before loading. On
this version a timing failure can still leave a config, so also require no ERROR
and a final PASS at 200 MHz in each nextpnr log. Default router1 succeeded here.
If the previously reported router1 assertion recurs, rerun nextpnr with
`--router router2`, retain the new log, and recheck timing/modes before packing.

Load SRAM and collect lines with the existing
`../hb11_ebr_narrow_rbw/run.py BITSTREAM D2 LOADS SECONDS` runner after checking its
hardcoded serial device. Emulator implementation did not invoke that runner,
a loader or a serial port. No flash operation is needed.

## Local validation, 2026-11-17

- Yosys `0.70+post`, git `0c11ea098be065e585a90a06e7b7da49936cf231`: both variants
  pass `synth_ecp5` and `check`; one EHXPLLL, two DCSC and two DP16KD per build.
- nextpnr `0.11.1-77-g6b438a1c`, `--85k --package CABGA381 --seed 1`, default
  router1: both fully route with timing passing at 200 MHz. Final reported fast
  Fmax: **214.64 MHz dynamic**, **222.07 MHz static**; UART domain passes at 25 MHz.
  Two warnings per build concern explicit DCS constraints overriding equal
  derived 6.25 MHz constraints. No router assertion or router2 workaround occurred.
- Routed config excerpts (actual mode selections):

  ```text
  dynamic.config:
  .tile MIB_R22C66:EBR_CMUX_UL
  enum: DCS0.DCSMODE POS
  .tile MIB_R70C66:EBR_CMUX_LL
  enum: DCS1.DCSMODE NEG

  static.config:
  .tile MIB_R22C66:EBR_CMUX_UL
  enum: DCS0.DCSMODE CLK0_LOW
  .tile MIB_R70C66:EBR_CMUX_LL
  enum: DCS1.DCSMODE CLK1_HIGH
  ```

- Trellis/ecppack `1.4-86-ga31ff70`: both compressed bitstreams pack successfully.
  The scratch outputs are not included as silicon evidence or repository fixtures.
- The explicit one-hot sequencer keeps four mapped state flip-flops, exactly one
  with REGSET=SET (BOOT), plus all sixteen startup-counter flip-flops. Forced
  Yosys FSM recoding was rejected during review because it ignored initialization
  and removed BOOT. The delivered design disables that recoding.
- `hc02_analyse.py --selftest`: **67 cases pass** after the board analysis update.
  They include the 48-rule sweep, new-clock skips 0..3, distinguishing the
  documented and measured sequences, and an impossible intermediate counter
  level even when endpoint totals recover. They also cover both POS/NEG
  directions, phase and edge-order boundaries, independent one-bin output timing
  variation, all 80 steady-table entries, bypass changes, wrong static levels,
  injected runt/extra/missing/same-bin/multiple edges, LOCK loss, corruption and
  truncation. Synthetic output levels evolve through a state machine separate
  from the fitter.
- Icarus transport simulation: two complete records per final variant, sequences
  0 and 1, exact 6180-byte framing, XOR, metadata, one control change at the header
  index, live source counters, and settled outputs all checked. Only the UART
  baud divider was accelerated; capture clocks/counters remained at their stated
  rates. Behavioral DCS stubs in scratch were used for transport verification;
  this does not validate physical DCS switching behavior.
