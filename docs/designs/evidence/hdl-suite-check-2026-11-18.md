# OSS CAD Suite 2026-10-22: two-host HDL measurements, 2026-11-18

Measured checkout: `claude/flow-vhdl-check`, commit `9477b0b8cf2426ff1c4b83866ef17931c27445cc`; initially clean. Read `docs/designs/hdl-languages.md` before probing. This revision has no §9: §5 specifies tool selection, §7 the suite smoke tests, and §8 contains the slice-2 publication gate. This file records manual CLI measurements, not an implementation of the proposed project flow.

Suite executables were called by absolute path. Caller `PATH`, default tools, Trellis installation, security attributes, and signatures were not changed. The suite’s `bin/yosys` and `bin/ghdl` shell wrappers themselves prepend their own `bin/` inside the child process; default nextpnr/ecppack ran outside that process by their resolved absolute paths. No quarantine removal or signing command ran. No commit was created.

Commands ran through a temporary Python harness that captured exit status, stdout and stderr. Tables render the captured argument vectors with the exact path bindings below; `cd` identifies the captured working directory. Key output is quoted; omitted synthesis/routing output is not an additional result. `PASS`/`FAIL` describes the named check, so expected parser rejections and the non-gating identity comparison are explicitly labelled. Source and byte comparisons were also performed in Python, using the fragments printed below.

Remote transport: `~/IDE Workspaces/RustroverProjects/ecp5_emu/tools/rserv.sh` with arguments `bash -c` and an inline Python harness encoded in base64 to preserve quoting. It synced this worktree to `$ROOT`; suite paths inside that command were derived from `$HOME/apps/oss-cad-suite-20261022`. Both remote invocations exited 0. Tables show the actual child commands, not suite downstream replacements.

## Inputs

All four designs used an unchanged copy of `examples/ulx3s/blinky_cycle/top.lpf`: clock G2 at 25 MHz; `led[0..7]` at B2, C2, C1, D2, D1, E2, E1, H3. Copy operation: `shutil.copy2(ROOT / "examples/ulx3s/blinky_cycle/top.lpf", d / "top.lpf")`. No placement seed or other routing option was added to the requested gate command.

`vhdl/top.vhd` (exact source; first line is line 1):

```vhdl
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity top is
  port (clk_25mhz : in std_logic;
        led : out std_logic_vector(7 downto 0));
end entity;

architecture rtl of top is
  signal count : unsigned(7 downto 0) := (others => '0');
begin
  process (clk_25mhz)
  begin
    if rising_edge(clk_25mhz) then
      count <= count + 1;
    end if;
  end process;
  led <= std_logic_vector(count);
end architecture;
```

`slang/import.sv` (exact source). §3 records the module-scoped `import pkg::*;` failure but supplies no source file; this fixture reproduces that tabulated construct in the same initialized 8-bit counter:

```systemverilog
package pkg;
  localparam int W = 8;
endpackage
module top(input logic clk_25mhz, output logic [7:0] led);
  import pkg::*;
  logic [W-1:0] count = '0;
  always_ff @(posedge clk_25mhz) count <= count + 1'b1;
  assign led = count;
endmodule
```

`examples/ulx3s/blinky_sv` is absent. Both `suite_builtin/top.sv` and `default_builtin/top.sv` used this inline equivalent of `blinky_cycle`:

```systemverilog
module top(input logic clk_25mhz, output logic [7:0] led);
  logic [7:0] count = '0;
  always_ff @(posedge clk_25mhz) count <= count + 1'b1;
  assign led = count;
endmodule
```

The extra invalid-input probes used these exact files in Mac `diagnostics/` and the rserv `$AUDIT` directory:

```vhdl
library ieee;
use ieee.std_logic_1164.all;
entity top is
  port (clk_25mhz : in std_logic);
end entity;
architecture rtl of top is
begin
  broken <= ;
end architecture;
```

```systemverilog
module top(input logic clk_25mhz, output logic led);
  assign led = ;
endmodule
```

## Mac

Platform: `macOS-15.6.1-arm64-arm-64bit-Mach-O`. Initial `GHDL_PREFIX` was unset. Exact bindings (the temporary directories were subsequently deleted):

```sh
SUITE=~/FPGA/oss-cad-suite-20261022
ROOT='<scratch worktree>'
WORK='<temp>'
YOSYS=/opt/homebrew/bin/yosys
NEXTPNR=/opt/homebrew/bin/nextpnr-ecp5
ECPPACK=/opt/homebrew/bin/ecppack
CARGO=~/.cargo/bin/cargo
EMU="$WORK/cargo/release/ecp5_emu"
DB=~/FPGA/prjtrellis/database
READER="$HOME/IDE Workspaces/RustroverProjects/ecp5_emu/target/trellis_image_reader"
```

