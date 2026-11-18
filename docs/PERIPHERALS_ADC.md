# ULX3S ADC

The answering model in [adc.rs](../src/peripherals/adc.rs) is U8, a MAX11125, eight channels
and 12 bits. It takes resolved pad samples in pure Rust, like the SPI flash. The source is the
[MAX11120–MAX11128 datasheet, Rev 5](https://www.analog.com/media/en/technical-documentation/data-sheets/MAX11120-MAX11128.pdf),
particularly the pinout on pp.15–16, Figure 2, and Tables 1–13. Wiring comes from
[the v3.0.8 PCB](../assets/ulx3s/ulx3s_v3.0.8.kicad_pcb), U8 pads through nets to U1 balls;
[board.rs](../src/board.rs) keeps those constants and their evidence.

## Balls and reference

The datasheet's AIN numbering matters: AIN0 is U8 pad **26**, not pad 27. Each header P/N
pair is reversed relative to an ordering that starts with GP14.

| Datasheet pin | U8 pad | PCB net | FPGA ball |
|---|---:|---|---|
| AIN0 | 26 | GN14 | U17 |
| AIN1 | 27 | GP14 | U18 |
| AIN2 | 28 | GN15 | P16 |
| AIN3 | 1 | GP15 | N17 |
| AIN4 | 2 | GN16 | M17 |
| AIN5 | 3 | GP16 | N16 |
| AIN6 | 4 | GN17 | L17 |
| AIN7 | 5 | GP17 | L16 |
| SCLK | 19 | ADC_SCLK | P17 |
| CS | 20 | ADC_CSn | R17 |
| DIN | 21 | ADC_MOSI | R16 |
| DOUT | 24 | ADC_MISO | U16 (FPGA input) |

U8 VDD (17,18) and REF+ (15) are on `/analog/ADC3V3`; REF- (13) is on GND.
The reference is **external**, nominally 3.3 V; this part does not supply an internal 3.3 V
reference. The model assumes exact 3.3 V for both the reference and FPGA high levels.

## Sampling and registers

`Adc::new()` starts with floating inputs at 0 V. `feed(time_ps, Pins { cs, clk, mosi, ain })`
takes 0/1/X/Z as 0/1/2/3. Resolved 0 and 1 mean 0 and 3.3 V, whether from a strong FPGA
drive or only a PULLMODE DOWN/UP. Floating or unknown uses `set_default_volts(channel, volts)`
(default 0 V). `set_volts(channel, volts)` overrides any pad value, including a pull or drive.
Both reject non-finite values and channels outside 0..7; finite voltages outside the reference
range are accepted and the result clamps. `Adc::from_env()` also applies
`ECP5_ADC_VOLTS="0=1.65,3=0.825,7=3.3"`; malformed entries are ignored individually.

SPI is **mode 3**, SCLK idle high, 16 clocks per CS assertion, MSB first. DIN is sampled on
rising SCLK; DOUT changes on falling SCLK and holds through rising SCLK. Falling CS samples
the voltage using the previously selected channel and settings. The command received in that
frame selects the channel/settings for the next frame. Short frames or unknown DIN/SCLK discard
the instruction. Extra clocks do not repeat the instruction. DOUT is Z when CS is high or
unknown, and after an extra falling clock beyond the word.

Implemented register identifiers are the family map, bits 15..11:

- ADC Mode Control: bit 15 zero; SCAN bits 14..11, CHSEL 10..7, RESET 6..5, PM 4..3,
  CHAN_ID 2, SWCNV 1. Manual (`SCAN=1`), Standard_Ext (`4`, 0..N), Upper_Ext (`6`, N..7),
  and Custom_Ext (`8`, selected channels ascending, wrapping) convert at each falling CS.
  SCAN=0 keeps the sequence, with the Table 3 exception for RESET/PM/SWCNV writes;
  reserved scan codes keep the previous mode. RESET=2 restores registers, leaving external
  voltage sources and diagnostic counts intact; RESET=1 only concerns the unimplemented FIFO.
- ADC Configuration: `10000` (`0x8000`); stores REFSEL, AVGON/NAVG, NSCAN, SPM and ECHO.
  REFSEL has no numerical effect here because the board grounds REF-. Nonzero SPM suppresses
  conversion. ECHO returns configuration instructions in the following frame. A configuration
  register write invalidates conversions until another mode instruction, as on p.34.
- Unipolar: `10001` (`0x8800`); per-pair UCH bits 10..7 select even-minus-odd differential
  input, and PDIFF_COM (bit 2) refers individual inputs to REF-, board ground.
- Bipolar: `10010` (`0x9000`); BCH bits 10..7 select signed differential pairs. UCH takes
  precedence, and PDIFF_COM takes precedence over both, following Table 9.
- RANGE: `10011` (`0x9800`); per-pair bits 10..7 select bipolar ±VREF/2 (0) or ±VREF (1).
- Custom Scan1: `10101` (`0xA800`); CHSCAN7..0 in bits 10..3. Custom Scan0 concerns channels
  8..15 and is unsupported on this eight-channel model.

Unipolar code = `round(V / 3.3 * 4095)`, clamped to 0..4095. Bipolar uses a signed 12-bit
value, `round((Veven - Vodd) / span * 4096)`, clamped to -2048..2047, with span 3.3 or
6.6 V; the output is two's complement. With CHAN_ID, DOUT is `(channel << 12) | code`;
without it, DOUT is `code << 3` (one leading and three trailing zeros), not a right-aligned code.

`status()` gives current source volts, each channel's last converted code (`None` before any
conversion), which header pads the design can drive, mode/configuration/range registers,
completed 16-clock frames, and unsupported-frame count. Codes change at the conversion's
falling CS. It does not fabricate conversions on the other channels.

