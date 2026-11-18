#!/bin/sh
# Rebuild the same controller/core at three sizes; all generated output stays in the build directory.
set -eu
folder=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$folder/../../.." && pwd)
for cores in 1 4 10; do
  out="$root/target/gallery/sha256_chains/scaling/$cores"
  python3 - "$folder" "$out/sources" "$cores" <<'PY'
from pathlib import Path
import re,shutil,sys
p,d=map(Path,sys.argv[1:3]);d.mkdir(parents=True,exist_ok=True)
for s in list(p.glob('*.v'))+[p/'top.lpf']:
    shutil.copyfile(s,d/s.name)
f=d/'top.v'
f.write_text(re.sub(r'parameter CORES=\d+', 'parameter CORES='+sys.argv[3], f.read_text()))
PY
  "$root/examples/ulx3s/build.sh" "$out/sources" "$out"
  sed -n '/Total LUT4s:/p; /Max frequency/p' "$out/nextpnr.log"
done
