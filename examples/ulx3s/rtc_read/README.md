# Read-only MCP7940N board check

Build from the repository root, with the toolchain from
[docs/TOOLCHAIN.md](../../../docs/TOOLCHAIN.md) installed:

```sh
examples/ulx3s/build.sh examples/ulx3s/rtc_read target/reference/rtc_read
```

[rtc_reader.v](rtc_reader.v) only sends DE, one pointer byte, repeated START, DF,
then reads with ACK and a final NACK, and STOP. There is no data-write state.
Do not add one: this check must preserve the board's clock and SRAM.
Every second [top.v](top.v) reads registers 00–1F and SRAM 20–5F in two
transactions, since the blocks wrap separately. SCL E12 and SDA B19 are
open-drain with PULLMODE=UP, about 100 kHz. The bus is shared with HDMI DDC.

UART L4 at 115200 prints `RTC t HH:MM:SS 20YY-MM-DD st 0|1`,
`RTC r <64 hex digits>` and `RTC s <128 hex digits>`.
The optional private run.py capture belongs only in `out/rtc/board_rtc.log`,
never in git. See [PERIPHERALS_RTC.md](../../../docs/PERIPHERALS_RTC.md)
for the image format, assumptions, source safety audit and replay test.
