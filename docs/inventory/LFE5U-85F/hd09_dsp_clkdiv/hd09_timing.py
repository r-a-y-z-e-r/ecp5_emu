"""Sample-level HD09 clock hypotheses and mismatch scoring (requires NumPy).

A cycle has two observed input snapshots: a before its rising edge, b after
fabric inputs change and before the next rising edge. A rise+fall model updates
from a and then b. The record cannot distinguish this from a suitable transparent
latch during the intervening clock phase; it does not measure edge waveforms.
GSR's measured hold across a->b is retained. Each phase is fitted per window.
"""
from dataclasses import dataclass
import numpy as np


@dataclass(frozen=True)
class Clock:
    mode: str
    period: int = 1
    reset: bool = False

    def label(self):
        text = {"rise": "rising", "fall": "falling", "both": "rise+fall", "never": "never captures"}[self.mode]
        if self.period > 1:
            text += f" /{self.period}" + (" GSR resets phase" if self.reset else " free phase")
        return text

    def active(self, state, half):
        return self.mode != "never" and (self.mode == "both" or self.mode == ("rise" if half == 0 else "fall")) and state == 0

    def advance(self, state, sample):
        return 0 if self.reset and sample["gsr"] else (state + 1) % self.period


CLOCKS = [Clock("rise"), Clock("fall"), Clock("both"), Clock("never")]
CLOCKS += [Clock(mode, period, reset) for period in (2, 4) for mode in ("rise", "fall") for reset in (False, True)]


def settle(q, sample, ce, rst, asynchronous, gsr):
    """Asynchronous reset settles before the next register observes this stage."""
    return 0 if gsr or (asynchronous and sample["rst"] >> rst & 1) else q


def capture(q, data, sample, ce, rst, active, gsr):
    if gsr:
        return 0
    if active:
        if sample["rst"] >> rst & 1:
            return 0
        if sample["ce"] >> ce & 1:
            return data
    return q


def settings(name, bit, probe=False):
    """HD07/HD09p's measured control lines, per recorded output bit."""
    first = bit in (0, 30)
    ce = 0
    rst = 1 if name == "ma" else (2 if name == "ec" and first else (1 if name == "ed" and first else 0))
    asynchronous = name in ("ma", "md", "ed") or (name == "ea" and first) or (name == "eb" and not first)
    if probe and name in ("ma", "mb", "mc"):
        ce, rst = {"ma": (1, 2), "mb": (2, 3), "mc": (3, 1)}[name]
    return ce, rst, asynchronous


def direct_candidates(windows, name, block, bits, data_bits, probe=False):
    """Return per-bit transition mismatch counts for every visible-stage hypothesis.

    Predictions condition on observed Q(a). Latencies 0..8 delay D only; current
    controls remain current. Divided-clock phase minimizes total bit mismatches
    separately in each window, with one shared phase for all bits of that cell.
    """
    q = np.array([[ [s[name][bit] for bit in bits] for s in w] for w in windows], dtype=np.uint8)
    d = np.array([[ [s["c"][bit] for bit in data_bits] for s in w] for w in windows], dtype=np.uint8)
    ce = np.array([[s["ce"+block] for s in w] for w in windows], dtype=np.uint8)
    rst = np.array([[s["rst"+block] for s in w] for w in windows], dtype=np.uint8)
    g = np.array([[s["gsr"] for s in w] for w in windows], dtype=bool)
    controls = [settings(name, bit, probe) for bit in bits]
    en = np.stack([((ce >> c) & 1).astype(bool) for c, _, _ in controls], axis=-1)
    re = np.stack([((rst >> r) & 1).astype(bool) for _, r, _ in controls], axis=-1)
    async_bits = np.array([a for _, _, a in controls], dtype=bool)
    rows = []
    nw, ns, nb = q.shape
    for clock in CLOCKS:
        for delay in (range(9) if clock.mode != "never" else (0,)):
            phases = []
            for phase in range(clock.period):
                state = np.full(nw, phase, dtype=int)
                active = [np.empty((nw, ns-1), dtype=bool) for _ in range(2)]
                for k in range(ns-1):
                    for half in (0, 1):
                        selected = clock.mode == "both" or clock.mode == ("rise" if half == 0 else "fall")
                        active[half][:, k] = selected & (state == 0)
                    state = np.where(g[:, k], 0, (state+1) % clock.period) if clock.reset else (state+1) % clock.period
                # Common coverage across rising and falling delayed-data models.
                start = delay
                want = q[:, start:-1].copy()
                held_gsr = (g[:, start:-1] | g[:, start+1:])[..., None]
                for half in (0, 1):
                    control_slice = slice(start+half, ns-1+half)
                    data_slice = slice(start+half-delay, ns-1+half-delay)
                    er, rr = en[:, control_slice], re[:, control_slice]
                    event = active[half][:, start:, None]
                    want = np.where(held_gsr | (rr & async_bits), 0, want)
                    want = np.where(event, np.where(rr, 0, np.where(er, d[:, data_slice], want)), want)
                    want = np.where(held_gsr, 0, want)
                phases.append((want != q[:, start+1:]).sum(axis=1))
            # shape: phase, window, bit. Phase must be common to all bits.
            errors = np.stack(phases)
            best_phase = errors.sum(axis=2).argmin(axis=0)
            picked = errors[best_phase, np.arange(nw)]
            rows.append(dict(label=clock.label()+f", D latency {delay}", family=clock.mode if clock.period == 1 else f"divide-{clock.period}",
                             miss=picked.sum(axis=0).tolist(), total=nw*(ns-1-delay), phases=best_phase.tolist(), clock=clock, delay=delay))
    a, b = q[:, :-1], q[:, 1:]
    for label, pred in [("frozen (hold observed Q)", a), ("constant zero", np.zeros_like(a)), ("constant one", np.ones_like(a)),
                        ("combinational D (ignore controls)", d[:, 1:]),
                        ("CE level latch using current inputs", np.where(g[:, 1:, None] | re[:, 1:], 0, np.where(en[:, 1:], d[:, 1:], a))),
                        ("GSR clears, otherwise frozen", np.where((g[:, :-1] | g[:, 1:])[..., None], 0, a)),
                        ("capture D only on GSR assertion", np.where((~g[:, :-1] & g[:, 1:])[..., None], d[:, 1:], a)),
                        ("capture D only on GSR release", np.where((g[:, :-1] & ~g[:, 1:])[..., None], d[:, 1:], a))]:
        rows.append(dict(label=label, family=label, miss=(pred != b).sum(axis=(0,1)).tolist(), total=nw*(ns-1), phases=[]))
    away = ~(g[:, :-1] | g[:, 1:])
    changes = ((a != b) & away[..., None]).sum(axis=(0,1)).tolist()
    return rows, changes


