# HC08: EHXPLLL analog defaults and feedback return

[run_1](run_1/) and [run_2](run_2/) establish that A/B/E
lock and run at 100 MHz, while C/D/F/G/H lock but run at 16.667 MHz. F/G vary
CLKOS3_DIV and H changes INT_OP to INT_OS without changing that rate. The
result supports raw-VCO feedback for these two fabric-return bit patterns and
rejects CLKOS3-divider feedback. Analog omissions do not change the measured
outcomes. The emulator's saved 100 MHz predictions for C/D/F/G/H are wrong.
Preparation and analysis used the records without further board or serial
access. Board collection comprised 24 SRAM loads, with flash untouched.

This separates the two changes combined in HI12's
[failed first run](../hi12_eclksync_none/DIAGNOSIS.md). A silent HI12 reporter did
not establish whether missing analog attributes, the INT_OP/fabric return, or
both prevented operation. HC08 reports even with LOCK low and CLKOP stopped.
The board-preparation stages did not change emulator source. The implementation
below now applies the qualified rule; no commit was created during that work.

## Reading: run_1

[run_output.txt](run_1/run_output.txt) and the raw `.bin` files contain three
8 s captures per build. All fifteen loads recover all 201 startup timestamps
and agree: synchronized LOCK is zero only at the initialized t=0 sample, is
one at 10 ms, and remains one at every later captured sample. Thus lock onset
is bounded by the first 10 ms observation, rather than measured at exactly
10 ms. Repeated startup records agree with the corresponding live records.

| Build | Loads | Steady CLKOP edges / 250000 pin ticks | CLKOP MHz | LOCK from 10 ms |
|---|---:|---|---:|---|
| A | 3 | 1000000 | 100.000 | 1 |
| B | 3 | 999999–1000001 | 100.000 | 1 |
| C | 3 | 166666–166667 | 16.667 | 1 |
| D | 3 | 166666–166667 | 16.667 | 1 |
| E | 3 | 1000000 | 100.000 | 1 |

The first window includes startup/CDC delay; the table uses subsequent
10 ms intervals. Single-edge endpoint differences do not alter the measured
mean rate. [run_1_reading.json](run_1_reading.json) records each load's counts,
mean frequency and lock observations, recomputed directly from raw captures.

B/A show that zero analog words do not prevent the correct 100 MHz output
with external CLKOP feedback in these loads. C/D agree despite their different
analog words. D/E isolate the return connection with analog words omitted:
fabric CLKOP with INT_OP gives 16.667 MHz; CLKINTFB return with INT_OP gives
100 MHz. LOCK alone cannot qualify the output frequency. C/D's measured rate
is `25 MHz * CLKFB_DIV(4) / CLKOP_DIV(6)`, consistent with feedback before the
CLKOP divider. This does not by itself identify the physical feedback node.
HC08 D reproduces HI12's PLL profile, not the whole silent HI12 design; the
earlier silence is not proof that this PLL profile cannot oscillate or lock.

Recompute the summary without a board:

```sh
probe=docs/inventory/LFE5U-85F/hc08_pll_analog
python3 -B "$probe/read_run.py" "$probe/run_1"
```

## Final Reading: run_1 and run_2

[run_2/run_output.txt](run_2/run_output.txt) and its nine raw captures show
the same 16.666667 MHz for F, G and H across three loads each. Every load
recovers all 201 startup timestamps, has initialized LOCK=0 at t=0, and shows
LOCK=1 from 10 ms through the remaining captured samples. Startup replays and
live records agree. Steady windows alternate 166666/166667 CLKOP edges per
250000 pin ticks. [run_2_reading.json](run_2_reading.json) retains per-load
counts, mean frequencies and lock observations recomputed from raw bytes.

| Profiles | CLKOS3_DIV | Board CLKOP | Final interpretation |
|---|---|---|---|
| A/B | 8 | 100 MHz | External CLKOP feedback, documented divider relation |
| E | 8 | 100 MHz | INT_OP with CLKINTFB returned to CLKFB, documented divider relation |
| C/D | 8 | 16.667 MHz | INT_OP plus fabric return, raw-VCO feedback |
| F | 3 | 16.667 MHz | Same INT_OP union, independent of CLKOS3_DIV |
| G | 2 | 16.667 MHz | Same INT_OP union, independent of CLKOS3_DIV |
| H | 3 | 16.667 MHz | INT_OS plus fabric return also selects raw-VCO feedback |

