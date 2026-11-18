# HI12: ECLKSYNC MODE NONE on the default CLKDIVF route (R09 / X13)

**HI12 run_2 closes R09's MODE NONE refusal.** On the ULX3S, MODE NONE is
consistent with the executed plain buffer: CDIVX follows the first input edges
counted by the fabric, with no /2 deficit; /3.5 has the expected ratio and rounded
counts with a free initial phase. The enabled ECLKSYNCB pin-clock control exposes
a separate startup discrepancy against HI04's current model; it remains open.
No enabled ECLKSYNCB execution was changed.

Board collection used three SRAM loads x two seconds per variant and retained
[run_2](run_2/), including raw bytes. The original five silent builds and the PLL
repair are documented in [DIAGNOSIS.md](DIAGNOSIS.md) and [failed_run_1](failed_run_1/).
The repaired profile uses HY01's external CLKOP feedback and analog settings,
with pin-clock heartbeats before measurement completion. Analysis used no
board, serial port or flash access, and made no commit.

## Build and checks

From the repository root, with Yosys, nextpnr-ecp5 and ecppack on PATH:

```sh
HI12=docs/inventory/LFE5U-85F/hi12_eclksync_none
python3 -B "$HI12/build.py" /private/tmp/hi12-repair-build
python3 -B "$HI12/check_support.py"
cargo build --release --bin ecp5_emu
python3 -B "$HI12/check_emulator.py" /private/tmp/hi12-repair-build
# Reproduce just the documented pin-clock startup comparison:
python3 -B "$HI12/check_emulator.py" /private/tmp/hi12-repair-build --variants none_default sync_2
```

The builder requires scratch outside the repository and uses `--85k --package
CABGA381 --seed 1`. It routes and packs five board designs plus six minimal
probes. Each directory has `top.v`, `top.lpf`, `top.json`, `routed.json`,
`top.config`, `top.bit` and tool logs. It checks all routed support/counter FFs
are GSR-disabled, all 96 counter bits start at RESET with the intended clock
polarity, CLKDIVF DIV/GSR, and the ECLKSYNC MODE/STOP path. It also checks
CLKFB=CLKOP, FEEDBK_PATH=CLKOP, and all four HY01 analog attributes both in the
routed cell and the written config words.

An explicit DCCA drives the support logic, leaving raw G2 on the divider input.
The board-only `route_x13.py` pre-route hook locks X13's direct G2 -> bank 7
ECLK1 chain. The board divider is at `X0/Y46/CLKDIV1`; the control synchronizer
is at `X0/Y46/ECLKSYNC1_BK7`. The checker requires the direct
`G_JPCLKT61 -> W2_JECLKI1` entry and bank-7 internal arc. No MODE enum is patched.

```sh
for v in none_default none_2 none_35 sync_2 sync_35; do
  echo "$v"
  rg 'enum: (ECLKSYNC|BRGECLKSYNC|CLKDIV|FEEDBK_PATH)|word: (ICP_CURRENT|LPF_RESISTOR|MFG_ENABLE_FILTEROPAMP|MFG_GMCREF_SEL)' /private/tmp/hi12-repair-build/$v/top.config
done
```

Omitted ECLKSYNC MODE means NONE. Tool versions are in [TOOLS.txt](TOOLS.txt),
checks in [validation.txt](validation.txt), and generated artifact hashes in
[SHA256SUMS.build](SHA256SUMS.build), with paths relative to the build directory.
The original checksums are archived under failed_run_1.

<a id="later-board-session-commands"></a>

## Later board commands

Install PySerial and set `HI12_PORT` to the ULX3S UART device. Then, from the
repository root (choose a fresh output prefix for any later retry):

```sh
HI12=docs/inventory/LFE5U-85F/hi12_eclksync_none
for v in none_default none_2 none_35 sync_2 sync_35; do
  python3 -B "$HI12/run.py" "/private/tmp/hi12-repair-build/$v/top.bit" \
    "$HI12_PORT" --loads 3 --seconds 2 --output "/private/tmp/hi12-run2/hi12-$v.log"
done
for v in none_default none_2 none_35 sync_2 sync_35; do
  python3 -B "$HI12/hi12_model.py" "/private/tmp/hi12-run2/hi12-$v.log"
done
```

