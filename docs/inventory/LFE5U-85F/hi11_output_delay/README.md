# HI11: output DELAYF / DELAYG after an SDR output register

HI11 and all HI11s variants were measured on the left/bottom banks; right/top banks were not run.
HI11 ran on the ULX3S on 2026-11-17 using JTAG SRAM loads, with flash untouched: three loads and
three seconds of capture per variant. The copied [records](records/) and routed configs
[hi11.config](hi11.config), [hi11g.config](hi11g.config), [hi11b.config](hi11b.config) are the
board evidence. All six HI11s configurations and their records are also retained: 27 SRAM
loads across the nine designs, 166 complete frames. Analysis used this evidence
and software checks without further board or serial access. Synthetic/emulated records are software checks.

## Setup

Target: ULX3S LFE5U-85F, CABGA381, 25 MHz on G2. The UART TX output is `ftdi_rxd` on L4,
115200 8N1 (25 MHz / 217 = 115207.37 baud). The host receives every byte **through the delay
being tested**. The extra output register adds one 40 ns cycle, with no inversion. Correct
frames at the host check data polarity and preservation; this does not measure tap duration.

| Design | TX path | DEL_VALUE | Record |
| --- | --- | --- | --- |
| [hi11.v](hi11.v) | fabric TX bit → OFS1P3BX → DELAYF → L4 | 10 | `H11F` plus the same DELAYF's CFLAG |
| [hi11g.v](hi11g.v) | fabric TX bit → OFS1P3BX → DELAYG → L4 | 127 | `H11G`, reserved CFLAG bit 0 |
| [hi11b.v](hi11b.v) | fabric TX bit → fabric FF → DELAYG → L4 | 127 | `H11B`, reserved CFLAG bit 0 |

All use [hi11.lpf](hi11.lpf). The fabric FF has `syn_useioff=0`; the routed netlist confirms
it remains a TRELLIS_FF and feeds the delay's IOLOGIC TXDATA0. [hi11_gen.py](hi11_gen.py)
reproduces the three designs (`f`, `g`, `fabric`). There are no button inputs or external
loopback wires. A fabric RAM stores 2048 four-bit samples, then the UART repeats that capture
with increasing sequence numbers. Reload SRAM to repeat the startup measurement.

## Program and records

LOADN, MOVE and DIRECTION are fabric registers. Each program step is one 25 MHz cycle. The
capture reads their stable values and CFLAG **before** the edge updates the controls, as HI08
does: sample 0 contains the initial controls `(1,0,1)`; sample i contains program step i−1.
There is a 65536-cycle warm-up with those initial controls and **no load**.

The program first observes the startup counter, pulses down before any load, and switches
DIRECTION with MOVE low. It then loads 10, gives 140 pulses up (including saturation at 127),
loads again and gives 140 pulses down (including saturation at 0). Direction-only changes at
an endpoint check CFLAG's combinational selection. Further loads precede 32-cycle MOVE-high
holds in each direction, then two-cycle MOVE pulses and pulses while LOADN is low. DIRECTION
is stable before each MOVE rise and throughout its high interval. No out-of-protocol samples
need to be omitted. The precise exclusive phase bounds are `PHASES` in the generator.

A frame is exactly 2063 bytes including CR LF, all hexadecimal digits uppercase:

```text
H11V SSSSSSSS [2048 sample nibbles] X CR LF
```

The spaces/brackets are explanatory and are not transmitted. V is F/G/B. S is a 32-bit
sequence starting at 0 and incrementing after each complete frame. Each sample nibble is
`{LOADN, MOVE, DIRECTION, CFLAG}`; X is the XOR of all 2048 nibbles. The capture is repeated
unchanged, with a 2500000-cycle pause between frames. The UART allows synchronous sample-RAM
reads to settle before starting each character.

