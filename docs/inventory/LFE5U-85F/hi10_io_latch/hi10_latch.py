"""Turn O0's output register and I0's input register of hi10 into latches.
Usage: python3 hi10_latch.py hi10_ff.config hi10.config

nextpnr cannot place IO latches (yosys has no cell for them), so the design
builds IO registers and this sets the latch bit of two of them in the text
configuration. O0 is on B2 (R35 PIO C, settings in tile MIB_R37C0:PICL2,
IOLOGICC), I0 on D1 (R38 PIO B, tile MIB_R38C0:PICL0, IOLOGICB); iodb.json
gives the pads."""
import sys

EDITS = {
    ("MIB_R37C0:PICL2", "enum: IOLOGICC.OUTREG.OUTREGMODE FF"): "enum: IOLOGICC.OUTREG.OUTREGMODE LATCH",
    ("MIB_R38C0:PICL0", "enum: IOLOGICB.FF.INREGMODE FF"): "enum: IOLOGICB.FF.INREGMODE LATCH",
}
tile, out, done = None, [], set()
for line in open(sys.argv[1]).read().split("\n"):
    if line.startswith(".tile "):
        tile = line.split()[1]
    key = (tile, line.strip())
    if key in EDITS:
        line = EDITS[key]
        done.add(key)
    out.append(line)
assert done == set(EDITS), f"not found: {set(EDITS) - done}"
open(sys.argv[2], "w").write("\n".join(out))
print("set", len(done), "latch bits")
