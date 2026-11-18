#!/bin/sh
# Regenerate the reference from precisely the RTL and hex used by build.sh.
set -eu
cd "$(dirname "$0")/.."
mkdir -p sim/build expected
python3 - <<'PY'
import json
from pathlib import Path
c = json.loads(Path('check.json').read_text())
Path('sim/build/input.bin').write_bytes(c['input'].encode('latin-1'))
Path('sim/build/cycles').write_text(str(c['cycles']))
assert len(c['input'].encode('latin-1')) < 512
PY
(cd sim/build && verilator --cc --exe -Wno-fatal --top-module top \
  --Mdir . ../../top.v ../../picorv32.v ../main.cpp > compile.log 2>&1 &&
  make -f Vtop.mk -j 4 CURDIR=. >> compile.log 2>&1)
sim/build/Vtop "$(cat sim/build/cycles)" sim/build/input.bin expected/uart.txt