[hi11_model.py](hi11_model.py) checks complete framing, increasing sequences, all control
samples and the checksum. For F it duplicates the one observed CFLAG into the two lanes of
HI08's record format and invokes the **existing** [hi08_model.py](../hi08_delay/hi08_model.py)
with values 10/10, from the first load onward. This reuses its saturating 0..127,
LOADN-priority, falling-MOVE-edge model without assuming startup is zero. Before the first
load the checker reports all initial counter values consistent with the flags under that
model. The fixed-length HI08 adapter replaces the pre-load prefix with copies of the first
load sample; every real post-load sample keeps its original index and is checked. G/B must
have their reserved CFLAG bit clear. Partial first and last frames from the host capture are
allowed; malformed complete frames fail.

## Exact build commands (Mac; no hardware access)

From the repository root:

```sh
HI11_SRC=docs/inventory/LFE5U-85F/hi11_output_delay
HI11_BUILD="$(mktemp -d /tmp/hi11.XXXXXX)"
python3 -u "$HI11_SRC/build.py" "$HI11_BUILD"
```

[build.py](build.py) copies each design and LPF to its own scratch subdirectory and executes:

```sh
yosys -Q -p 'read_verilog top.v; synth_ecp5 -top top -json top.json'
nextpnr-ecp5 --85k --package CABGA381 --seed 1 --json top.json --lpf top.lpf --textcfg top.config --write top_routed.json
ecppack top.config top.bit
```

Inspect each routed configuration and the IOLOGIC mode (run from the same shell):

```sh
for name in hi11 hi11g hi11b; do
  rg 'DELAY.OUTDEL ENABLED|IOLOGIC[A-D]\.MODE |DATAMUX_OREG|DELAY.DEL_VALUE' "$HI11_BUILD/$name/top.config"
done
```

All three final designs passed synthesis, routing and ecppack with seed 1. Tool versions and
configuration/bitstream fingerprints are in [mac_build.txt](mac_build.txt). The logs, JSON,
configs and bits remain outside the repository under the scratch path recorded there.

The L4 IOLOGIC is **side-bank IOLOGICC**, BEL `X0/Y56/IOLOGICC`, with its settings in tile
`MIB_R58C0:MIB_CIB_LR`. F/G use `MODE=IREG_OREG`, `DELAY.OUTDEL=ENABLED`, and
`PIOC.DATAMUX_OREG=IOLDO`; B uses `MODE=NONE`, `DELAY.OUTDEL=ENABLED`, with the default
`DATAMUX_OREG=PADDO` (omitted from the config). All have `WAIT_FOR_EDGE=DISABLED`. nextpnr
calls the output net IOLDO: `IOLDOC_IOLOGIC → IOLDOC → IOLDOC_PIO`; OUTDEL selects the delayed
IOLDOD producer in the device model. F also routes `JCFLAGC_IOLOGIC` back into the capture
logic. B routes a fabric FF's Q into `JTXDATA0C_IOLOGIC`, bypassing the IO register.

<a id="exact-board-commands-planning-session-only"></a>

## Exact board commands

[run.py](run.py) loads SRAM with
`openFPGALoader -b ulx3s <bit>` and waits for it to exit successfully **before** opening the
serial port, following HB11's runner. It never passes a flash option. Requirements are
`openFPGALoader` and Python `pyserial`; set `ECP5_SERIAL_PORT` or pass `--port`
with the board UART device (for example, `/dev/cu.usbserial-<FTDI-serial>`). Retain `HI11_SRC` and `HI11_BUILD` from the build shell:

```sh
HI11_RECORDS="$(mktemp -d /tmp/hi11-records.XXXXXX)"
python3 "$HI11_SRC/run.py" "$HI11_BUILD/hi11/top.bit" H11F 3 3 --out "$HI11_RECORDS"
python3 "$HI11_SRC/run.py" "$HI11_BUILD/hi11g/top.bit" H11G 3 3 --out "$HI11_RECORDS"
python3 "$HI11_SRC/run.py" "$HI11_BUILD/hi11b/top.bit" H11B 3 3 --out "$HI11_RECORDS"
python3 "$HI11_SRC/hi11_model.py" "$HI11_RECORDS"/*.txt
```

The runner saves raw UART bytes and a timestamped loader log per SRAM load. Check all three
loads of each variant, save the records/checker output and tool versions, and record the
board revision and JTAG IDCODE when adding observations here.

## Observations