For these measured fabric-return combinations, the feedback divider sees the
undivided VCO, so the emulator's qualified frequency rule should be:

```text
Fvco = Fin * CLKFB_DIV / CLKI_DIV
Fout = Fvco / Dout
```

Here `25 * 4 / 1 = 100 MHz` VCO and `100 / 6 = 16.666667 MHz` CLKOP.
The board's LOCK still rises as usual at the reporter's resolution: first
high at 10 ms, with its actual onset somewhere before that synchronized
sample. The measurements reject CLKOS3-divider feedback: changing its divider
8 -> 3 -> 2 does not change the observed frequency. Raw VCO is the supported
operational model; the captures do not directly probe an internal analog node.

For A/B and E, retain the documented relation
`Fvco = Fin * CLKFB_DIV * Dfeedback / CLKI_DIV`; their CLKOP feedback divider
is 6, giving 600 MHz VCO and 100 MHz CLKOP. Zero versus 12/8/1/2 analog words
does not alter the measured outcomes across A–H: A/B, C/D and D/E isolate
the analog omission and the return connection. This qualifies those two
analog profiles, rather than arbitrary analog values or startup conditions.

There are **two** measured selector unions, not one shared encoding:
INT_OP/fabric is `{F1B1,F2B1,F3B1,F4B1}` (C/D/F/G);
INT_OS/fabric additionally asserts F5B1 (H). Neither equals a listed internal
feedback enum. Both require the new VCO-feedback rule. The original proposed
planner classification, refusal boundary and kernel interface are in
[EMULATOR_RULE.md](EMULATOR_RULE.md). That rule is now implemented in PLL
planning; saved original CLI evidence remains explicitly pre-fix.

The supplied run_2 files were audited for personal paths; none were present,
so no normalization or raw-byte changes were necessary.
[run_2_audit.json](run_2_audit.json) records the input hashes and audit result.
To reanalyse offline:

```sh
python3 -B docs/inventory/LFE5U-85F/hc08_pll_analog/analyse.py \
  docs/inventory/LFE5U-85F/hc08_pll_analog/run_2/*.bin
```

## Builds and written settings

All use G2 at 25 MHz, CLKI_DIV=1, CLKFB_DIV=4, CLKOP_DIV=6,
CLKOP_CPHASE=2, CLKOP_FPHASE=0 (600 MHz nominal VCO, 100 MHz CLKOP).
Static controls and the CLKOP profile follow HY01; its unused CLKOS is omitted
in every build. Only the feedback selection/connection and analog attributes
vary. Optional E is included.

| Build | FEEDBK_PATH in routed config | Source CLKFB return | ICP_CURRENT | LPF_RESISTOR | MFG_ENABLE_FILTEROPAMP | MFG_GMCREF_SEL | Route/pack |
|---|---|---|---:|---:|---:|---:|---|
| A, control | CLKOP | CLKOP through fabric | 12 | 8 | 1 | 2 | PASS |
| B, omit analog | CLKOP | CLKOP through fabric | 0 | 0 | 0 | 0 | PASS |
| C, INT_OP + analog | INT_OP | CLKOP through fabric | 12 | 8 | 1 | 2 | PASS |
| D, HI12 failure profile | INT_OP | CLKOP through fabric | 0 | 0 | 0 | 0 | PASS |
| E, HC07 return, omit analog | INT_OP | CLKINTFB connected to CLKFB on `fb` | 0 | 0 | 0 | 0 | PASS |

A/C specify the four attributes as strings; B/D/E omit them entirely rather
than explicitly requesting zero. `build_checks.json` retains the literal routed
enum/word lines, feedback arcs, PLL BEL, and packed-bitstream SHA256. The builder
also checks the routed JSON connections: A–D have CLKFB=CLKOP and no CLKINTFB
return; E has CLKFB=CLKINTFB. All inferred FFs have GSR disabled, so observation
and UART startup do not wait for a PLL-dependent global reset.

## UART and startup retention

L4 (`ftdi_rxd`) transmits 115200 8N1 (25 MHz / 217 = 115207.37 baud).
Every 250000 pin cycles (10 ms), the pin domain snapshots a synchronized LOCK,
its own 32-bit free-running pin count, and the synchronized 32-bit count of
CLKOP rising edges. Only that last counter and its registered Gray encoding
use CLKOP. Two FF stages cross each Gray bit and LOCK into the pin domain.
No counter, snapshot, RAM access, or UART state waits for LOCK or CLKOP edges.
The first snapshot is at pin count zero; synchronized signals initially read
zero even if raw LOCK is already high.

