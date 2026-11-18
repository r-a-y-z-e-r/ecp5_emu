"""Find which CE, which RST and what GSR the ALU54B output register takes
(HD06). Usage: python3 hd06_analyse.py observations.log

Each window holds 256 samples of {cea[3:0], rsta[3:0], ceb[3:0], rstb[3:0],
000, gsr_n, c[50], c[30], c[20], c[0], and R[50], R[30], R[20], R[0] of ea,
eb, ec, ed}; sample k is the state before 25 MHz edge k. Every ALU computes
R = C. For each ALU and each of the four bits (one per group of R) every
model is tried:
  enable: none, or one of the block's CE0..3;
  reset:  none, or one of the block's RST0..3, synchronous or asynchronous;
  GSR:    ignored, or an asynchronous clear while gsr_n is low.
With e, r the enable and the reset before edge k, and r', g' the reset and
GSR after it:
  R(k+1) = 0 if g' (GSR)
           else 0 if r (a reset of either kind holds through the edge)
           else 0 if r' and the reset is asynchronous (it clears at once)
           else C(k) if e else R(k).
Prints the models that give every sample of every window."""
import ast
import itertools
import sys

lines = set()
for line in open(sys.argv[1]):
    lines |= {x for x in ast.literal_eval(line.partition("distinct=")[2]) if len(x) == 2561}
print("distinct windows:", len(lines))
ALUS = [("ea", "a"), ("eb", "a"), ("ec", "b"), ("ed", "b")]
BITS = [50, 30, 20, 0]


def samples(x):
    bits = bin(int(x[1:], 16))[2:].zfill(10240)
    out = []
    for i in range(256):
        w = bits[40 * i:40 * i + 40]
        v = lambda a, b: int(w[a:b], 2)
        s = dict(cea=v(0, 4), rsta=v(4, 8), ceb=v(8, 12), rstb=v(12, 16), gsr=1 - v(19, 20), c=v(20, 24))
        for n, (name, _) in enumerate(ALUS):
            s[name] = v(24 + 4 * n, 28 + 4 * n)
        out.append(s)
    return out


windows = [samples(x) for x in sorted(lines)]
print("samples:", 256 * len(windows), "GSR low in", sum(s["gsr"] for w in windows for s in w),
      "reset bits high in", sum(bin(s["rsta"] | s["rstb"] << 4).count("1") for w in windows for s in w))


def fits(name, block, bit, ce, rst, mode, gsr):
    for w in windows:
        for k in range(255):
            a, b = w[k], w[k + 1]
            e = ce is None or a["ce" + block] >> ce & 1
            r = rst is not None and a["rst" + block] >> rst & 1
            r2 = rst is not None and b["rst" + block] >> rst & 1
            if (gsr and b["gsr"]) or r or (mode == "async" and r2):
                want = 0
            elif e:
                want = a["c"] >> bit & 1
            else:
                want = a[name] >> bit & 1
            if b[name] >> bit & 1 != want:
                return False
    return True


for name, block in ALUS:
    for bit, label in enumerate(BITS[::-1]):
        found = []
        for ce, rst, mode, gsr in itertools.product([None, 0, 1, 2, 3], [None, 0, 1, 2, 3], ["sync", "async"], [False, True]):
            if rst is None and mode == "async":
                continue
            if fits(name, block, bit, ce, rst, mode, gsr):
                found.append(f"CE{ce if ce is not None else ' none'}, RST{rst if rst is not None else ' none'}"
                             + (f" {mode}" if rst is not None else "") + (", GSR clears" if gsr else ", GSR ignored"))
        print(f"{name} R[{label}]: {found or 'NO MODEL FITS'}")
