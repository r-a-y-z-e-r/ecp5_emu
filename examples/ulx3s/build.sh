#!/bin/sh
# Builds one reference design for the ULX3S (LFE5U-85F, CABGA381):
#   Yosys, nextpnr with the routed JSON for HDL names, ecppack.
# The sources are copied into OUT_DIR, so paths with spaces work.
# A design whose firmware is swapped lists its memory images in ecpbram.txt.
# Place and route then sees random placeholders of the same size
# (top_placeholder.config), and swap.sh puts the images in, as it does after
# each firmware rebuild.
# Usage: build.sh DESIGN_DIR OUT_DIR
set -eu
mkdir -p "$2"
here=$(cd "$(dirname "$0")" && pwd)
design=$(cd "$1" && pwd)
out=$(cd "$2" && pwd)
cp "$design"/*.v "$design"/top.lpf "$out"/
# Memory images ($readmemh), if the design has any.
for hex in "$design"/*.hex; do [ -e "$hex" ] && cp "$hex" "$out"/; done
cd "$out"
images=""
if [ -e "$design/ecpbram.txt" ]; then
  images=$(grep -v '^#' "$design/ecpbram.txt")
  seed=0
  for image in $images; do
    seed=$((seed + 1))
    depth=$(($(wc -l < "$image")))
    width=$(($(head -n 1 "$image" | tr -d ' \r\n' | wc -c) * 4))
    ecpbram -g "placeholder_$image" -w "$width" -d "$depth" -s "$seed" > /dev/null
    cp "placeholder_$image" "$image"
  done
fi
config=top.config
[ -n "$images" ] && config=top_placeholder.config
yosys -q -l yosys.log -p "read_verilog $(ls *.v | tr '\n' ' '); synth_ecp5 -top top -json top.json"
nextpnr-ecp5 --85k --package CABGA381 --json top.json --lpf top.lpf \
  --textcfg "$config" --write top_routed.json --seed 1 -q -l nextpnr.log
if [ -n "$images" ]; then
  "$here/swap.sh" "$design" "$out"
else
  ecppack --compress top.config top.bit
fi
