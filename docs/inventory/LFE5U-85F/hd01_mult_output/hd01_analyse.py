"""Find which CE and RST drive the MULT18X18D output register (HD01).
Usage: python3 hd01_analyse.py observations.log

Each window holds 256 samples of {ce[3:0], rst[3:0], a, b, p0, p1}; sample
k is the state after 25 MHz edge k. The model tried for each multiplier and
every choice of CE (none or CE0..3) and RST (none or RST0..3), with a
synchronous reset: P(k) = rst(k-1) ? 0 : ce(k-1) ? product(k-1) : P(k-1),
on the low 4 bits (m0: a*b, m1: a*(b+1))."""
import ast
import sys

lines = set()
for line in open(sys.argv[1]):
    lines |= {x for x in ast.literal_eval(line.partition("distinct=")[2]) if len(x) == 1538}
print("distinct windows:", len(lines))


def samples(x):
    bits = bin(int(x[2:], 16))[2:].zfill(6144)
    out = []
    for i in range(256):
        w = bits[24 * i:24 * i + 24]
        v = lambda a, b: int(w[a:b], 2)
        out.append(dict(ce=v(0, 4), rst=v(4, 8), a=v(8, 12), b=v(12, 16), p0=v(16, 20), p1=v(20, 24)))
    return out


windows = [samples(x) for x in sorted(lines)]


def fits(which, ce_i, rst_i):
    for s in windows:
        for k in range(1, 256):
            prev = s[k - 1]
            b = prev["b"] if which == "p0" else (prev["b"] + 1) & 15
            product = (prev["a"] * b) & 15
            r = rst_i is not None and (prev["rst"] >> rst_i) & 1
            c = ce_i is None or (prev["ce"] >> ce_i) & 1
            want = 0 if r else (product if c else s[k - 1][which])
            if s[k][which] != want:
                return False
    return True


for which, params in (("p0", "m0: REG_OUTPUT_CE CE2, REG_OUTPUT_RST RST3"),
                      ("p1", "m1: REG_OUTPUT_CE CE1, REG_OUTPUT_RST RST2")):
    res = [(c, r) for c in [None, 0, 1, 2, 3] for r in [None, 0, 1, 2, 3] if fits(which, c, r)]
    print(f"{which} ({params}): fits (CE index, RST index) = {res}")