def report_direct(windows, name, block, bits, data_bits, probe=False):
    rows, changes = direct_candidates(windows, name, block, bits, data_bits, probe)
    families = list(dict.fromkeys(r["family"] for r in rows))
    for j, bit in enumerate(bits):
        print(f"{name} bit {bit}: changes away from GSR={changes[j]}")
        for family in families:
            options = [r for r in rows if r["family"] == family]
            best = min(options, key=lambda r: (r["miss"][j]/r["total"], -r["total"], r["label"]))
            print(f"  {best['label']}: {best['miss'][j]}/{best['total']} mismatches")
        winner = [r for r in rows if r["label"] == "rise+fall, D latency 0"][0]
        print(f"  measured rise+fall model: {winner['miss'][j]}/{winner['total']}")
    return rows


def selftest():
    """Construct independent two-update traces, then fit the serialized parser data."""
    import random
    import hd09m_analyse as m
    rng = random.Random(909117)
    rows = []
    for _ in range(256):
        rows.append(dict(cea=15, rsta=0, ceb=rng.randrange(16), rstb=rng.randrange(16) if rng.randrange(4)==0 else 0,
                         gsr=int(rng.randrange(32)==0), c={b:rng.randrange(2) for b in (50,30,20,0,16,1)}))
    q = 0
    for k, s in enumerate(rows):
        for name, _ in m.ALUS: s[name] = dict.fromkeys(m.ALU_BITS, 0)
        for name, _ in m.MULTS: s[name] = dict.fromkeys(m.MULT_BITS, 0)
        s['ma'][1] = q
        if k != 255:
            b=rows[k+1]
            for half in (s,b):
                if s['gsr'] or b['gsr'] or half['rstb'] & 2:q=0
                elif half['ceb'] & 1:q=half['c'][1]
    chunks=[]
    for s in rows:
        word=f'{s["cea"]:04b}{s["rsta"]:04b}{s["ceb"]:04b}{s["rstb"]:04b}000{1-s["gsr"]}'
        word+=''.join(str(s['c'][b]) for b in m.ALU_BITS)+'0'*16+'00'+str(s['c'][16])+str(s['c'][1])
        word+=''.join(str(s[n][b]) for n,_ in m.MULTS for b in m.MULT_BITS)
        chunks.append(f'{int(word,2):015X}')
    parsed=[m.samples('M'+''.join(chunks))]
    fits,_=direct_candidates(parsed,'ma','b',[1],[1])
    assert next(r for r in fits if r['label']=='rise+fall, D latency 0')['miss']==[0]
    assert next(r for r in fits if r['label']=='rising, D latency 0')['miss'][0]>0
    assert next(r for r in fits if r['label']=='falling, D latency 0')['miss'][0]>0
    print('HD09 timing selftest: independent serialized rise+fall trace, wrong-edge rejection: PASS')
