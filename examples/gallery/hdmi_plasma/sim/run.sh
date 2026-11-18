#!/bin/sh
set -eu
folder=$(cd "$(dirname "$0")/.." && pwd)
out="$folder/target/sim"
mkdir -p "$out"
python3 - "$folder" "$out" <<'PY'
import json, sys
from pathlib import Path
cfg = json.loads((Path(sys.argv[1])/'check.json').read_text())
(Path(sys.argv[2])/'input.bin').write_bytes(cfg['input'].encode('latin-1'))
(Path(sys.argv[2])/'cycles.txt').write_text(str(cfg['cycles']))
PY
cp "$folder/top.v" "$folder/render.v" "$folder/io.v" "$folder/sim/stubs.v" \
   "$folder/sim/uart_tb.cpp" "$folder/sim/Makefile" "$out/"
cd "$out"
verilator --cc --top-module top -Wno-fatal --Mdir obj top.v render.v io.v stubs.v > build.log 2>&1
make -j 1 >> build.log 2>&1
cd "$folder"
"$out/obj/Vtop" "$(cat "$out/cycles.txt")" golden "$out/input.bin" "$out/uart_raw.txt"
"$out/obj/Vtop" "$(cat "$out/cycles.txt")" golden "$out/input.bin" "$out/uart_rerun.txt"
cmp "$out/uart_raw.txt" "$out/uart_rerun.txt"
"$out/obj/Vtop" 64100000 controls "$out/input.bin" "$out/controls_uart.txt"
python3 - "$folder" "$out" <<'PY'
import sys
from pathlib import Path
folder, out = map(Path, sys.argv[1:])
raw = (out/'uart_raw.txt').read_bytes()
assert raw.count(b'RESTART\r\n') == 1
golden = raw[raw.index(b'RESTART\r\n'):]
assert golden.count(b'\n') == 61, 'expected marker plus 60 frames'
path = folder/'expected/uart.txt'
if path.exists():
    assert path.read_bytes() == golden, 'golden changed: inspect target/sim/uart_raw.txt'
else:
    path.write_bytes(golden)
print(f'Golden: {len(golden)} bytes, 60 frame lines; byte-identical rerun PASS')
PY
python3 "$folder/model.py" "$folder/expected/uart.txt" "$out/controls_uart.txt"
