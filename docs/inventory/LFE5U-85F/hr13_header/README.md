# HR13: the header inputs and four driven pads on the board

[header_probe](../../../../examples/ulx3s/header_probe/top.v) reads all 56 J1/J2
signals as inputs with alternating pulls and reports four internal PWM period
counters. [header_probe/driven](../../../../examples/ulx3s/header_probe/driven/top.v)
drives **GP0..GP3 only, at DRIVE=4**, and reads them back through explicit ECP5
BB input buffers. Its other 52 header signals remain inputs with pulls.
[PERIPHERALS_HEADER.md](../../../PERIPHERALS_HEADER.md) describes the mapping,
measurement windows and model.

## Permission and setup

J1/J2 were empty on 2026-11-17; only the power cable was plugged into the board.
The driven variant was restricted to GP0..GP3, DRIVE=4, SRAM loads only.
These are J1 pads **6/8/10/12**,
U1 balls **B11/A10/A9/B9**. The PCB connects only J1 and U1 on those four nets;
the plain audit in [tests/header.rs](../../../../tests/header.rs) checks the
component endpoints in
[assets/ulx3s/ulx3s_v3.0.8.kicad_pcb](../../../../assets/ulx3s/ulx3s_v3.0.8.kicad_pcb)
(title block **v3.0.7**). ESP32-shared GP/GN11..13 and ADC-shared GP/GN14..17
were never driven by either design. The inputs-only variant remains the default;
running driven/ requires the headers to stay empty.

- **Board:** ULX3S 85K v3.0.8, under the shared board lock on 2026-11-17.
- **Runner:** [../hr04_hdmi_oled/run.py](../hr04_hdmi_oled/run.py), prefix `HEADER`,
  **two SRAM loads of 3 s per design**. Inputs-only loads began at
  `2026-11-17T19:50:15Z` and `2026-11-17T19:50:24Z`; driven loads at
  `2026-11-17T19:50:32Z` and `2026-11-17T19:50:41Z`.
- **Builds:** on the Mac with `examples/ulx3s/build.sh`, nextpnr seed 1.
  [header_probe.config](header_probe.config) copies
  `target/reference/header_probe/top.config`; maximum frequency **96.18 MHz**
  against **25 MHz**, SHA-256
  `de2f65fd35a8172ed9044e7c15b5ba185852b133cdc57016f1d202ec461bdb4c`.
  [header_probe_driven.config](header_probe_driven.config) copies
  `target/reference/header_probe_driven/top.config`; maximum **83.06 MHz**
  against **25 MHz**, SHA-256
  `99b38b031c68b330e0ca093c48a312f6c5bc69a0ad9c2396a11501445ebc2d9d`.
  The loaded bits are not stored; their hashes are in [SHA256SUMS](SHA256SUMS).
- **Records:** [observations.log](observations.log) contains
  `out/header/board_inputs.log` followed by `out/header/board_driven.log`, each
  verbatim under a `# design …` header. These captures contain no private data.
  [TOOLS.txt](TOOLS.txt) and [SHA256SUMS](SHA256SUMS) identify tools and files.

## Observations

Each load captured three complete lines: **six per design, twelve in all**.
Both loads of each design printed the same three-line sequence.

Inputs only:

```text
HEADER 000003E9 000007D2 0000138D 0000271A 55555555555555 E1
HEADER 000007D1 00000FA2 00002715 00004E2A 55555555555555 E1
HEADER 00000BB9 00001772 00003A9D 0000753A 55555555555555 E1
```

Driven:

```text
HEADER 000003E8 000007D0 00001388 00002710 P 000003E8/005F5E10 000007D0/00BEBC20 00001388/011E1A30 00002710/002625A0 55555555555555 E1
HEADER 000007D0 00000FA0 00002710 00004E20 P 000003E8/005F5E10 000007D0/00BEBC20 00001388/011E1A30 00002710/002625A0 55555555555555 E1
HEADER 00000BB8 00001770 00003A98 00007530 P 000003E8/005F5E10 000007D0/00BEBC20 00001388/011E1A30 00002710/002625A0 55555555555555 E1
```

