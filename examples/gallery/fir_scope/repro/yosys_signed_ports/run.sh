#!/bin/sh
# Reproducer for the yosys 0.70 failure this demo hit: declaring a user
# module's ports `signed` makes yosys re-elaborate the module from its
# retained AST at the final hierarchy check of synth_ecp5, AFTER flatten
# has deleted the sibling modules, and the re-elaboration of `top` then
# fails. The only difference between the two dsp_mac_*.v files is the
# `signed` keyword on the port list.
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for v in signed unsigned; do
  echo "=== dsp_mac_$v.v ==="
  yosys -q -p "read_verilog dsp_mac_$v.v filters.v io.v top.v; synth_ecp5 -top top -json $tmp/signed_ports_$v.json" 2>&1 | grep -E "ERROR|Reprocessing" | head -4
  if [ -f "$tmp/signed_ports_$v.json" ]; then echo "$v: OK"; else echo "$v: FAILED"; fi
done
