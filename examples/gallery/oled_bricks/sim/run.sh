#!/bin/sh
set -eu
folder=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$folder/../../.." && pwd)
out="${ORCHARD_SIM_OUT:-$root/target/gallery/oled_bricks/sim}"
mkdir -p "$out"
out=$(cd "$out" && pwd)
python3 - "$folder" "$out" <<'PYCONFIG'
import json,sys
from pathlib import Path
config=json.loads((Path(sys.argv[1])/'check.json').read_text())
(Path(sys.argv[2])/'input.bin').write_bytes(config['input'].encode('latin-1'))
(Path(sys.argv[2])/'cycles.txt').write_text(str(config['cycles']))
PYCONFIG
# Copy relative source names for the space-safe, single-job Makefile.
cp "$folder"/*.v "$folder/sim/uart_tb.cpp" "$folder/sim/"*playtest.h "$folder/sim/game_tb.v" "$folder/sim/game_tb.cpp" "$out/"
cd "$out"
verilator --cc --top-module top -Wno-fatal --Mdir obj top.v game.v render.v text.v io.v > build.log 2>&1
verilator --cc --top-module game_tb -DORCHARD_GAME_TEST -Wno-fatal --Mdir game_obj game.v game_tb.v >> build.log 2>&1
make -j 1 -f "$folder/sim/Makefile" >> build.log 2>&1
cd "$folder"
"$out/obj/Vtop" "$(cat "$out/cycles.txt")" "$out/input.bin" "$out/uart.txt" "$out/frame.rgb565"
# Only explicit physics/layout revisions may replace the golden transcript.
if [ "${ORCHARD_REGENERATE:-0}" = 1 ]; then
  cp "$out/uart.txt" "$folder/expected/uart.txt"
else
  cmp "$folder/expected/uart.txt" "$out/uart.txt"
fi
python3 "$folder/sim/png.py" "$out/frame.rgb565" "$folder/frame.png"

# The physical board starts with all switches ON. Attract must be identical.
"$out/obj/Vtop" "$(cat "$out/cycles.txt")" "$out/input.bin" "$out/all_on.txt" "$out/all_on.rgb565" 15
cmp "$folder/expected/uart.txt" "$out/all_on.txt"
# A second restart while a frame's UART line is printing must preserve its
# current byte, then recover the exact same transcript from the new marker.
"$out/obj/Vtop" "$(($(cat "$out/cycles.txt") + 500000))" "$out/input.bin" "$out/late.txt" "$out/late.rgb565" 0 late
"$out/obj/Vtop" 10000000 "$out/input.bin" "$out/human.txt" "$out/human.rgb565" 0 human
"$out/obj/Vtop" 300000000 "$out/input.bin" "$out/p18.txt" "$out/p18.rgb565" 0 p18
"$out/obj/Vtop" 120000000 "$out/input.bin" "$out/p27.txt" "$out/p27.rgb565" 0 p27
"$out/obj/Vtop" 300000000 "$out/input.bin" "$out/p28.txt" "$out/p28.rgb565" 0 p28
"$out/game_obj/Vgame_tb"
python3 - "$folder" "$out" <<'PYCHECK'
from pathlib import Path
import sys
folder, out = map(Path, sys.argv[1:])
expected = (folder / 'expected/uart.txt').read_bytes()
late = (out / 'late.txt').read_bytes()
assert late[late.rindex(b'RESTART'):].startswith(expected)
human = (out / 'human.txt').read_bytes()
assert b'PLAYER\r\n' in human
assert b'SW 0100\r\n' in human and b'SW 1111\r\n' in human
print('All-ON attract, restart during UART, and human controls: PASS')
PYCHECK
python3 "$folder/sim/test_tooling.py"
