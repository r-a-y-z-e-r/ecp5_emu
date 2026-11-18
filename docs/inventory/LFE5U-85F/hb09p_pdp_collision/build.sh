#!/bin/sh
# Build only. This script never opens a board or serial port.
set -eu
series_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
scratch_dir=${1:?Usage: sh build.sh /tmp/hb09p-build}
mkdir -p "$scratch_dir"
cd "$scratch_dir"
yosys -V > TOOLS.txt
nextpnr-ecp5 --version >> TOOLS.txt 2>&1
ecppack --version >> TOOLS.txt 2>&1
python3 "$series_dir/hb09p_analyse.py" --selftest
for design in hb09p hb09ps; do
    if [ "$design" = hb09ps ]; then
        python3 "$series_dir/hb09p_gen.py" s > "$design.v"
    else
        python3 "$series_dir/hb09p_gen.py" > "$design.v"
    fi
    yosys -Q -p "read_verilog $design.v; synth_ecp5 -top top -json $design.json" > "$design.yosys.log" 2>&1
    nextpnr-ecp5 --85k --package CABGA381 --json "$design.json" \
        --lpf "$series_dir/hb09p.lpf" --textcfg "$design.config" \
        --write "${design}_routed.json" --seed 1 > "$design.nextpnr.log" 2>&1
    ecppack --compress "$design.config" "$design.bit"
done
shasum -a 256 hb09p.v hb09ps.v hb09p.config hb09ps.config hb09p.bit hb09ps.bit > SHA256SUMS
printf 'Built hb09p.bit and hb09ps.bit in %s\n' "$scratch_dir"
