#!/bin/bash
# build.sh PROGRAM.c OUT : builds the floor harness (make_harness.py) over a generated fast-mode
# program: its units (-DTU=k, the count is in its `/* ecp5_emu units: N */` line) and the harness.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
c="$1"; out="$2"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
python3 "$here/make_harness.py" "$c" "$work/h.c"
units=$(head -20 "$c" | sed -n 's|^/\* ecp5_emu units: \([0-9]*\) \*/|\1|p'); units=${units:-1}
seq 0 $((units - 1)) | xargs -P "${JOBS:-8}" -I{} clang -O3 -march=native -DTU={} -c "$c" -o "$work/u.{}.o"
clang -O3 -march=native -Wno-empty-body "$work/h.c" "$work"/u.*.o -o "$out"