Each 24-byte line is `HC08L` or `HC08S`, one LOCK hex digit (0 or 1), eight pin
count hex digits, eight CLKOP count hex digits, CR LF. For example,
`HC08L10003D090000F4233` means LOCK=1, 250000 pin ticks, 999987 observed CLKOP
edges. `L` is live; `S` is a replay of an immutable startup sample. Times are
configuration-relative pin ticks, never host arrival times.

The first 201 snapshots, t=0,10,...,2000 ms, remain in two EBRs. From t=2010 ms,
each tick sends its live line and one startup line; the startup replay wraps
and repeats forever. Two lines take about 4.17 ms, within the 10 ms tick. Thus
opening serial after SRAM loading can still recover all of the first two
seconds, including a slow lock. Eight seconds of capture covers a complete
replay even when opening is delayed. The runner requires all 201 startup
timestamps, keeps partial/raw data on failure, and can be rerun with a longer
window if necessary. It never flushes the input buffer.

`analyse.py` prints each load's recovered startup samples in time order and
live samples in arrival order, including LOCK, delta_pin, delta_CLKOP, ratio,
and MHz. Its `S` table reconstructs startup from either live or replay records.
Repeated timestamps must agree. For adjacent samples:

```text
ratio = (CLKOP_count_new - CLKOP_count_old) mod 2^32
        / ((pin_count_new - pin_count_old) mod 2^32)
CLKOP_MHz = 25 * ratio
```

Nominal ratio is 4 and delta_CLKOP is 1000000 per 250000 pin ticks. A zero
delta indicates no observed edges during that window; another ratio measures
another rate, independently of LOCK. The Gray synchronizer adds a few output
edges of endpoint uncertainty; the first interval also includes clock startup
and synchronization latency. Absolute frequency inherits the 25 MHz reference's
accuracy. LOCK is sampled at 10 ms resolution, so briefer transitions can be
missed. Registered Gray transitions require inter-bit route skew shorter than
an output period; routed crossing delays are retained in `timing.txt` and are
below 10 ns for these nominal 100 MHz builds. These digital observations do
not measure analog stability or jitter. Use short captures (8 s recommended):
the pin timestamp wraps after about 171.8 s, and differentials cannot recover
multiple unseen counter wraps across long gaps.

## Exact build and emulator commands

The current planner interprets the complete joint FEEDBK_PATH/CLKFB pattern.
A/B/E retain 100 MHz CLKOP (10 ns); C/D/F/G/H now use raw-VCO feedback and
produce 16.666667 MHz (60 ns), with LOCK high under the existing configured
startup convention. INT_OS H's former ambiguity no longer falls back silently
to external feedback. Unmeasured unions and unsupported raw-VCO return/control
combinations are refused with FEEDBK_PATH, CLKFB source and HC08 in the message.
Both execution engines consume the corrected VCO ratio; `fast.rs` is unchanged.

`tests/pll_feedback_native.rs` builds packed A–H, checks every CLKOP period
and compares full event/fast traces at 1 ns resolution. It also checks the
other raw-VCO output dividers, rejection cases, and canonical INT_OS3 (which
sets the mux bits too but must retain output feedback). Run it on rserv with:

```sh
cargo test --release -p ecp5_emu --test pll_feedback_native -- --include-ignored
```

The tables and `emulator/` / `followup/emulator/` captures below preserve the
original, wrong 100 MHz prediction for C/D/F/G/H; rerunning the commands uses
the corrected behavior. Implementation validation is recorded in
`implementation_validation.txt`.

Run from the repository root. Build products, Cargo output and support-test
products stay in `mktemp` scratch outside the repository. These commands do not
access a board. Tools must be on PATH; see `TOOLS.txt` for the checked versions.

```sh
probe=docs/inventory/LFE5U-85F/hc08_pll_analog
scratch=$(mktemp -d "${TMPDIR:-/tmp}/hc08.XXXXXX")
python3 -B "$probe/build.py" "$scratch/build"
CARGO_TARGET_DIR="$scratch/cargo" cargo build --release --bin ecp5_emu
python3 -B "$probe/check_emulator.py" "$scratch/build" \
  --emulator "$scratch/cargo/release/ecp5_emu"
python3 -B "$probe/check_support.py" "$scratch/support"
grep -E '^(enum: FEEDBK_PATH|word: (ICP_CURRENT|LPF_RESISTOR|MFG_ENABLE_FILTEROPAMP|MFG_GMCREF_SEL)) ' \
  "$scratch/build/"*/top.config
```

