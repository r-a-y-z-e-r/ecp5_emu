# ULX3S DIP switch inputs

`sw[0..3]` is SW1-SW4 on E8/D8/D7/E7, sw[0] first in `SW xxxx` and the leftmost
lever in the 3D model. ON is toward the board's top edge: it connects the net through
R51 (130 Ohm) to the bank supply, reading 1. OFF floats and takes the FPGA pad's pull.

`top.lpf` uses DOWN on all four input pads. `led[3:0] = sw[3:0]`; the other LEDs
are zero. UART L4 is 115200 8N1. After initial 10 ms settling, the design prints
`SW xxxx`, then after each change stable for about 10 ms, and again every second.
Only LEDs and UART are outputs; no FPGA output drives a switch net.

`top_pullup.lpf` is the same constraints with UP on all four switches. The buildable
`pullup/` variant has those constraints as its `top.lpf` and a **copy** of `../top.v`.
The test checks that both HDL files are identical and only the four pull settings differ.
With UP, every switch reads 1 whether OFF or ON. With DOWN, OFF reads 0 and ON reads 1.
Loading both variants can identify physical positions when nobody is at the board.
No board was accessed while adding this design.

Build either variant from the repository root, with the toolchain from
[docs/TOOLCHAIN.md](../../../docs/TOOLCHAIN.md) installed:

```sh
examples/ulx3s/build.sh examples/ulx3s/switches target/reference/switches
examples/ulx3s/build.sh examples/ulx3s/switches/pullup target/reference/switches_pullup
```

For the live runner, `ECP5_SWITCHES=1010` means SW1/SW3 ON, default all OFF.
`live::Command::Switch(index, on)` changes a position and records it for rewind/replay.
The model resolves OFF to DOWN=0 / UP=1; no or unknown pull uses 0, since fast mode's
pure input pads cannot be set to Z. The physical pin floats without a pull.
See [PERIPHERALS.md](../../../docs/PERIPHERALS.md#dip-switches).
