<a id="measured-on-the-mac-2026-11-17-flow-session-yosys-070post-homebrew-git-0c11ea098"></a>

# Measured on the Mac, 2026-11-17, Yosys 0.70+post (Homebrew, git 0c11ea098)
Each construct as its own small top, `yosys -q -p "read_verilog -sv F.sv; synth_ecp5 -top top -json F.json"`:

| Construct | read_verilog -sv |
|---|---|
| logic, always_ff, always_comb | OK |
| always_latch | OK |
| typedef enum logic [1:0] {…} + case on it | OK |
| typedef struct packed, field access | OK |
| typedef union packed (with a nested packed struct) | OK |
| interface + modport, instantiated, passed to a submodule port `bus_if.src b` | OK |
| unique case | OK |
| `.*` port connections | OK |
| immediate `assert` inside always_ff | OK (accepted; not checked what it synthesises to) |
| package with localparam, referenced as `pkg::W` | OK |
| `import pkg::*;` in the module body | FAIL: syntax error, unexpected TOK_IMPORT |
| `import pkg::*;` at file scope | FAIL: syntax error, unexpected ';', expecting '(' or '[' |
| `module top import pkg::*; (…)` (header import) | FAIL: syntax error, unexpected TOK_IMPORT |
| function with `return x + 1;` (in a module or a package) | FAIL: syntax error, unexpected TOK_ID |
| function with a typedef'd return type, body `inc = x + 1;` | OK |
"OK" means the read and synth_ecp5 passed; behaviour was not simulated.

src attributes: the JSON of the -sv designs carries `"src": "enum.sv:1.1-6.10"` and the like, the source file:line.col, as for plain Verilog (cross-probing keeps working).

GHDL: /opt/homebrew/bin/ghdl -> Homebrew cask ghdl 6.0.0 (ghdl-llvm-6.0.0-macos15-aarch64). It does not start:
`dyld: Library not loaded: @rpath/libgcc_s.1.1.dylib … code signature in … libgcc_s.1.1.dylib not valid for use in process: library load disallowed by system policy` (`ghdl --version` exits 134; `ghdl synth` 137).
No yosys plugins directory (/opt/homebrew/Cellar/yosys/0.70/share/yosys/plugins does not exist), so no ghdl.so / slang.so.
rserv: Yosys 0.70, no ghdl, no plugins.
No VHDL path works on either machine as installed today. Fixing the Mac's GHDL requires a security-setting change (clearing the cask's quarantine or re-signing), or another install.
