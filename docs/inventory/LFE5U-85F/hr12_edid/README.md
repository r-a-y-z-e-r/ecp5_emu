# HR12: no responding monitor EDID on the board

[edid_read](../../../../examples/ulx3s/edid_read/top.v) reads the monitor's EDID
EEPROM at I2C address **0x50** on the ULX3S GPDI DDC bus, shared with the RTC.
It also drives the copied hdmi_bars DVI source. The reader can send only **A0,
00, A1**: write address, pointer zero, then read address, with a repeated START,
128 reads, ACKs and a final NACK, STOP. **There is no data-write state**; neither
the EEPROM nor the RTC is written. The plain Verilog audit in
[tests/rtc.rs](../../../../tests/rtc.rs)
`reference_verilog_can_only_send_address_pointer_address_then_read` covers it.
[PERIPHERALS_EDID.md](../../../PERIPHERALS_EDID.md) describes the model and method.

## Setup

- **Board:** ULX3S 85K v3.0.8, SRAM loads under the shared board lock on 2026-11-17.
- **Runner:** [../hr04_hdmi_oled/run.py](../hr04_hdmi_oled/run.py), prefix `EDID`,
  three loads of 3 s at `2026-11-17T18:41:16Z`, `2026-11-17T18:41:25Z` and
  `2026-11-17T18:41:34Z`.
- **Configuration:** [edid_read.config](edid_read.config), copied from
  `target/reference/edid_read/top.config`, built on the Mac with
  `examples/ulx3s/build.sh`, nextpnr seed 1. Maximum frequencies were **89.36 MHz**
  on clk_25mhz and **111.54 MHz** on the pixel clock, both against **25 MHz**
  constraints. The copied configuration's SHA-256 is
  `4166e128ecd23768751c1d2602153afdd6430ab8c10c643d16a4f2933b3e81a0`.
  The loaded `top.bit` is not stored; its hash is in [SHA256SUMS](SHA256SUMS).
- **Records:** [observations.log](observations.log), copied verbatim from
  `out/edid/board_edid.log`, including the runner's load headers;
  [TOOLS.txt](TOOLS.txt); [SHA256SUMS](SHA256SUMS). This capture contains only
  NACK reports, **no EDID bytes or serial numbers**, and may be stored here.

## Observations

Each load captured **three complete lines, nine in all**. Every line was:

```text
EDID nack
```

| Load | UTC start, 2026-11-17 | Lines | Every line |
|---|---|---:|---|
| 1 | 18:41:16Z | 3 | EDID nack |
| 2 | 18:41:25Z | 3 | EDID nack |
| 3 | 18:41:34Z | 3 | EDID nack |

**Nothing answered at 0x50**: no monitor, or no responding EDID, was on the
GPDI connector at **18:41Z**. The NACKs do not distinguish those possibilities.
They say **nothing about HR09's run at 07:43Z**, which was a different moment;
its monitor-presence question remains unrecorded.

## Connector power and pull-ups

The evidence is
[assets/ulx3s/ulx3s_v3.0.8.kicad_pcb](../../../../assets/ulx3s/ulx3s_v3.0.8.kicad_pcb),
the PCB file used by the 3D board, whose **title block says v3.0.7**:

- GPDI1 pin **18** is on **+5V**.
- U11 **PCA9306D** VREF2, pin **7**, is `/gpdi/VREF2`, fed from +5V through
  **R24, 100 kOhm** (pad 1 +5V, pad 2 /gpdi/VREF2).
- **R25, 4.7 kOhm**, pulls **GPDI_SCL** to +5V (pad 1 +5V, pad 2 GPDI_SCL).
- **R26, 4.7 kOhm**, pulls **GPDI_SDA** to +5V (pad 1 +5V, pad 2 GPDI_SDA).
  These reach GPDI1 pins **15/16**, respectively.

The board powers the connector side: a connected monitor's EDID EEPROM would
have been supplied through GPDI pin 18. These are PCB connectivity facts, not
voltage measurements from this run. The result is no responding EDID despite
that board-provided power path; it does not prove a physical monitor was absent.
The emulator does not model the level shifter's electrical behavior.

## The comparison

With **ECP5_EDID=off**, the emulator prints **EDID nack**, exactly the board's
line. [tests/edid.rs](../../../../tests/edid.rs)
`edid_off_nacks_while_the_source_still_drives_dvi` passes that setting to a child
CLI process and checks the line, disconnected status and NACK count. The companion
`edid_read_prints_the_synthetic_base_block` checks the default fictional **XEM**
block with **sum ok**. That block is the model's fixture; **it was not compared
with any monitor on this board**. No new cargo run was made to write this record.

## A later monitor read

No EDID was read here, so the **real-EDID image replay comparison does not apply**.
The existing `the_board_read_reproduces_in_the_emulator` test also understands
NACK-only captures, but that path cannot validate any EEPROM contents.

**Next step:** connect a monitor, record the connection and power state, and
repeat the same read-only SRAM check. Keep its raw UART log as
`out/edid/board_edid.log`. The ignored replay test parses its first complete,
indented EDID report, supplies those bytes to the emulator through a temporary
image in out/edid, and compares the printed bytes and checksum result:

```sh
cargo test --release -p ecp5_emu --test edid -- --ignored
```

A real monitor's EDID can identify it through serial numbers. **Its log, raw
image and identifying decoded fields stay in out/edid/ only, outside git.**
Git gets hashes and non-identifying decoded facts, such as the EDID version and
preferred mode. Unlike this NACK-only record, that later capture must not be
copied into observations.log or any other tracked file.