For every load the runner completes exactly `openFPGALoader -b ulx3s BITSTREAM`
(SRAM), then opens UART at 115200 baud. It never writes flash. Each parsed
`hi12-NAME.log` has companions `hi12-NAME.load-001.bin`, `load-002.bin`, and
`load-003.bin` beside it. Every byte read is flushed to the raw file immediately;
empty, partial, binary, and failed captures are retained too. Existing output
files are refused before loading. Parsed summaries are also printed to stdout.

All requested loads are attempted even if a load produces no complete record.
A missing HC record makes the command exit unsuccessfully after capture, with
separate messages for a received heartbeat and no heartbeat. More than one
distinct complete HC record in a load needs investigation. HC records repeat
unchanged so opening UART after loading does not lose startup information.

## UART formats

Before completion, `HB<id> <2 hex flags> <8 hex pin count>\r\n` repeats. It starts
from the 25 MHz support clock without waiting for LOCK or observer activity.
The flags and input counter are latched at the beginning of each line:

| Flag bit | Meaning |
|---|---|
| 0 | PLL LOCK, synchronized to the pin clock |
| 1 | at least one observer rising edge seen, synchronized |
| 2 | midpoint snapshot done, synchronized |
| 3 | counters frozen, synchronized |
| 4 | finished, synchronized |
| 7:5 | zero |

`00` with an increasing pin count shows a live UART without lock or observer
edges. `02` shows observer activity without LOCK. These sticky observer/progress
flags show whether an event has occurred, not the observer's current frequency.
Once finished is synchronized, the serializer switches at a line boundary to
complete HC records. Heartbeats do not freeze or reset any measurement counter.

A complete line is `HC<id> <176 uppercase hex digits>\r\n`, unchanged from the
original series. The payload, oldest sample first, is:

- 24 hex digits: midpoint I, P, F (eight each).
- 24 hex digits: final frozen I, P, F.
- 128 hex digits: 256 two-bit phase samples, CLKI in bit 1 and CDIVX in bit 0.

| ID | Build | Divider input | DIV |
|---|---|---|---|
| 0 | none_default | direct G2, implicit ECLKSYNC MODE NONE | omitted (2.0) |
| 1 | none_2 | direct G2, implicit ECLKSYNC MODE NONE | 2.0 |
| 2 | none_35 | direct G2, implicit ECLKSYNC MODE NONE | 3.5 |
| 3 | sync_2 | G2 through explicit ECLKSYNCB, STOP=0 | 2.0 |
| 4 | sync_35 | G2 through explicit ECLKSYNCB, STOP=0 | 3.5 |

## Measurement

All support/counter FFs have GSR DISABLED. The three 32-bit counters start at
configuration, independently of LOCK: I counts pin-clock rising edges, P counts
CDIVX rising edges, and F counts CDIVX falling edges. The pin-edge counter and
UART use the always-enabled DCCA. There is no GSR/SGSR primitive. CLKDIVF has
no explicit GSR and has RST=0, ALIGNWD=0.

The separate observer uses HY01's 25 -> 100 MHz, 600 MHz VCO profile:
CLKI_DIV=1, CLKFB_DIV=4, CLKOP_DIV=6, CPHASE=2, FPHASE=0, external CLKOP feedback,
ICP_CURRENT=12, LPF_RESISTOR=8, MFG_ENABLE_FILTEROPAMP=1, MFG_GMCREF_SEL=2.
HY01's unused CLKOS output is omitted. LOCK never resets/enables the counters.

After LOCK, the observer takes a midpoint snapshot once I >= 524288 and freezes
all counters once I >= 1048576. It acts on the second consecutive low CLKI
sample, nominally 10 ns after CLKI falls, away from counted edges. It waits
16 more observer cycles before copying the frozen counts. Confirm the margin
using the phase samples before interpreting board results. No counter wraps
in the expected 42 ms window.

After LOCK and I >= 262144, a separate register records 256 consecutive
`{CLKI, CDIVX}` samples at 100 MHz. This places divider edges to 10 ns. Skew
can separate simultaneous input/divider edges by one observer sample; these
are phase observations, not calibrated propagation delays.

## Reading

