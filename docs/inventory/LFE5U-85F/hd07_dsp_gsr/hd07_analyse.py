"""Find which CE, which RST, what reset kind and what GSR the DSP output
registers take (HD07). Usage: python3 hd07_analyse.py observations.log

Each window holds 256 samples of {cea[3:0], rsta[3:0], ceb[3:0], rstb[3:0],
000, gsr_n, c[50], c[30], c[20], c[0], R[50], R[30], R[20], R[0] of ea, eb,
ec, ed, 00, c[16], c[1], P[33], P[18], P[16], P[1] of ma, mb, mc, md};
sample k is the state before 25 MHz edge k. Every ALU computes R = C, every
multiplier P[16:1] = P[33:18] = c[16:1]. For each cell and each recorded bit
every model is tried:
  enable: none, or one of the block's CE0..3;
  reset:  none, or one of the block's RST0..3, synchronous or asynchronous;
  GSR:    ignored; asynchronous (clears while gsr_n is low and holds the
          register through an edge it is low before); asynchronous without
          the hold; synchronous (clears at an edge it is low before).
With e, r, g the enable, the reset and GSR before edge k, and r', g' after it:
  R(k+1) = 0 if GSR clears (g' or g, g' alone, or g alone, by kind)
           else 0 if r (a reset of either kind holds through the edge)
           else 0 if r' and the reset is asynchronous (it clears at once)
           else D(k) if e else R(k).
Prints the models that give every sample of every window."""
import ast
import itertools
import sys

lines = set()
loads = []
for line in open(sys.argv[1]):
    items = {x for x in ast.literal_eval(line.partition("distinct=")[2]) if len(x) == 3841}
    loads.append(items)
    lines |= items
print("loads:", len(loads), "full windows per load:", [len(l) for l in loads],
      "in all loads:", len(set.intersection(*loads)), "distinct:", len(lines))

# cell, block of its controls, field offset, data bits (sample field, bit) per recorded bit
ALUS = [("ea", "a"), ("eb", "a"), ("ec", "b"), ("ed", "b")]
MULTS = [("ma", "b"), ("mb", "b"), ("mc", "b"), ("md", "b")]
ALU_BITS = [50, 30, 20, 0]          # recorded bits of R, MSB of the field first
MULT_BITS = [33, 18, 16, 1]         # recorded bits of P
MULT_DATA = {33: 16, 18: 1, 16: 16, 1: 1}  # the c bit each P bit carries


def samples(x):
    bits = bin(int(x[1:], 16))[2:].zfill(15360)
    out = []
    for i in range(256):
        w = bits[60 * i:60 * i + 60]
        v = lambda a, b: int(w[a:b], 2)
        s = dict(cea=v(0, 4), rsta=v(4, 8), ceb=v(8, 12), rstb=v(12, 16), gsr=1 - v(19, 20))
        s["c"] = {50: v(20, 21), 30: v(21, 22), 20: v(22, 23), 0: v(23, 24), 16: v(42, 43), 1: v(43, 44)}
        for n, (name, _) in enumerate(ALUS):
            s[name] = {b: v(24 + 4 * n + k, 25 + 4 * n + k) for k, b in enumerate(ALU_BITS)}
        for n, (name, _) in enumerate(MULTS):
            s[name] = {b: v(44 + 4 * n + k, 45 + 4 * n + k) for k, b in enumerate(MULT_BITS)}
        out.append(s)
    return out


windows = [samples(x) for x in sorted(lines)]
print("samples:", 256 * len(windows), "GSR low in", sum(s["gsr"] for w in windows for s in w),
      "reset bits high in", sum(bin(s["rsta"] | s["rstb"] << 4).count("1") for w in windows for s in w))

GSR_KINDS = ["ignored", "async", "async, no hold", "sync"]


def fits(name, block, bit, data, ce, rst, mode, gsr):
    for w in windows:
        for k in range(255):
            a, b = w[k], w[k + 1]
            e = ce is None or a["ce" + block] >> ce & 1
            r = rst is not None and a["rst" + block] >> rst & 1
            r2 = rst is not None and b["rst" + block] >> rst & 1
            g = {"ignored": False, "async": a["gsr"] or b["gsr"],
                 "async, no hold": b["gsr"], "sync": a["gsr"]}[gsr]
            if g or r or (mode == "async" and r2):
                want = 0
            elif e:
                want = a["c"][data]
            else:
                want = a[name][bit]
            if b[name][bit] != want:
                return False
    return True


def models(name, block, bit, data):
    found = []
    for ce, rst, mode, gsr in itertools.product([None, 0, 1, 2, 3], [None, 0, 1, 2, 3], ["sync", "async"], GSR_KINDS):
        if rst is None and mode == "async":
            continue
        if fits(name, block, bit, data, ce, rst, mode, gsr):
            found.append(f"CE{ce if ce is not None else ' none'}, RST{rst if rst is not None else ' none'}"
                         + (f" {mode}" if rst is not None else "") + f", GSR {gsr}")
    return found or "NO MODEL FITS"


for name, block in ALUS:
    for bit in ALU_BITS[::-1]:
        print(f"{name} R[{bit}]: {models(name, block, bit, bit)}")
for name, block in MULTS:
    for bit in MULT_BITS[::-1]:
        print(f"{name} P[{bit}]: {models(name, block, bit, MULT_DATA[bit])}")
