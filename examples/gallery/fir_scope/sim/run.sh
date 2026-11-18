#!/bin/sh
# Regenerates expected/uart.txt: an independent RTL simulation of the demo's
# own sources (Verilator, C++ harness uart_tb.cpp), not the emulator.
# sim/dsp_models.v stands in for the MULT18X18D/ALU54B primitives here; the
# bitstream build uses the real cells. `verilator --cc` + a direct c++ call,
# not `--binary`: GNU Make rejects the spaces in this workspace's path.
# Everything generated stays in target/.
set -eu
folder=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$folder/../../.." && pwd)
out="$root/target/gallery/fir_scope/sim"
rm -rf "$out"
mkdir -p "$out"
python3 - "$folder" "$out" <<'PYCONFIG'
import json,sys
from pathlib import Path
config=json.loads((Path(sys.argv[1])/'check.json').read_text())
(Path(sys.argv[2])/'input.bin').write_bytes(config['input'].encode('latin-1'))
(Path(sys.argv[2])/'cycles.txt').write_text(str(config['cycles']))
PYCONFIG
cp "$folder"/*.v "$folder/sim/uart_tb.cpp" "$folder/sim/dsp_models.v" "$out/"
cd "$out"
verilator --cc --top-module top -Wno-fatal --Mdir obj *.v > build.log 2>&1
vroot=$(verilator -V | sed -n 's/^ *VERILATOR_ROOT *= *//p' | head -1)
c++ -std=c++17 -O2 -pthread -Iobj -I"$vroot/include" -I"$vroot/include/vltstd" \
  obj/*.cpp uart_tb.cpp "$vroot/include/verilated.cpp" \
  "$vroot/include/verilated_threads.cpp" -o obj/Vtop >> build.log 2>&1
# The model reads sine.hex/coeff.hex from its working directory, as the
# bitstream build does.
cd "$folder"
"$out/obj/Vtop" "$(cat "$out/cycles.txt")" "$out/input.bin" "$folder/expected/uart.txt"
