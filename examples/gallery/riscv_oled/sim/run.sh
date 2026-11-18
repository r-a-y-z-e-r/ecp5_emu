#!/bin/sh
# No new computer/toolchain: shell's cycle-level Verilator host, OLED pad checks.
set -eu
folder=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$folder/../../.." && pwd)
out="$root/target/gallery/riscv_oled/sim"
mkdir -p "$out" "$folder/expected"
python3 - "$folder" "$out" <<'PY'
import json, sys
from pathlib import Path
folder, out = map(Path, sys.argv[1:])
c = json.loads((folder / 'check.json').read_text())
assert c['input'] == 'r' and c['marker'] == 'RESTART'
(out / 'input.bin').write_bytes(c['input'].encode('latin-1'))
(out / 'cycles').write_text(str(c['cycles']))
PY
cp "$folder"/*.v "$folder"/fw_*.hex "$folder/sim/uart_tb.cpp" "$out/"
cd "$out"
verilator --cc --exe --top-module top -Wno-fatal --Mdir obj \
  top.v picorv32.v oled_dma.v uart_tb.cpp -CFLAGS '-std=c++17 -O2' -LDFLAGS '-lz' > build.log 2>&1
# CURDIR=. keeps GNU Make happy in this workspace's path containing spaces.
(cd obj && make -f Vtop.mk -j 1 CURDIR=. >> ../build.log 2>&1)
./obj/Vtop "$(cat cycles)" input.bin "$folder/expected/uart.txt" frame.rgb565
python3 "$folder/sim/png.py" frame.rgb565 "$folder/frame.png"
# Also exercise check.py's immediate-r schedule, and FIRE1 with late arrival.
./obj/Vtop "$(cat cycles)" input.bin early.txt early.rgb565 early
./obj/Vtop "$(cat cycles)" input.bin fire.txt fire.rgb565 fire
python3 - "$folder" "$out" <<'PY'
from pathlib import Path
import sys
folder, out = map(Path, sys.argv[1:])
expected = (folder / 'expected/uart.txt').read_bytes().split(b'RESTART', 1)[1]
for name in ('early', 'fire'):
    got = (out / f'{name}.txt').read_bytes().split(b'RESTART', 1)[1]
    assert got.startswith(expected), f'{name} restart differs from late r'
print('PASS: immediate r and FIRE1 restart have the identical checked prefix')
PY
