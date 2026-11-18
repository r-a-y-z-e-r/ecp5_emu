#!/bin/sh
# RTL simulation of the repro: drives ../dsp_mac.v through ../top.v's
# seven phases and prints the ALU54B accumulator against a fabric
# reference. Build products go to target/, not into the source folder.
set -e
root="$(cd "$(dirname "$0")/../../../../.." && pwd)"
out="$root/target/gallery/fir_scope_repro/sim"
mkdir -p "$out"
cd "$(dirname "$0")"
iverilog -g2005 -o "$out/tb" tb.v ../top.v ../dsp_mac.v dsp_models.v
"$out/tb"
