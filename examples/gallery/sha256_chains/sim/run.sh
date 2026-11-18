#!/bin/sh
set -eu
folder=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$folder/../../.." && pwd)
out="$root/target/gallery/sha256_chains/sim"
mkdir -p "$out"
cp "$folder"/*.v "$folder/sim/uart_tb.cpp" "$out/"
python3 - "$folder" "$out" <<'CONFIG'
import json,re,sys
from pathlib import Path
p,out=map(Path,sys.argv[1:])
c=json.loads((p/'check.json').read_text())
(out/'input.bin').write_bytes(c['input'].encode('latin-1'))
(out/'cycles.txt').write_text(str(c['cycles']))
params={k:int(v) for k,v in re.findall(r'parameter (CORES|STEP_EVERY|STEPS)=([0-9]+)', (p/'top.v').read_text())}
(out/'model.json').write_text(json.dumps(params))
CONFIG
cd "$out"
# Compile directly so spaces in the checkout path do not confuse GNU Make.
verilator --cc --assert --public-flat-rw --top-module top -Wno-fatal \
  --Mdir obj top.v sha256_core.v io.v > build.log 2>&1
vroot=$(verilator -V | sed -n 's/^ *VERILATOR_ROOT *= *//p' | head -1)
c++ -std=c++17 -O2 -pthread -Iobj -I"$vroot/include" -I"$vroot/include/vltstd" \
  obj/*.cpp uart_tb.cpp "$vroot/include/verilated.cpp" \
  "$vroot/include/verilated_threads.cpp" -o obj/Vtop >> build.log 2>&1
"$out/obj/Vtop" "$(cat "$out/cycles.txt")" "$out/input.bin" "$out/nominal"
python3 "$folder/sim/check.py" "$out/nominal" "$folder/expected/uart.txt"
# Wait until both power-on KAT lines have started before sending the only r.
late_cycles=$(($(cat "$out/cycles.txt") + 200000))
"$out/obj/Vtop" "$late_cycles" "$out/input.bin" "$out/late" late
python3 "$folder/sim/check.py" "$out/late" --late "$folder/expected/uart.txt"
# Interrupt a UART byte and compression rounds; then debounce FIRE1.
"$out/obj/Vtop" 7270000 "$out/input.bin" "$out/controls" controls
python3 "$folder/sim/check.py" "$out/controls"
# Exercise both forms of the final odd-sized block: one core and three cores.
for n in 1 3; do
  variant="$out/padding_$n"
  mkdir -p "$variant"
  cp "$folder"/*.v "$folder/sim/uart_tb.cpp" "$variant/"
  python3 - "$out/model.json" "$variant/model.json" "$n" <<'MODEL'
import json,sys
from pathlib import Path
c=json.loads(Path(sys.argv[1]).read_text());c['CORES']=int(sys.argv[3])
Path(sys.argv[2]).write_text(json.dumps(c))
MODEL
  (
    cd "$variant"
    verilator --cc --assert --public-flat-rw --top-module top -Wno-fatal \
      -GCORES="$n" --Mdir obj top.v sha256_core.v io.v > build.log 2>&1
    c++ -std=c++17 -O2 -pthread -Iobj -I"$vroot/include" -I"$vroot/include/vltstd" \
      obj/*.cpp uart_tb.cpp "$vroot/include/verilated.cpp" \
      "$vroot/include/verilated_threads.cpp" -o obj/Vtop >> build.log 2>&1
    case "$n" in 1) cycles=1390000 ;; 3) cycles=2470000 ;; esac
    ./obj/Vtop "$cycles" "$out/input.bin" "$variant/nominal"
    python3 "$folder/sim/check.py" "$variant/nominal" "$variant/uart.txt"
  )
done
