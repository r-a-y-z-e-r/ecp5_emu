# HI13: enabled ECLKSYNCB with pre-running and delayed input clocks

**run_1 measured P=0, G=4 missing ECLKI periods in all three SRAM loads.**
The ULX3S 85K was loaded three times, leaving flash untouched.
Raw captures, runner output and analysis are retained in [run_1](run_1/).
Emulator implementation used those records without further board access;
no commit was made.

One bitstream compares the [HI12](../hi12_eclksync_none/README.md) pin-clock
control with an input that starts after fabric counting. Optional lane F is
omitted. Both explicit ECLKSYNCB cells have STOP=0; each feeds CLKDIVF /2 with
RST=0, ALIGNWD=0 and written GSR=DISABLED. There is no PLL, GSR or SGSR primitive.
Every fabric FF has written GSR=DISABLED and all counters start at RESET.

| Lane | ECLKI | Routed ECLKSYNCB | Routed CLKDIVF |
|---|---|---|---|
| P | raw 25 MHz G2 pin, running during configuration | X0/Y46/ECLKSYNC1_BK7, MODE=ECLKSYNCB | X0/Y46/CLKDIV1, DIV=2.0 |
| G | G2 through dynamic DCCA | X0/Y47/ECLKSYNC0_BK6, MODE=ECLKSYNCB | X0/Y46/CLKDIV0, DIV=2.0 |

These are side-bank lanes in separate banks, with one synchronizer used per
bank and distinct divider indices. Dynamic DCCA is X3/Y46/LDCC6,
`MIB_R46C3:LMID_0 DCC_L6.MODE=DCCA`. An always-enabled support DCCA is
X3/Y46/LDCC1, `DCC_L1.MODE=DCCA`. Lane P's hook retains HI12's direct
`G_JPCLKT61 -> W2_JECLKI1` entry; lane G enters through the routed CIB path.
`--allow-fabric-eclk` permits that delayed input. No configuration is patched.
The actual extracted settings are in [validation.txt](validation.txt).

`I` counts pin rises beginning at fabric activation. This is the observable
"from configuration" reference used by HI12, not a count of oscillator edges
during configuration. CE starts at zero and rises on the falling edge following
the 64th observed pin rise; its recorded timestamp is `CE_at=64`. It stays high.
`A` counts pin rises after CE, and `G_in` independently counts DCCA output rises.
Require `A=I-CE_at=G_in` before interpreting G. Thus a DCCA startup discrepancy
cannot silently become an ECLKSYNC deficit. Each lane counts both CDIVX polarities.

The midpoint is I=524288 (20.97152 ms), and freeze is I=1048576 (41.94304 ms).
Both controls act on the pin-clock falling edge, nominally 20 ns after all
counted rises and /2 divider transitions. Counters are copied after 32 more
support-clock rises at freeze. The midpoint is captured directly in the low
half-cycle. The routed timing summary is retained in [timing.txt](timing.txt);
cross-clock delays are tool estimates, not board-calibrated startup skew.
The counters do not wait for test-clock activity, so stopped lanes still report.

UART on **ftdi_rxd L4**, 115200 baud, 8N1, uses only the support pin clock.
Before completion it repeats `HBD FF IIIIIIII\r\n`: two hex flag digits (bit 0
CE, bit 1 freeze, bit 2 finished) and an independent, never-frozen live pin count.
After completion it repeats `HCD ` plus 128 uppercase hex digits and CRLF.
The payload is midpoint then final; each snapshot has eight 32-bit hex words:
`I A CE_at G_in P_r P_f G_r G_f`, most significant digit first.

For /2, each passed ECLKI rising edge makes one CDIVX transition. Therefore
`D_P=I-P_r-P_f` and `D_G=G_in-G_r-G_f` count missing input periods without
assuming which CDIVX polarity came first. A constant deficit at both snapshots
identifies startup loss; a changing deficit, bad edge balance, incorrect freeze
boundary or failed DCCA qualification makes the measurement suspect.

## Reading

[run_1/hi13.log](run_1/hi13.log) and [run_1/run_output.txt](run_1/run_output.txt)
record three successful SRAM loads on 2026-11-17. The raw files total 69,365
bytes and contain 515 complete records (171, 172, 172), with one identical
immutable record in each load and across all loads. No heartbeats were captured;
opening serial after loading can miss the early heartbeat period.
[run_1/analysis.json](run_1/analysis.json) preserves analyze.py's per-load output
(three consecutive JSON objects). All loads give:

| Snapshot | I | A = G_in | CE_at | P rising / falling | G rising / falling | D_P / D_G |
|---|---:|---:|---:|---:|---:|---:|
| Midpoint | 524288 | 524224 | 64 | 262144 / 262144 | 262110 / 262110 | 0 / 4 |
| Frozen | 1048576 | 1048512 | 64 | 524288 / 524288 | 524254 / 524254 | 0 / 4 |

G_in=A=I-64 independently qualifies the DCCA: the four-period loss belongs to
the enabled ECLKSYNCB chain after ECLKI starts toggling. P reproduces HI12
sync_2's zero deficit. G reproduces HI04's usual four-edge start for a clock
that begins after wake-up. Neither deficit grows between snapshots.

