# ULX3S real-time clock

The pure Rust answering model [rtc.rs](../src/peripherals/rtc.rs) is U7, a Microchip
**MCP7940N** at seven-bit I2C address **0x6F** (`DE` write, `DF` read). Its source is
[Microchip DS20005010J](https://ww1.microchip.com/downloads/aemDocuments/documents/MPD/ProductDocuments/DataSheets/MCP7940N-Battery-Backed-I2C-RTCC-with-SRAM-20005010J.pdf),
particularly the register map in Table 5-1, sections 5.3–5.7 and 6.1, and the I2C timing table.
The v3.0.8 PCB (`assets/ulx3s/ulx3s_v3.0.8.kicad_pcb`) connects U7 pad 6 SCL through
**FPDI_SCL** to U1 ball **E12**, and pad 5 SDA through **FPDI_SDA** to ball **B19**.
R22/R23 are 3.3 kOhm pull-ups. [board.rs](../src/board.rs) records this evidence as
`RTC_SCL` and `RTC_SDA`.

## Shared bus and pad answers

U11, a PCA9306 level shifter, connects the same bus to the HDMI connector's DDC. A plugged-in
monitor brings its EDID EEPROM at **0x50** onto this bus. The RTC answers **only 0x6F**;
the [EDID model](PERIPHERALS_EDID.md) answers 0x50 when the HDMI receiver is attached,
unless ECP5_EDID=off. Their open-drain SDA answers are combined into one driver. The
level shifter's electrical behavior is not modelled; pixel decoding is independent.

`feed(time_ps, Pins { scl, sda })` takes resolved 0/1/X/Z pad levels. A released Z resolves
high through the board pull-up. SDA transitions with SCL high detect START, repeated START
and STOP; bits are received on rising SCL and answers change on falling SCL. The ninth bit
is ACK/NACK, including the master's final NACK on reads. Incomplete bytes are discarded by
START/STOP. Unknown inputs release SDA and abort until the next START. There is no stretching.

**SDA is open-drain**, shared with the FPGA. `Rtc::sda()` returns `Some(false)` to pull low
and `None` to release, never `Some(true)`. `Attached` attaches only when the design drives
SCL and has bidirectional SDA. It traces E12/B19 and installs an external driver on B19.
The existing tristate-pad path in [FAST_MODE.md](FAST_MODE.md) resolves that driver against
the FPGA's `oe ? 0 : z` and PULLMODE=UP. Released high comes from the pull, not a driven one.
SCL may be push-pull or open-drain; the RTC never drives it.

The answering horizon is **one board-clock edge during a transfer**, from START through STOP,
even if another address is selected or the master has NACKed. In idle it is **16 edges**.
At the ULX3S's 25 MHz, 16 edges take 0.32 us, shorter than the datasheet's minimum START
hold time of 0.6 us at 2.5–5.5 V (4 us for standard 100 kHz). Thus idle batching reaches
the START before the first bit clock and long before the address ACK. This assumes a
board-clock-derived I2C master up to 400 kHz; a faster PLL clock is outside this runner's
answer timing. Answers are applied 1 ps after the reached instant, with `set_at_levels`.
No kernel, compiler or fast executor changes are needed.

## Registers, time and image

`Rtc::new([u8; 96])` supplies registers **00–1F** and the **64-byte SRAM at 20–5F**.
`ECP5_RTC_IMAGE` names an exactly 96-byte binary file. Missing, unreadable or incorrectly
sized files use the documented default: **ST=1, 2026-01-01 00:00:00**, 24-hour format,
weekday 5 (Sunday=1 convention), VBATEN=0, control/trim/alarms/stamps and SRAM zero.
OSCRUN is derived, so the default time registers read `80 00 00 25 01 01 26`.
This is deterministic emulated time, unrelated to the host's wall clock or timezone.

The pointer survives STOP and repeated START. Sequential reads and writes wrap within the
selected block, **1F → 00** and **5F → 20**, as section 6.1 specifies. Consequently a single
96-byte read from zero does not read SRAM. Addresses 60–FF are NACKed. The first data read
uses the current pointer; a completed byte increments it even when the master then NACKs.
Time registers 00–06 are buffered when a register read begins and when its pointer wraps to
zero, preventing mixed dates across a tick. SRAM reads do not latch the time.

Register and SRAM **writes work in the model**, although the board check never sends one.
Time is BCD with 12/24-hour carry, AM/PM, weekday 7 → 1, month lengths, leap years by the
two-digit year modulo four, and year 99 → 00. ST=0 freezes the clock; ST=1 advances one
second per **10^12 ps** of supplied trace time, preserving fractional seconds across feeds.
OSCRUN follows ST immediately. Writing RTCSEC resets the fractional second phase.
Read-only LPYR follows the year; alarm hour-format bits follow RTCHOUR and ALM1 polarity
follows ALM0. Unimplemented bits read zero. Reserved 09/10/17 retain the image's byte on
load and read it back, following [HR09](inventory/LFE5U-85F/hr09_rtc/README.md): the chip
returned 00 at 09 and 01 at 10/17. **Writes to reserved bytes are assumed ignored**:
the datasheet only says reserved, and the read-only board check cannot test writes.
The default image is unchanged. CONTROL and OSCTRIM are stored, VBATEN is writable,
and clearing a previously set PWRFAIL clears both stamps.
Power-fail stamp fields otherwise support their documented writes and masks, without
inventing power events. Alarm masks select the matching field (or all six), set ALMxIF on
matches, and writing ALMxWKDAY clears ALMxIF; a continuing match sets it again. Matching
is evaluated on writes and second ticks. Reserved alarm masks never match.

`status()` supplies the date/time, ST, OSCRUN, active transfer, last seven-bit bus address,
read/write direction, last completed register address, bytes read/written and NACKs. Pointer
writes do not count as data writes. NACKs include wrong addresses, invalid pointers/data
and each normal terminating NACK from the master.

## Runner and window

`Attached::Snapshot` clones all 96 bytes, read buffer, I2C state, current SDA answer, pointer,
fractional second and last trace time. Rewind restores them, and replay regenerates answers;
answers are not logged as user inputs. `View.rtc` supplies a small **RTC (MCP7940N)** window
with date/time, ST, OSCRUN and last access/counts. The window record emits `# rtc …` when
these displayed fields change. Only [monitor_view.rs](../gui/src/monitor_view.rs) changes
in the GUI; `app.rs` needs no change.

## Read-only board method

[rtc_read/top.v](../examples/ulx3s/rtc_read/top.v) starts a full read every **25,000,000 board
cycles (1 s)**. Its plain Verilog master runs near **100 kHz** (125 board clocks per half
period, with extra low/setup intervals at byte boundaries). Both pads use open-drain
`oe ? 0 : z` and PULLMODE=UP. Each transaction can only do:

1. START, `DE`, one pointer byte (00 for 32 registers, then 20 for 64 SRAM bytes).
2. Repeated START, `DF`, N reads with ACK, a final NACK, STOP.

**There is no data-byte write state, arbitrary command input or writable transmit buffer
port.** The pointer ACK always leads to a repeated START. No transaction can change the
board's clock, configuration or SRAM. Do not add time-setting or SRAM writes to this design.
It prints on `ftdi_rxd` L4 at 115200 8N1:

```text
RTC t HH:MM:SS 20YY-MM-DD st 0|1
RTC r <64 uppercase hex digits, 00–1F>
RTC s <128 uppercase hex digits, 20–5F>
```

The human-readable time converts 12-hour mode to 24-hour mode; the raw lines retain the
register representation. A device NACK prints `RTC NACK` and retries the same reads next
second. Run build and emulator checks from the repository root, with the toolchain
from [docs/TOOLCHAIN.md](TOOLCHAIN.md) installed. Set
`ECP5_IMAGE_READER` to the native image reader and `ECP5_DB` to the Trellis
database. No board access is needed:

```sh
cargo test --release --test rtc
cargo test --release --lib
cargo test --release --test rtc -- --ignored --nocapture
cargo test --release --test live_peripherals -- --ignored --test-threads=1
```

**Keep the board's register/SRAM read out of git.** The optional board capture is
`out/rtc/board_rtc.log`, already ignored under `out/`. Store the output of
[run.py](inventory/LFE5U-85F/hr04_hdmi_oled/run.py) there: load headers followed by
**two-space-indented UART lines**. Do not commit the log, an image reconstructed from it,
or raw register/SRAM values in another document. Record only non-sensitive comparison
metadata. [HR09](inventory/LFE5U-85F/hr09_rtc/README.md) records the 2026-11-17 board check:
three loads, twelve identical register/SRAM reads, with the oscillator stopped. The first
emulator comparison found a reserved-byte model error at 10/17; preserving image bytes
corrects it. The post-fix private-log comparison passed on rserv on 2026-11-17: 3 passed,
0 failed, including `the_board_read_reproduces_in_the_emulator`. HR09 stores only the time
line, hashes and comparison metadata, not the register/SRAM hex.

`the_board_read_reproduces_in_the_emulator` takes the first complete indented `r` + `s` pair,
constructs its image in memory with **ST exactly as read**, and asserts identical raw lines
at the emulator's first read. It does not compare moving time text. Headers, malformed or
partial lines cannot combine separate loads. A missing file prints a skip message and
returns cleanly. The rserv wrapper excludes `out/`, so private captures must be supplied
separately on that machine if this optional check is wanted there.

## Checks and assumptions

Plain `cargo test` needs no FPGA toolchain for the read-only audit. It checks every Verilog
file in rtc_read: only the DE / pointer / DF transmit assignments and shifting are allowed,
the pointer stage goes directly to repeated START, and the read-address stage goes to receive.
Comment/whitespace-independent SHA-256 seals of the reviewed controller, wrapper and UART
make this fail closed against extra states, drivers or bypass logic, including newly added
Verilog files. Changes to those sources require reviewing the complete FSM before updating
the seals. Mutation tests reject a changed address, data byte, transition or extra pin driver.
LPF checks require both board balls and PULLMODE=UP.

Rust unit tests cover framing, repeated START, ACK/NACK, current/pointer/sequential reads,
both block wraps, SRAM and register writes, ST stop/restart, leap/month/year and 12-hour
rollovers, buffered reads, fractional seconds, alarm flags, read-only masks, image-fed
reserved bytes with ignored writes, unknown pads and checkpoints. Ignored
[tests/rtc.rs](../tests/rtc.rs) routes the actual design and checks known image output and
seconds 00/01/02 one emulated second apart through the answering loop.
[tests/live_peripherals.rs](../tests/live_peripherals.rs) checks two complete reports and
identical RTC state and UART after seeking into an active transfer and back before boot.
GUI tests paint through an in-memory egui context and check records; no window is opened.

On rserv (2026-11-17), the fast tests at default parallelism measured:

| Image / reads | Board cycles | Wall time | Cycles/s |
|---|---:|---:|---:|
| Known stopped image, one full read | 777,728 | 0.136 s | 5,706,052 |
| Default running image, three full reads | 50,777,600 | 9.000 s | 5,641,690 |

The running image printed `RTC t 00:00:00 2026-01-01 st 1`, then seconds `01` and
`02`; all register and SRAM lines matched, with zero data writes and two terminating
master NACKs per full read. Build, configuration load and C compilation are excluded.
The complete serial live-peripheral suite passed all seven tests (264.42 s), including the
RTC reports at 3.97 MHz and exact state/UART replay after both seeks. The Mac GUI suite
passed 65 tests (six existing ignored tests); no window or board access was used.

Conservative modelling choices and limits:

- The **battery**, supply switching and physical **power-fail events** are not modelled.
  Loaded PWRFAIL/stamps persist until written/cleared; the model never creates such events.
- The **MFP output** is not driven. Alarm comparison flags are available in registers;
  alarm/square-wave/general-purpose electrical output is absent.
- **OSCTRIM's effect on rate** and EXTOSC's external oscillator are not modelled. ST alone
  controls an ideal one-second clock, including when CONTROL stores EXTOSC.
- OSCRUN has no crystal startup/failure detection delay. There is no RC rise time, bus
  arbitration, stretching, voltage/temperature drift, propagation delay or pulse-width checking.
- Invalid BCD/calendar values have undefined behavior on silicon; tests use valid dates.
  The century is interpreted as 2000–2099 and repeats the two-digit leap-year rule.
- RTCSEC writes reset the subsecond phase; hardware oscillator phase/startup is idealised.
  A received data byte is applied after its eighth bit, before the ACK edge.
