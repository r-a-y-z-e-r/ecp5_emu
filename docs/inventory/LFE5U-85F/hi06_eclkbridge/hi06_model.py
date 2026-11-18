"""Check every HI06 line against the bridge model.
Usage: python3 hi06_model.py observations.log mode|nomode

Sample u holds {c0, c1, sel, pad, p2, n2}; c0 = div[2] and c1 = div[3] with
div = u mod 16, so c0 changes at every u = 4m and c1 at every u = 8m; sel
and STOP (from the experiment table) change between those.
Model:
- ECLKBRIDGECS with MODE=ECLKBRIDGECS: ECSOUT = SEL ? CLK1 : CLK0, at once
  (a plain mux: switching can cut or add a pulse). With MODE=NONE (what
  nextpnr writes): ECSOUT = CLK0 whatever SEL is.
- BRGECLKSYNC (an ECLKSYNCB): STOP through four stages on falling edges of
  its input, closed at the start (HI04); E00 tries both starts HI04 saw (the
  gate opening after four or five falling edges).
- CLKDIVF /2 toggles at each rising edge of ECLKO; p2/n2 toggle on its
  rising/falling edges.
- The pad (ODDRX2F with data 1,0,1,0) equals ECLKO once loaded; E00 is
  checked from the first SCLK rise + 40 samples on (the gearbox pipeline).
The free state at a window start is CDIVX's level; the line fits if one
level gives every sample."""
import sys
sys.path.insert(0, "../hi04_eclk")
from hi04_show import lines, samples

TABLE = {1: ("sel", 34, 68), 2: ("sel", 38, 68), 3: ("sel", 42, 68), 4: ("sel", 46, 68),
         5: ("stop", 34, 8), 6: ("stop", 34, 64), 7: ("stop", 38, 16)}


def simulate(rec, mode, controls, q0, start):
    stages = [1] * start if start else [0, 0, 0, 0]
    q, eclko, src_prev = q0, 0, None
    out = []
    for u in range(256):
        c0, c1 = int(u % 8 >= 4), int(u % 16 >= 8)
        ctl = {k: int(a <= u < a + d) for k, (a, d) in controls.items()}
        src = (c1 if ctl.get("sel") else c0) if mode else c0
        if src_prev is not None and src != src_prev:
            if src:
                if not stages[-1]:
                    eclko = 1
                    q ^= 1
            else:
                if eclko:
                    eclko = 0
                stages = [ctl.get("stop", 0)] + stages[:-1]
        src_prev = src
        out.append((c0, c1, ctl.get("sel", 0), eclko, q))
    return out


def toggles(wave, first, rising):
    v, res = first, []
    for u, x in enumerate(wave):
        if u and x != wave[u - 1] and x == rising:
            v ^= 1
        res.append(v)
    return res


if __name__ == "__main__":
    mode = sys.argv[2] == "mode"
    bad = 0
    for line in lines(sys.argv[1]):
        ident, h = line.split()
        n = int(ident[1:], 16)
        rec = list(zip(*samples(h)))
        controls = {} if n == 0 else {TABLE[n][0]: TABLE[n][1:]}
        good = []
        states = [(0, 4), (0, 5)] if n == 0 else [(0, 0), (1, 0)]
        for q0, start in states:
            sim = simulate(rec, mode, controls, q0, start)
            cd = [x[4] for x in sim]
            pred = [[x[0] for x in sim], [x[1] for x in sim], [x[2] for x in sim],
                    [x[3] for x in sim], toggles(cd, rec[4][0], 1), toggles(cd, rec[5][0], 0)]
            first = 0
            if n == 0:
                rises = [u for u in range(1, 256) if cd[u] and not cd[u - 1]]
                first = rises[0] + 40 if rises else 256
            ok = all(list(pred[k]) == list(rec[k]) for k in (0, 1, 2, 4, 5))
            ok = ok and list(pred[3][first:]) == list(rec[3][first:])
            if ok:
                good.append((q0, start))
        note = f" (gate opens after {good[0][1]} falling edges)" if n == 0 and good else ""
        print(ident, ("fits" + note) if good else "NO FIT")
        bad += not good
    print("lines without a fit:", bad)
