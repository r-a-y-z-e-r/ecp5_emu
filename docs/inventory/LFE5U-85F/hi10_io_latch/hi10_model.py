"""Check the HI10 records against models of the IO latch.
Usage: python3 hi10_model.py observations_ff.log observations.log

Each line is 256 samples of {sclk, div0, ce, rst, din, dout, ob1, ob0, qi1,
qi0, 0, 0}, one per 25 MHz edge, taken before the edge. sclk = 25 MHz / 4;
din, dout, ce and rst change only at edges between sclk edges. O1 and I1 are
flip-flops (OFS1P3DX/IFS1P3DX: CE, asynchronous clear) in both builds; O0 and
I0 are flip-flops in hi10_ff and latches in hi10 (hi10_latch.py).
The latch that fits (`hold`): transparent while sclk is low and CE is high,
holding its output otherwise (also when LSR and CE fall together); rst clears
it at once and while high. Rejected variants: transparent while sclk is high;
CE ignored; at a closing edge, D before or after the edge instead of the held
output.
Unknown state (None) is not compared: each window after E00 starts unknown."""
import re, sys, itertools

FIELDS = ["sclk", "div0", "ce", "rst", "din", "dout", "ob1", "ob0", "qi1", "qi0"]


def samples(item):
    bits = bin(int(item[4:], 16))[2:].zfill(3072)
    return [dict(zip(FIELDS, map(int, bits[12 * i:12 * i + 10]))) for i in range(256)]


def flip_flop(q, a, b, d):
    if not a["sclk"] and b["sclk"] and a["ce"]:
        q = a[d]
    return 0 if b["rst"] else q


def latch(level, gated, late):
    def step(q, a, b, d):
        x = b if late else a
        if b["sclk"] == level and (b["ce"] if gated else 1):
            q = b[d]
        elif a["sclk"] == level and (a["ce"] if gated else 1):
            q = x[d]  # closing at this edge: D before or after it
        return 0 if b["rst"] else q
    return step


def hold(level):
    def step(q, a, b, d):
        if b["sclk"] == level and b["ce"]:
            q = b[d]
        return 0 if b["rst"] else q
    return step


def check(items, kinds):
    """kinds: (model for O0, model for I0); O1/I1 are flip-flops."""
    compared, bad = 0, 0
    for item in items:
        s = samples(item)
        z = 0 if item.startswith("E00") else None
        q = {"ob0": z, "ob1": z, "qi0": z, "qi1": z}
        for k in range(255):
            a, b = s[k], s[k + 1]
            for name, d, model in [("ob0", "dout", kinds[0]), ("ob1", "dout", flip_flop),
                                   ("qi0", "din", kinds[1]), ("qi1", "din", flip_flop)]:
                q[name] = model(q[name], a, b, d)
                if q[name] is not None:
                    compared += 1
                    bad += q[name] != b[name]
    return compared, bad


def items(path):
    return sorted(set(re.findall(r"'(E[0-9A-F]{2} [0-9A-F]{768})'", open(path).read())))


ff, la = items(sys.argv[1]), items(sys.argv[2])
c, b = check(ff, (flip_flop, flip_flop))
print(f"hi10_ff, all flip-flops: {c} compared, {b} differ")
c, b = check(la, (flip_flop, flip_flop))
print(f"hi10, O0/I0 as flip-flops: {c} compared, {b} differ")
c, b = check(la, (hold(0), hold(0)))
print(f"hi10, latch transparent while sclk=0 and CE=1, else holding its output: {c} compared, {b} differ")
c, b = check(la, (hold(1), hold(1)))
print(f"hi10, the same transparent while sclk=1: {c} compared, {b} differ")
for level, gated, late in itertools.product([1, 0], [True, False], [False, True]):
    m = latch(level, gated, late)
    c, b = check(la, (m, m))
    print(f"hi10, latch transparent while sclk={level}, CE {'gates' if gated else 'ignored'}, "
          f"closing takes D {'after' if late else 'before'} the edge: {c} compared, {b} differ")