The fifteen raw captures contain **1,885 complete HC records in 347,010 bytes**,
125 or 126 complete records per load. Each load repeats one immutable record.
No heartbeats were captured: UART is opened after SRAM loading, by which time
normal measurement has completed. This does not show that startup heartbeats
were absent. The ten supplied .log/.stdout files already contain capture
basenames, with no absolute scratch paths to retain; normalization and the
raw-vs-parsed byte/record counts were checked. Raw .bin bytes were not modified.
See [capture_checks.json](run_2/capture_checks.json) for each raw-file hash and
[analysis.txt](run_2/analysis.txt) for the complete model output.

| Build / loads | Midpoint (I,P,F) | Final (I,P,F) |
|---|---|---|
| none_default, none_2, sync_2; all three loads each | (524288,262144,262144) | (1048576,524288,524288) |
| none_35; all three loads; sync_35 load 1 | (524288,149796,149797) | (1048576,299593,299593) |
| sync_35 load 2 | (524288,149797,149797) | (1048576,299593,299594) |
| sync_35 load 3 | (524288,149797,149796) | (1048576,299593,299593) |

**MODE NONE.** Default and explicit /2 give I=P+F and I-2P=0 at both snapshots:
there is no missed input-edge count relative to the fabric counters on the
same running pin clock. /3.5 has a rising interval of 14 observer samples
(140 ns), high for 8 and low for 6 (80/60 ns), as HI04 measured. /2 has an
8-sample rising interval and 4/4 high/low widths. Every captured divider
transition is within **0 observer samples** of a CLKI transition, at 10 ns
resolution. These observations support the immediate buffer rule on the used
bank-7 lane-1 path. STOP was unrouted: no claim about logic-driven STOP is added.

**The /3.5 analysis correction.** Counts are integers, but the expected cycle
count is fractional: 2I/7 is 149796 + 4/7 at midpoint and 299593 + 1/7 at final.
The old checker forced the divider to be low just before its first counted
edge (phase 6), even though the pin clock can already have been running when
the fabric counters start. A first counted falling edge can therefore make
F=P+1. That is an initial phase, not evidence of a lost divider cycle.

Use one common phase phi in 0..6 before the first counted half edge. CDIVX is
high in states 0..3 and advances modulo 7 on each CLKI half edge. For H=2(I-D):

```text
P = floor((phi+H)/7)   - floor(phi/7)
F = floor((phi+H-4)/7) - floor((phi-4)/7)
```

The checker searches the same phi and omitted-input-period count D against
both snapshots. Every /3.5 capture fits D=0. none_35 and sync_35 load 1 admit
phi=0/1/2; sync_35 load 2 admits phi=3; load 3 admits phi=4/5. Thus sync_35 has
three different boot-phase records across loads, each constant within a load.
Some records also fit D=1 with another phase, so cumulative /3.5 counts alone
do not uniquely prove the first-edge phase or distinguish that one-period
ambiguity. They require no additional startup deficit, and D=4 does not fit.
The /2 zero-deficit result supplies the sharper startup observation.

`hi12_model.py` retains phase 6 as the default for strict emulator checks,
allows free initial phase when analysing board counts, and reports ambiguity
instead of rounding it into a spurious error. It ignores heartbeats and
incomplete trailing records. Reproduce the analysis without hardware:

```sh
for v in none_default none_2 none_35 sync_2 sync_35; do
  python3 -B docs/inventory/LFE5U-85F/hi12_eclksync_none/hi12_model.py \
    "docs/inventory/LFE5U-85F/hi12_eclksync_none/run_2/hi12-$v.log"
done
```

## Control startup discrepancy

The fresh actual-emulator replay uses the repaired packed bitstreams with
`--startup configured --scope observed`, G2 at 40 ns, and 1,600,000 pin cycles
(64 ms). [Replay evidence](run_2/emulator/) preserves UART bytes, status logs
and the planner rule. It prints 29 HB lines then one complete HC record per
variant. Counts beside the board are:

| Source | none_default midpoint P,F | none_default final P,F | sync_2 midpoint P,F | sync_2 final P,F |
|---|---|---|---|---|
| Board, every load | 262144,262144 | 524288,524288 | 262144,262144 | 524288,524288 |
| Event-kernel plan in fast execution | 262144,262144 | 524288,524288 | 262142,262142 | 524286,524286 |

