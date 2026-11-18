#!/bin/sh
# Puts the memory images that DESIGN_DIR/ecpbram.txt lists into a design
# that build.sh placed and routed in OUT_DIR, without placing and routing
# again. ecpbram replaces each random placeholder in top_placeholder.config
# with its image; ecppack then writes top.bit. Takes seconds, so a firmware
# rebuild goes straight to the emulator or the board.
# Usage: swap.sh DESIGN_DIR OUT_DIR
set -eu
design=$(cd "$1" && pwd)
out=$(cd "$2" && pwd)
[ -e "$out/top_placeholder.config" ] || {
  echo "swap.sh: no top_placeholder.config in $out; build the design with build.sh first" >&2
  exit 1
}
cp "$out/top_placeholder.config" "$out/top.config"
for image in $(grep -v '^#' "$design/ecpbram.txt"); do
  ecpbram -i "$out/top.config" -o "$out/top.swap.config" \
    -f "$out/placeholder_$image" -t "$design/$image"
  mv "$out/top.swap.config" "$out/top.config"
  cp "$design/$image" "$out/$image"
done
ecppack --compress "$out/top.config" "$out/top.bit"
