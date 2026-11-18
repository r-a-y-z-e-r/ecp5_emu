"""HI07b: the IDDR71B outputs sampled at 25 MHz. Usage: python3 hi07b_check.py
Reads pass_b.txt ("O=... Q=... I=... N=..."). Checks that HI07b's O and I equal
HI07's (pass.txt), finds where Q0..Q6 change, and fits the word published at
each sclk rising edge P to the pad (lw[0], recorded with Q) at seven
consecutive eclk edges."""
import itertools

x = open("pass_b.txt").read().split()
h7 = open("pass.txt").read().split()
print("O and I as in HI07:", x[0] == h7[0], x[2] == h7[1], "N:", x[3], h7[2])
o, q = x[0][2:], x[1][2:]
ob = bin(int(o, 16))[2:].zfill(4 * len(o))
qb = bin(int(q, 16))[2:].zfill(4 * len(q))
S = [[int(c) for c in ob[4 * k:4 * k + 4]] for k in range(384)]
Q = [[int(c) for c in qb[8 * k:8 * k + 8]] for k in range(384)]
p = [s[1] for s in S]
rise = [k for k in range(1, 384) if p[k] != p[k - 1]]
lw0 = [v[0] for v in Q]
qw = [tuple(v[1:]) for v in Q]
print("sclk rising edges:", rise)
print("Q changes at:", [k for k in range(1, 384) if qw[k] != qw[k - 1]])
fits = []
for base in range(-200, 1, 8):
    for order in itertools.permutations(range(7)):
        ok, used = True, 0
        for P in rise:
            if P + base < 1:
                continue
            pads = [lw0[P + base + 8 * e] for e in range(7)]
            if tuple(pads[order[k]] for k in range(7)) != qw[P]:
                ok = False
                break
            used += 1
        if ok and used >= 3:
            fits.append((base, order, used))
print("fits (first edge from P, edge index held by Q0..Q6, words checked):", fits)
