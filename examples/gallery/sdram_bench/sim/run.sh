#!/bin/sh
set -eu
folder=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$folder/../../.." && pwd)
out="$root/target/gallery/sdram_bench/sim"
mkdir -p "$out"
python3 - "$folder" "$out" <<'PYCONFIG'
import json,sys
from pathlib import Path
config=json.loads((Path(sys.argv[1])/'check.json').read_text())
(Path(sys.argv[2])/'input.bin').write_bytes(config['input'].encode('latin-1'))
(Path(sys.argv[2])/'cycles.txt').write_text(str(config['cycles']))
PYCONFIG
# GNU Make cannot handle spaces in this workspace; compile generated C++
# directly, as ebr_life does. Build products stay entirely under target/.
cp "$folder"/*.v "$folder"/sim/*.v "$folder"/sim/*.cpp "$folder"/sim/*.h "$out/"
cd "$out"
verilator --cc --top-module sim_top -Wno-fatal --Mdir obj sim_top.v top.v controller.v io.v report.v > build.log 2>&1
vroot=$(verilator -V | sed -n 's/^ *VERILATOR_ROOT *= *//p' | head -1)
c++ -std=c++17 -O2 -pthread -Iobj -I"$vroot/include" -I"$vroot/include/vltstd" \
 obj/*.cpp uart_tb.cpp "$vroot/include/verilated.cpp" \
 "$vroot/include/verilated_threads.cpp" -o obj/Vsim_top >> build.log 2>&1
c++ -std=c++17 -O2 model_test.cpp -o obj/model_test >> build.log 2>&1
"$out/obj/model_test"
"$out/obj/Vsim_top" "$(cat "$out/cycles.txt")" "$out/input.bin" "$folder/expected/uart.txt"
