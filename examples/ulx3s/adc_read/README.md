# ADC read without extra hardware

[top.v](top.v) reads U8 (MAX11125), channels 0..7 in manual mode, and prints
`ADC 0FFF 0000 0FFF 0000 0FFF 0000 0FFF 0000` in the emulator at 115200 8N1 on L4,
five times a second. LED[7:0] shows channel 0's top eight bits.

All eight analog/header pads are **inputs**, with alternating weak UP/DOWN pulls in
[top.lpf](top.lpf). The FPGA never drives them. Leave the shared header nets unconnected
for the future board check: real codes should be near the rails, within 256 codes.
The datasheet numbers AIN0 at U8 pad 26 (GN14/U17), not GP14/U18.

Build from the repository root, with the toolchain from
[docs/TOOLCHAIN.md](../../../docs/TOOLCHAIN.md) installed. Set `ECP5_IMAGE_READER` to the native
image reader and `ECP5_DB` to the Trellis database:

```sh
examples/ulx3s/build.sh examples/ulx3s/adc_read target/reference/adc_read
cargo test --release --test adc -- --ignored --nocapture
```

See [PERIPHERALS_ADC.md](../../../docs/PERIPHERALS_ADC.md) for the wiring evidence,
frame pipeline, emulator assumptions and the measured-code board record to come.
