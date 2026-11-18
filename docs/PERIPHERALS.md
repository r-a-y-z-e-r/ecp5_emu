# Peripherals of the ULX3S

Added on 2026-11-16. The chips the ULX3S has around the FPGA, emulated from what a design drives on
their pads, so a design that uses them can be seen and checked. The models are in
[src/peripherals/](../src/peripherals/). They do not depend on either executor: they take pad
samples with their times. The balls are in [src/board.rs](../src/board.rs), checked against the
v3.0.8 PCB by [tools/check_peripheral_balls.py](../tools/check_peripheral_balls.py) (60 of 60).

| Peripheral | Model | State |
|---|---|---|
| J1/J2 headers | [PERIPHERALS_HEADER.md](PERIPHERALS_HEADER.md) | pad-fed levels, frequency/duty and external inputs; [HR13](inventory/LFE5U-85F/hr13_header/README.md): both variants match, including GP0..GP3 pad readback at DRIVE=4 and the 56-input pull pattern; empty headers confirmed 2026-11-17 |
| DIP switches, SW1-SW4 | [DIP switches](#dip-switches) | live inputs, configured pulls and recorded physical positions |
| HDMI (GPDI) | a DVI/HDMI receiver: frames from the TMDS lanes | here |
| Monitor EDID, DDC EEPROM | [PERIPHERALS_EDID.md](PERIPHERALS_EDID.md) | answering 0x50 on the RTC bus; [HR12](inventory/LFE5U-85F/hr12_edid/README.md): all nine board lines EDID nack at 18:41Z, matching ECP5_EDID=off; no real EDID read |
| Audio jack, L/R/V resistor DACs | [PERIPHERALS_AUDIO.md](PERIPHERALS_AUDIO.md) | output-only scopes; [HR11](inventory/LFE5U-85F/hr11_audio/README.md): first three board UART lines match the emulator in all three loads; analog signals modelled, not measured |
| OLED, SSD1331 96×64 | [PERIPHERALS_OLED.md](PERIPHERALS_OLED.md) | here |
| SDRAM, MT48LC16M16A2 | [PERIPHERALS_SDRAM.md](PERIPHERALS_SDRAM.md) | here |
| Configuration flash, IS25LP128F | [PERIPHERALS_FLASH.md](PERIPHERALS_FLASH.md) | SPI, USRMCLK; writes in emulator memory only |
| ADC, MAX11125 | [PERIPHERALS_ADC.md](PERIPHERALS_ADC.md) | answering SPI, 8 pad-fed voltages |
| RTC, MCP7940N | [PERIPHERALS_RTC.md](PERIPHERALS_RTC.md) | answering open-drain I2C, emulated clock and 64-byte SRAM; [HR09](inventory/LFE5U-85F/hr09_rtc/README.md): board read found reserved-byte model error |
| SD card, SPI and native 1/4-bit bus | [PERIPHERALS_SD.md](PERIPHERALS_SD.md) | [HR05](inventory/LFE5U-85F/hr05_sd_card/README.md), SPI; [HR14](inventory/LFE5U-85F/hr14_sd_native/README.md), ten block-0 reports with valid four-bit CRC16 and passing private-block replay |

## DIP switches

`board::SWITCHES` maps SW1-SW4 / `sw[0..3]` to **E8, D8, D7, E7**. The v3.0.8 PCB
connects ON through **R51, 130 Ohm**, to `2V5_3V3`, reading 1. OFF leaves the real ball
floating, so its configured pad pull decides the level. No other part is on these nets.

The live model is **ON=1, OFF=the configured pull**: DOWN is 0, UP is 1. It reads the
configured device's joint PIO decode through `board::input_pull`. NONE, missing, ambiguous
or other settings use **0**: the real pin floats there, but fast mode is two-valued.
Routed tests confirm that pure inputs reject `set_at_levels(None)` today; the event kernel
resolves released UP/DOWN pads correctly. SD/flash MISO always send Boolean levels, and ADC
maps released DOUT to Boolean idle high; none sends `None` to a pure input pad. RTC SDA uses
the separate, Z-capable bidirectional pad path.

**ECP5_SWITCHES** sets initial live positions: exactly four `0`/`1` characters, sw[0] first;
`1010` means SW1/SW3 ON, absent means all OFF, and malformed values fail with an error.
Tests and embedders pass positions through `live::spawn_with` and `live::RunOptions`;
`ECP5_SWITCHES` remains the window's start setting when no override is supplied.
`Static::switches: [bool; 4]` identifies which switch inputs the design reads;
`Shared::switches: [bool; 4]` holds the physical positions at the current instant.
`live::Command::Switch(index, on)` queues a flip at now + 1 ns, like a button. The Boolean
pin level and physical position are logged together and checkpointed: going back undoes
later flips, and Step/Run forward replay them. With UP, both positions read 1 while the
recorded positions remain distinct. The 3D levers will drive this API after GUI integration;
sw[0] is leftmost and ON is toward the board's top edge. The GUI is unchanged.

[switches/top.v](../examples/ulx3s/switches/top.v) uses input-only DOWN pads,
`led[3:0] = sw[3:0]`, and `SW xxxx` on UART L4 at 115200 8N1, sw[0] first: after initial
10 ms settling, after changes stable for about 10 ms, and every second. The buildable
[UP variant](../examples/ulx3s/switches/pullup/top.lpf) copies the same HDL and changes only
the four pulls: all switches read 1, OFF or ON. Only LEDs and UART are outputs. [HR10](inventory/LFE5U-85F/hr10_switches/README.md) records four SRAM loads on 2026-11-17,
all twelve lines `SW 1111` with DOWN and UP, matching the all-ON emulator setting. [tests/live_switches.rs](../tests/live_switches.rs) checks both
pulls, startup/flip/OFF, the one-second repeat, source-copy agreement, and positions/LEDs/UART
after rewind and Step/Run replay.

## From the command line

`emulate --fast --peripherals` opts into the same pad-fed models as the live runner.
Without the flag the existing CLI path, inputs and output stay unchanged. The event-kernel
CLI path refuses `--peripherals`; it requires `--fast` and the usual clock/startup options.
For example, after building `sdram_test`:

```sh
ecp5_emu emulate target/reference/sdram_test/top.config \
  --names target/reference/sdram_test/top_routed.json \
  --fast --peripherals --startup configured --scope observed \
  --clock net:clk_25mhz --period-ns 40 --cycles 1000000 \
  --console-out net:ftdi_rxd
```

The console prints `SDRAM W8000 E0000` on stdout. The final model counters go to stderr,
independently of whether a console is enabled. `--console-in`, `--stimulus`, `--probe` and
`--vcd` keep their usual meaning: timed inputs are merged with clock edges, UART bytes stay
on stdout, and requested probes go into the VCD. Peripheral probes are additional internal
traces, not extra displayed probes or VCD columns. `--cycles` still counts rising edges.
The CLI uses `board::used_balls` and `Attached::new`; only models called for by the configured
balls attach. It installs the models' initial inputs, including Z on released bidirectional DQ/SDA/CMD/DAT pads,
and traces their output/bidirectional pads after each instant. It does not choose or tune
fast programs while tracing. As in live.rs, SDRAM, selected SPI/I2C devices and active native SD transfers answer every
edge through `run_edges_with`, in bounded batches. Flash and ADC are observed every edge
also in idle, so short commands cannot pass unseen. Other idle runs are bounded by
`Attached::horizon`, with feed/answer after each chunk. Changed answers are applied **1 ps**
after the reached edge or timed input, and that answer's pad trace is fed back too.
The initial event-kernel warm start is the same as the live runner's; peripheral tracing
starts when fast mode takes over. Existing peripheral clock/timing limits still apply.

Images and voltage settings are the same as in the window: **ECP5_SD_IMAGE** (else demo card),
**ECP5_FLASH_IMAGE** (else demo firmware), **ECP5_RTC_IMAGE** (else the deterministic default
clock), **ECP5_EDID_IMAGE** (128/256 bytes, else synthetic monitor), and **ECP5_ADC_VOLTS**
(else configured pad pulls/floating defaults). EDID attaches with HDMI; **ECP5_EDID=off**
disconnects its EEPROM while video decoding continues. See each model's
document for the image format. The flag does not change their write/read-only policies.

In both event-kernel and fast `emulate`, configured board inputs get the window's idle
levels even without `--peripherals`: PWR (D6) is 1, the other buttons (R1/T1/R18/V1/U1/H16)
are 0, UART RX (M1) is 1, and DIP inputs E8/D8/D7/E7 use **ECP5_SWITCHES** (sw[0] first,
default `0000`). A DIP ON reads 1; OFF reads its configured pull, with NONE/unknown using 0.
`board::idle_inputs(device, switches)` supplies the same levels to the CLI and live runner.
These defaults apply only where the design reads the ball and no initial value was supplied.
`--set pin:R1=1` or a time-0 stimulus value overrides the default; later stimulus events
change it at their timestamps, starting from the idle level before the first event.
`--console-in`, `--clock`, and an attached peripheral's answering inputs retain their own
initial levels. Other input balls retain their existing behavior. Invalid ECP5_SWITCHES
values are rejected, as in the window.

### Final stderr report

Each attached model emits one summary line in this fixed order: **sdram, sd, flash, adc,
rtc, oled, hdmi, edid, header, audio**. Absent models emit nothing. Every line begins `peripheral:` and uses
space-separated `key=value` fields in the order below. Counters and dimensions are unsigned
decimal; `yes|no` and `0|1` are the stated Boolean encodings. Hex is uppercase without `0x`.
These lines are additional to the ordinary fast-mode statistics.

```text
peripheral: sdram init=yes|no reads=N writes=N timing_violations=N unknown_samples=N refresh_shortfalls=N
peripheral: sd state=STATE last_command=N last_argument=HHHHHHHH blocks_read=N blocks_written=N crc_errors=N illegal_commands=N mode=spi|sd1|sd4 card_state=STATE rca=HHHH bus_width=1|4 commands=N responses=N dat3_pullup=0|1
peripheral: flash jedec=HHHHHH last_command=HH last_address=HHHHHH bytes_read=N ignored_writes=N programs=N erases=N protected=N busy_ignored=N config_writes=N
peripheral: adc frames=N unsupported_frames=N mode=HHHH codes=HHHH,HHHH,HHHH,HHHH,HHHH,HHHH,HHHH,HHHH
peripheral: rtc time=YYYY-MM-DDTHH:MM:SS st=0|1 oscrun=0|1 last_bus_address=HH last_address=HH bytes_read=N bytes_written=N nacks=N
peripheral: oled frames=N crc32=HHHHHHHH commands=N data_bytes=N errors=N unknown_commands=N display_on=yes|no
peripheral: hdmi frames=N width=N height=N crc32=HHHHHHHH invalid_symbols=N relocks=N
peripheral: edid present=yes|no reads=N bytes_read=N last_pointer=HH nacks=N
peripheral: header NAME=FHz/D% ...
peripheral: audio l_hz=F l_vpp=F r_hz=F r_vpp=F v_hz=F v_vpp=F transitions=N
```

SD `state` is `idle`, `ready`, `reading` or `writing`; its command is a decimal CMD number,
or `--` before any command. The appended SD fields give protocol, native `card_state`
(`idle/ready/ident/stby/tran/data/rcv/prg`, or the legacy SPI state), hex RCA, active bus width,
command/response frame counts, and the native card's internal DAT3 pull-up flag (zero for SPI).
See [PERIPHERALS_SD.md](PERIPHERALS_SD.md#native-sd-bus-mode) for framing, fixed delays,
fictional identity and private CID/block-0 capture rules. Other unavailable hex fields use one dash per hex digit:
`--` for command/RTC address, `------` for flash address, `----` for an unconverted ADC code,
and `--------` for HDMI CRC before a full frame. HDMI width/height are zero before a frame.
The OLED CRC is its last received frame's data CRC; HDMI CRC is the last decoded RGB frame's
CRC-32. RTC time is in emulated time, and `nacks` includes the normal final master NACK.

EDID `reads` counts accepted A1 transactions; `bytes_read` counts complete bytes.
`last_pointer` is the last pointer set or byte address read before increment (7F after a
base-block read). `nacks` counts declined addresses, rejected data writes and the final
master NACK; another device may ACK an address this model declines, so this is not a
resolved-bus error count. Its EEPROM is write-protected; real captures stay in out/edid.
See [PERIPHERALS_EDID.md](PERIPHERALS_EDID.md) for the fictional XEM identity, shared
open-drain answers, image format and read-only board method.

Flash `programs`/`erases` count completed accepted commands; `protected` counts
BP-rejected array commands; `busy_ignored` counts forbidden commands while WIP=1.
`ignored_writes` now means rejected write/control commands (unsupported legacy
opcodes, malformed, no WEL, protected or busy), once per command. Accepted WREN,
WRDI, WRSR, programs and erases no longer increment it. Writes affect memory only:
`ECP5_FLASH_IMAGE` is never modified. `config_writes` counts well-formed attempts
at the FPGA configuration area, even if refused. The first event adds a separate
`peripheral: flash warning: command HH range [START,END): this would overwrite the board's FPGA configuration (bitstream) on a real ULX3S`
line after the flash summary. See [PERIPHERALS_FLASH.md](PERIPHERALS_FLASH.md) for
the 0x250000 boundary, timing/protection assumptions and cheap checkpoints.
The board rule remains SRAM loads only: never write its configuration flash.

Header reports list output/bidirectional pins in J1/J2 pad order; F is Hz and D is
percent high, both with three decimals or `--` when unavailable. Inputs-only models
print `peripheral: header` with no fields. **ECP5_HEADER="GP0=1,GN3=0"** supplies
header input defaults in both executors; values 0/1/z, with `--set` and stimulus
overrides. Release uses a pure input's pull (NONE → 0), or Z on a bidirectional pad.
The window records HeaderLevel changes and restores them on rewind. An external
AIN-shared drive wins over AdcVolts until released. See [the mapping, API and
inputs-only board safety condition](PERIPHERALS_HEADER.md).

Audio `*_hz` fields use three decimal places, `*_vpp` six decimal volts; `--` means
no estimate or undefined voltage range. DC has no frequency estimate and zero Vpp.
`transitions` sums changes of the three settled pad words, excluding startup. Audio
voltages, Vpp and scopes are **modelled from the PCB, not measured**, assuming the jack
is unplugged; no load, filtering or playback. See [the model and board method](PERIPHERALS_AUDIO.md).

After the SDRAM summary, its bounded first diagnostic messages, if any, each occupy one
additional line, with free text after the second colon:

```text
peripheral: sdram message: TEXT
```

[tests/cli_peripherals.rs](../tests/cli_peripherals.rs) runs routed `sdram_test`,
`picorv32_sdram` and `sd_read` through the binary, including the VCD/probe path and the
report fields. Without the flag `sdram_test` reads the pad pulls and reports data errors;
with it the model supplies DQ and the error count is zero. The PicoRV32 pass-0 primes count
is `303, 855498 cycles`. No board or window is needed.

Checked on rserv on 2026-11-17: all five CLI tests passed at default parallelism, including
`SDRAM W8000 E0000` with the model and `SDRAM W8000 E7FFF` without the flag. The library
suite passed 122 tests, the serial fast-mode regression run passed 17, and all four SDRAM
tests passed. Formatting and workspace clippy (`--all-targets -- -D warnings`) also passed.
The headless `cd examples/gallery && python3 check.py all --emulator` run, without the
flag, matched all eight demos' expected UART transcripts from their restart markers.

## HDMI (GPDI)

The GPDI connector carries DVI/HDMI as four TMDS pairs: D0 (blue) A16/B16, D1 (green) A14/C14,
D2 (red) A12/A13 and the pixel clock A17/B18 (`gpdi_dp[0..3]` / `gpdi_dn[0..3]`). The receiver
([video.rs](../src/peripherals/video.rs), [tmds.rs](../src/peripherals/tmds.rs)) reads the
positive pads. It decodes them as a monitor would, so it does not matter how the design
serialises: `ODDRX1F` on a shift clock, the X2 gearbox elsewhere, or plain fabric.
(The X2 and 7:1 gearboxes do not exist on the top bank, where GPDI is.)

### Pad samples at every instant

A TMDS lane changes ten times per pixel. With the usual ULX3S pipeline that is every edge of a
125 MHz PLL clock, five times per edge of the 25 MHz board clock. `FastSim::run_edges_sampled`
samples only at board-clock edges, so fast mode has a trace
([src/fast.rs](../src/fast.rs), `FastSim::trace` / `take_trace`). It records the given signals
after every instant the program runs, and the time of each in picoseconds.
- Every path keeps it: the host loop, whole cycles in the program, and the schedule table, which
  then samples every instant.
- The caller's own samples come out byte-identical with and without a trace
  ([tests/fast_trace.rs](../tests/fast_trace.rs): `ddr_pattern`, `blinky_pll`, and `hdmi_bars` in
  tests/hdmi.rs, on each path, and across program changes every 97 cycles).
- `take_trace` drains the buffer. It grows by 12 bytes per instant for four pads, which is about
  3 GB per second of board time at 125 MHz DDR, so it is taken after every run.
- A run that traces does not tune (`FastSim::tune`), since the trace is part of what a tuning
  measurement would time.
- Restoring a snapshot marks a gap, and the receiver starts its lock again.

In the emulator a pad changes only at an instant, so its value at any time is the last row at or
before that time.

### Receiving

1. **Bit timing.** The pixel period is the time between rising edges of the clock lane. It must
   be the same within 1% over eight periods to lock. Each pixel has ten bit cells from the
   clock's rising edge, and each lane is sampled in the middle of each cell. The clock word
   itself (0000011111 or another) is not assumed to line up with the data words.
2. **Word alignment.** Each lane looks for the bit offset at which control tokens come eight
   times in a row during blanking, as a real receiver does, and keeps it. Its words then go
   through the TMDS decoder. That decoder knows every code the encoder can make: the 1024-entry
   table is built from the DVI 1.0 encoder of Figure 3-5 with every disparity.
3. **Periods.**
   - **Control.** Control tokens on all three lanes are a control period. HSYNC and VSYNC are
     C0 and C1 of lane 0.
   - **Video.** A run of data words on all three lanes is video, one pixel per word triple
     (red from lane 2, green from lane 1, blue from lane 0).
   - **HDMI.** Lanes 1 and 2 carry the preamble before a period. CTL0..3 = 1000 is video, and
     the two guard band symbols after it are dropped. 1010 is a data island (TERC4: audio,
     InfoFrames), which is skipped. DVI has no preamble, and its pixel bytes 0x55/0xAB, whose
     codes are also the guard band's, stay pixels.
4. **Frames.** A line is one run of video pixels. A frame starts at the first line after more
   than 1.5 lines without video (the vertical blank) and ends at the next such start.
   - A frame is kept only if it began after lock and saw no gap or invalid symbol.
   - Measured for each frame: width, height, total pixels per line and lines per frame, sync
     widths and polarities (the active level is the one held for less time), the pixel clock and
     refresh rate in emulated time, and whether every line had the same width.
5. **CRC.** Each frame carries the CRC-32 of its pixels: the bytes R, G, B of each pixel in
   raster order, CRC-32/ISO-HDLC (zlib's `crc32`).

`peripherals::Attached` is what a runner holds. `Attached::new(used)` enables the receiver when
the design drives all four `gpdi_dp` balls. `balls()` lists the balls to trace, `feed(trace)`
takes `take_trace()`, and `view()` gives a cheap snapshot (`VideoStatus`, with the last frame in
an `Arc`) for a window.

### Reference designs

- [hdmi_bars](../examples/ulx3s/hdmi_bars/top.v): 640×480 eight colour bars, 800×525 at exactly
  25 MHz (59.52 Hz, which monitors take as 60 Hz).
  - One EHXPLLL gives the 125 MHz shift clock (CLKOP) and the 25 MHz pixel clock (CLKOS) from one
    VCO, so they are in phase.
  - A TMDS encoder per lane runs in the pixel domain. A mod-5 counter in the shift domain loads
    each 10-bit word and shifts two bits per clock into one `ODDRX1F` per lane, D0 first. The
    clock lane sends 0000011111.
- [hdmi_moving](../examples/ulx3s/hdmi_moving/top.v): the same pipeline with scrolling bars and a
  bouncing square, so every frame differs. After each frame it prints `F0001 CD022261D` on the
  UART: the frame number, then the CRC-32 the design computed over the pixels it sent (a parallel
  CRC, 24 bits per pixel clock). The emulator decodes the pads, and each decoded frame's CRC must
  equal the line the design printed for it.

nextpnr: `hdmi_bars` and `hdmi_moving` pass at 125 and 25 MHz (`hdmi_moving` 244 and 79 MHz after
routing); `hdmi_moving` is 929 LUT4, 259 FF, 4 SIOLOGIC, one PLL.

### Checks

- `cargo test --release --lib peripherals` (no toolchain needed). Pad rows are made in Rust from
  pictures with the 640×480 CEA timing (800×525, negative syncs), two rows per bit as a DDR
  serializer gives, through the encoder of tmds.rs. Cases:
  - the colour bars, exact, with the zlib CRC (`crc32(b"123456789") = CBF43926`) and the timing;
  - a start mid-frame at an odd bit offset;
  - two different frames in a row;
  - a gap, after which the next frame is exact;
  - HDMI preambles, guard bands and a data island;
  - DVI pixels 0x55/0xAB;
  - noise, constant and X lanes, which never lock.
- [tests/hdmi.rs](../tests/hdmi.rs) (`--ignored`: clang, Yosys/nextpnr and the Trellis database)
  builds both designs and runs 1.3 million cycles in fast mode with the trace.
  - Every decoded frame is 640×480, 800×525, 25 MHz in emulated time, syncs 96 pixels and 2
    lines, both negative.
  - `hdmi_bars`: the frame equals the expected bars byte for byte, CRC 2EB122D9.
  - `hdmi_moving`: frames 1 and 2 differ, and their decoded CRCs D022261D and ADBFD61C are the
    ones the design printed (`F0001 CD022261D`, `F0002 CADBFD61C`; frame 0 started before the
    receiver locked).
- On the board ([HR04](inventory/LFE5U-85F/hr04_hdmi_oled/README.md)), `hdmi_moving` printed
  the emulator's line for every frame it reported: frames 4–243, three loads, 719 lines. That
  checks the design's logic, PLL and CRC on the board. The board's pads were not decoded: that
  needs a TMDS capture or a camera on a monitor.

### Speed

The emulator does not run at the real pixel rate, so the receiver decodes at the rate the
design runs and reports times in emulated time. Apple M3 Max, 2026-11-16, tests/hdmi.rs:

| Design | Pixel clocks per second | Seconds per frame | Share of real time |
|---|---:|---:|---:|
| `hdmi_bars` | 2.2 M | 0.19 | 8.8% |
| `hdmi_moving` | 0.7–1.2 M | 0.36–0.59 | 2.8–4.6% |

The receiver takes 7–14 ns per row of four pads, five rows per board-clock edge.

### In the window

The live runner ([src/live.rs](../src/live.rs)) holds an `Attached` for the design's pins:
- It traces the peripherals' balls after every instant and feeds them after each run of edges
  and each input. While a design is being traced, the runner does not tune (`FastSim::tune`).
- It publishes `Attached::view` in `Shared::peripherals` with everything else.
- Each checkpoint of the way back keeps the OLED's controller and RAM and the SD card's state.
  Going back restores them and replays the run's pads into them. The HDMI receiver locks again
  instead, and its next frame is the first shown.
- A peripheral that answers drives a design input. The SD card drives MISO in SPI mode,
  or CMD/DAT through external drivers on native bidirectional pads.
  - `Attached::horizon` bounds the edges of each run: one while the card is selected, since it
    answers after each SCLK falling edge and before the next rising one; SPI idle uses 64,
    native idle 16 (a 48-bit command is longer than an idle chunk).
    SDRAM keeps a horizon of one. While answering every edge, the runner reuses slots and buffers
    with `FastSim::run_edges_with` in batches of at most 1024 edges, checking commands, inputs,
    pacing, checkpoints and publishing between batches. Going back uses the same answer path.
  - After each run the runner feeds the pads, then gives what `respond` returns 1 ps after the
    instant it reached.
  - These inputs are not in the way back's log, as buttons and keys are: going back restores the
    card, and it answers the same again.
  - A design that clocks SCLK from a PLL faster than the board clock would see the answer up to
    an edge late. That case is not handled.
- [tests/live_peripherals.rs](../tests/live_peripherals.rs) runs `hdmi_moving`, `oled_pattern`,
  `sd_read` and `sd_native_read` there. Native SD replays its state, pad answers and UART after
  rewinding before initialization. Every frame shown carries a CRC the design printed, also after going back
  500 000 cycles and running on. `sd_read` prints the demo card's OCR and boot sector, the same
  line again after going back to before its read.

The window ([gui/src/monitor_view.rs](../gui/src/monitor_view.rs)) opens the *OLED (SSD1331)*
when first driven and the *Monitor (HDMI)* once its first DVI frame decodes; control-only
GPDI lanes stay closed until opened manually, then show no-frame, symbol-lock and invalid-symbol diagnostics.
Closed panels can be reopened from **Peripherals**, next to **Design**, or with
`panel NAME open|close` in a window script (`monitor`, `oled`, `sd`, `flash`, `rtc`,
`audio`, `adc`, `sdram`, `header`); entries are enabled once their model is seen for
the current design, and an unavailable scripted panel is a no-op with an error.
EDID is part of the Monitor window.

- The monitor shows the last frame, its mode in emulated time, the frames received, and how fast
  frames come here against the board's rate.
- An *SDRAM* window shows the chip's mode, data beats read and written, open rows, timing
  violations and the first messages about them.
- A *SPI flash* window shows JEDEC, the last command/address, bytes read, rejected writes,
  completed programs/erases and protected/busy counts. Configuration-area attempts add a
  visible warning and a `# flash warning: …` record note with the first event and count.
  Its image is `ECP5_FLASH_IMAGE`, else the XIP demo described in [PERIPHERALS_FLASH.md](PERIPHERALS_FLASH.md).
- An *RTC (MCP7940N)* window shows date/time, ST/OSCRUN and last access/counts
  ([PERIPHERALS_RTC.md](PERIPHERALS_RTC.md)); its private image is `ECP5_RTC_IMAGE`.
- An *ADC (MAX11125)* window shows eight voltages/codes, with sliders on undriven header pads;
  voltage inputs are checkpointed and replayed ([PERIPHERALS_ADC.md](PERIPHERALS_ADC.md)).
- An *SD card* window shows SPI or native 1/4-bit mode, state, RCA, bus width, command/response
  counts, DAT3 pull-up, last command, blocks read and written, and
  the errors it answered. The card's image is the file `ECP5_SD_IMAGE` names, else a demo card
  with a boot sector (`SdCard::demo`).
- The 3D board shows the OLED picture on its module and an HDMI cable with the mode
  ([gui/src/board_view.rs](../gui/src/board_view.rs) `set_oled`, `set_hdmi`).
- The window's record (`ECP5_GUI_TRACE`, [gui/src/trace.rs](../gui/src/trace.rs)) gets these
  lines, each when a picture is first painted where it can be seen:
  - `# monitor frame N WxH crc C refresh R here F at X,Y WxH`, and `# monitor signal …` when the
    signal changes;
  - `# oled picture … frames N data crc C picture crc P at …`;
  - `# flash JEDEC ID last CMD address A read R write command ignored W programs P erases E protected B busy ignored I config writes C`, when that changes;
  - `# flash warning: …; first event, config_writes=C`, when the configuration warning count changes;
  - `# sd STATE last CMDn ARG read R written W crc errors E illegal I mode MODE card_state STATE rca HHHH width N commands N responses N dat3_pullup 0|1`, when that changes;
  - `# rtc YYYY-MM-DD HH:MM:SS ST S OSCRUN O bus A read|write register R read N written W NACKs K`, on change;
  - `# adc c0 … c7 mode MODE frames N`, when codes change, at most once a second;
  - `# sdram init yes|no reads R writes W violations V refresh shortfalls S`, at most once a
    second while those change.

Checked on 2026-11-16 from the window's own record, with the display awake and unlocked, a script
running each design at full speed:
- `hdmi_moving`, 20 s: frames 1–50 were drawn at 630×472 points, 2.6 frames a second (4.4% of
  the board's 59.52 Hz). Each CRC equals the design's UART line for that frame, from an `emulate
  --fast --console-out` run of the same bitstream: 50 of 50. No window frame took more than
  34 ms.
- `oled_pattern`, 10 s: 257 OLED frames. Each one's data CRC equals the design's UART line for
  that frame: 257 of 257.
- `sd_read`, 12 s: the card went Idle, then Ready after CMD58, then CMD17 with one block read,
  and no CRC error or illegal command.
- `picorv32_sdram`, 40 s: 12.2 M cycles (0.3 MHz, one edge at a time). The SDRAM initialised,
  with about a million data beats read and 131 000 written, 0 timing violations, and no refresh
  shortfall. A later headless rserv comparison on **8ac90ff**, using its default widening cone
  folding up to 12 inputs on both sides, measured 371,565 → 624,491 cycles/s for pass 0
  and 371,397 → 624,015 after going back. The fast tests measured 278,923 → 654,853
  cycles/s for PicoRV32 and 577,617 → 2,822,764 for `sdram_test`. These replace the earlier,
  non-comparable host-loop measurements. See
  [PERIPHERALS_SDRAM.md](PERIPHERALS_SDRAM.md#checks-and-measured-speeds) for wall times,
  the allocation experiment, exact UART counts and replay checks.

### Not modelled

- Electrical TMDS: levels, skew between lanes, jitter, cable and hot plug detect.
  The pad's logic level is the bit. The DDC bus carries the answering RTC and
  [monitor EDID EEPROM at 0x50](PERIPHERALS_EDID.md); the PCA9306's electrical behavior
  is not modelled.
- Pixel-clock tolerances of a real monitor: any stable clock locks.
- HDMI data islands (audio, InfoFrames) are skipped, not decoded.
- The negative pads, `gpdi_dn`, are not read. A design that drives only them (inverted) shows
  no picture.
