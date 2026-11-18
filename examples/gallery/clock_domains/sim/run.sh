#!/bin/sh
set -eu
folder=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$folder/../../.." && pwd)
out="$root/target/gallery/clock_domains/sim"
mkdir -p "$out"
trace=""
case "${1:-}" in
  "") ;;
  --fst) trace="$out/synchronisers.fst" ;;
  *) echo "usage: sh sim/run.sh [--fst]" >&2; exit 2 ;;
esac
python3 - "$folder" "$out" <<'PY'
import json,sys
from pathlib import Path
p,out=map(Path,sys.argv[1:])
c=json.loads((p/'check.json').read_text())
(out/'input.bin').write_bytes(c['input'].encode('latin-1'))
(out/'cycles.txt').write_text(str(c['cycles']))
PY
# GNU Make rejects workspaces with spaces. Compile the generated C++
# directly, with every path quoted, as the ebr_life reference does.
cp "$folder"/*.v "$folder/sim/primitives.v" "$folder/sim/uart_tb.cpp" "$out/"
cd "$out"
verilator --cc --assert --trace-fst -DCDC_SIM --top-module top -Wno-fatal \
  --Mdir obj top.v pll.v cdc.v fifo.v handshake.v io.v primitives.v > build.log 2>&1
vroot=$(verilator -V | sed -n 's/^ *VERILATOR_ROOT *= *//p' | head -1)
c++ -std=c++17 -O2 -pthread -Iobj -I"$vroot/include" -I"$vroot/include/vltstd" \
  -I/opt/homebrew/include -L/opt/homebrew/lib \
  obj/*.cpp uart_tb.cpp "$vroot/include/verilated.cpp" \
  "$vroot/include/verilated_threads.cpp" "$vroot/include/verilated_fst_c.cpp" \
  -lz -llz4 -o obj/Vtop >> build.log 2>&1
cycles=$(cat "$out/cycles.txt")
"$out/obj/Vtop" "$cycles" "$out/input.bin" "$out/nominal.txt" 0 "$trace"
"$out/obj/Vtop" "$cycles" "$out/input.bin" "$out/plus006.txt" 1
"$out/obj/Vtop" "$cycles" "$out/input.bin" "$out/minus03.txt" 2
python3 - "$folder" "$out" <<'PY'
from pathlib import Path
import sys
p,out=map(Path,sys.argv[1:])
def marked(name):
    b=(out/name).read_bytes()
    assert b.count(b'RESTART\n')==1, (name,'restart count')
    return b[b.index(b'RESTART\n'):]
gold=marked('nominal.txt')
assert gold==marked('plus006.txt')==marked('minus03.txt'), 'phase/frequency changed UART bytes'
(p/'expected/uart.txt').write_bytes(gold)
print('Phase reruns: identical UART bytes at nominal, OSC +0.6%, OSC -3%; PLL ratio 6:5 throughout')
PY
# Exercise a restart during a UART line, another during a transfer, FIRE1,
# then a complete one-second frequency measurement. This is not golden text.
"$out/obj/Vtop" 28000000 "$out/input.bin" "$out/controls.txt" 0 "" controls
