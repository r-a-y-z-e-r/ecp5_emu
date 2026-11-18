# HB09P: raw PDP36 same-clock collisions return the new word (R02)

Measured 2026-11-17 on the HB09 ULX3S setup. This extends
[HB09](../hb09_ebr_collision/README.md) B05 with a PDP subset. The event kernel
now implements that subset; proposed fast-mode changes are supplied separately.

## Setup

Target: LFE5U-85F/CABGA381 on ULX3S, 25 MHz oscillator on G2, UART TX on L4,
115200 8N1. The designs were built with [build.sh](build.sh), and JTAG SRAM
loads were collected with HB09's runner. Emulator implementation used these
records without further board or serial access. Tool versions:
[TOOLS.txt](TOOLS.txt); file hashes: [SHA256SUMS](SHA256SUMS).

**These are raw primitive outputs, not inferred-RAM `q`.** The recorded
[hb09p.v](hb09p.v) and [hb09ps.v](hb09ps.v) are byte-identical to the generator.
They instantiate eight DP16KD primitives explicitly with DATA_WIDTH_A=B=36;
nextpnr writes MODE=PDPW16KD, DATA_WIDTH_R=36 in the recorded
[hb09p.config](hb09p.config) and [hb09ps.config](hb09ps.config). The recorder
samples their DOA/DOB wires directly. Yosys adds the LFSR, recording FFs and
UART logic, but **no transparency/forwarding mux between the EBR outputs and
the recorder**. Both WRITEMODE parameters are NORMAL, both REGMODEs NOREG,
reset/release ASYNC, GSR disabled. CEA/CEB=1/1, OCEA/OCEB=0/1; clocks coincide;
reset/CS pins are zero. ADA[3:0] are full-word masks (1111 or 0000). WEA/WEB
are 1/0 in HDL; PDP execution uses the masks, not separate WE pins.

## Inference scope

[check_inference.py](check_inference.py) runs unchanged `synth_ecp5` on 32 plain
RAMs: widths 16/18/32/36 × depths 32/64/128/256/512/1024/2048/4096.
[mapping.json](mapping.json) records parameters, controls and cell counts.
Widths below are configured A/B; in the DP cases **B is disabled (CEB=0)**,
and A supplies the synchronous read/write with READBEFOREWRITE. All use NOREG.

| Depth | Width 16 | Width 18 | Width 32 | Width 36 |
|---|---|---|---|---|
| 32 | LUTRAM | LUTRAM | LUTRAM | LUTRAM |
| 64 | LUTRAM | LUTRAM | 1 PDP 36/36 | 1 PDP 36/36 |
| 128/256/512 | 1 DP 18/18 | 1 DP 18/18 | 1 PDP 36/36 | 1 PDP 36/36 |
| 1024 | 1 DP 18/18 | 1 DP 18/18 | 2 DP 18/18 | 2 DP 18/18 |
| 2048 | 2 DP 9/18 | 2 DP 9/18 | 4 DP 9/18 | 4 DP 9/18 |
| 4096 | 4 DP 4/18 | 4 DP 9/18 | 8 DP 4/18 | 8 DP 9/18 |

Every PDP case has the X03 profile above: NORMAL defaults, NOREG, full
four-lane masks from WE, CE=1/1, OCE=0/1. These eight profiles are the series'
eight blocks. No OUTREG, mixed-width or public-PDP pin-order variant is added.
These boundaries apply to the recorded tool version and default synthesis;
rerun the matrix after changing tools or RAM coding style.

## Bench and analysis

[hb09p_gen.py](hb09p_gen.py) reuses HB09's INITVAL packing and LFSR (32 steps
per 25 MHz edge) and adapts its recorder/UART. Direct **DP16KD wrappers** expose
raw PDP reads without Yosys's forwarding mux or nextpnr's public-PDP output
swap. Modes, widths, CE/OCE, masks, reset/GSR and tied address pins match the
eight inferred profiles. Width-32 blocks tie the four unused data bits to zero
and record only 32 bits; width-36 blocks exercise every bit in all four lanes.
INITVAL holds distinct known pseudo-random data per block.

Each block's hot words are **0,1,depth-2,depth-1**. LFSR[1:0] picks the write
word; LFSR[5] forces the same read word on half the edges, otherwise LFSR[3:2]
picks it. WE=LFSR[4], data=LFSR[41:6]. Expected collision frequency is 5/16;
the first window has **23 collisions per block among 63 scored edges** and
19 clean reads of words last written in a collision. These are traffic counts,
not hardware results. Masks are always 1111 or 0000.

