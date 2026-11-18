# Audio tone

Three output-only four-bit DAC buses: L 440 Hz, R 660 Hz, V held at code 8.
The table is limited to codes **4..12**, not full scale. Samples update at exactly
25 kHz from the 25 MHz clock; LEDs show `{R,L}`. Only DAC pads, LEDs and UART are
outputs. There is no audio playback library or input/answering protocol.

At 115200 8N1, UART L4 prints one line per second:

```text
AUDIO 25000 F1BFDEEF
```

This is the expected first line: decimal sample pairs in that second, then
CRC-32/ISO-HDLC over L,R code bytes, zero-extended, in sample order. Continuous
phase means later seconds can have other CRCs. The emulator test also computes
the CRC from the traced DAC pads, independently of the Verilog checksum logic.

Build with `examples/ulx3s/build.sh examples/ulx3s/audio_tone target/reference/audio_tone`.
The emulator's `--fast --peripherals` mode and the live window show three modelled
voltage traces. [Model, assumptions and board method](../../../docs/PERIPHERALS_AUDIO.md):
all voltages assume the jack is unplugged. Headphones load the resistor ladder;
the emulator's open-circuit predictions are not measured levels or loaded levels.
No board or analog check has been performed here. UART agreement would check only
the digital stream; analog levels need an instrument or an ADC cable.
