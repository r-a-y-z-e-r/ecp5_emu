#!/bin/sh
set -eu
folder=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$folder/../../.." && pwd)
out="${CHIP_SYNTH_SIM_OUT:-$root/target/gallery/chip_synth/sim}"
mkdir -p "$out"
cp "$folder"/*.v "$folder/sim/tb.cpp" "$out/"
cd "$out"
verilator --cc --top-module top -Wall -Wno-fatal --Mdir obj top.v synth.v oled.v render.v io.v text.v > build.log 2>&1
make -j 1 -f "$folder/sim/Makefile" >> build.log 2>&1
"$out/obj/Vtop" 25000000 "$out/base" base
"$out/obj/Vtop" 25000000 "$out/repeat" base
cmp "$out/base.uart" "$out/repeat.uart"
cmp "$out/base.samples" "$out/repeat.samples"
cmp "$out/base.frames" "$out/repeat.frames"
for frame in "$out"/base.frame*.rgb565; do
  repeat="$out/repeat${frame#"$out/base"}"
  cmp "$frame" "$repeat"
done
"$out/obj/Vtop" 25000000 "$out/late" late
"$out/obj/Vtop" 8500000 "$out/controls" controls
"$out/obj/Vtop" 6000000 "$out/restart" restart
"$out/obj/Vtop" 24900000 "$out/power" power
"$out/obj/Vtop" 205000000 "$out/loop" base
python3 "$folder/sim/model.py" "$out"
python3 "$folder/sim/png.py" "$out" "$folder"
if test -f "$folder/expected/uart.txt"; then
  cmp "$folder/expected/uart.txt" "$out/base.uart"
else
  cp "$out/base.uart" "$folder/expected/uart.txt"
fi
printf '%s\n' 'Byte-identical rerun and protected expected transcript: PASS'
