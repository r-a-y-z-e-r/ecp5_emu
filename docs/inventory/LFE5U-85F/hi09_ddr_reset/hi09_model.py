"""Check the HI09 records against models of RST and GSR on ODDRX1F/IDDRX1F.
Usage: python3 hi09_model.py observations.log

Each line is 256 samples of {sclk, rst, gsr_n, din, d0, d1, ob1, ob0, q0e,
q1e, q0d, q1d}, one per 25 MHz edge, taken before the edge. sclk = 25 MHz / 4;
rst, gsr_n and din change only at edges between sclk edges; d0/d1 change just
after sclk rises. Unit 0 of each kind has GSR=ENABLED, unit 1 GSR=DISABLED.
The DDR paths are the ones HI02 measured (kernel/iologic.rs):
- ODDRX1F: at a sclk rise s0 <- D0, s1 <- D1, s2 <- s0, s3 <- s1, s4 <- s3,
  Q <- s2; at a fall Q <- s4.
- IDDRX1F: at a rise Q0 <- s0, Q1 <- s1, s0 <- D; at a fall s1 <- D.
Variants of a reset (RST, and GSR low on a GSR=ENABLED unit):
- scope: every stage and the outputs, or the outputs only;
- timing: asynchronous (at once, held while active) or at the next sclk edge.
Unknown state (None) is not compared: each window after E00 starts unknown."""
import re, sys, itertools

# ob is a [1:0] vector in hi09.v: ob[1] (GSR=DISABLED) comes first.
FIELDS = ["sclk", "rst", "gsr_n", "din", "d0", "d1", "ob1", "ob0", "q0e", "q1e", "q0d", "q1d"]


def samples(item):
    bits = bin(int(item[4:], 16))[2:].zfill(3072)
    return [dict(zip(FIELDS, map(int, bits[12 * i:12 * i + 12]))) for i in range(256)]


def run(s, known, scope, timing):
    """Model outputs after each edge; returns (compared, differing indices)."""
    z = 0 if known else None
    od = [[z] * 5 + [z] for _ in range(2)]   # stages s0..s4, Q
    idd = [[z, z, z, z] for _ in range(2)]   # s0, s1, Q0, Q1
    bad, compared = [], 0
    for k in range(255):
        a, b = s[k], s[k + 1]
        rise = not a["sclk"] and b["sclk"]
        fall = a["sclk"] and not b["sclk"]
        for u in range(2):
            o, i = od[u], idd[u]
            if rise:
                o[0:6] = [a["d0"], a["d1"], o[0], o[1], o[3], o[2]]
                i[0:4] = [a["din"], i[1], i[0], i[1]]
            if fall:
                o[5] = o[4]
                i[1] = a["din"]
            # Reset active after this edge (b), or at the edge (a) for sync.
            on = lambda x: x["rst"] or (u == 0 and not x["gsr_n"])
            if timing == "async":
                active = on(b)
            else:
                active = on(a) and (rise or fall)
            if active:
                if scope == "all":
                    o[:] = [0] * 6
                    i[:] = [0] * 4
                else:
                    o[5] = 0
                    i[2] = i[3] = 0
        model = [od[0][5], od[1][5], idd[0][2], idd[0][3], idd[1][2], idd[1][3]]
        seen = [b["ob0"], b["ob1"], b["q0e"], b["q1e"], b["q0d"], b["q1d"]]
        for m, v in zip(model, seen):
            if m is None:
                continue
            compared += 1
            if m != v:
                bad.append(k + 1)
    return compared, bad


items = sorted(set(re.findall(r"'(E[0-9A-F]{2} [0-9A-F]{768})'", open(sys.argv[1]).read())))
for scope, timing in itertools.product(["all", "outputs"], ["async", "sync"]):
    total, wrong, lines = 0, 0, []
    for item in items:
        c, bad = run(samples(item), item.startswith("E00"), scope, timing)
        total += c
        wrong += len(bad)
        if bad:
            lines.append((item[:3], bad[:4]))
    print(f"reset {scope:7} {timing:5}: {total} compared, {wrong} differ; {lines[:4]}")
