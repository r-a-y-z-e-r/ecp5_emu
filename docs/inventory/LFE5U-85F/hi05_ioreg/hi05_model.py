"""Check every HI05 line against the IO register model.
Usage: python3 hi05_model.py observations.log

A line "E<id> <768 hex>" holds 256 samples of {d, ce, sr, gsr_n, ob3..ob0,
qi3..qi0}. Sample k is the state after 25 MHz edge k (before edge k+1), so a
register's value in sample k was captured at edge k from sample k-1.

Model (each register, with REGSET R):
- synchronous set/reset (I, J; SRMODE=LSR_OVER_CE):
  Q(k) = sr(k-1) ? R : ce(k-1) ? D(k-1) : Q(k-1), so sr wins over ce;
- asynchronous (B, D; SRMODE=ASYNC): Q(k) = R while sr(k), and the edge
  after a release still sees sr high: Q(k) = R if sr(k) or sr(k-1);
- GSR enabled: the user GSR (gsr_n low) acts like an asynchronous reset;
  GSR disabled: it does nothing;
- D is d for the output registers and the pad (driven with d) for the
  input registers;
- E00 starts every register at R. Its sample 0 (t = 0, the first edge after
  the start) is reported apart: in one load of three every bit read 0 there,
  fabric registers included, and the model starts at sample 1."""
import ast
import sys

# name: (bit in the sample, REGSET, synchronous, GSR enabled)
REGS = {
    "O0 OFS1P3BX": (7, 1, False, True),
    "O1 OFS1P3DX": (6, 0, False, False),
    "O2 OFS1P3IX": (5, 0, True, True),
    "O3 OFS1P3JX": (4, 1, True, False),
    "I0 IFS1P3BX": (11, 1, False, False),
    "I1 IFS1P3DX": (10, 0, False, True),
    "I2 IFS1P3IX": (9, 0, True, False),
    "I3 IFS1P3JX": (8, 1, True, True),
}


def samples(hexs):
    bits = bin(int(hexs, 16))[2:].zfill(4 * len(hexs))
    return [[int(bits[12 * i + j]) for j in range(12)] for i in range(len(bits) // 12)]


def predict(s, bit, r, sync, gsr_en, start):
    d = [x[0] for x in s]
    ce = [x[1] for x in s]
    sr = [x[2] for x in s]
    gsr = [1 - x[3] if gsr_en else 0 for x in s]
    q = [start]
    for k in range(1, len(s)):
        if gsr[k] or gsr[k - 1]:
            v = r
        elif sync:
            v = r if sr[k - 1] else d[k - 1] if ce[k - 1] else q[-1]
        else:
            v = r if (sr[k] or sr[k - 1]) else d[k - 1] if ce[k - 1] else q[-1]
        q.append(v)
    return q


def lines(path):
    out = set()
    for l in open(path):
        if "distinct=" in l:
            out |= {x for x in ast.literal_eval(l.partition("distinct=")[2]) if len(x) == 772}
    return sorted(out)


if __name__ == "__main__":
    bad = 0
    events = {"sr while ce=0": 0, "gsr pulses": 0}
    for line in lines(sys.argv[1]):
        ident, h = line.split()
        s = samples(h)
        events["sr while ce=0"] += sum(1 for x in s if x[2] and not x[1])
        events["gsr pulses"] += sum(1 for k in range(1, len(s)) if not s[k][3] and s[k - 1][3])
        wrong = []
        if ident == "E00":
            print("E00 sample 0 (t = 0):", s[0])
            s = s[1:]
        for name, (bit, r, sync, gsr_en) in REGS.items():
            obs = [x[bit] for x in s]
            start = r if ident == "E00" else obs[0]
            if ident == "E00" and obs[0] != r:
                wrong.append(f"{name} starts at {obs[0]}")
            pred = predict(s, bit, r, sync, gsr_en, start)
            miss = [k for k in range(len(s)) if pred[k] != obs[k]]
            if miss:
                wrong.append(f"{name} differs at {miss[:6]}")
        print(ident, "fits" if not wrong else "NO FIT: " + "; ".join(wrong))
        bad += bool(wrong)
    print("lines:", len(lines(sys.argv[1])), "without a fit:", bad, "| exercised:", events)
