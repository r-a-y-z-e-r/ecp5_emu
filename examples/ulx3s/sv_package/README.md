# SystemVerilog package import

The 8-bit LED counter uses a package and module-scoped `import pkg::*;`.
Yosys's built-in SystemVerilog reader rejects this with `unexpected TOK_IMPORT`.
The explicit `[synthesis] sv_frontend = "slang"` pin selects yosys-slang from
OSS CAD Suite 2026-10-22. This frontend is included in v0.1.0.

Build with `ecp5_emu build examples/ulx3s/sv_package`. Set `ECP5_OSS_CAD_SUITE`
to the suite root, or run `source <suite>/environment` so the suite's Yosys is
on PATH. PATH discovery requires `bin/yosys`, `libexec/yosys` and
`share/yosys/plugins/slang.so` in that root. Only synthesis uses suite Yosys;
nextpnr/ecppack remain the default tools. Auto and builtin
always use the built-in reader; slang is never selected automatically and a
failed slang compile is never retried. Routed source locations point at
`top.sv`, and the emulator counts on the LEDs as in `blinky_cycle`.