`hb09p` records fresh windows while traffic runs continuously. `hb09ps`
resends the first configuration window while traffic continues. UART is
115200 8N1: `R<16 hex window LFSR><4352 hex samples>\r\n`. Each of 64 samples
packs 272 bits (68 hex), fields in this order, first in the MSBs:
p32_64, p32_128, p32_256, p32_512, p36_64, p36_128, p36_256, p36_512.
Sample k is before edge k; sample k+1 reflects edge k. Sample 0 is not scored.

[hb09p_analyse.py](hb09p_analyse.py) accepts HB09 runner logs or raw lines.
It reports idle/disjoint reads, collisions and clean reads after a colliding
write, per block and per load. Collision categories are old/new/equal/mix/other,
both per word and per nine-bit lane (last lane has five observed bits at width
32). Hold matches, bits outside both candidates and mixture/other examples
are also reported. Free windows start with unknown memory and score old data
after writes; `--once` requires the configuration SEED and known INITVAL.
Repeated frozen windows are not independent trials. Synthetic old/new
selftests validate the software, not the chip.

## Observations

[observations.log](observations.log): three loads × 30 seconds, starting
13:10:02, 13:10:42 and 13:11:22 UTC. Each contains **78 complete windows**;
the sets are identical across loads, so the deduplicated analysis uses 78
windows, not 234 independent windows. [analysis.json](analysis.json) was
recomputed from the supplied log and agrees exactly with the stored report.
For **each** of the eight width/depth cases:

- 1,299/1,299 known-old collisions return the **NEW** word; zero old, mix or
  other word results, zero held matches, zero bits outside old/new.
- 2,158/2,158 known idle reads and 807/807 known disjoint-write reads match.
- 1,853/1,853 clean reads of words last written in a collision match the
  requested write data. The single writer's array contents are preserved.
- 196 additional collisions have unknown old data at the start of a free
  window and are excluded from old/new discrimination. Counts are per case,
  not multiplied by the repeated identical loads.

[observations_s.log](observations_s.log): three loads × 8 seconds, starting
13:12:02, 13:12:20 and 13:12:38 UTC. Each repeats the same first configuration
window. [analysis_s.json](analysis_s.json): per case, 23/23 collisions return
NEW, all 40 clean reads match, and all 19 later clean reads after colliding
writes match. Sample 0 is not scored. Lane reports distinguish equal old/new
candidates; no lane has a contradictory result. These are hardware records,
whereas the analyser's synthetic selftest is only a software check.

## Reading

1. In the measured **raw PDP36/NOREG/NORMAL profile**, a same-word read on the
   same definite clock edge as a full four-lane write returns the **new DI word**.
   Both physical halves follow that result, in HB01's DOA-low/DOB-high order.
2. A colliding read does not corrupt the single writer's stored word: later
   clean reads agree. There is no PDP double-write result here; PDP has one writer.
3. This does **not** demonstrate a Yosys RTL-mapping mismatch. This bench has
   no inferred `reg m[]` and does not measure an inferred RAM's `q`.
   In the separate exact-X03 build, Yosys registers write address, data and WE,
   delays the physical write by one cycle, adds a delayed-data FF bank and a
   registered forwarding select, and muxes delayed data against the raw EBR
   read (75 fabric FFs total). Thus raw EBR NEW is the *previous logical write*,
   not the current RTL `d`. `q <= m[a]` with a nonblocking write requires old
   logical memory at that edge. The mapping structure and changing-D kernel
   regression agree with read-old semantics. Constant D=1 alone cannot tell
   read-old and read-new apart after the first write. No inferred-`q` board
   trace is claimed by HB09P; a `q` that returned current D on a same-address
   changing-D write would be an RTL/mapping mismatch and must be documented
   separately if observed. The emulator follows the measured raw hardware.

## Not covered

Only read width **36** was produced in the PDP inference matrix. Width 32 is a
logical subset of that same 36-bit physical port; it is not a physical 32-bit
read mode. Other PDP read widths 1/2/4/9/18, OUTREG, other WRITEMODE profiles,
partial/unknown masks, ambiguous addresses or edges, CE/OCE gating, unmatched
CS, reset/GSR/GWDIS activity, different clocks/phases and collision timing
windows remain conservative. In hardware each profile uses four hot words
(0,1,depth-2,depth-1), not an exhaustive address sweep. This is evidence for
this device/setup/profile, not an electrical collision guarantee for all chips.
B06's DP double-write and mixed-width/mask questions remain open.

## Emulator check