For each of A–E, `build.py` generates `top.v`, copies `reporter.v` and
`hc08.lpf` as `top.lpf`, then executes exactly these commands inside that
profile's scratch directory:

```sh
yosys -Q -q -p 'read_verilog top.v reporter.v; synth_ecp5 -top top; setparam -set GSR "DISABLED" t:TRELLIS_FF; write_json top.json'
nextpnr-ecp5 --85k --package CABGA381 --seed 1 --json top.json \
  --lpf top.lpf --textcfg top.config --write routed.json
ecppack top.config top.bit
```

`check_emulator.py` runs each **packed bitstream** with configured startup,
observed scope, and G2 driven at 40 ns. Its event-kernel run drives G2's value
at every 1 ns timestamp (the value changes only every 20 ns), saves the plan,
and measures LOCK/CLKOP in VCD. Sampling only at G2 transitions would alias
the 100 MHz output. A second CLI run uses `--clock pin:G2 --period-ns 40
--cycles 800000 --fast --console-out pin:L4` for 32 ms of UART, verifying the
actual reporter/count path. The JSON, VCD, stdout/stderr and bytes remain in
scratch; compact evidence is retained here.

Original pre-fix CLI observations:

| Build | Planner | Raw LOCK | First CLKOP rise | CLKOP rising-edge period | UART at 20–30 ms |
|---|---|---|---|---|---|
| A | accepts | 1 at 0 ns | 70 ns | 10 ns | ratio 4, 100 MHz |
| B | accepts | 1 at 0 ns | 70 ns | 10 ns | ratio 4, 100 MHz |
| C | accepts | 1 at 0 ns | 70 ns | 10 ns | ratio 4, 100 MHz |
| D | accepts | 1 at 0 ns | 70 ns | 10 ns | ratio 4, 100 MHz |
| E | accepts | 1 at 0 ns | 70 ns | 10 ns | ratio 4, 100 MHz |

These are the pre-fix emulator's answers, not hardware findings. Its INT_*
planner substituted internal feedback for C/D. The corrected planner now
produces 60 ns for C/D; A/B/E and the configured LOCK convention are unchanged.

A further A run simulated 101000000 pin cycles (4.04 s). It produced 404 live
and 203 replay records. All 201 stored startup timestamps matched the original
live samples exactly. Dropping all initial live startup records still recovered
the complete 0–2000 ms set from replay alone. See
`emulator/A/startup_check.json` and its raw bytes, log and analysis. Reproduce
this longer capture from the repository root after the build above:

```sh
"$scratch/cargo/release/ecp5_emu" emulate "$scratch/build/A/top.bit" \
  --names "$scratch/build/A/routed.json" --startup configured --scope observed \
  --clock pin:G2 --period-ns 40 --cycles 101000000 --fast --console-out pin:L4 \
  --probe net:locked --probe net:clkop \
  > "$scratch/build/A/emulator.startup.uart.bin" \
  2> "$scratch/build/A/emulator.startup.log"
python3 -B "$probe/analyse.py" "$scratch/build/A/emulator.startup.uart.bin" \
  > "$scratch/build/A/emulator.startup.analysis.txt"
```

## Original A–E run commands

These board commands were not executed during preparation or analysis.
Install pyserial in the Python environment and set `SERIAL_PORT` to the
board's explicit serial port.

```sh
python3 -B "$probe/run.py" "$scratch/build" "$SERIAL_PORT" \
  --output "$probe/run_1" --loads 3 --seconds 8
python3 -B "$probe/analyse.py" "$probe/run_1/"*.bin \
  > "$probe/run_1/reanalysis.txt"
```

For every profile and load, `run.py` executes
`openFPGALoader -b ulx3s <bitstream>` with no flash option, waits for success,
then opens the named serial port. It saves exact raw bytes (including empty
captures), loader output, and per-load analysis, continues through failed
loads, and exits nonzero for loader/serial errors or incomplete startup data.
LOCK low or a stopped CLKOP is a valid experimental result, not a capture error.

Both runs' final interpretation is above; the following paper audit and
prediction table preserve the reasoning and pre-board alternatives.

## Paper check: the union is not INT_OS3