The nine SRAM loads each gave ten complete frames, sequences 1..10:
**90 frames** with correct framing, control samples and XOR. The host receives correct UART
bytes through OFS1P3BX → DELAYF, OFS1P3BX → DELAYG(127), and fabric FF → DELAYG(127), proving
data preservation and polarity on those output paths. No physical tap timing is inferred.

All three F loads have sample 0 `{LOADN=1, MOVE=0, DIRECTION=1, CFLAG=0}`. The eight completed
down pulses have their falling edges at samples 18, 22, 26, 30, 34, 38, 42 and 46, and CFLAG
stays zero. Under HI08's measured stepping model this bounds the initial output counter to
**9..127**, rather than zero, from that short program alone. The HI11s runs below resolve
the complete startup rule. The first
load is sample 57; all 1991 real samples from that load onward fit the HI08 model in every
frame. CFLAG reaches the upper endpoint at sample 530 after 117 up steps from 10, and the
lower endpoint at sample 670 after 10 down steps from a load. The other visible controls,
including saturation, MOVE-high holds, direction-only changes and LOADN priority, also fit.
[model_check.txt](model_check.txt) records the checker output for all nine files.

Regenerate that report from the repository root:

```sh
python3 -B docs/inventory/LFE5U-85F/hi11_output_delay/hi11_model.py \
  docs/inventory/LFE5U-85F/hi11_output_delay/records/H11[FBG]-*.txt \
  > docs/inventory/LFE5U-85F/hi11_output_delay/model_check.txt
```

## Final Reading

1. **Output data.** All three original variants deliver UART data unchanged and uninverted:
   OFS1P3BX → DELAYF, OFS1P3BX → DELAYG(127), and fabric FF → DELAYG(127). Their nine loads
   gave 90 valid complete frames. Physical tap duration was not measured; the emulator uses
   zero propagation time for these paths, as for every other propagation delay.
2. **Counter after configuration.** Every load agrees. The measured initial counts are:

   | DEL_VALUE | 1 | 10 | 63 | 64 | 100 | 127 |
   | --- | ---: | ---: | ---: | ---: | ---: | ---: |
   | Left IOLOGIC: output L4 and input A2 | 0 | 10 | 62 | 0 | 36 | 62 |
   | Bottom SIOLOGIC: input R1 | 0 | — | 0 | 0 | — | 0 |

   Thus **IOLOGIC starts at `DEL_VALUE & 0x3E`**, loading bits 1..5 but not bits 0 or 6 at
   configuration; **SIOLOGIC starts at 0**. HI08 also measured bottom-bank FIRE inputs at 0
   for DEL_VALUE 10 and 120. The dash means that exact R1/DEL_VALUE combination was not run
   in HI11s. The evidence covers left output/input and bottom input. **Right and top banks
   were not run**; execution applies each kind's rule there as an explicit assumption.
   [model_check_s.txt](model_check_s.txt) covers 25 two-lane frames; the 51 three-lane frames
   are checked in [model_check_boundary.txt](model_check_boundary.txt).
3. **After the first load.** Every lane matches HI08 with **0 differing samples**: 1439 real
   samples per path per HI11s frame from sample 609 onward (1991 from sample 57 in original
   HI11F). LOADN low loads **all seven DEL_VALUE bits**, immediately and while held; MOVE
   steps at its falling edge, with saturation at 0/127. CFLAG selects the directional endpoint.
   The startup mask does not limit the loaded value or subsequent stepping. Original HI11
   also covers MOVE holds, direction-only changes and load priority. HI08's documented
   out-of-protocol DIRECTION/MOVE limitations remain; HI11 uses valid control sequences.

Reproduce the startup reports from the repository root:

```sh
python3 -B docs/inventory/LFE5U-85F/hi11_output_delay/hi11_model.py \
  docs/inventory/LFE5U-85F/hi11_output_delay/records/H11S[01]-*.txt \
  > docs/inventory/LFE5U-85F/hi11_output_delay/model_check_s.txt
python3 -B docs/inventory/LFE5U-85F/hi11_output_delay/hi11_model.py \
  docs/inventory/LFE5U-85F/hi11_output_delay/records/H11S[2-5]-*.txt \
  > docs/inventory/LFE5U-85F/hi11_output_delay/model_check_boundary.txt
```