## Runner and window

`Attached` attaches when SCLK, CS and MOSI are outputs and MISO receives. It traces those
three SPI outputs and all configured AIN pads, including inputs with only pulls. An AIN pad
absent from the configured design is treated as floating, without compiling an inactive PIO.
MISO is an answering input. The runner resolves released DOUT to logical idle high, like SD
and flash MISO (the reference host enables a pull-up); fast mode pure inputs accept only
Boolean levels. This idle-level assumption does not affect selected data bits. Its horizon is one edge while selected, 64 otherwise; the runner
observes every edge even in idle batches, so a short transaction cannot pass unseen. Like the
other answering peripherals this supports SCLK derived from the board clock, not a faster PLL
SPI clock. No executor changes are required.

`View.adc` supplies the *ADC (MAX11125)* window: eight volts/codes, mode and frames. A channel
whose pad cannot drive gets a 0..3.3 V slider. It sends `live::Command::AdcVolts(channel, volts)`.
That external input is recorded alongside buttons and keys in the way back. Checkpoints clone
the complete ADC protocol and source state; both seeking and running forward replay voltage
inputs. The ADC then regenerates its MISO answers, which are not themselves logged inputs.
The window record gets `# adc c0 … c7 mode MODE frames N` when codes change, at most once per
wall-clock second; `----` means not converted yet.

## Reference design and checks

[adc_read/top.v](../examples/ulx3s/adc_read/top.v) is a plain Verilog manual-mode SPI host,
390.625 kHz (25 MHz / 64), with CHAN_ID enabled. It waits over 2 ms at boot, configures
single-ended operation, primes the channel pipeline, then reads 0..7. Every 5,000,000 clocks
(5 Hz) it prints four hex digits per 12-bit code on `ftdi_rxd` (L4), 115200 8N1:

```text
ADC 0FFF 0000 0FFF 0000 0FFF 0000 0FFF 0000
```

`led[7:0] = channel0[11:4]`. **All eight AIN balls are inputs with PULLMODE only**, alternating
UP/DOWN in datasheet channel order, starting UP on AIN0. Kept input buffers retain those pulls;
there is no FPGA output or bidirectional driver on any header pin. The only outputs are ADC
SCLK/CS/DIN, LEDs and UART. It needs no cable, sensor, jumper or external voltage source.

Run from the repository root, with the toolchain from
[docs/TOOLCHAIN.md](TOOLCHAIN.md) installed. Set `ECP5_IMAGE_READER` to the native
image reader and `ECP5_DB` to the Trellis database:

```sh
cargo test --release --lib
cargo test --release --test adc -- --ignored --nocapture
cargo test --release --test live_peripherals -- --ignored --test-threads=1
```

