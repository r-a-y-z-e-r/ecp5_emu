"""Check the HB02 record: every EBR whose port never reads shows 0 from the
first clock on; e7, which reads word 5 (all ones) at every clock, shows 0
before the first edge and 0x1FF from then on.
Usage: python3 hb02_check.py observations.log observations_2.log"""
import ast
import sys

RECORD = [("e0", "a", 9), ("e0", "b", 9), ("e1", "a", 9), ("e2", "a", 9), ("e3", "a", 18),
          ("e4", "a", 18), ("e4", "b", 18), ("e5", "a", 1), ("e6", "a", 9), ("e7", "a", 9)]
loads = []
for path in sys.argv[1:]:
    for line in open(path):
        items = {x for x in ast.literal_eval(line.partition("distinct=")[2]) if len(x) == 449}
        loads.append(items)
print(f"{len(loads)} loads: {sum(1 for l in loads if l)} gave a line, "
      f"{sum(1 for l in loads if not l)} none in their window; distinct lines: {len(set().union(*loads))}")
ok = True
for line in set().union(*loads):
    bits = bin(int(line[1:], 16))[2:].zfill(448 * 4)
    for k in range(16):
        w, pos = bits[112 * k:112 * k + 112], 0
        for name, port, width in RECORD:
            v = int(w[pos:pos + width], 2)
            pos += width
            want = 0x1FF if name == "e7" and k > 0 else 0
            if v != want:
                ok = False
                print(f"sample {k} {name}.DO{port.upper()} = {v:x}, not {want:x}")
print("every unread latch reads 0, and e7 its word from the first edge" if ok else "NO FIT")
