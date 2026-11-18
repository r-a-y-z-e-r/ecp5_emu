# HR09: the RTC on the board, against the emulator

[rtc_read](../../../../examples/ulx3s/rtc_read/top.v) reads the ULX3S's MCP7940N RTC
(U7) at I2C address **0x6F**, as [PERIPHERALS_RTC.md](../../../PERIPHERALS_RTC.md)
describes. The controller can send only the write address, one register-pointer byte,
repeated START, the read address, reads with ACK and a final NACK, then STOP.
**There is no data-byte write state.** It cannot set the clock or change the SRAM.

## Setup

- **Board:** ULX3S 85K v3.0.8, SRAM loads under the shared board lock on 2026-11-17.
- **Runner:** [../hr04_hdmi_oled/run.py](../hr04_hdmi_oled/run.py), prefix `RTC`, three loads
  of 4 s each at `2026-11-17T07:43:03Z`, `2026-11-17T07:43:12Z` and
  `2026-11-17T07:43:22Z`.
- **Configuration:** [rtc_read.config](rtc_read.config), a copy of
  `target/reference/rtc_read/top.config`, built on the Mac with `examples/ulx3s/build.sh`
  (nextpnr seed 1). At the 25 MHz constraint, nextpnr reported a maximum frequency of
  **81.04 MHz**. The loaded `top.bit` is not stored; its hash is in [SHA256SUMS](SHA256SUMS).
- **Records:** [TOOLS.txt](TOOLS.txt), [SHA256SUMS](SHA256SUMS), and the configuration copy.
  The board capture stays only in `out/rtc/board_rtc.log`, outside git. This record was
  written from the supplied run facts without opening that private log.

## Observations

Each load captured 12 complete `RTC` lines: four reads of `t`/`r`/`s`, giving **36 lines
and 12 full reads** across the three loads. Every read printed the same time line:

```text
RTC t 00:00:00 2001-01-01 st 0
```

The oscillator is stopped (ST=0), the time registers hold reset-like values, and CONTROL
(0x07) is 0x80. The `r` line was identical across all three loads and all twelve reads.
The `s` line was also identical across all twelve reads; its 64 SRAM bytes are not all zero.
The raw register and SRAM hex and the log are deliberately absent from this record.

| Private capture content | SHA-256 |
|---|---|
| `RTC r`: its 64 hex characters only, without prefix or newline | `a86e91520332cc8dd7d0091e472aafc261e34830949679374cd4edc08592d816` |
| `RTC s`: its 128 hex characters only, without prefix or newline | `e92c741eb36b8b90ed4e33a1d6f446e57869e78b11fcffc54715ac044c99b6ad` |
| Whole `out/rtc/board_rtc.log` | `fe250c1eff56e67dae84bc6a4f7d402f98f85db3c8aad95e001dbbd84c032801` |

## The comparison

[tests/rtc.rs](../../../../tests/rtc.rs) `the_board_read_reproduces_in_the_emulator`
constructs an image in memory from the first complete private `r` + `s` pair, keeps ST
exactly as read, and compares the emulator's first raw register/SRAM lines. The moving
time text is not part of that equality check.

| Model / rserv comparison | Result |
|---|---|
| Original RTC model, before this correction | **FAILED**, at 0x10 and 0x17 only: the chip returned 0x01 and the model forced zero. Every other register and all 64 SRAM bytes matched. |
| Corrected model, preserving image-fed reserved bytes | **PASS**, observed on rserv on 2026-11-17 with `cargo test --release -p ecp5_emu --test rtc -- --ignored`: `the_board_read_reproduces_in_the_emulator ... ok`; 3 passed, 0 failed. |

The board check found a model error. Reserved registers **0x09, 0x10 and 0x17** now keep
the image's byte on load and read it back, including the board's 0x00 at 0x09 and 0x01 at
0x10/0x17. A unit test checks those values, arbitrary image bytes and ignored writes.
No other behavior is fitted to the board, and the default image is unchanged.

**Ignoring writes to reserved registers is an assumption.** The datasheet only labels
them reserved; this read-only board check cannot test their write behavior. HR09 supports
the read-back correction, not a claim about writes.

## Read-only evidence

1. The plain test `reference_verilog_can_only_send_address_pointer_address_then_read`
   audits the permitted transmit bytes and mandatory transition from pointer to repeated
   START, with source seals rejecting added states or bypass drivers. It needs no FPGA
   toolchain.
2. The reader's SDA is `assign rtc_sda = sda_low ? 1'b0 : 1'bz;`; SCL uses the same
   open-drain form. Both pads have `PULLMODE=UP` in the LPF.
3. The register `r` line is identical across the three SRAM loads and all twelve reads.

## What this check does not establish

- **Timekeeping:** ST=0 on the board, and the read-only design cannot start the oscillator.
  One second per emulated second is checked only by
  `rtc_read_advances_once_per_emulated_second`, not by this board run.
- **Writes:** the design cannot write register or SRAM data, by construction. Their model
  semantics and the reserved-write assumption are covered by emulator unit tests only.
- **HDMI/DDC:** whether a monitor was plugged into GPDI during this run is not recorded.
  The design addresses only 0x6F; a monitor's EDID EEPROM at 0x50 would not answer it.
  Every read completed with consistent data. This does not establish monitor presence.

## Emulator checks for the correction

The correction was checked through the repository's rserv wrapper:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --release -p ecp5_emu --lib
cargo test --release -p ecp5_emu --test rtc -- --ignored --skip the_board_read_reproduces_in_the_emulator
cargo test --release -p ecp5_emu --test rtc -- --ignored
```

Checked on 2026-11-17 on rserv: formatting and workspace clippy passed; the library
suite passed **122 tests** (two existing ignored tests), including the reserved-byte
regression; both permitted fast-mode RTC tests passed. The separate plain read-only
audit `reference_verilog_can_only_send_address_pointer_address_then_read` passed.
The operator then ran the full RTC suite against the private log on 2026-11-17:
**3 passed, 0 failed**, including `the_board_read_reproduces_in_the_emulator`.
