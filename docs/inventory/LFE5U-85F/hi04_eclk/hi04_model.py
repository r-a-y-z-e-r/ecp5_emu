"""The HI04 edge-clock model, checked against every recorded line.
Usage: python3 hi04_model.py hi04|hi04b observations.log

Time is the sample index u of a 256-sample window; ECLKI rises between
samples 8m+3 and 8m+4 and falls between 8m+7 and 8m+8 (an edge "at u" is
between u-1 and u). A control is high from sample a to a+dur-1.

Model:
- ECLKSYNCB: STOP goes through four stages clocked by ECLKI falling edges;
  ECLKO = ECLKI while the last stage is 0. At the start all stages are 1.
- CLKDIVF: RST clears CDIVX at once; after release the first ECLKO rising
  edge is consumed and the second raises CDIVX. /2 toggles at each rising
  edge; /3.5 counts both edges, high for 4 of every 7.
- ALIGNWD is sampled at ECLKO rising edges. The first sample that sees it
  high (after one that saw it low) starts a slip two rising edges later: the
  divider ignores that rising edge and the falling edge after it. /3.5 slips
  on every ALIGNWD; /2 on every second one (RST clears the count).
- Not measured, and as the emulator has it: RST also clears the ALIGNWD
  sample and pending slips, and a slip due in the wait after a reset is
  dropped.
For each line the free state at the window start (CDIVX phases, the /2
ALIGNWD count) is searched; the line fits if one state gives every sample."""
import itertools
import sys

from hi04_show import lines, samples

HI04B = {1: (1, 0, 34, 8), 2: (1, 0, 34, 16), 3: (1, 0, 38, 8)}
for k, a in enumerate([33, 34, 35, 37, 38, 39]):
    HI04B[4 + k] = (2, 0, a, 1)
    HI04B[10 + k] = (3, 0, a, 1)
for k, a in enumerate(range(66, 98, 4)):
    HI04B[16 + k] = (4, 1, a, 8)
for k, a in enumerate([66, 74, 82, 90]):
    HI04B[24 + k] = (4, 1, a, 16)
for k, a in enumerate(range(66, 94, 4)):
    HI04B[28 + k] = (5, 1, a, 8)
for k, a in enumerate([66, 70, 74, 78]):
    HI04B[35 + k] = (5, 1, a, 16)

HI04 = {}
for k, ph in enumerate([0, 1, 2, 4, 5, 6]):
    HI04[1 + k] = (1, 0, 33 + ph, 64)
for e in range(7, 23):
    kind = 2 + (e - 7) // 4
    ph = 5 if (e - 7) % 2 else 1
    dur = 24 if (e - 7) % 4 >= 2 else 1
    HI04[e] = (kind, 0, 33 + ph, dur)


class Divider:
    def __init__(self, n, q, c=0, parity=0):
        self.n, self.q, self.c, self.parity = n, q, c, parity
        self.wait = 0          # 2: consume a rising edge, 1: next rising edge starts
        self.rst = False
        self.al_prev = 0
        self.pipe = []         # rising edges left until a slip
        self.skip = 0          # edges still ignored by a slip

    def reset(self, level):
        # RST clears every stage (as the emulator does); only what follows
        # its release was measured.
        if level:
            self.q, self.rst, self.wait = 0, True, 2
            self.parity, self.al_prev, self.pipe, self.skip = 0, 0, [], 0
        else:
            self.rst = False

    def edge(self, rising, align):
        if self.rst:
            return
        if rising:
            # ALIGNWD sampling and the slip pipeline.
            self.pipe = [p - 1 for p in self.pipe]
            if self.pipe and self.pipe[0] == 0:
                self.pipe.pop(0)
                # A slip that falls in the wait after a reset is dropped
                # (not measured).
                self.skip = 0 if self.wait else 2
            if align and not self.al_prev:
                if self.n == 7 or self.parity == 1:
                    self.pipe.append(2)
                if self.n == 2:
                    self.parity ^= 1
            self.al_prev = align
        if self.wait:
            if rising:
                self.wait -= 1
                if self.wait == 0:
                    self.q, self.c = 1, 0
            return
        if self.skip:
            self.skip -= 1
            return
        if self.n == 2:
            if rising:
                self.q ^= 1
        else:
            self.c = (self.c + 1) % 7
            self.q = int(self.c < 4)


