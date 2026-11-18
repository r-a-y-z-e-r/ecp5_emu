"""Hypothesis search for the HI03 line (X2 gearbox cycle behaviour).
Usage: python3 hi03_analyse.py "O=... I=..."

Time: sample i of O is taken at the 25 MHz edge where t = 2048 + i; every
register of hi03.v starts at its init value with t = 0, so the 4-bit divider
div equals t mod 16 before each edge. eclk_in = div[3] rises between samples
16m+7 and 16m+8 and falls between 16m+15 and 16m+16."""
import itertools
import sys

line = sys.argv[1]
o_hex, i_hex = line.split()[0][2:], line.split()[1][2:]
# N: sclk rising edges before sample 0, counted by the design itself.
N = int(line.split()[2][2:], 16)
print("N =", N)
o_bits = bin(int(o_hex, 16))[2:].zfill(576)
i_bits = bin(int(i_hex, 16))[2:].zfill(128)
samples = [tuple(int(o_bits[3 * i + k]) for k in range(3)) for i in range(192)]


def lfsr(seed, tap, n):
    s, out = seed, []
    for _ in range(n):
        out.append(s)
        s = ((s << 1) & 0x7F) | (((s >> 6) ^ (s >> tap)) & 1)
    return out


SEEDS = [(0x01, 5), (0x55, 4), (0x3A, 5), (0x6C, 4)]
lanes = [lfsr(seed, tap, 400) for seed, tap in SEEDS]  # lanes[d][n]: after n sclk edges
lw = lfsr(0x2B, 5, 3000)                               # lw[j]: after j updates

# Clocks: eclk_in and the sclk toggle as recorded.
eclk = [s[0] for s in samples]
assert all(eclk[i] == int(i % 16 >= 8) for i in range(192)), "eclk_in pattern"
st = [s[1] for s in samples]
sclk_at = [i for i in range(1, 192) if st[i] != st[i - 1]]
print("sclk rising edges seen at samples", sclk_at, "(an eclk rising edge between i-1 and i)")
# sclk rises at every other eclk rising edge: at 8 mod 32 in the recorded
# build, at 24 mod 32 in the first build (the design before the N field),
# the same in every load of a build.
P = sclk_at[0] % 32
assert P in (8, 24) and all(i % 32 == P for i in sclk_at), "sclk phase"
HS = P // 8  # the eclk half that starts with an sclk rising edge

# ODDRX2F: the pad in each eclk half period (8 samples) must be constant.
halves = []
for h in range(24):
    vals = {samples[8 * h + k][2] for k in range(8)}
    assert len(vals) == 1, ("pad changes inside half", h)
    halves.append(vals.pop())
print("pad per eclk half (h even: eclk low):", "".join(map(str, halves)))

# Hypothesis: from the half that starts at an sclk edge (h = 3 + 4n), the
# four halves carry the four lanes in some order, from data captured at an
# sclk edge `lat` edges before, with an unknown count n0 of sclk edges
# before sample 0. The starting half may also be shifted by `shift` halves.
fits = []
for n0 in [N]:
    for lat in range(0, 8):
        for shift in range(-3, 4):
            for order in itertools.permutations(range(4)):
                ok, count = True, 0
                for h in range(24):
                    rel = h - HS - shift
                    group, pos = divmod(rel, 4)
                    n = n0 + group + 1 - lat  # sclk edge whose data is shown
                    if n - 1 < 0:
                        continue
                    lane = order[pos]
                    pred = lanes[lane][n - 1] & 1
                    count += 1
                    if pred != halves[h]:
                        ok = False
                        break
                if ok and count >= 20:
                    fits.append((n0, lat, shift, order))
print("ODDRX2F fits (n0, latency in sclk edges, half shift, lane per half):")
for f in fits:
    print("  ", f)

# IDDRX2F: record at sclk edge with k = 64..95 (pre-edge count) of the Q0..Q3
# present before it. Pad samples: at an eclk rising edge between samples
# 16m+7 and 16m+8 the pad holds lw[j(16m+8)], at a falling edge between
# 16m+15 and 16m+16 it holds lw[j(16m+16)], where j(i) = (2048 + i + 4) // 8.
q = [tuple(int(i_bits[4 * c + k]) for k in range(4)) for c in range(32)]


def j(i):
    return (2048 + i + 4) // 8


# Absolute sample index (relative to sample 0 at t = 2048) of each sclk edge:
# sclk rises at the eclk rising edge between 32p+23 and 32p+24. The k-th
# sclk edge (k counted from the first) lies at 32(k - k0) + 24 for some k0.
ifits = []
for k0 in [N]:
    for lat in range(0, 8):
        for order in itertools.permutations(range(4)):
            for base in range(-4, 4):
                ok = True
                for c in range(32):
                    k = 64 + c
                    edge = 32 * (k - k0) + P  # sample index of this sclk edge
                    # the four eclk edges (rise, fall, rise, fall) before the
                    # capture `lat` sclk edges earlier, shifted by base halves
                    first = edge - 32 * lat + 16 * 0 + 8 * base
                    pads = []
                    for e in range(4):
                        at = first - 32 + 8 * e
                        pads.append(lw[j(at)] & 1)
                    pred = tuple(pads[order[x]] for x in range(4))
                    if pred != q[c]:
                        ok = False
                        break
                if ok:
                    ifits.append((k0, lat, order, base))
print("IDDRX2F fits (k0, latency, eclk edge per Q0..Q3, half shift):")
for f in ifits[:20]:
    print("  ", f)
print(len(ifits), "IDDRX2F fits")
