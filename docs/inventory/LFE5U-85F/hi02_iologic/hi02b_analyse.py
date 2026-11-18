"""HI02b: decode the samples and test models of IOLOGIC start-up, reset and IO registers."""
import sys
line = sys.argv[1]
bits = bin(int(line[2:], 16))[2:].zfill(2304)
T = list(range(0, 128)) + list(range(1008, 1264))
NAMES = ["oddr", "q0", "q1", "oreg", "ts", "ireg"]
S = {t: dict(zip(NAMES, map(int, bits[6 * i: 6 * i + 6]))) for i, t in enumerate(T)}

def lfsr(seed, tap, n):
    s, out = seed, []
    for _ in range(n):
        out.append(s)
        s = ((s << 1) & 0x7F) | (((s >> 6) ^ (s >> tap)) & 1)
    return out
la, lb, lc, ld = lfsr(0x01, 5, 400), lfsr(0x55, 4, 400), lfsr(0x2B, 5, 400), lfsr(0x4C, 4, 400)
lw, lv = lfsr(0x3A, 5, 3000), lfsr(0x66, 4, 3000)
rst = lambda t: 1028 <= t <= 1099           # value seen by the sample at edge t
rises = lambda t: (t + 8) // 16             # sclk rising edges before sample t (rising after edges 7, 23, ...)
falls = lambda t: t // 16                   # falling edges before sample t (after edges 15, 31, ...)
pad_w_at_rise = lambda m: lw[8 * m - 4] & 1
pad_w_at_fall = lambda m: lw[8 * m] & 1
pad_v_at_rise = lambda m: lv[8 * m - 4] & 1

def show(window):
    print("   t div sclk rst | oddr q0 q1 oreg ts ireg")
    for t in window:
        s = S[t]
        print(f"{t:5d} {t % 16:3d} {int(t % 16 >= 8):4d} {int(rst(t)):3d} | {s['oddr']:4d} {s['q0']:2d} {s['q1']:2d} {s['oreg']:4d} {s['ts']:2d} {s['ireg']:4d}")

def runs(name, window):
    out, prev, start = [], None, None
    for t in window:
        v = S[t][name]
        if v != prev:
            if prev is not None:
                out.append(f"{prev}@{start}-{t - 1}")
            prev, start = v, t
    out.append(f"{prev}@{start}-{window[-1]}")
    return " ".join(out)

for w, window in (("A", list(range(0, 128))), ("B", list(range(1008, 1264)))):
    print(f"== window {w}")
    for n in NAMES:
        print(f"  {n:5s} {runs(n, window)}")

# Steady-state models (HI02 for DDR; plain FFs for IO registers).
def model(t):
    r, f = rises(t), falls(t)
    high = t % 16 >= 8
    m = {}
    # ODDR: high phase after rising #r shows D0 captured at rising #(r-2), low phase after falling #f shows D1 captured at rising #(f-2)
    m["oddr"] = (la[r - 3] & 1) if high else (lb[f - 3] & 1)
    # IDDR: outputs updated at rising #r: Q0 = pad at rising #(r-1), Q1 = pad at falling #(r-1)
    m["q0"], m["q1"] = pad_w_at_rise(r - 1), pad_w_at_fall(r - 1)
    m["oreg"] = lc[r - 1] & 1
    m["ts"] = 1 - (ld[r - 1] & 1)
    m["ireg"] = pad_v_at_rise(r)
    return m
for n in NAMES:
    bad = [t for t in T if rises(t) >= 4 and not rst(t) and S[t][n] != model(t)[n]]
    print(f"steady model {n:5s}: mismatches at {bad[:40]}{' ...' if len(bad) > 40 else ''}")
if len(sys.argv) > 2:
    show([t for t in T if int(sys.argv[2]) <= t <= int(sys.argv[3])])

# TSREG: per sclk period, the predicted T (tq after rising #r) and the 16 observed pad samples.
print("== TSREG per period (T predicted; pad samples from the rising edge on)")
for w in (range(8, 128 - 16, 16), range(1016, 1264 - 16, 16)):
    for s0 in w:
        r = rises(s0)
        tq = ld[r - 1] & 1
        obs = "".join(str(S[t]["ts"]) for t in range(s0, s0 + 16))
        print(f"  rising #{r:3d} at sample {s0:4d}: T={tq} expected pad={1 - tq}  observed {obs}")