Recorded archive SHA-256, read from `$SUITE.sha256` (record only; no archive rehash or published-checksum comparison):

```text
378e87e60db8cb3d077c63613cbed75e52306cf5ebdfeb95596629f343e536c0  oss-cad-suite-darwin-arm64-20261022.tgz
```

| Check | Command / captured Python operation | Result | Key output |
| --- | --- | --- | --- |
| Suite Yosys version | `cd "$WORK"; "$SUITE/bin/yosys" -V` | PASS | exit 0; `Yosys 0.70+190 (git sha1 485b63564, Release, Clang /usr/local/osxcross/target/bin/aarch64-apple-darwin25.5-clang++ 21.1.8)` |
| Suite GHDL version | `cd "$WORK"; "$SUITE/bin/ghdl" --version` | PASS | exit 0; `GHDL 7.0.0-dev (6.0.0.r504.gdd4f97400) [Dunoon edition]`<br>` Compiled with GNAT Version: 14.2.0`<br>` llvm 23.1.0 code generator` |
| Suite nextpnr version | `cd "$WORK"; "$SUITE/bin/nextpnr-ecp5" --version` | PASS | exit 0; `"nextpnr-ecp5" -- Next Generation Place and Route (Version nextpnr-0.11.1-77-g6b438a1c)` |
| Default pinned nextpnr version | `cd "$WORK"; "$NEXTPNR" --version` | PASS | exit 0; `"nextpnr-ecp5" -- Next Generation Place and Route (Version nextpnr-0.11.1-77-g6b438a1c)` |
| Default Yosys version | `cd "$WORK"; "$YOSYS" -V` | PASS | exit 0; `Yosys 0.70+post (git sha1 0c11ea098be065e585a90a06e7b7da49936cf231, Release, AppleClang clang++ 17.0.0.17000604)` |
| GHDL plugin load | `cd "$WORK"; "$SUITE/bin/yosys" -m ghdl -p 'help ghdl'` | PASS | exit 0; `ghdl [options] unit [arch]`; `ghdl [options] files... -e [unit]`; <code>--std=(93&#124;08)</code> |
| Slang plugin load | `cd "$WORK"; "$SUITE/bin/yosys" -m slang -p 'help read_slang'` | PASS | exit 0; `read_slang [options] [filename]`; `--top <name>`; `-I,--include-directory,+incdir <dir-pattern>[,...]` |
| macOS attribute inspection | `cd "$WORK"; /usr/bin/xattr -lr "$SUITE/bin/yosys" "$SUITE/libexec/yosys" "$SUITE/bin/ghdl" "$SUITE/libexec/ghdl" "$SUITE/share/yosys/plugins/ghdl.so" "$SUITE/share/yosys/plugins/slang.so"` | PASS | exit 0; Six inspected executable/plugin paths reported only `com.apple.provenance: \x01\x02`; no `com.apple.quarantine` shown. No macOS policy/signature error was emitted in these execution probes. |
| VHDL requested synthesis | `cd "$WORK/vhdl"; "$SUITE/bin/yosys" -m ghdl -p 'ghdl --std=08 top.vhd -e top; synth_ecp5 -top top -json top.json'` | FAIL | exit 1; `warning: ieee library directory '../../install/lib/ghdl/ieee/v08/' not found`<br>`error: cannot find "std" library`<br>`ERROR: vhdl import failed.` |
| VHDL synthesis, explicit per-command prefix | `cd "$WORK/vhdl"; /usr/bin/env "GHDL_PREFIX=$SUITE/lib/ghdl" "$SUITE/bin/yosys" -m ghdl -p 'ghdl --std=08 top.vhd -e top; synth_ecp5 -top top -json top.json'` | PASS | exit 0; `Found and reported 0 problems.`; JSON backend completed. This is a separately recorded invocation, not the result of the failed command above. |
| VHDL default nextpnr JSON gate | `cd "$WORK/vhdl"; "$NEXTPNR" --85k --package CABGA381 --lpf top.lpf --json top.json --write top_routed.json --textcfg top.config` | PASS | exit 0; `Info: Routing complete.`; `Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 354.99 MHz (PASS at 25.00 MHz)`; `Info: Program finished normally.` |
| VHDL default ecppack | `cd "$WORK/vhdl"; "$ECPPACK" top.config top.bit` | PASS | exit 0; `(no stdout/stderr)`; top.bit produced. |
| Original VHDL source locations | `cd "$WORK/vhdl"; python3 -c "$SRC_CHECK"` | FAIL | exit 0; `top.json: total_src=1864; top.vhd_src=0; top_routed.json: total_src=21; top.vhd_src=0`; no `top.vhd:line` attribute exists to quote. See exact counterexample below. |
| Built-in reader control (expected rejection) | `cd "$WORK/slang"; "$YOSYS" -p 'read_verilog -sv import.sv; synth_ecp5 -top top -json builtin.json'` | FAIL (expected) | exit 1; `import.sv:5: ERROR: syntax error, unexpected TOK_IMPORT` |
| Slang synthesis | `cd "$WORK/slang"; "$SUITE/bin/yosys" -m slang -p 'read_slang --top top import.sv; synth_ecp5 -top top -json top.json'` | PASS | exit 0; `Found and reported 0 problems.`; `8   TRELLIS_FF`; JSON backend completed. |
| Slang default nextpnr JSON gate | `cd "$WORK/slang"; "$NEXTPNR" --85k --package CABGA381 --lpf top.lpf --json top.json --write top_routed.json --textcfg top.config` | PASS | exit 0; `Info: Routing complete.`; `Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 354.99 MHz (PASS at 25.00 MHz)`; `Info: Program finished normally.` |
| Slang default ecppack | `cd "$WORK/slang"; "$ECPPACK" top.config top.bit` | PASS | exit 0; `(no stdout/stderr)`; top.bit produced. |
| Original SV source locations | `cd "$WORK/slang"; [in-process Python SRC_COLLECTOR below]` | PASS | `netnames["count[0]"].attributes.src = "import.sv:4.54"` |
| Suite built-in SV synthesis | `cd "$WORK/suite_builtin"; "$SUITE/bin/yosys" -p 'read_verilog -sv top.sv; synth_ecp5 -top top -json top.json'` | PASS | exit 0; `Found and reported 0 problems.`; `8   TRELLIS_FF`; JSON backend completed. |
| Suite built-in SV default nextpnr JSON gate | `cd "$WORK/suite_builtin"; "$NEXTPNR" --85k --package CABGA381 --lpf top.lpf --json top.json --write top_routed.json --textcfg top.config` | PASS | exit 0; `Info: Routing complete.`; `Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 354.99 MHz (PASS at 25.00 MHz)`; `Info: Program finished normally.` |
| Suite built-in SV default ecppack | `cd "$WORK/suite_builtin"; "$ECPPACK" top.config top.bit` | PASS | exit 0; `(no stdout/stderr)`; top.bit produced. |
| Default built-in SV synthesis | `cd "$WORK/default_builtin"; "$YOSYS" -p 'read_verilog -sv top.sv; synth_ecp5 -top top -json top.json'` | PASS | exit 0; `Found and reported 0 problems.`; `8   TRELLIS_FF`; JSON backend completed. |
| Default built-in SV default nextpnr JSON gate | `cd "$WORK/default_builtin"; "$NEXTPNR" --85k --package CABGA381 --lpf top.lpf --json top.json --write top_routed.json --textcfg top.config` | PASS | exit 0; `Info: Routing complete.`; `Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 353.98 MHz (PASS at 25.00 MHz)`; `Info: Program finished normally.` |
| Default built-in SV default ecppack | `cd "$WORK/default_builtin"; "$ECPPACK" top.config top.bit` | PASS | exit 0; `(no stdout/stderr)`; top.bit produced. |
| Build measured-checkout emulator | `cd "$ROOT"; "$CARGO" build --locked --release -p ecp5_emu --bin ecp5_emu --target-dir "$WORK/cargo"` | PASS | exit 0; ``Finished `release` profile [optimized] target(s) in 40.79s`` (no source changes). |
| VHDL emulator, 320 rising edges | `cd "$WORK/vhdl"; "$EMU" emulate top.config --names top_routed.json --startup configured --scope observed --clock net:clk_25mhz --probe net:led --period-ns 40 --cycles 320 --architecture "$ROOT/docs/inventory/LFE5U-85F" --db "$DB" --image-reader "$READER" --output emulate.json` | PASS | exit 0; `Executed FF: 8; LUT4: 2; LUTRAM columns: 0; RAMW: 0; carry halves: 10; PIO: 9; routes: 152`; 641 samples, 0 counter mismatches; at 12800 ns led = 64. Raw samples below. |
| Slang emulator, 320 rising edges | `cd "$WORK/slang"; "$EMU" emulate top.config --names top_routed.json --startup configured --scope observed --clock net:clk_25mhz --probe net:led --period-ns 40 --cycles 320 --architecture "$ROOT/docs/inventory/LFE5U-85F" --db "$DB" --image-reader "$READER" --output emulate.json` | PASS | exit 0; `Executed FF: 8; LUT4: 2; LUTRAM columns: 0; RAMW: 0; carry halves: 10; PIO: 9; routes: 152`; 641 samples, 0 counter mismatches; at 12800 ns led = 64. Raw samples below. |
| VHDL invalid-input diagnostics | `cd "$WORK/diagnostics"; /usr/bin/env "GHDL_PREFIX=$SUITE/lib/ghdl" "$SUITE/bin/yosys" -m ghdl -p 'ghdl --std=08 bad.vhd -e top; synth_ecp5 -top top -json bad.json'` | FAIL (expected) | exit 1; `bad.vhd:8:13:error: primary expression expected`<br>`  broken <= ;`<br>`            ^`<br>`ERROR: vhdl import failed.` |
| Slang invalid-input diagnostics | `cd "$WORK/diagnostics"; "$SUITE/bin/yosys" -m slang -p 'read_slang --top top bad.sv; synth_ecp5 -top top -json bad.json'` | FAIL (expected) | exit 1; `bad.sv:2: ERROR: expected expression`<br>`ERROR: Design elaboration failed; see full log for details` |
| Suite/default built-in SV config identity (non-gating) | `[in-process Python COMPARE below, WORK as bound above]` | FAIL (different; non-gating) | `{"default_bytes": 16165, "same_config": false, "suite_bytes": 16141}` |

VHDL provenance counterexample: `top_routed.json` contains only 21 `src` attributes, none naming `top.vhd`. One actual attribute (first found by the full JSON walk):

```text
first src='$SUITE/share/yosys/lattice/arith_map_ccu2c.v:74.7-80.4'
```

Config SHA-256 values from the measured outputs:

| Producer | top.config SHA-256 |
| --- | --- |
| VHDL | `d82e543ee81c9925dbc0ae41cec3796262a76d1c30154b4ef859f436982859ce` |
| Slang | `d82e543ee81c9925dbc0ae41cec3796262a76d1c30154b4ef859f436982859ce` |
| Suite built-in SV | `d82e543ee81c9925dbc0ae41cec3796262a76d1c30154b4ef859f436982859ce` |
| Default built-in SV | `c6e91d637c69a391a6c5a5c3ad6b09889222d94f6e33f2640ba9800a75653bf5` |

## rserv

Platform: `Linux-7.0.0-38-generic-x86_64-with-glibc2.43`. Initial `GHDL_PREFIX` was unset. Exact bindings (the temporary directories were subsequently deleted):

```sh
SUITE=~/apps/oss-cad-suite-20261022
ROOT=~/work/wt-vhdl
WORK='<temp>'
YOSYS=~/apps/fpga/bin/yosys
NEXTPNR=~/apps/fpga/bin/nextpnr-ecp5
ECPPACK=~/FPGA/trellis-install/bin/ecppack
CARGO=~/.cargo/bin/cargo
EMU="$WORK/cargo/release/ecp5_emu"
DB=~/FPGA/prjtrellis/database
READER=~/work/bin/trellis_image_reader
AUDIT='<temp audit>'
```

Recorded archive SHA-256, read from `$SUITE.sha256` (record only; no archive rehash or published-checksum comparison):

```text
45b7401d677f1e6cffb19dcaafb8e92c95a49dfc05b332ae4d00b84c94f62f6e  oss-cad-suite-linux-x64-20261022.tgz
```

| Check | Command / captured Python operation | Result | Key output |
| --- | --- | --- | --- |
| Suite Yosys version | `cd "$WORK"; "$SUITE/bin/yosys" -V` | PASS | exit 0; `Yosys 0.70+190 (git sha1 485b63564, Release, Clang /usr/bin/clang++ 21.1.8)` |
| Suite GHDL version | `cd "$WORK"; "$SUITE/bin/ghdl" --version` | PASS | exit 0; `GHDL 7.0.0-dev (6.0.0.r548.gc966be324.dirty) [Dunoon edition]`<br>` Compiled with GNAT Version: 14.3.0`<br>` llvm 21.1.8 code generator` |
| Suite nextpnr version | `cd "$WORK"; "$SUITE/bin/nextpnr-ecp5" --version` | PASS | exit 0; `"nextpnr-ecp5" -- Next Generation Place and Route (Version nextpnr-0.11.1-77-g6b438a1c)` |
| Default pinned nextpnr version | `cd "$WORK"; "$NEXTPNR" --version` | PASS | exit 0; `"nextpnr-ecp5" -- Next Generation Place and Route (Version nextpnr-0.11.1-77-g6b438a1c)` |
| Default Yosys version | `cd "$WORK"; "$YOSYS" -V` | PASS | exit 0; `Yosys 0.70 (git sha1 28ba3cb92, Release, GNU /usr/bin/c++ 15.2.0)` |
| GHDL plugin load | `cd "$WORK"; "$SUITE/bin/yosys" -m ghdl -p 'help ghdl'` | PASS | exit 0; `ghdl [options] unit [arch]`; `ghdl [options] files... -e [unit]`; <code>--std=(93&#124;08)</code> |
| Slang plugin load | `cd "$WORK"; "$SUITE/bin/yosys" -m slang -p 'help read_slang'` | PASS | exit 0; `read_slang [options] [filename]`; `--top <name>`; `-I,--include-directory,+incdir <dir-pattern>[,...]` |
| VHDL requested synthesis | `cd "$WORK/vhdl"; "$SUITE/bin/yosys" -m ghdl -p 'ghdl --std=08 top.vhd -e top; synth_ecp5 -top top -json top.json'` | PASS | exit 0; `Found and reported 0 problems.`; JSON backend completed. |
| VHDL default nextpnr JSON gate | `cd "$WORK/vhdl"; "$NEXTPNR" --85k --package CABGA381 --lpf top.lpf --json top.json --write top_routed.json --textcfg top.config` | PASS | exit 0; `Info: Routing complete.`; `Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 354.99 MHz (PASS at 25.00 MHz)`; `Info: Program finished normally.` |
| VHDL default ecppack | `cd "$WORK/vhdl"; "$ECPPACK" top.config top.bit` | PASS | exit 0; `(no stdout/stderr)`; top.bit produced. |
| VHDL source-audit resynthesis | `cd "$AUDIT"; "$SUITE/bin/yosys" -m ghdl -p 'ghdl --std=08 top.vhd -e top; synth_ecp5 -top top -json top.json'` | PASS | exit 0; `Found and reported 0 problems.`; same inline VHDL source. |
| VHDL source-audit rerouting | `cd "$AUDIT"; "$NEXTPNR" --85k --package CABGA381 --lpf top.lpf --json top.json --write top_routed.json --textcfg top.config` | PASS | exit 0; `Info: Routing complete.`; `Info: Program finished normally.` |
| Original VHDL source locations | `cd "$AUDIT"; python3 -c "$SRC_CHECK"` | FAIL | exit 0; `top.json: total_src=1864; top.vhd_src=0; top_routed.json: total_src=21; top.vhd_src=0`; no `top.vhd:line` attribute exists to quote. See exact counterexample below. |
| Built-in reader control (expected rejection) | `cd "$WORK/slang"; "$YOSYS" -p 'read_verilog -sv import.sv; synth_ecp5 -top top -json builtin.json'` | FAIL (expected) | exit 1; `import.sv:5: ERROR: syntax error, unexpected TOK_IMPORT` |
| Slang synthesis | `cd "$WORK/slang"; "$SUITE/bin/yosys" -m slang -p 'read_slang --top top import.sv; synth_ecp5 -top top -json top.json'` | PASS | exit 0; `Found and reported 0 problems.`; `8   TRELLIS_FF`; JSON backend completed. |
| Slang default nextpnr JSON gate | `cd "$WORK/slang"; "$NEXTPNR" --85k --package CABGA381 --lpf top.lpf --json top.json --write top_routed.json --textcfg top.config` | PASS | exit 0; `Info: Routing complete.`; `Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 354.99 MHz (PASS at 25.00 MHz)`; `Info: Program finished normally.` |
| Slang default ecppack | `cd "$WORK/slang"; "$ECPPACK" top.config top.bit` | PASS | exit 0; `(no stdout/stderr)`; top.bit produced. |
| Original SV source locations | `cd "$WORK/slang"; [in-process Python SRC_COLLECTOR below]` | PASS | `netnames["count[0]"].attributes.src = "import.sv:4.54"` |
| Suite built-in SV synthesis | `cd "$WORK/suite_builtin"; "$SUITE/bin/yosys" -p 'read_verilog -sv top.sv; synth_ecp5 -top top -json top.json'` | PASS | exit 0; `Found and reported 0 problems.`; `8   TRELLIS_FF`; JSON backend completed. |
| Suite built-in SV default nextpnr JSON gate | `cd "$WORK/suite_builtin"; "$NEXTPNR" --85k --package CABGA381 --lpf top.lpf --json top.json --write top_routed.json --textcfg top.config` | PASS | exit 0; `Info: Routing complete.`; `Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 354.99 MHz (PASS at 25.00 MHz)`; `Info: Program finished normally.` |
| Suite built-in SV default ecppack | `cd "$WORK/suite_builtin"; "$ECPPACK" top.config top.bit` | PASS | exit 0; `(no stdout/stderr)`; top.bit produced. |
| Default built-in SV synthesis | `cd "$WORK/default_builtin"; "$YOSYS" -p 'read_verilog -sv top.sv; synth_ecp5 -top top -json top.json'` | PASS | exit 0; `Found and reported 0 problems.`; `8   TRELLIS_FF`; JSON backend completed. |
| Default built-in SV default nextpnr JSON gate | `cd "$WORK/default_builtin"; "$NEXTPNR" --85k --package CABGA381 --lpf top.lpf --json top.json --write top_routed.json --textcfg top.config` | PASS | exit 0; `Info: Routing complete.`; `Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 353.98 MHz (PASS at 25.00 MHz)`; `Info: Program finished normally.` |
| Default built-in SV default ecppack | `cd "$WORK/default_builtin"; "$ECPPACK" top.config top.bit` | PASS | exit 0; `(no stdout/stderr)`; top.bit produced. |
| Build measured-checkout emulator | `cd "$ROOT"; "$CARGO" build --locked --release -p ecp5_emu --bin ecp5_emu --target-dir "$WORK/cargo"` | PASS | exit 0; ``Finished `release` profile [optimized] target(s) in 28.21s`` (no source changes). |
| VHDL emulator, 320 rising edges | `cd "$WORK/vhdl"; "$EMU" emulate top.config --names top_routed.json --startup configured --scope observed --clock net:clk_25mhz --probe net:led --period-ns 40 --cycles 320 --architecture "$ROOT/docs/inventory/LFE5U-85F" --db "$DB" --image-reader "$READER" --output emulate.json` | PASS | exit 0; `Executed FF: 8; LUT4: 2; LUTRAM columns: 0; RAMW: 0; carry halves: 10; PIO: 9; routes: 152`; 641 samples, 0 counter mismatches; at 12800 ns led = 64. Raw samples below. |
| Slang emulator, 320 rising edges | `cd "$WORK/slang"; "$EMU" emulate top.config --names top_routed.json --startup configured --scope observed --clock net:clk_25mhz --probe net:led --period-ns 40 --cycles 320 --architecture "$ROOT/docs/inventory/LFE5U-85F" --db "$DB" --image-reader "$READER" --output emulate.json` | PASS | exit 0; `Executed FF: 8; LUT4: 2; LUTRAM columns: 0; RAMW: 0; carry halves: 10; PIO: 9; routes: 152`; 641 samples, 0 counter mismatches; at 12800 ns led = 64. Raw samples below. |
| VHDL invalid-input diagnostics | `cd "$AUDIT"; "$SUITE/bin/yosys" -m ghdl -p 'ghdl --std=08 bad.vhd -e top; synth_ecp5 -top top -json bad.json'` | FAIL (expected) | exit 1; `bad.vhd:8:13:error: primary expression expected`<br>`  broken <= ;`<br>`            ^`<br>`ERROR: vhdl import failed.` |
| Slang invalid-input diagnostics | `cd "$AUDIT"; "$SUITE/bin/yosys" -m slang -p 'read_slang --top top bad.sv; synth_ecp5 -top top -json bad.json'` | FAIL (expected) | exit 1; `bad.sv:2: ERROR: expected expression`<br>`ERROR: Design elaboration failed; see full log for details` |
| Suite/default built-in SV config identity (non-gating) | `[in-process Python COMPARE below, WORK as bound above]` | FAIL (different; non-gating) | `{"default_bytes": 16165, "same_config": false, "suite_bytes": 16141}` |

VHDL provenance counterexample: `top_routed.json` contains only 21 `src` attributes, none naming `top.vhd`. One actual attribute (first found by the full JSON walk):

```text
first src='$SUITE/share/yosys/lattice/arith_map_ccu2c.v:74.7-80.4'
```

Config SHA-256 values from the measured outputs:

| Producer | top.config SHA-256 |
| --- | --- |
| VHDL | `d82e543ee81c9925dbc0ae41cec3796262a76d1c30154b4ef859f436982859ce` |
| Slang | `d82e543ee81c9925dbc0ae41cec3796262a76d1c30154b4ef859f436982859ce` |
| Suite built-in SV | `d82e543ee81c9925dbc0ae41cec3796262a76d1c30154b4ef859f436982859ce` |
| Default built-in SV | `c6e91d637c69a391a6c5a5c3ad6b09889222d94f6e33f2640ba9800a75653bf5` |

## Exact source inspection and output analysis

`SRC_CHECK` is the following literal `python3 -c` argument used on each host. It traverses all JSON objects, including library modules in synthesis JSON; the routed JSON contains the top. It exited 0 while establishing the failed source-location check:

```python
import json
for f in ('top.json', 'top_routed.json'):
 j=json.load(open(f)); attrs=[]
 def visit(x):
  if isinstance(x,dict):
   if isinstance(x.get('attributes'),dict) and 'src' in x['attributes']: attrs.append(x['attributes']['src'])
   for v in x.values(): visit(v)
  elif isinstance(x,list):
   for v in x: visit(v)
 visit(j)
 own=[s for s in attrs if 'top.vhd:' in s]
 print(f + ': total_src=' + str(len(attrs)) + '; top.vhd_src=' + str(len(own)))
 print('first src=' + repr(attrs[0] if attrs else None))
 print('count attributes=' + repr(j['modules']['top']['netnames'].get('count',j['modules']['top']['netnames'].get('count[0]',{})).get('attributes')))
```

`SRC_COLLECTOR` is this in-process fragment from the measurement harness, run immediately after each successful route (`d` is that flow’s absolute temporary directory). The Slang table quotes the captured `count[0]` entry:

```python
j = json.loads((d / "top_routed.json").read_text())
srcs = []
for kind in ("netnames", "cells"):
    for name, obj in j["modules"]["top"].get(kind, {}).items():
        if "src" in obj.get("attributes", {}):
            srcs.append({"kind": kind, "name": name,
                         "src": obj["attributes"]["src"]})
```

`COMPARE` is the in-process byte comparison used on each host (`WORK` as bound in its table). Its captured result is printed in the identity row:

```python
a = (WORK / "suite_builtin/top.config").read_bytes()
b = (WORK / "default_builtin/top.config").read_bytes()
comparison = {"same_config": a == b,
              "suite_bytes": len(a), "default_bytes": len(b)}
```

All four successful emulator runs (Mac VHDL with prefix, Mac Slang, rserv VHDL, rserv Slang) emitted the same 641 LED samples, including the following exact lines:

```text
0 ns: led[0]=0 led[1]=0 led[2]=0 led[3]=0 led[4]=0 led[5]=0 led[6]=0 led[7]=0
20 ns: led[0]=1 led[1]=0 led[2]=0 led[3]=0 led[4]=0 led[5]=0 led[6]=0 led[7]=0
60 ns: led[0]=0 led[1]=1 led[2]=0 led[3]=0 led[4]=0 led[5]=0 led[6]=0 led[7]=0
10180 ns: led[0]=1 led[1]=1 led[2]=1 led[3]=1 led[4]=1 led[5]=1 led[6]=1 led[7]=1
10220 ns: led[0]=0 led[1]=0 led[2]=0 led[3]=0 led[4]=0 led[5]=0 led[6]=0 led[7]=0
12800 ns: led[0]=0 led[1]=0 led[2]=0 led[3]=0 led[4]=0 led[5]=0 led[6]=1 led[7]=0
```

For every captured line matching `TIME ns: led[0]=... led[7]=...`, decoded `sum(bit[i] << i)` was compared with `((TIME + 20) // 40) % 256`. Each run had 641 matches and zero mismatches: initial 0; rising-edge values 1, 2, …, 255, 0, …, 64 at edge 320. This also checks the unchanged values at falling edges. The kernel reported one undriven wire in each run; none of the eight observed LED bits was unknown.

## Gate verdicts

| Host | Frontend | Default-nextpnr JSON gate | Measured frontend gate including original source locations and emulation | What would change the verdict |
| --- | --- | --- | --- | --- |
| Mac | VHDL/GHDL | PASS with command-local `GHDL_PREFIX`; requested unprefixed synthesis FAILS before producing JSON | **FAIL**: no original VHDL `src` in synthesis or routed JSON | A measured invocation must both resolve the suite’s VHDL libraries and preserve original `top.vhd:line` attributes through default routing, with the LED run still passing. The prefix alone fixes only library lookup. |
| rserv | VHDL/GHDL | PASS with the requested unprefixed synthesis | **FAIL**: no original VHDL `src` in synthesis or routed JSON | Original `top.vhd:line` attributes must be measured through default routing, with the LED run still passing. |
| Mac | yosys-slang | PASS | **PASS for the requested fixture**: import accepted, default routing/packing accepted, original SV locations present, LED count verified | No missing condition in these fixture checks; a failing repeat of plugin loading, import synthesis, JSON acceptance, original-source retention, or counting would change this measured verdict. |
| rserv | yosys-slang | PASS | **PASS for the requested fixture**: import accepted, default routing/packing accepted, original SV locations present, LED count verified | No missing condition in these fixture checks; a failing repeat of plugin loading, import synthesis, JSON acceptance, original-source retention, or counting would change this measured verdict. |

The complete §8 slice-2 publication gate is not established by these CLI measurements: editor bidirectional source probing, VHDL case/basic-vector lookup, project selection/provenance/cache behavior, missing/incompatible-plugin handling, and pin/no-retry regressions were not exercised. The invalid-input diagnostics above are captured frontend errors, not tests of the proposed diagnostic integration. Built-in SV configuration byte inequality is a recorded comparison, not a frontend gate failure.

Temporary synthesis, routed JSON/config/bitstream, emulator builds and logs were deleted from the Mac and both rserv temporary directories after extracting this evidence. Only this evidence file was added to the worktree; no commit was created.

## Slice-2 runtime files for packaging (2026-11-18)

Re-measured with the exact `top.vhd` above and
`ghdl --std=08 top.vhd -e top; synth_ecp5 -top top -json top.json` (exit 0
on both hosts). These are observed successful opens/loaded images, relative to
`$SUITE`, normalized to remove `..`; keep symlink targets too. This is the
counter's measured closure, not a claim that all VHDL features need only these
files. No standalone GHDL compiler/backend executable was launched.

rserv: ran through `tools/rserv.sh`, with child
`GHDL_PREFIX=$SUITE/lib/ghdl`, using
`strace -f -e trace=openat,execve -o trace.log "$SUITE/bin/yosys" -m ghdl -p 'ghdl --std=08 top.vhd -e top; synth_ecp5 -top top -json top.json'`.
Collected only successful opens/execs under the suite root, including the
wrapper's loader and ABC helper. No root needed.

Mac: the plain `bin/yosys` run succeeded but emitted no dyld trace: macOS strips
`DYLD_*` at the system-shell wrapper boundary. Repeated the same synthesis with
`GHDL_PREFIX=$SUITE/lib/ghdl DYLD_PRINT_LIBRARIES=1 DYLD_LIBRARY_PATH=$SUITE/lib "$SUITE/libexec/yosys" -m ghdl -p 'ghdl --std=08 top.vhd -e top; synth_ecp5 -top top -json top.json'`.
The traced direct run produced JSON byte-for-byte identical to the wrapper run.
Added a temporary native interposer through `DYLD_INSERT_LIBRARIES` to record
successful `open`, `openat` and `fopen` calls (no root). It recorded the GHDL
library catalogue/source reads and Yosys technology files. Repeated dyld loading
for `$SUITE/libexec/yosys-abc -c quit` (exit 0), because the shell used to launch
ABC also strips the tracing environment. ABC added no library beyond the list
below. The interposer and logs stayed outside the repo and were removed after
recording this section; they are not product dependencies. No wrapper was read
or modified. System libraries/frameworks outside the suite are excluded.

| Host | libexec/ | lib/ (beyond lib/ghdl) | Frameworks/ |
| --- | --- | --- | --- |
| Mac | `yosys`, `yosys-abc` | `libffi.8.dylib`, `libgcc_s.1.1.dylib`, `libghdl-7_0_0_dev.dylib`, `libgnat-14.dylib`, `libiconv.2.dylib`, `liblzma.5.dylib`, `libncurses.6.dylib`, `libreadline.8.3.dylib`, `libz.1.3.2.dylib` | none observed |
| rserv | `yosys`, `yosys-abc` | `ld-linux-x86-64.so.2`, `libc.so.6`, `libffi.so.8`, `libgcc_s.so.1`, `libghdl-7_0_0_dev.so`, `libgnat-14.so`, `libm.so.6`, `libreadline.so.8`, `libstdc++.so.6`, `libtcl8.6.so`, `libtinfo.so.6`, `libz.so.1`, `yosys-abc` | none observed |

Both hosts opened the same GHDL subset under `lib/ghdl/`:
`std/v08/std-obj08.cf`, `ieee/v08/ieee-obj08.cf`,
`src/std/v08/{textio,textio-body}.vhdl`, and
`src/ieee2008/{std_logic_1164,std_logic_1164-body,numeric_std,numeric_std-body}.vhdl`.
Keep the catalogue/source directory layout; lib/ghdl alone is insufficient.

Both hosts opened these same files under `share/yosys/`:

```text
abc9_map.v
abc9_model.v
abc9_unmap.v
cmp2lut.v
lattice/arith_map_ccu2c.v
lattice/brams_16kd.txt
lattice/brams_map_16kd.v
lattice/ccu2c_sim.vh
lattice/cells_bb_ecp5.v
lattice/cells_ff.vh
lattice/cells_io.vh
lattice/cells_map_trellis.v
lattice/cells_sim_ecp5.v
lattice/common_sim.vh
lattice/dsp_map_18x18.v
lattice/latches_map.v
lattice/lutrams_map_trellis.v
lattice/lutrams_trellis.txt
mul2dsp.v
plugins/ghdl.so
techmap.v
```

For packaging, preserve `bin/yosys` and its helper launcher/loader wiring,
`libexec/yosys`, `libexec/yosys-abc`, the listed shared libraries, `lib/ghdl`,
and the listed `share/yosys` data/plugin paths. Linux also opened `lib/yosys-abc`
as recorded above. No suite `Frameworks/` path was observed on either host.
