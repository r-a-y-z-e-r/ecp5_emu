"""HI02b: explicit register model of ODDRX1F/IDDRX1F/OREG/TSREG/IREG, checked on every sample.
Model: DDR stages start at 0, async reset clears all stages while RST is high;
IO registers start at REGSET (SET here), OREG async set by RST."""
import sys
sys.argv = sys.argv[:2]
import pathlib
exec((pathlib.Path(__file__).parent / "hi02b_analyse.py").read_text().split("for w, window in")[0])  # decoding and LFSRs
A = [0, 0]; B = [0, 0]; C = [0, 0]       # ODDR stages (D0, D1)
P = N = Q0 = Q1 = 0                       # IDDR
oq, tq, iq = 1, 1, 1
released_at = None                        # sample at which the TSREG released the pad
pred, last_tq = {}, 1
for t in range(0, 1264):
    # Events between sample t-1 and t: clk edge t-1 moved div to t % 16.
    r = rises(t)
    if t % 16 == 8:                       # sclk rising edge #r
        C, B, A = B[:], A[:], [la[r - 1] & 1, lb[r - 1] & 1]
        Q0, Q1, P = P, N, lw[8 * r - 4] & 1
        oq, tq, iq = lc[r - 1] & 1, ld[r - 1] & 1, lv[8 * r - 4] & 1
    elif t % 16 == 0 and t > 0:           # sclk falling edge
        N = lw[8 * (t // 16)] & 1
    if rst(t):                            # asynchronous: rst rose mid-phase
        A = [0, 0]; B = [0, 0]; C = [0, 0]; P = N = Q0 = Q1 = 0
        oq = 1
    sclk = t % 16 >= 8
    if tq != last_tq:
        released_at = t if tq else None
        last_tq = tq
    pred[t] = {"oddr": C[0] if sclk else C[1], "q0": Q0, "q1": Q1, "oreg": oq,
               "ts": 1 - tq, "ireg": iq, "ts_released": released_at}
bad = {n: [] for n in NAMES}
lags = set()
for t in T:
    for n in NAMES:
        p, o = pred[t][n], S[t][n]
        if n == "ts" and p == 0 and o == 1 and pred[t]["ts_released"] is not None:
            lags.add(t - pred[t]["ts_released"])   # analog discharge after release
            continue
        if p != o:
            bad[n].append(t)
for n in NAMES:
    print(f"{n:5s}: {len(bad[n])} mismatches {bad[n][:20]}")
print("TSREG release-to-low lag (samples still high after release):", sorted(lags))
