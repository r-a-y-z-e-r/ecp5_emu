#!/bin/sh
# Regenerate the reference from precisely the RTL and hex the build uses.
set -eu
folder=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$folder/../../.." && pwd)
out="$root/target/gallery/riscv_terminal/sim"
mkdir -p "$out"
python3 - "$folder" "$out" <<'PYCONFIG'
import json,sys
from pathlib import Path
config=json.loads((Path(sys.argv[1])/'check.json').read_text())
(Path(sys.argv[2])/'input.bin').write_bytes(config['input'].encode('latin-1'))
(Path(sys.argv[2])/'cycles.txt').write_text(str(config['cycles']))
assert len(config['input'].encode('latin-1')) < 512
PYCONFIG
# GNU Make rejects workspaces with spaces. Generate C++ and compile directly;
# all generated files remain under target/gallery, with every path quoted.
# sim/stubs.v stands in for the PLL and the TMDS path (see its comment);
# top.v and picorv32.v are the files the build synthesises.
cp "$folder/top.v" "$folder/picorv32.v" "$folder/sim/stubs.v" "$folder/sim/uart_tb.cpp" "$out/"
cd "$out"
verilator --cc --top-module top -Wno-fatal --Mdir obj top.v picorv32.v stubs.v > build.log 2>&1
vroot=$(verilator -V | sed -n 's/^ *VERILATOR_ROOT *= *//p' | head -1)
c++ -std=c++17 -O2 -pthread -Iobj -I"$vroot/include" -I"$vroot/include/vltstd" \
  obj/*.cpp uart_tb.cpp "$vroot/include/verilated.cpp" \
  "$vroot/include/verilated_threads.cpp" -o obj/Vtop >> build.log 2>&1
cd "$folder"
"$out/obj/Vtop" "$(cat "$out/cycles.txt")" "$out/input.bin" "$folder/expected/uart.txt"