[paper_checks.json](followup/paper_checks.json) records the database rows,
file hashes, routed words and actual packed CRAM checks. C/D were rebuilt from
the unchanged original profiles: their packed SHA256 values exactly match
`build_checks.json`, so this is also a check on the original routed designs.
The relevant tile is `MIB_R4C0:PLL0_UL`, whose Trellis database file is
`ECP5/tiledata/PLL0_UL/bits.db`. `F#B#` coordinates below are local to that tile.
`-` is an empty set of asserted bits. The field's complete bit mask is
F1B1,F2B1,F3B1,F4B1,F5B1,F6B1; unlisted bits in this mask are zero for an
exact encoding comparison.

```text
.mux E1_CLKFB
E1_CLKINTFB -
E1_JCLKFB1 F1B1
E1_JCLKFB2 F2B1
E1_JCLKFB3 F1B1 F2B1

.config_enum FEEDBK_PATH USERCLOCK
CLKOP -
CLKOS -
CLKOS2 -
CLKOS3 -
INT_OP F2B1 F3B1 F4B1
INT_OS F1B1 F3B1 F4B1 F5B1
INT_OS2 F3B1 F4B1 F6B1
INT_OS3 F1B1 F2B1 F4B1 F5B1 F6B1
USERCLOCK -
```

C and D both write `FEEDBK_PATH INT_OP` and
`arc: E1_CLKFB E1_JCLKFB3`. Their bitwise union asserts
**F1B1 F2B1 F3B1 F4B1**, with F5B1=0 and F6B1=0. It equals **none** of the
listed FEEDBK_PATH encodings. Relative to INT_OS3 it has an extra F3B1 and
lacks F5B1/F6B1. Both loading the routed config through Trellis and reading
the packed bitstream's CRAM confirm precisely this union. Textual enum
decoding can still report INT_OP; that is not equality with the full bit mask
or evidence about the hardware's interpretation of the combined selector.

| C and D routed word | Written (divider minus one) | Physical divider |
|---|---|---:|
| CLKI_DIV | 0000000 | 1 |
| CLKFB_DIV | 0000011 | 4 |
| CLKOP_DIV | 0000101 | 6 |
| CLKOS_DIV | 0000111 | 8 |
| CLKOS2_DIV | 0000111 | 8 |
| CLKOS3_DIV | 0000111 | 8 |

The actual nextpnr-written unused dividers are 8, regardless of what the
database's all-zero word default would mean. Even if the union had equalled
INT_OS3, this design would predict `25 * 4 * 8 / 6 = 133.333 MHz`, not the
observed 16.667 MHz. The proposed exact INT_OS3-alias explanation therefore
fails both the encoding check and the routed-divider check. Raw-VCO feedback
was consistent with run_1 and is now supported by run_2's divider-independence
test. The database alone did not assign the unlisted selector a physical
meaning; the final rule comes from the board results.

INT_OS also overlaps this fabric arc: its union is
F1B1,F2B1,F3B1,F4B1,F5B1 (F6B1=0), again no exact enum match. INT_OS2 gives
F1B1,F2B1,F3B1,F4B1,F6B1, also unlisted. INT_OS3 already contains both fabric
arc bits and remains INT_OS3. H therefore uses INT_OS as a second distinct
unlisted union, rather than claiming it is a known alternate selector.

## F/G/H: pre-board predictions and emulator evidence

F changes only C's CLKOS3_DIV to 3; G changes only that divider to 2. H is F
with FEEDBK_PATH=INT_OS, keeping CLKOS3_DIV=3 and the fabric CLKOP return.
All retain analog attributes 12/8/1/2, CLKFB_DIV=4, CLKOP_DIV=6, the identical
reporter and the remaining default dividers. H's CLKOS_DIV is still 8.

| Build | Source selector | CLKOS3_DIV | If feedback were CLKOS3's divider output | If feedback is raw VCO | Pre-fix emulator | Corrected kernel/fast |
|---|---|---:|---:|---:|---|---|
| F | INT_OP | 3 | 50.000 MHz | 16.667 MHz | accepts, 100 MHz, LOCK=1 at 0 ns | 16.666667 MHz, LOCK=1 |
| G | INT_OP | 2 | 33.333 MHz | 16.667 MHz | accepts, 100 MHz, LOCK=1 at 0 ns | 16.666667 MHz, LOCK=1 |
| H | INT_OS | 3 | 50.000 MHz | 16.667 MHz | accepts, 100 MHz, LOCK=1 at 0 ns | 16.666667 MHz, LOCK=1 |

