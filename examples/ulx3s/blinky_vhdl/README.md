# VHDL counter

An initialized 8-bit `numeric_std` unsigned counter on `clk_25mhz` drives `led`,
like `blinky_cycle`. Build with `ecp5_emu build examples/ulx3s/blinky_vhdl`, then
open the project in the GUI to emulate its LEDs and watch signals in Waves.

Included in v0.1.0: VHDL-2008, library `work`, one elaborated entity, VHDL-only
roots in declared analysis order. The GHDL Yosys plugin and Yosys come from OSS
CAD Suite 2026-10-22; nextpnr/ecppack remain the default tools. Set
`ECP5_OSS_CAD_SUITE` to the suite root, or run `source <suite>/environment` so
the suite's Yosys is on PATH. The root must include `bin/yosys`, `libexec/yosys`
and `share/yosys/plugins/ghdl.so` for PATH discovery.
Existing `GHDL_PREFIX` is respected; otherwise only the synthesis child receives
`GHDL_PREFIX` pointing at the suite's `lib/ghdl`.

Source probing is not available for VHDL yet: the GHDL plugin writes no source locations.
Build, routing, bitstream generation and LED emulation work on Mac and Linux;
[measured evidence](../../../docs/designs/evidence/hdl-suite-check-2026-11-18.md)
shows why source probing cannot be offered. No cross-language VHDL hierarchy.