Unit tests cover framing and edges, delayed channel selection, both output formats, pulls and
floating defaults, overrides/parser/clamping, differential/range configuration, power suppression,
reset, external scans, echo, unknown/short/extra clocks, and a checkpoint mid-frame.
[tests/adc.rs](../tests/adc.rs) builds and routes the real host, asserts every AIN is an input
and every output is allowed, and checks two complete UART lines, their 5 Hz period, status and
LEDs. A second test overrides channels, including clipping, through `set_volts`.
[tests/live_peripherals.rs](../tests/live_peripherals.rs) changes channel 0 to 1.65 V while
paused, checks the next UART line, goes back before the input, seeks across it, then replays
with Step to compare ADC state, UART and LEDs exactly.

Checked on 2026-11-17 on rserv, with the two fast tests at default parallelism: the pull
test reached two complete lines in 5,179,904 cycles, 1.223 s (4,234,430 cycles/s); the
override test took 1.227 s (4,223,104 cycles/s). Both lines matched exactly and their
spacing was 5,000,000 cycles. The live check also replayed 40 slider moves made while
paused, without advancing any board-clock edge during those moves.

### Board check: HR08

[HR08](inventory/LFE5U-85F/hr08_adc/README.md) records the 2026-11-17 run on the ULX3S
85K v3.0.8: three SRAM loads of 3 s each, 45 complete `ADC` lines. The design reads its
own header nets through the real ADC using input-only weak pulls, never driving them.
Every measured code is within 30 of the emulator's corresponding 0/4095 prediction.
The largest differences on AIN0..AIN7 are `[8, 0, 30, 0, 9, 0, 8, 1]`; HR08 stores the
raw observations, configuration, tool versions and hashes, and gives the per-channel table.

The comparison rule is now **every board code within 32 codes (0.8% of full scale) of the
emulator's**. It replaces the a-priori 256-code allowance: this board's measured maximum
of 30 is rounded up to 32. It is a measured bound for this board and record, not a
MAX11125 datasheet accuracy figure or a guaranteed tolerance for other boards.
[tests/adc.rs](../tests/adc.rs) `the_board_codes_are_within_the_measured_bound` parses all
45 stored lines, runs the design in fast mode, compares each channel with the first emulator
line and prints the largest difference per channel.

The emulator models a pull as the rail. The pull's settling against the ADC's sampling
capacitor and the net's leakage are not modelled. All channels except AIN2 stay within 9
codes of the rail; AIN2 (P16, GN15) reads up to 30 below it. Its cause is unmeasured;
leakage on that header net is one possibility. AIN2 shows that the analog path matters at
the 1% level. Keep the measured codes in the record rather than replacing them with ideal
endpoints or fitting the emulator to their offsets.

## Assumptions and limits

- The unipolar endpoint formula is the requested 4095 scaling; the silicon defines 1 LSB as
  VREF/4096, so some intermediate ideal codes can differ by one. No noise or gain/offset error.
- Rails/reference are exactly 3.3 V, and all Z/X analog pads use their settable default.
  There is no weak-pull resistance, acquisition droop, capacitance or settling simulation.
- Conversion is instantaneous at falling CS; the first/invalid result is deterministic zero.
  No power-up, auto-shutdown/standby wake-up, SPI pulse-width or propagation delays are enforced.
  PM is stored but its analog power timing is not simulated. The reference design discards
  startup results and leaves a full SCLK period high between CS assertions.
- The logical no-ID output always supplies all three trailing zeros. The hardware releases
  DOUT near the end of that word (timing table: 14th falling edge); electrical release timing
  within those padding bits is not simulated.
- Assumption for ECHO: configuration instructions are echoed one frame later, while mode
  instructions resume conversion rather than being echoed. Echo startup latency is idealised.
- Assumption for long frames: only the first 16 clocks write an instruction; further clocks
  under the same CS assertion do not repeat it. Short or unknown frames discard the instruction.
- Assumption for differential CHAN_ID: it reports the requested member of the pair, while both
  members convert even minus odd as Table 9 specifies.
- Internal-clock modes (Repeat, Standard_Int, Upper_Int, Custom_Int), SWCNV/CNVST, FIFO,
  averaging, EOC and SampleSet sequences are unimplemented. Unsupported conversions return zero
  and increment the diagnostic count. This is not an approximation of their timing.
