"""Hypothesis search for the HI07 line (7:1 gearbox cycle behaviour).
Usage: python3 hi07_analyse.py "O=... I=... N=..."

Sample i of O is taken at the 25 MHz edge where t = 2048 + i, with
div = t mod 16 before it: eclk_in rises between samples 16m+7 and 16m+8 and
falls between 16m+15 and 16m+16, so an ECLK half period is 8 samples and
half h covers samples 8h..8h+7. p/n toggle on sclk rising/falling edges."""
import itertools
import sys

line = sys.argv[1].split()
o_hex, i_hex, N = line[0][2:], line[1][2:], int(line[2][2:], 16)
bits = bin(int(o_hex, 16))[2:].zfill(4 * len(o_hex))
S = [tuple(int(bits[4 * i + k]) for k in range(4)) for i in range(len(bits) // 4)]
eclk, p, n, pad = (list(c) for c in zip(*S))
assert all(eclk[i] == int(i % 16 >= 8) for i in range(len(S))), "eclk_in pattern"
rise = [i for i in range(1, len(S)) if p[i] != p[i - 1]]
fall = [i for i in range(1, len(S)) if n[i] != n[i - 1]]
print("N =", N)
print("sclk rising edges at samples", rise)
print("sclk falling edges at samples", fall)
halves = []
for h in range(len(S) // 8):
    v = {pad[8 * h + k] for k in range(8)}
    assert len(v) == 1, ("pad changes inside half", h)
    halves.append(v.pop())
print("pad per eclk half:", "".join(map(str, halves)))


def lfsr(seed, tap, count):
    s, out = seed, []
    for _ in range(count):
        out.append(s)
        s = ((s << 1) & 0x7F) | (((s >> 6) ^ (s >> tap)) & 1)
    return out


SEEDS = [(0x01, 5), (0x55, 4), (0x3A, 5), (0x6C, 4), (0x17, 5), (0x62, 4), (0x4D, 5)]
lanes = [lfsr(seed, tap, 600) for seed, tap in SEEDS]  # lanes[d][m]: after m sclk edges

# ODDR71B: for the j-th sclk rising edge in the window (absolute edge N+1+j),
# the 7 halves from half (rise_j // 8) + shift carry D_order[0..6] of the word
# captured at absolute edge N+1+j-lat (lane value after N+j-lat edges).
fits = []
for lat in range(0, 6):
    for shift in range(-8, 9):
        choice = []
        for pos in range(7):
            cands = []
            for d in range(7):
                ok, used = True, 0
                for j, r in enumerate(rise):
                    h = r // 8 + shift + pos
                    m = N + j - lat
                    if h < 0 or h >= len(halves) or m < 0:
                        continue
                    used += 1
                    if (lanes[d][m] & 1) != halves[h]:
                        ok = False
                        break
                if ok and used >= 3:
                    cands.append(d)
            choice.append(cands)
        if all(choice) and len({c[0] for c in choice if len(c) == 1}) == sum(len(c) == 1 for c in choice):
            fits.append((lat, shift, choice))
print("ODDR71B fits (latency in sclk edges, half shift from the rising sclk edge's half, lanes per half):")
for f in fits:
    print("  ", f)

# IDDR71B: record c (sclk edge k = 64 + c, pre-edge count) holds Q0..Q6
# published at an earlier sclk edge. Pad at an eclk edge at sample s (between
# s-1 and s): lw after j(s) = (2048 + s + 4) // 8 steps... lw steps at
# div[2:0] == 3, i.e. mid half, so the pad is stable at every eclk edge.
ibits = bin(int(i_hex, 16))[2:].zfill(4 * len(i_hex))
Q = [tuple(int(ibits[7 * c + x]) for x in range(7)) for c in range(16)]
lw = lfsr(0x2B, 5, 4000)


def pad_at_edge(s):
    return lw[(2048 + s + 4) // 8] & 1


# Absolute sample position of the k-th sclk rising edge: from the window,
# rises are 28 apart; edge number N+1+j at rise[j].
period = rise[1] - rise[0]
ifits = []
for base in range(-240, 40, 8):
    for order in itertools.permutations(range(7)):
        ok = True
        for c in range(16):
            k = 64 + c                       # count before the recording edge
            # The recording edge is edge k+1 and in-window edge j = k - N; it
            # reads Q published at the edge before, in-window edge j - 1.
            j = k - N - 1
            edge = rise[0] + j * period      # sample where that Q was published
            pads = [pad_at_edge(edge + base + 8 * x) for x in range(7)]
            if tuple(pads[order[x]] for x in range(7)) != Q[c]:
                ok = False
                break
        if ok:
            ifits.append((base, order))
print("IDDR71B fits (first of seven consecutive eclk edges, in samples from the sclk rising edge that published the word; which edge each Q0..Q6 holds):")
for f in ifits:
    print("  ", f)