**Implemented rule (HI04 + HI12 + HI13):** STOP still passes four stages on
ECLKI falling edges. At configuration, initialize all four stages, including
the gate output, to 0 when the selected ECLKI path comes from a primary clock
input pin through routes/buffers and DCC bypasses or DCCA with constant CE=1.
Initialize them to 1 for fabric logic or DCC/DCCA with CE other than constant 1.
With STOP=0 the latter swallows four ECLKI rising edges and passes the fifth.
This represents a running pin's stages already cleared at fabric activation;
it does not bypass runtime STOP.

**Unmeasured:** PLL and OSCG output activity during configuration. Both retain
the HI04 rule (stages at 1), even if a PLL's reference comes from a primary pin.
The matched experiment does not directly observe configuration-time hard-block
state, exclude pre-activation DCCA transients, distinguish an externally withheld
pin clock, settle HI04's occasional fifth missing edge, or establish exact PUR
and fabric activation times. The constant-enabled DCCA and other placements are
source-path model extensions of this measurement.

`tests/eclksync_start.rs` builds and replays the packed HI13 design and HI12
sync_2 with configured/observed startup. It compares complete watched transition
traces between the event kernel and fast mode through both snapshots, checks
the counts above against all three raw loads, and qualifies G_in independently.
`tests/eclksync_none.rs` covers source classification and retains MODE NONE
qualification. Fast mode takes initial stage values from the kernel; its source
files need no change. HI04 native board-line tests check the fabric start rule.
`check_emulator.py` now expects the board's P=0/G=4 counts in actual CLI replay.
The complete-design CLI replay at 1,600,000 pin cycles passes with the original
board bitstream (SHA-256 matching SHA256SUMS.build), as well as a fresh rserv
build. [emulator_checks_corrected.json](emulator_checks_corrected.json) records
the original-bitstream replay; [emulator_checks_corrected_rserv.json](emulator_checks_corrected_rserv.json)
records the rebuilt image with its own hash. Each actual CLI run emits one
complete record and 29 early heartbeats. These are working-tree binaries,
including concurrent planner work, not clean-commit baselines. The raw board
files and historical emulator baseline are unchanged.

See [VALIDATION.md](VALIDATION.md) for execution checks and commands.

### Historical emulator baseline

Before this change, the actual CLI replay (`--startup configured --scope
observed`) predicted four missing input periods for both lanes: P rising/falling
262142/262142 at midpoint and 524286/524286 at freeze; G already matched the
board. [emulator_checks.json](emulator_checks.json) and
[emulator.uart.bin](emulator.uart.bin) preserve that executed prediction,
not the corrected result. Replay used dev at 3ee3b75 with pre-existing source
modifications; the recorded binary hash is not a clean-commit baseline.
The baseline is retained unchanged so the correction remains reviewable.

From the repository root, with Yosys, nextpnr-ecp5, ecppack, Cargo, Python 3
and Icarus Verilog on PATH, exact build and hardware-free verification commands:

```sh
HI13=docs/inventory/LFE5U-85F/hi13_eclksync_start
BUILD=$(mktemp -d /private/tmp/hi13-build.XXXXXX)
python3 -B "$HI13/build.py" "$BUILD"
rg 'enum: (ECLKSYNC|CLKDIV|DCC)' "$BUILD/top.config"
cargo build --release --locked --bin ecp5_emu --target-dir "$BUILD/cargo"
python3 -B "$HI13/check_support.py"
python3 -B "$HI13/check_emulator.py" "$BUILD" --emulator "$BUILD/cargo/release/ecp5_emu"
python3 -B "$HI13/analyze.py" "$BUILD/emulator.uart.bin"
```

The builder rejects scratch inside the repository. It uses `--85k --package
CABGA381 --seed 1`, checks routed STOP constants, source connections, modes,
dividers, FF initialization and polarity, then writes hashes. All generated
Verilog/JSON/config/bitstream/tool logs remain in scratch. Tool versions for
this Mac are in [TOOLS.txt](TOOLS.txt); artifact hashes in [SHA256SUMS.build](SHA256SUMS.build).
Nine support checks cover raw bytes, load-before-serial ordering, failed/partial
loads, immutable records, deficit analysis and UART with absent lane clocks.

For a future board repeat, install PySerial, set `HI13_PORT` to the
board's UART device, and retain BUILD from above. Choose a fresh RUN directory:

```sh
HI13=docs/inventory/LFE5U-85F/hi13_eclksync_start
RUN=$(mktemp -d /private/tmp/hi13-run.XXXXXX)
python3 -B "$HI13/run.py" "$BUILD/top.bit" "$HI13_PORT" \
  --loads 3 --seconds 2 --output "$RUN/hi13.log"
python3 -B "$HI13/analyze.py" "$RUN"/*.bin
```

For each load, run.py executes exactly `openFPGALoader -b ulx3s BITSTREAM`
(SRAM), waits for success, then opens serial. It flushes every received byte to
`hi13.load-NNN.bin`, including partial/binary data and failed captures. Existing
outputs are refused. All loads are attempted, and missing or changing complete
records make capture fail. The log records timestamps, byte/record counts and
distinct records. Opening serial after loading may miss heartbeats; frozen
records repeat unchanged. Keep every raw file and analyze each load separately.

Files: `top.v`, `top.lpf`, `route_p.py` define the experiment; `build.py` builds
and checks it; `run.py` captures hardware; `analyze.py` interprets bytes/logs;
`check_emulator.py` replays packed bits; `check_support.py` checks diagnostics.
The remaining files retain build validation, timing, versions, hashes and the
historical emulator evidence; run_1 retains the board result.