The stored loader logs retain the loader output and timestamps; their record-path metadata
has been normalized to series-relative paths; checker reports use repository-relative paths.
The UART record bytes are unchanged. [TOOLS.txt](TOOLS.txt) records the versions and loading
method, and [SHA256SUMS](SHA256SUMS) covers every stored series file except itself. Commands
are repository-relative and scratch directories come from `mktemp`.

## Emulator and audit scope

R08/X12 uses Q=A2: OFS1P3BX → DELAYG(1). Mac nextpnr writes `MODE=IREG_OREG`, OUTDEL=ENABLED,
and `PIOA.DATAMUX_OREG=IOLDO` at `X0/Y35/IOLOGICA`. The equivalent output DELAYF uses the same
path and exposes CFLAG. A fabric FF → DELAYG(127) → A2 uses `MODE=NONE`, OUTDEL=ENABLED, and
the default PIO fabric selection. **Before this change that fabric case already passed**,
with one fabric FF, because the planner followed the fabric route and never visited IOLDOD.
It now explicitly plans the delay and checks its settings too.

`src/compile/iologic.rs` executes IOLDOD as IOLDO with zero propagation time, consistent with
HI08's input-delay treatment. IOLDO comes from the IO register or bypassed TXDATA0. Output
DELAYF CFLAG uses the existing path-neutral counter. WAIT_FOR_EDGE remains refused. No fast
executor changes are required: only existing buffers, IO registers and delay-counter gates
are planned. `tests/output_delay.rs` checks both executors on A2, L4 and top-bank A12, with
controls driven through inputs, exact-edge pad values and off-edge register holds.

R08 is fixed in the audit and settings coverage. `execute_delay_flag` passes the kind-specific
initial count into the kernel's DELAYF counter drivers. Fast mode reads those initialized
kernel values and needs no implementation change. `tests/output_delay.rs` replays the six
HI11s native configurations: every captured CFLAG/control sample matches the board and both
executors have equal traces throughout warm-up and both phases of the clock. The measured
left/bottom scope and right/top assumptions are recorded in the execution report.


## HI11s: distinguish output-path and side-bank startup

[hi11s10.v](hi11s10.v) and [hi11s100.v](hi11s100.v) use DEL_VALUE 10 and 100 respectively on
**both** DELAYF. [hi11s_gen.py](hi11s_gen.py) reproduces them. The output delay is on the
same OFS1P3BX → DELAYF → L4 UART path as F. The second DELAYF is an **input-only J1 GP9,
package A2**, with `LVCMOS33 PULLMODE=UP` in [hi11s.lpf](hi11s.lpf). Leave this header signal
unconnected. Its delayed Z is also recorded to check that the input actually reads high.