def simulate(controls, q2, c35, parity, start=None):
    """Samples {eclk_in, eclko, cdivx2, cdivx35} for u = 0..255."""
    d2 = Divider(2, q2, parity=parity)
    d35 = Divider(7, int(c35 < 4), c=c35)
    stages = [0] * 4 if start is None else [1] * start
    if start is not None:
        d2.q, d35.q, d35.c = 0, 0, 6
    out = []
    eclki = eclko = 0
    ctl = {"stop": 0, "rst2": 0, "rst35": 0, "al2": 0, "al35": 0}
    for u in range(256):
        for name, (on, off) in controls.items():
            if u == on or u == off:
                ctl[name] = int(u == on)
                if name == "rst2":
                    d2.reset(ctl[name])
                if name == "rst35":
                    d35.reset(ctl[name])
        new = int(u % 8 >= 4)
        if new != eclki:
            eclki = new
            if new and not stages[-1]:
                eclko = 1
                d2.edge(True, ctl["al2"])
                d35.edge(True, ctl["al35"])
            elif not new:
                if eclko:
                    eclko = 0
                    d2.edge(False, ctl["al2"])
                    d35.edge(False, ctl["al35"])
                stages = [ctl["stop"]] + stages[:-1]
        out.append((eclki, eclko, d2.q, d35.q))
    return out


def toggles(wave, first, rising):
    v, out = first, []
    for u, x in enumerate(wave):
        if u and x != wave[u - 1] and x == rising:
            v ^= 1
        out.append(v)
    return out


def fits(rec, controls, start=None):
    """The free states that reproduce the record."""
    good = []
    space = [(0, 0, 0)] if start is not None else itertools.product((0, 1), range(7), (0, 1))
    for q2, c35, parity in space:
        sim = simulate(controls, q2, c35, parity, start)
        cd2 = [x[2] for x in sim]
        cd35 = [x[3] for x in sim]
        pred = {
            "eclk_in": [x[0] for x in sim],
            "p2": toggles(cd2, rec["p2"][0], 1),
            "n2": toggles(cd2, rec["n2"][0], 0),
            "p35": toggles(cd35, rec["p35"][0], 1),
            "n35": toggles(cd35, rec["n35"][0], 0),
        }
        if start is None:
            pred["pad"] = [x[1] for x in sim]
        if all(pred[k] == rec[k] for k in pred):
            good.append((q2, c35, parity))
    return good


if __name__ == "__main__":
    table = HI04B if sys.argv[1] == "hi04b" else HI04
    names = ["eclk_in", "pad", "p2", "n2", "p35", "n35"]
    bad = 0
    for line in lines(sys.argv[2]):
        ident, h = line.split()
        n = int(ident[1:], 16)
        s = samples(h)
        rec = {k: [x[i] for x in s] for i, k in enumerate(names)}
        if n == 0:
            res = [st for st in (4, 5) if fits(rec, {}, start=st)]
            print(ident, "start-up: gate opens after", res, "falling edges")
            bad += not res
            continue
        kind, pre, a, dur = table[n]
        name = ["", "stop", "rst2", "rst35", "al2", "al35"][kind]
        controls = {name: (a, a + dur)}
        if pre:
            controls[{4: "rst2", 5: "rst35"}[kind]] = (10, 11)
        res = fits(rec, controls)
        print(ident, name, a, dur, "fits" if res else "NO FIT", res[:4])
        bad += not res
    print("lines without a fit:", bad)
