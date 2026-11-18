"""Check the HD08z record: Z = 7 reads ~RNDPAT & (RNDPAT - 1), the mask of
RNDPAT's trailing zeros. Usage: python3 hd08z_model.py observations_z.log"""
import ast
import sys

M54 = (1 << 54) - 1
# name: RNDPAT, GSR (hd08z_gen.py)
ALUS = [(0x00000000000000, "ENABLED"), (0x00000000000000, "DISABLED"),
        (0x0123456789ABC5, "ENABLED"), (0x00000000000010, "DISABLED"),
        (0x00000000000100, "ENABLED"), (0x00000000000002, "DISABLED"),
        (0x20000000000000, "ENABLED"), (0x3FFFFFFFFFFFF0, "DISABLED")]
loads = [sorted(x for x in ast.literal_eval(line.partition("distinct=")[2]) if len(x) == 16)
         for line in open(sys.argv[1])]
print(f"{len(loads)} loads; the same eight lines in all: {all(l == loads[0] and len(l) == 8 for l in loads)}")
ok = True
for line in loads[0]:
    n, r = int(line[1]), int(line[2:], 16)
    rnd, gsr = ALUS[n]
    want = ~rnd & (rnd - 1) & M54
    ok &= r == want
    print(f"z{n}: RNDPAT {rnd:014X} GSR {gsr:<8} R {r:014X} model {want:014X} {'=' if r == want else 'DIFFERS'}")
print("model fits" if ok else "NO FIT")