The official [v2.x/v3.0.x constraints](https://github.com/emard/ulx3s/blob/master/doc/constraints/ulx3s_v20.lpf)
map GP9 to A2, without a peripheral alias. The [board manual](https://github.com/emard/ulx3s/blob/master/doc/MANUAL.md)
identifies GP/GN 11..13 as shared with Wi-Fi on these boards and GP/GN 14..17 with ADC; GP9
is outside those groups and is absent from the flash, SD, SDRAM, USB and other peripheral
assignments. It is a free header signal on the stock board. The repository package inventory
places A2 at R35C0/PIOA, side bank 7; L4 uses side bank 6. Both routed delays are full IOLOGIC,
so this compares side-bank input against side-bank output, with HI08's bottom SIOLOGIC input
as the existing reference. It does not prove all banks/sites share one startup rule.

The controls start `(LOADN=1,MOVE=0,DIRECTION=1)` through the 65536-cycle warm-up. There is no
load during 140 down pulses at program steps 16..575, followed by endpoint holds and
direction-only changes. The first load appears at **sample 609**, then 140 up pulses, another
load, and 140 down pulses. DIRECTION is stable before each MOVE rise. Sample i reads controls
from program step i−1, as in HI11. Thus the number of completed startup down pulses before
the first CFLAG gives the initial count if the startup stepping follows HI08; the checker
reports separately when no initial 0..127 value fits that assumption.

HI11s frame (4113 bytes including CR LF; spaces below are explanatory):

```text
H11SV SSSSSSSS [2048 two-digit sample bytes] XX CR LF
```

V=0 means DEL_VALUE=10, V=1 means 100. Each sample byte is
`{2'b0, delayed_input, LOADN, MOVE, DIRECTION, input_CFLAG, output_CFLAG}`. XX is the byte XOR
of all samples. The checker requires the reserved bits zero and delayed_input high, checks
all controls, and infers startup independently for the two CFLAG lanes. From the first load
on it passes the two actual lanes to HI08's checker (1439 real samples/path). Comparing the
two parameter builds distinguishes startup at DEL_VALUE from a fixed count; comparing input
and output lanes tests whether the difference also appears on a side-bank input. Each frame
repeats the same capture with an increasing sequence, so each SRAM reload is a new startup
trial. The longer frame takes about 0.36 seconds plus a 0.1-second inter-frame pause; a
three-second capture should contain several complete frames, rather than HI11's ten.

Exact build commands, from the repository root (no hardware access):

```sh
HI11_SRC=docs/inventory/LFE5U-85F/hi11_output_delay
HI11S_BUILD="$(mktemp -d /tmp/hi11s.XXXXXX)"
python3 -u -B "$HI11_SRC/build.py" "$HI11S_BUILD" hi11s10 hi11s100
for name in hi11s10 hi11s100; do
  rg 'DELAY.OUTDEL|DELAY.DEL_VALUE|DELAY.WAIT_FOR_EDGE|IOLOGIC[A-D]\.MODE ' "$HI11S_BUILD/$name/top.config"
done
```

The builder uses the same Yosys / nextpnr-ecp5 `--85k --package CABGA381 --seed 1` / ecppack
commands listed above. Both Mac builds passed. For each build the routed config sets input
IOLOGICA at X0/Y35 to `OUTDEL=DISABLED`, output IOLOGICC at X0/Y56 to `OUTDEL=ENABLED`, both
`MODE=IREG_OREG`, matching DEL_VALUE (0001010 or 1100100), and WAIT_FOR_EDGE=DISABLED. Both CFLAG
are routed into sample RAM and the controls share nets. The A2 PIO is input-only with no I/T
drivers. [hi11s_mac_build.txt](hi11s_mac_build.txt) contains versions, grep output, scratch
paths, hashes and historical software checks. The board results are summarized in Final Reading;
the emulated startup of zero in that older build report is not a board observation.

These board commands were not run during analysis. The same runner
loads SRAM first and opens serial only after the loader exits:

```sh
HI11S_RECORDS="$(mktemp -d /tmp/hi11s-records.XXXXXX)"
python3 "$HI11_SRC/run.py" "$HI11S_BUILD/hi11s10/top.bit" H11S0 3 3 --out "$HI11S_RECORDS"
python3 "$HI11_SRC/run.py" "$HI11S_BUILD/hi11s100/top.bit" H11S1 3 3 --out "$HI11S_RECORDS"
python3 -B "$HI11_SRC/hi11_model.py" "$HI11S_RECORDS"/*.txt
```

Software checks (no board or serial access):

```sh
python3 -B -m unittest discover -s docs/inventory/LFE5U-85F/hi11_output_delay -p 'test_*.py'
```


## Three-lane boundary measurements

The new [hi11s1.v](hi11s1.v), [hi11s63.v](hi11s63.v), [hi11s64.v](hi11s64.v) and
[hi11s127.v](hi11s127.v) keep the **same 2048-step program**: 140 down pulses before any load,
then the loaded up/down runs. Every DELAYF in a build has that build's DEL_VALUE. The third
lane is an **input DELAYF on FIRE1/R1**, bottom-bank SIOLOGIC, with the exact HI08 input
settings `IO_TYPE=LVCMOS33 PULLMODE=DOWN`. Keep FIRE1 released. The other lanes remain output
DELAYF on UART L4 and input DELAYF on free J1 GP9/A2, pulled up and left unconnected.
[hi11s3.lpf](hi11s3.lpf) adds R1 to the original HI11s constraints; the original two-lane
sources and their format are preserved by the generator.

| Build | DEL_VALUE | Record prefix | Measured left-side startup | Measured bottom startup |
| --- | ---: | --- | ---: | ---: |
| hi11s1 | 1 | H11S2 | 0 | 0 |
| hi11s63 | 63 | H11S3 | 62 | 0 |
| hi11s64 | 64 | H11S4 | 0 | 0 |
| hi11s127 | 127 | H11S5 | 62 | 0 |

All three SRAM loads of each build agreed. The checker infers and reports the initial count
separately for output, side-bank input and bottom-bank input; it does not force the formula. It checks
all three lanes after the first load through the existing HI08 checker, projecting two
lanes at a time and duplicating the last lane for its second invocation.

The frame keeps the HI11s length of 4113 bytes including CR LF. For H11S2..H11S5 the sample
byte is `{bottom_data, bottom_CFLAG, side_data, LOADN, MOVE, DIRECTION, side_CFLAG, output_CFLAG}`.
All three flags and both delayed input data bits are sampled at the same edge. The checker
expects side_data=1 and bottom_data=0 for the stated pin setup, validates all controls, then
reports startup independently from the pre-load flags. H11S0/S1 retain their two reserved
upper bits. The UART sequence and XOR are unchanged; `run.py` recognizes all six HI11s IDs.

All four boundary builds synthesized, routed and packed successfully on the Mac with
`nextpnr-ecp5 --85k --package CABGA381 --seed 1` and ecppack, in a `mktemp` directory outside
the repository. The routed settings and BELs are:

| Lane | Pin / resource BEL | MODE | OUTDEL |
| --- | --- | --- | --- |
| Output | L4 / X0/Y56/IOLOGICC | IREG_OREG | ENABLED |
| Side input | A2 / X0/Y35/IOLOGICA | IREG_OREG | DISABLED |
| Bottom input | R1 / X4/Y95/SIOLOGICA | IREG_OREG | DISABLED |

Each config contains the same requested DEL_VALUE on all three delay resources and
WAIT_FOR_EDGE=DISABLED. All CFLAG are routed into the sample RAM, all controls share nets,
and both input PIOs have no output drivers. [hi11s_boundary_mac_build.txt](hi11s_boundary_mac_build.txt)
records the original build versions, config grep output, BELs and fingerprints. Its emulated
zero startup was a historical software result before this fix, not board evidence. The copied
board records/configurations and normalized model_check_boundary.txt establish the final rule.

Exact build and inspection commands, from the repository root:

```sh
HI11_SRC=docs/inventory/LFE5U-85F/hi11_output_delay
HI11S_BUILD="$(mktemp -d /tmp/hi11s-boundary.XXXXXX)"
python3 -u -B "$HI11_SRC/build.py" "$HI11S_BUILD" hi11s1 hi11s63 hi11s64 hi11s127
for name in hi11s1 hi11s63 hi11s64 hi11s127; do
  rg 'DELAY.OUTDEL|DELAY.DEL_VALUE|DELAY.WAIT_FOR_EDGE|IOLOGIC[A-D]\.MODE ' "$HI11S_BUILD/$name/top.config"
  python3 -c 'import json,sys; cells=json.load(open(sys.argv[1]))["modules"]["top"]["cells"]; print("\n".join(c["attributes"]["NEXTPNR_BEL"] for c in cells.values() if c["type"] in ("IOLOGIC","SIOLOGIC")))' "$HI11S_BUILD/$name/top_routed.json"
done
```

These board commands were not run during analysis. The runner always
loads SRAM first, then opens serial, without a flash option:

```sh
HI11S_RECORDS="$(mktemp -d /tmp/hi11s-boundary-records.XXXXXX)"
for spec in hi11s1:H11S2 hi11s63:H11S3 hi11s64:H11S4 hi11s127:H11S5; do
  python3 "$HI11_SRC/run.py" "$HI11S_BUILD/${spec%:*}/top.bit" "${spec#*:}" 3 3 --out "$HI11S_RECORDS"
done
python3 -B "$HI11_SRC/hi11_model.py" "$HI11S_RECORDS"/*.txt
```
