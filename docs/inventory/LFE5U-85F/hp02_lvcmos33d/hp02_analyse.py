"""HP02: checks a "P=" line against the pseudo-differential model.
Samples t = 1024..1279 of {true pad (gp3 via jumper to gp4), complement pad
(gn3 via jumper to gn4)}. Model: the true pad is the ODDRX1F output of HI02
(same LFSRs, same sclk); the complement pad is its inverse; while the shared
tristate is high (samples 1156..1211) both pads are released and read 0
through the pull-downs, possibly after a short analog tail."""
import sys
line = sys.argv[1]
bits = bin(int(line[2:], 16))[2:].zfill(512)
T = list(range(1024, 1280))
true = {t: int(bits[2 * i]) for i, t in enumerate(T)}
comp = {t: int(bits[2 * i + 1]) for i, t in enumerate(T)}

def lfsr(seed, tap, n):
    s, out = seed, []
    for _ in range(n):
        out.append(s)
        s = ((s << 1) & 0x7F) | (((s >> 6) ^ (s >> tap)) & 1)
    return out
la, lb = lfsr(0x01, 5, 400), lfsr(0x55, 4, 400)
def oddr(t):
    # HI02: high phase after rising r shows D0 captured at r-2, low phase D1.
    high = t % 16 >= 8
    r, f = (t + 8) // 16, t // 16
    return (la[r - 3] & 1) if high else (lb[f - 3] & 1)
released = lambda t: 1156 <= t <= 1211
if all(v == 0 for v in true.values()) and all(v == 0 for v in comp.values()):
    sys.exit("both inputs read 0 everywhere: the jumpers are not in place")
driven = [t for t in T if not released(t)]
print("true pad = ODDR model:", sum(true[t] == oddr(t) for t in driven), "of", len(driven))
print("complement = !true:   ", sum(comp[t] == 1 - true[t] for t in driven), "of", len(driven))
print("released, true pad 0: ", [t for t in T if released(t) and true[t]])
print("released, comp pad 0: ", [t for t in T if released(t) and comp[t]])
mismatch = [t for t in driven if true[t] != oddr(t) or comp[t] != 1 - true[t]]
print("driven mismatches:", mismatch[:40])