The unchanged HI04 startup model leaves the sync_2 control four CLKI periods
late: two fewer CDIVX rising edges and two fewer falling edges at both snapshots.
The event/fast ECLK tests agree on that implemented model; this comparison
identifies a hardware-startup accuracy gap, not a kernel/fast disagreement.

[HI04](../hi04_eclk/README.md) makes ECLKI from bit 2 of a fabric counter and
uses `--allow-fabric-eclk`; the edge clock begins when that fabric runs.
`hi04_model.py` starts E00 with high STOP stages and low dividers and tests
four or five falling-edge delays; later windows search a free divider phase.
HI12 instead uses a 25 MHz pin already running throughout SRAM configuration.
Its GSR-disabled counters begin observing after fabric activation, not at the
physical oscillator's first edge. HI04's written slice/divider GSR settings
are also DISABLED, so a simplistic ENABLED-versus-DISABLED GSR explanation
does not account for the difference.

`src/compile/system.rs` maps configured FFs to REGSET regardless of slice GSR,
sets `wake=None` for configured/ready-unknown, and only creates logical wake
phases for the explicit wake-phases profile. User GSR, startup GSR and the hard
clock block's startup are separate. Neither HI12 nor the STOP model connects
a user GSR to the synchronizer. [SYSTEM_STARTUP.md](../../../SYSTEM_STARTUP.md)
keeps physical POR/configuration/PUR and the exact wake-clock boundaries
unmodelled. GSR-disabled counters therefore do not prove that hard ECLK state
and fabric counting began simultaneously. The emulator starts both its input
stimulus and its initialized STOP state after loading, with no prior clock
history, explaining its four-edge lag in this profile.

**Smallest proposed rule:** keep the four-falling-edge STOP pipeline, but
advance it whenever ECLKI is available, including before fabric counters start.
A pre-running pin with STOP=0 may already have cleared its stages by that
boundary; a newly created fabric edge clock still pays the four falls. A
pre-running /3.5 divider may likewise have a free phase at fabric start.
This is consistent with both series without bypassing runtime STOP or inventing
a universal pin-specific pipeline. A future implementation needs explicit clock
history: a pin reference alone does not prove the clock ran before loading. It is a hypothesis about the boundary/history, not a
measured configuration timeline, and it does not settle HI04's occasional
extra startup edge.

**Deciding board run:** use matched pin/fabric paths and GSR-disabled counters.
Use the same bitstream and clock-capable input for two loads: keep an external
generator running through one configuration, and hold it low until after the
other configuration before starting it. Holding the signal inside fabric would
not equalize the source histories. The
history rule predicts a four-falling-edge startup in that delayed-pin case.
After both clocks are running, assert STOP long enough to fill the pipeline
and release it; both should retain the four-edge runtime response. If equal
clock histories still give different starts, a source-specific or PUR/init
rule is needed. Neither HI04 nor HI12 performs that matched experiment.
No new board run and no enabled ECLKSYNCB execution change is made here.

## Minimal probes and execution scope

The six minimal probes use G2 and B2, with default, explicit 2.0, or 3.5 DIV,
from either the pin or the corrected PLL. Pin probes retain
`LFE5U-85F/R46C0/ECLKSYNC1_BK7`. With the corrected PLL profile, nextpnr instead
uses `LFE5U-85F/R47C0/ECLKSYNC1_BK6` and the neighbor path into bank 7 ECLK1:
`S1W2_ECLKI_ECLKSYNC1 -> S1W2_JECLKO_ECLKSYNC1 -> W2_JNEIGHBORECLK1`.
All use `CIB_R46C2:ECLK_L`, implicit MODE NONE, no routed STOP, and CLKDIV1
GSR DISABLED. The PLL entry is `G_JLLQECLKCIB1`; no BRGECLKSYNC is used. The
board-only hook keeps the board experiments on X13's bank-7 direct path.
See [routing.txt](routing.txt) for exact extracted routes.

R09 is fixed for the qualified NONE profile: ECLKO=ECLKI with unrouted STOP;
connected STOP remains refused. The resource message cites the HI12 result.
The shared audit/coverage/clock/IOLOGIC documents now record that result and
keep the enabled pin-clock startup discrepancy open. ECLKSYNCB execution is
unchanged; the tests' NONE report-string assertion follows the measured message.
