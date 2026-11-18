#!/bin/sh
# Reproduced an emulator gap, fixed on dev in d4f83ec: an EBR whose output
# latch had not been read since configuration was unknown, and fast mode would
# not start until every latch it reads was known. riscv_shell reads every bank
# during reset (top.v, `!resetn ||` in the bank read enable). This builds
# riscv_shell without that read and runs 2000 cycles. Before d4f83ec fast mode
# did not start:
#   "output 0 of EBR LFE5U-85F/R22C4/EBR0 port 0 is unknown;
#    the event kernel ran all 2000 cycles"
# (nextpnr seed 1). The board shows such latches reading 0 (HB02), and the
# configured startup now starts them there, so fast mode starts at once.
set -eu
root=$(cd "$(dirname "$0")/../../../../.." && pwd)
src="$root/examples/gallery/riscv_shell"
work="$root/target/gallery/riscv_shell_ebr_unread"
rm -rf "$work/src" && mkdir -p "$work/src"
cp -L "$src"/*.v "$src"/*.hex "$src"/top.lpf "$work/src/"
sed -i.orig 's/if (!resetn || (request \&\& ram_selected \&\& mem_addr\[14:13\] == bank))/if (request \&\& ram_selected \&\& mem_addr[14:13] == bank)/' "$work/src/top.v"
! cmp -s "$work/src/top.v" "$work/src/top.v.orig" || { echo "the reset-time read was not found in top.v" >&2; exit 1; }
rm "$work/src/top.v.orig"
"$root/examples/ulx3s/build.sh" "$work/src" "$work"
"$root/target/release/ecp5_emu" emulate "$work/top.config" --names "$work/top_routed.json" \
  --fast --startup configured --scope observed --clock net:clk_25mhz \
  --period-ns 40 --cycles 2000 --probe net:led
