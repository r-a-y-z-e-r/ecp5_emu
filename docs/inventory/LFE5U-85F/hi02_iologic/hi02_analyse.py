"""Hypothesis search for the HI02 line (ODDRX1F/IDDRX1F cycle behaviour)."""
import sys
line = sys.argv[1]
o_hex, i_hex = line.split()[0][2:], line.split()[1][2:]
o_bits = bin(int(o_hex, 16))[2:].zfill(256)
i_bits = bin(int(i_hex, 16))[2:].zfill(64)

def lfsr(seed, tap, n):
    s, out = seed, []
    for _ in range(n):
        out.append(s)
        s = ((s << 1) & 0x7F) | (((s >> 6) ^ (s >> tap)) & 1)
    return out
la = lfsr(0x01, 5, 300)   # la[n] = state after n sclk rising edges
lb = lfsr(0x55, 4, 300)
lw = lfsr(0x3A, 5, 2000)  # lw[j] = state after j updates

# ODDR: 8 periods, samples T = 1024 + 16p + s.
lows, highs = [], []
for p in range(8):
    seg = o_bits[32 * p: 32 * p + 32]
    pairs = [(int(seg[2 * i]), int(seg[2 * i + 1])) for i in range(16)]
    assert all(s == 0 for s, _ in pairs[:8]) and all(s == 1 for s, _ in pairs[8:])
    lo, hi = {d for _, d in pairs[:8]}, {d for _, d in pairs[8:]}
    assert len(lo) == 1 and len(hi) == 1, (p, pairs)
    lows.append(lo.pop()); highs.append(hi.pop())
print("low phases :", lows)
print("high phases:", highs)
# Low phase of period p follows rising #64+p; high phase follows rising #65+p.
# A value captured at rising #r from register X is X[r-1] (pre-update).
for name, phase, base in (("low", lows, 64), ("high", highs, 65)):
    for src_name, src in (("D0=la", la), ("D1=lb", lb)):
        for lat in range(0, 4):
            # value on the pad in this phase = D captured at rising #(base+p-lat)
            pred = [src[base + p - lat - 1] & 1 for p in range(8)]
            if pred == phase:
                print(f"  {name} phase = {src_name} captured {lat} rising edge(s) earlier than the edge that opened the period")

# IDDR: record at rising #m for m = 65..96 of the (Q0, Q1) present before it.
q = [(int(i_bits[2 * i]), int(i_bits[2 * i + 1])) for i in range(32)]
# pad at rising #m = lw[8m-4][0], at falling #m = lw[8m][0]
rise = lambda m: lw[8 * m - 4] & 1
fall = lambda m: lw[8 * m] & 1
for qi in (0, 1):
    seq = [x[qi] for x in q]
    for kind, f in (("rising", rise), ("falling", fall)):
        for back in range(0, 4):
            pred = [f(m - back) for m in range(65, 97)]
            if pred == seq:
                print(f"Q{qi} at rising #m = pad at {kind} #m-{back}")