The first four fields are cumulative internal period counts. The driven **P**
section is sampled through the **GP0..GP3 pads and BB.O input buffers**, not
from the internal PWM wires. Each pair is rising edges / high cycles over a
**25,000,000-cycle second**:

| Pad | Rising edges / second | High cycles / second | Frequency | Duty |
|---|---:|---:|---:|---:|
| GP0 | 1000 | 6250000 | 1000 Hz | 25% |
| GP1 | 2000 | 12500000 | 2000 Hz | 50% |
| GP2 | 5000 | 18750000 | 5000 Hz | 75% |
| GP3 | 10000 | 2500000 | 10000 Hz | 10% |

Every P section has those values, across all three seconds in both loads.
The input word **55555555555555** is the alternating UP/DOWN pull pattern,
as expected for the empty headers. In the inputs-only variant, all 56 inputs
read as configured. This includes ESP32-shared GP/GN11..13: **the ESP32 was
not driving them at that moment**.

## The emulator comparison

The operator ran both configurations through the CLI on rserv at branch HEAD
`9a408a11b1e645d9c0f20449224f43f9db0d1398`, using **80,000,000 cycles**
(3.2 s of emulated time). For each configuration and its corresponding routed
names file:

```sh
ecp5_emu emulate target/reference/header_probe_driven/top.config \
  --names target/reference/header_probe_driven/top_routed.json \
  --fast --peripherals --startup configured --scope observed \
  --clock net:clk_25mhz --period-ns 40 --cycles 80000000 \
  --console-out net:ftdi_rxd
```

Use `target/reference/header_probe/` for the inputs-only run.

| Design | Emulator lines compared | Board lines, each load | Result |
|---|---|---|---|
| Inputs only | 2–4 | 1–3 | **PASS**, byte-identical, 6 of 6 board lines |
| Driven | 1–3 | 1–3 | **PASS**, byte-identical, 6 of 6 board lines |

The inputs-only emulator first prints its startup line near the start of the
run (the design's initial 1 ms report):

```text
HEADER 00000001 00000002 00000005 0000000A 55555555555555 E1
```

**The board capture misses this startup line because run.py opens the serial
port after the loader finishes.** The emulator's next three lines are the
board's three lines. This capture offset is **not a mismatch**. The driven
variant waits a full second before its first report, so its first three
emulator lines directly match the board's.

The driven CLI report was:

```text
peripheral: header GP0=1000.000Hz/25.000% GP1=2000.000Hz/50.000% GP2=5000.000Hz/75.000% GP3=10000.000Hz/10.000%
```

## Emulator regression check

[tests/header.rs](../../../../tests/header.rs) runs both designs for
80,000,000 cycles through the fast CLI and answering-peripheral trace path.
It checks inputs-only lines **2–4** and driven lines **1–3** against this
record's observations, verifies that the two board loads agree, and retains
checks of the startup line, pad counts, frequency/duty and input overrides.

```sh
cargo fmt --all -- --check
cargo test --release -p ecp5_emu --test header -- --ignored
```

Checked on **2026-11-17** on rserv at that HEAD with the record and test
changes in the working tree: `cargo fmt --all -- --check` passed; the ignored
header suite at default parallelism passed **3 tests, 0 failed**. Both
80,000,000-cycle comparisons passed, including inputs-only lines 2–4 and driven
lines 1–3 against all twelve stored board lines through the identical loads.

## What this check establishes and does not

The **GP0..GP3 drive and pad readback path**, and the design's reported logic,
match the emulator. The pulls on the **56 inputs** read as configured in the
inputs-only design. The driven check uses only the four private nets permitted
by the empty-header setup, leaving ESP32- and ADC-shared nets as inputs.

It does **not** establish drive strength, voltages or behavior under load, or
the **5 V pins**. DRIVE=4 is the configured setting, not a measured current or
loaded output characteristic. These unloaded digital observations do not
qualify driving other pins or using a board with attached header circuits.
