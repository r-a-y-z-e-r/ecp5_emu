# HI12 failed observer profile and emulator finding

Follow-up: the repaired [run_2 Reading](README.md#reading) now supplies successful
ULX3S records. The notes below retain the original failed-profile diagnosis and
repair checks; the old-profile emulator/board mismatch remains a separate finding.

All five original builds were loaded into ULX3S SRAM. Every
runner capture had no complete HC record, including the enabled-sync controls;
[failed_run_1](failed_run_1/) retains those stdout/stderr logs. A separate
three-second raw read also returned zero bytes after
loading none_default. These failures do not measure ECLKSYNC MODE NONE.

## Concrete defects and the limit of the evidence

The original observer used FEEDBK_PATH=INT_OP but connected CLKFB to CLKOP
through the fabric, with no CLKINTFB return. The routed config writes
`arc: E1_CLKFB E1_JCLKFB3`; the routed JSON has CLKFB and CLKOP on the same
net and no CLKINTFB connection. That mixes an internal-feedback selection
with the external-feedback wiring. HC07's working INT_* wiring connects
CLKINTFB back to CLKFB; HY01's working external-feedback wiring instead uses
FEEDBK_PATH=CLKOP with CLKFB=CLKOP.

The original design also omitted HY01's analog attributes. nextpnr's writer
uses zero when these attributes are absent (`ecp5/bitstream.cc`), and the
routed config contains:

| Setting | Failed profile | HY01 / revised profile |
|---|---|---|
| FEEDBK_PATH | INT_OP | CLKOP |
| ICP_CURRENT | 0 | 12 |
| LPF_RESISTOR | 0 | 8 |
| MFG_ENABLE_FILTEROPAMP | 0 | 1 |
| MFG_GMCREF_SEL | 0 | 2 |

The extracted settings are retained in [failed_run_1/pll_settings.txt](failed_run_1/pll_settings.txt)
and [pll_settings.txt](pll_settings.txt). The failed source and original build
checksums are retained beside the failed logs. Supplied traceback source paths
were normalized to repository-relative names; their error messages are unchanged.

The UART itself was conditional on `&ready`, which came only from `finished`.
Both snapshots and `finished` required edges of the observer and a filled
`lock_history`. An unlocked or absent observer therefore made the entire design
silent, including controls. This is the definite diagnostic-design failure.
The invalid/missing PLL profile is the concrete common defect found. Because
no original LOCK or observer samples reached the host, the evidence does not
isolate the feedback mismatch from the missing analog settings, or directly
prove LOCK stayed low on the board. The revision fixes both and exposes the
intermediate states before waiting for a complete measurement.

## The actual emulator disagrees with the silent board

The original packed **bitstreams**, not just behavioral Verilog stubs, were run
through the current CLI with `--startup configured --scope observed`, G2 at
40 ns, and the names from their routed JSON:

```sh
cargo build --release --bin ecp5_emu
python3 -B docs/inventory/LFE5U-85F/hi12_eclksync_none/check_emulator.py /private/tmp/r09-hi12-build --legacy --variants none_default sync_2
```

Both none_default and sync_2 plan resource `LFE5U-85F/R4C1/EHXPLL_UL` with
FEEDBK_PATH resolved as INT_OP. In the event kernel, LOCK is **1 at time zero**;
it does not wait to rise when the reference period becomes known. The output
starts after the reference period is learned. In fast mode both designs reach
finished=1 and emit a complete HC0/HC3 record within 1,600,000 pin cycles
(64 ms). The counts match the provisional NONE / HI04 control models. Each
64 ms capture contains one full HC record plus the beginning of another.
No HB records exist in the original design.

The exact rule is recorded in [failed_run_1/emulator_checks.json](failed_run_1/emulator_checks.json),
with UART bytes and CLI status logs alongside it. The relevant current code is:

- `src/compile/mod.rs:1527`: INT_OP selects internal CLKOP feedback.
- `src/compile/mod.rs:1571`: INT_* selects CLKINTFB instead of tracing the
  actual fabric CLKFB route.
- `src/compile/mod.rs:1637`: the CLKINTFB root and INT_* selection qualify
  the chosen output as feedback without requiring the source design's return.
- `src/compile/mod.rs:1674`: the resource rule says LOCK is high from the start,
  citing HC05. The planner's supported-setting checks do not qualify the analog
  attributes above.
- `src/kernel/pll.rs:156`: LOCK/INTLOCK drivers are created at Logic::One.

This is an emulator qualification finding for the failed INT_OP plus fabric
CLKOP-return profile with zero analog attributes under configured/observed
startup. Its prediction cannot validate that profile on hardware. No PLL
planner, kernel, fast-mode code, or R09 execution was changed in this repair.

The earlier Icarus preparation used an idealized PLL stub that always supplied
a clock and LOCK=1. It could check counts/serialization but could not detect
these feedback or analog defects. The revised support tests explicitly inject
LOCK=0 and a missing observer; they test UART independence, not analog PLL
operation.

## Revision checks

The revised PLL copies HY01's CLKOP feedback, divider/phase settings, static
control settings and analog attributes. Its unused CLKOS output is omitted.
All eleven designs route and pack; the builder now checks the PLL parameters,
feedback net and written analog words, along with the existing ECLK/FF checks.

Each revised board design runs through the actual emulator for 64 ms and
prints 29 HB records followed by a complete HC record; both count snapshots
fit the expected provisional or HI04 startup. See [emulator_checks.json](emulator_checks.json)
and [emulator_checks](emulator_checks/). These are emulator observations, not
new ULX3S measurements.

`check_support.py` runs seven tests without a board. Its four HDL fault cases
combine none_default/sync_2 with an unlocked PLL or no observer edges; each
prints two heartbeats with increasing pin counts and no false HC record.
The runner tests cover empty captures, partial/binary data, read and loader
failures, all requested loads, output preservation, and SRAM-load-before-serial
ordering. Raw bytes are retained exactly, including an empty file for a silent
load. The subsequent board run is analysed in README.md; these repair checks preceded it.