The CLKOS3 column is the requested hypothesis's **conditional** prediction
`25 * 4 * CLKOS3_DIV / 6`; the database has already rejected its literal
encoding premise, including for H. No known FEEDBK_PATH value is assigned to
either unlisted union. These probes still distinguish dependence on CLKOS3_DIV
from divider-independent raw-VCO behavior. For H, ideal intended INT_OS
feedback through CLKOS_DIV=8 would instead predict 133.333 MHz. None of these
conditional frequency calculations establishes acquisition or LOCK behavior
for the undocumented combinations; retain LOCK and startup samples on every
load, including if a changed divider does not lock.

Every F/G/H build routes and packs on this Mac with both timing targets met.
The pre-fix emulator accepted all three, emitted a 10 ns CLKOP period, and its
UART counts 1000000 CLKOP edges per 250000 pin ticks after startup. F/G decode
INT_OP and substitute internal CLKOP feedback. H's plan records an ambiguous
FEEDBK_PATH with candidates INT_OP and INT_OS: `pll_setting` raises an error,
but `execute_pll`'s match falls through to external feedback on that error,
traced the fabric CLKOP return, and still predicted 100 MHz. This was an observed
additional qualification issue, fixed by the exact joint-bit classification.
See `followup/emulator_checks.json` and `followup/emulator/` for evidence.

Exact offline commands, from the repository root (Trellis/pytrellis paths shown
are the system installation on this Mac):

```sh
probe=docs/inventory/LFE5U-85F/hc08_pll_analog
scratch=$(mktemp -d "${TMPDIR:-/tmp}/hc08-followup.XXXXXX")
python3 -B "$probe/build.py" "$scratch/build" --profiles C D F G H
CARGO_TARGET_DIR="$scratch/cargo" cargo build --release --bin ecp5_emu
python3 -B "$probe/paper_check.py" "$scratch/build" \
  --db /opt/homebrew/share/trellis/database \
  --pytrellis-dir /opt/homebrew/lib/trellis \
  --output "$scratch/paper_checks.json"
python3 -B "$probe/check_emulator.py" "$scratch/build" \
  --emulator "$scratch/cargo/release/ecp5_emu" --profiles F G H
```

The expanded Yosys/nextpnr/ecppack commands are unchanged from the earlier
build section, including `--85k --package CABGA381 --seed 1`. The original
scripts continue to default to A–E; F/G/H require explicit `--profiles`.
`followup/build_checks.json` and `followup/timing.txt` retain the routed
settings, packed hashes and final timing, and `followup/TOOLS.txt` the versions.

These run_2 board commands were not executed during analysis.
Install pyserial and set `SERIAL_PORT`
when reproducing the run; use a new output directory to preserve run_2:

```sh
python3 -B "$probe/run.py" "$scratch/build" "$SERIAL_PORT" \
  --profiles F G H --output "$probe/run_2_repeat" --loads 3 --seconds 8
python3 -B "$probe/analyse.py" "$probe/run_2_repeat/"*.bin \
  > "$probe/run_2_repeat/reanalysis.txt"
```

Each load remains `openFPGALoader -b ulx3s <bitstream>` (SRAM only), followed
by opening serial. Run_2 now answers the discriminator: F/G remain at
16.667 MHz despite the changed divider, and H preserves that behavior with
the INT_OS union. Use the final Reading and proposed rule above.

## Files

`gen.py`, `reporter.v`, and `hc08.lpf` define the series. `build.py` validates
written settings and packing. `run.py` and `analyse.py` capture and interpret
each load. `check_emulator.py` records actual CLI predictions. `check_support.py`
and `fault_tb.v` exercise heartbeat, counters and startup replay with stopped,
unlocked, delayed-lock and wrong-frequency clock inputs, plus capture failures
and byte preservation. The fault bench shortens only the retained startup set
to four snapshots, preserving the real 10 ms interval and UART baud; these
fault tests do not model physical PLL behavior.
`build_checks.json`, `pll_settings.txt`, `timing.txt`, `TOOLS.txt`,
`emulator_checks.json`, `emulator/`, and `validation.txt` retain preparation
evidence. Bitstreams, full routed configs/JSON and full CLI plans are regenerated
in scratch, with their packed hashes recorded for comparison.
`read_run.py` and `run_1_reading.json` summarize the supplied board run.
`paper_check.py` and `followup/` retain the bit-union audit and F/G/H evidence.
`run_2_reading.json` and `EMULATOR_RULE.md` record the final run_2 Reading and
the feedback rule now implemented in the planner. `implementation_validation.txt`
records the implementation checks; original pre-fix captures remain unchanged.
