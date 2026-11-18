# HR08: the ADC on the board, against the emulator

[adc_read](../../../../examples/ulx3s/adc_read/top.v) reads the ULX3S's own header pins
through the real MAX11125 ADC (U8), as [PERIPHERALS_ADC.md](../../../PERIPHERALS_ADC.md)
describes. It uses only the FPGA's weak pulls on the eight shared analog/header nets,
alternating UP/DOWN from AIN0. **Every AIN pad is an input; the FPGA never drives these
header pins.** Only the ADC SPI signals, LEDs and UART are outputs. No extra voltage
source or other hardware is needed for this check.

## Setup

- **Board:** ULX3S 85K v3.0.8, SRAM loads under the shared board lock on 2026-11-17.
- **Runner:** [../hr04_hdmi_oled/run.py](../hr04_hdmi_oled/run.py), prefix `ADC`, three loads
  of 3 s each at `06:54:58Z`, `06:55:07Z` and `06:55:16Z`.
- **Configuration:** [adc_read.config](adc_read.config), a copy of
  `target/reference/adc_read/top.config`, built on the Mac with `examples/ulx3s/build.sh`
  (nextpnr seed 1). The loaded `top.bit` is not stored; its hash is in [SHA256SUMS](SHA256SUMS).
- **Records:** [observations.log](observations.log), copied verbatim from
  `out/adc/board_adc.log`; [TOOLS.txt](TOOLS.txt); [SHA256SUMS](SHA256SUMS).
  The log holds ADC codes only and is stored with this record.
- **Prediction:** fast mode in [tests/adc.rs](../../../../tests/adc.rs) prints:

```text
ADC 0FFF 0000 0FFF 0000 0FFF 0000 0FFF 0000
```

## Observations

The runner captured 15 complete `ADC` lines per load, 45 total. The table covers all 45
samples per channel; codes are decimal, and means are rounded to one decimal place.

| Channel | Pull | min | max | mean | max \|code − emulator\| |
|---|---|---:|---:|---:|---:|
| AIN0 | UP | 4087 | 4095 | 4093.8 | 8 |
| AIN1 | DOWN | 0 | 0 | 0.0 | 0 |
| AIN2 | UP | 4065 | 4088 | 4076.4 | 30 |
| AIN3 | DOWN | 0 | 0 | 0.0 | 0 |
| AIN4 | UP | 4086 | 4095 | 4093.2 | 9 |
| AIN5 | DOWN | 0 | 0 | 0.0 | 0 |
| AIN6 | UP | 4087 | 4095 | 4093.5 | 8 |
| AIN7 | DOWN | 0 | 1 | 0.0 | 1 |

Every board code is within 30 codes of the emulator's. Apart from AIN2, all channels
are within 9 codes of their rail. AIN2 is ball P16, net GN15; it reads up to 30 below
4095. The cause is unmeasured. Leakage on that header net is one possible explanation.

## The comparison

The rule for this board record is **every board code within 32 codes (0.8% of full scale)
of the emulator's corresponding channel**. This is the measured maximum of 30, rounded
up to 32; it is not a datasheet accuracy figure or a guaranteed bound for other boards.
All 360 channel readings meet it. It replaces the a-priori 256-code allowance.

The emulator models a pull as its rail, giving exact 0 or 4095. The pull's settling
against the ADC's sampling capacitor and the net's leakage are not modelled. AIN2 shows
that these analog effects matter at the 1% level, although their individual contributions
have not been measured. The raw measured codes stay in the record; they are not replaced
with ideal endpoints, and the emulator is not fitted to this board's offsets.

[tests/adc.rs](../../../../tests/adc.rs) `the_board_codes_are_within_the_measured_bound`
parses the stored observations, runs `adc_read` in fast mode with the answering ADC,
and compares every board channel with the first complete emulator line. It prints the
largest differences for AIN0..AIN7: `[8, 0, 30, 0, 9, 0, 8, 1]`.

```sh
cargo test --release --test adc -- --ignored --nocapture
```

Checked on 2026-11-17 on rserv with default test parallelism: `3 passed`, including all
45 board lines (360 channel readings) within 32 codes. `cargo fmt --all -- --check` and
workspace clippy with `--all-targets -- -D warnings` also passed.
