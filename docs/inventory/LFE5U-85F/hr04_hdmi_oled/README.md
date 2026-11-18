# HR04: hdmi_moving and oled_pattern on the board, against the emulator

Two reference designs of [examples/ulx3s](../../../../examples/ulx3s/README.md) that drive the
ULX3S's peripherals ([docs/PERIPHERALS.md](../../../PERIPHERALS.md)). They ran on the board on
2026-11-16, in the HR series of whole designs compared with the emulator (see
[HR01](../hr01_picorv32_console/README.md)). Each one prints the CRC-32 of every frame it sends:
- [hdmi_moving](../../../../examples/ulx3s/hdmi_moving/top.v): 640×480 DVI on the GPDI connector,
  with a PLL, a 125 MHz shift clock and `ODDRX1F` per lane. It prints `F<frame> C<CRC>` over the
  RGB bytes of the frame's pixels.
- [oled_pattern](../../../../examples/ulx3s/oled_pattern/top.v): the SSD1331 OLED over 6.25 MHz
  SPI. It prints `O<frame> C<CRC>` over the data bytes it sent for the frame.

The pictures change every frame, so each line is a different number.

## What this checks, and what not

- **Checked: the logic on the board against the emulator.** The board prints the same CRC for the
  same frame number as `emulate --fast`. This covers the PLL's two outputs, the TMDS encoders,
  the pixel counters and the parallel CRC.
- **Checked, separately: the emulator's peripherals against the designs.** In the emulator, the
  HDMI receiver decodes the GPDI pads into frames, and the OLED model decodes the SPI pads. Each
  decoded frame's CRC equals the design's line for it (tests/hdmi.rs, tests/oled.rs,
  tests/live_peripherals.rs, and the window's record, docs/PERIPHERALS.md).
- **Not checked: the board's pads.** That needs a TMDS capture or a camera on a monitor, and a
  look at the OLED. No monitor or OLED was observed in this run.

## Setup

- **Board:** ULX3S "85K v3.0.8" (LFE5U-85), CABGA381. Loaded over JTAG into SRAM with
  `openFPGALoader -b ulx3s`; the flash was not written. The shared board lock
  (`out/board.lock` in the main folder) was held for the whole run.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Configurations:** [hdmi_moving.config](hdmi_moving.config) and
  [oled_pattern.config](oled_pattern.config) are the configurations that ran:
  `examples/ulx3s/build.sh`, nextpnr seed 1. The window's record of docs/PERIPHERALS.md is of
  the same bitstreams.
- **Runner:** `run.py BITSTREAM PREFIX 3 4` ([run.py](run.py)) loads the bitstream three times.
  After each load it reads the serial port for 4 s and prints every line that starts with PREFIX,
  in order. The port opens after the load, so the first frames come before it. The line at either
  edge of the window can be cut.
- **Prediction:** before the board run, `emulate --fast --console-out net:ftdi_rxd` printed the
  lines of [emulator_hdmi_moving.txt](emulator_hdmi_moving.txt) (frames 0–400) and
  [emulator_oled_pattern.txt](emulator_oled_pattern.txt) (frames 0–320) for these configurations.

## Observations

[observations.log](observations.log); every line compared by frame number with the prediction:

| Design | Load | Frames | Lines equal to the emulator's | Different |
|---|---:|---|---:|---:|
| hdmi_moving | 1 | 4–242 | 239 | 0 |
| hdmi_moving | 2 | 4–243 | 240 | 0 |
| hdmi_moving | 3 | 4–243 | 240 | 0 |
| oled_pattern | 1 | 3–219 | 217 | 0 |
| oled_pattern | 2 | 3–219 | 217 | 0 |
| oled_pattern | 3 | 3–219 | 217 | 0 |

The board sends 240 HDMI frames in 4 s, the design's 59.52 Hz. Every hdmi_moving window and the
first oled_pattern window also ended in a cut line, which is not counted.
