#!/usr/bin/env bash
# fabric_overlay's two default tests load these files unconditionally.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
PREFIX=${ECP5_TRELLIS_PREFIX:-$ROOT/target/packaging/trellis-prefix}
SOURCE=${ECP5_TRELLIS_SOURCE:-$ROOT/target/packaging/prjtrellis}
if [[ $(uname -s) == Darwin ]]; then
    CPATH="$(brew --prefix)/include${CPATH:+:$CPATH}" \
        python3 tools/build_image_reader.py --trellis "$SOURCE" --prefix "$PREFIX"
else
    mkdir -p target
    clang++ -std=c++14 -O2 -Wall -Wextra -Werror \
        -I "$SOURCE/libtrellis/include" -I "$PREFIX/include" tools/trellis_image_reader.cpp \
        -L "$PREFIX/lib/trellis" -ltrellis -Wl,-rpath,"$PREFIX/lib/trellis" \
        -o target/trellis_image_reader
fi
for DESIGN in blinky_cycle uart_hello; do
    examples/ulx3s/build.sh "examples/ulx3s/$DESIGN" "target/reference/$DESIGN"
done
