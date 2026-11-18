# SystemVerilog counter

An 8-bit counter using `logic` and `always_ff`, driven by `clk_25mhz` and
shown on `led`. Each rising edge increments the LEDs, like `blinky_cycle`.
Build with `ecp5_emu build examples/ulx3s/blinky_sv`, then open the project
in the GUI to emulate single cycles and probe the original `top.sv` source.

`sources.hdl` opts into ordered compilation by extension. The default
`synthesis.sv_frontend = "auto"` uses the installed Yosys built-in reader;
`builtin` pins that reader. Verilog and the measured SystemVerilog subset
below are supported. VHDL through the GHDL plugin and an explicit `sv_frontend = "slang"` pin are
also supported in v0.1.0 using OSS CAD Suite. Auto never selects slang. See
[blinky_vhdl](../blinky_vhdl/README.md) for the VHDL source-probing limit and
[sv_package](../sv_package/README.md) for package imports.

For headers, set `sources.include_dirs` (default `["."]`) and list each header
in `sources.files`. Built-in Yosys requires include directory names containing
only ASCII letters, digits and `_./-`; its `-Idir` option does not unquote paths.

The following subset was measured on the Mac with Yosys 0.70+post using
`read_verilog -sv FILE; synth_ecp5 -top top -json FILE.json`, as recorded in
[the HDL language design](../../../docs/designs/hdl-languages.md#3-the-measured-built-in-systemverilog-subset).
Pass means parsing and synthesis, not simulated behaviour or general language
conformance. Unmeasured constructs are outside this subset.

| Construct | Measured result |
| --- | --- |
| `logic`, `always_ff`, `always_comb` | Pass. |
| `always_latch` | Pass. |
| `typedef enum logic [1:0]` and case | Pass. |
| Packed struct and field access | Pass. |
| Packed union containing a packed struct | Pass. |
| Instantiated interface/modport passed to `bus_if.src` submodule port | Pass. |
| `unique case` | Pass. |
| `.*` port connections | Pass. |
| Immediate assertion inside `always_ff` | Accepted; assertion synthesis meaning was not checked. |
| Package localparam referenced as `pkg::W` | Pass. |
| `import pkg::*;` inside module | Fail: unexpected `TOK_IMPORT`. |
| File-scope `import pkg::*;` | Fail: unexpected `;`, expecting `(` or `[`. |
| Module-header import | Fail: unexpected `TOK_IMPORT`. |
| Function `return x + 1;`, in module or package | Fail: unexpected `TOK_ID`. |
| Typedef return type, assigning the function name | Pass. |