`src/kernel/ebr.rs` shares a static `hb09p_profile` qualifier with the proposed
fast port: PDP write/read 36/36, NORMAL, NOREG, ASYNC reset/release, GSR disabled.
At capture both accesses/edges must be definite on the same clock and polarity,
CE/CS selected, resets/GWDIS inactive, OCE=0/1, addresses known and all four
write masks enabled. Same-word reads take the pending write's exact bit value;
all other PDP collision handling stays conservative. Selected CS is compared
in the compiler's raw-pin/decode representation, including constant inversions.
PDP WE pins are unused in this representation and are not qualification inputs.

[tests/ebr_r02.rs](../../../../tests/ebr_r02.rs) covers every physical PDP36
word and bit, conservative fallbacks, a raw 17,408-bit replay of hb09ps, and
exact inferred X03 over all 512 rows. X03 also changes D between same-address
edges to check logical read-old semantics; its repeated-write q is known and
matches the new *physical* word. Resolved accesses produce no conservative
collision notices. Before this change the raw read was X on INIT9 rows 0–3;
Yosys's unselected forwarding input hid it from q (historical
[reproduction.json](reproduction.json)). The current kernel diagnosis is
[reproduction_after.json](reproduction_after.json); fast still awaits A/B.

[fast_a_hb09p.patch](fast_a_hb09p.patch) ports the same qualifier and NEW-word
forwarding to `src/kernel/fast.rs`. [fast_b_selected_demand.patch](fast_b_selected_demand.patch)
is applied after A: only when every consumer of an EBR output is a checked mux
or LUT does its X check move from capture to a lazy runtime read. LUTs become
conditional decision trees with non-EBR inputs first; equal cofactors omit the
unused EBR read. Those lines cannot fold into an unchecked shift/index. Direct
stateful/opaque/control consumers retain the capture check. Runtime selected-X
reads still set error 2, including on a selector change without another EBR
edge; worker reporters claim the error slots atomically. Initial-unknown
startup policy is unchanged. This is deliberately narrower than general
four-valued fast execution. Neither fast source file was edited;
`src/fast.rs` needs no change because error 2 already reports an unknown EBR read.
The two fast regressions are explicitly ignored pending application of A/B.
The two native kernel tests are ignored separately for their database/image-reader/toolchain requirements, as in the other native
suites; `--include-ignored` runs them. The legacy HB09 UART regression now
allows the conservative first-raw-X stop but requires refusal no later than
the first FF capture of X, so selected-demand checks can defer an unused X.

On rserv, targeted `--include-ignored` checks passed: ebr 19, ebr_native 3,
ebr_collision_native 3, new kernel tests 5, and both HB11 fast tests (the long
event replay was not selected). Explicitly including the two pending new fast
tests reproduces their expected failures in the untouched fast implementation.
An isolated preview with A then B passes all 32 targeted tests, including both
new fast tests, plus the two HB11 fast tests. Exact X03 also completes its
one/two-cycle fast checks with the hunks. The whole suite was run once and
passed; fmt and release clippy `--all-targets -- -D warnings` pass for both the
working tree and patched preview. [validation.json](validation.json) records
counts, scope, hashes and limitations.

## Exact commands

From the repository root, with a scratch path without spaces:

```sh
series="$PWD/docs/inventory/LFE5U-85F/hb09p_pdp_collision"
scratch=/private/tmp/hb09p-build
python3 "$series/check_inference.py" /private/tmp/hb09p-inference > /private/tmp/hb09p-mapping.json
cargo build --release
python3 "$series/reproduce_r02.py" /private/tmp/r02-replay
sh "$series/build.sh" "$scratch"
```

`build.sh` generates both Verilog files, runs the analysis selftest and, inside
the scratch directory, runs the following for `design=hb09p` and `design=hb09ps`:

```sh
yosys -Q -p "read_verilog $design.v; synth_ecp5 -top top -json $design.json"
nextpnr-ecp5 --85k --package CABGA381 --json "$design.json" \
  --lpf "$series/hb09p.lpf" --textcfg "$design.config" \
  --write "${design}_routed.json" --seed 1
ecppack --compress "$design.config" "$design.bit"
```

**Board operator commands used for these records.** Use HB09's
SRAM/UART runner (serial path `/dev/cu.usbserial-<FTDI-serial>`) under the
exclusive board lock:

```sh
python3 "$series/../hb09_ebr_collision/run.py" "$scratch/hb09p.bit" R 3 30 | tee "$scratch/observations.log"
python3 "$series/../hb09_ebr_collision/run.py" "$scratch/hb09ps.bit" R 3 8 | tee "$scratch/observations_s.log"
python3 "$series/hb09p_analyse.py" "$scratch/observations.log" > "$scratch/analysis.json"
python3 "$series/hb09p_analyse.py" "$scratch/observations_s.log" --once > "$scratch/analysis_s.json"
```

