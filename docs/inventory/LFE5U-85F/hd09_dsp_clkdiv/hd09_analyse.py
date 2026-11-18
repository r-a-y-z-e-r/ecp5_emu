"""Fit HD09 DISABLED-divider C/opcode timing jointly with its R output.

Usage: hd09_analyse.py observations.log [--legacy]; ... --selftest
Default (NumPy): retain HD08c's field masks and controls, compose internal
rising/falling/rise+fall, /2,/4 and never/bypass hypotheses with HD09m's
measured rise+fall output model. Print numerical mismatches and unknown-state
coverage. The single startup window additionally tests zero hidden initial state.

--legacy retains HD08's full field/CE/RST/GSR search and its rising-only R
model for ENABLED reference records. Flags and IN_1 remain unmeasured.
All phases use sorted window order because runner sets discard chronology.
Physical edge sensitivity versus equivalent phase transparency is not identified.
"""
import argparse
import itertools
from pathlib import Path
import sys
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "hd02_alu54"))
from hd02_model import M54, alu
from hd09m_analyse import read_lines, RATES, rate_name, phase_name

RNDPAT = 0

A0, B0, A1, B1 = 0x2A5C3, 0x1B6E9, 0x31D47, 0x0F5A3

CM0, CM1 = 0x2C6B1, 0x13A5E

CALU = 0x2D5B3C1E4F6A97 & M54

SECOND = 0x7FC0000 | 0x3FE00000000000

C_ALUS = [("c1", (50, 30, 20, 0)), ("c2", (50, 44, 40, 36)), ("c3", (50, 30, 20, 0)), ("c4", (50, 30, 20, 0))]

C_DATA = {"c1": (50, 30, 20, 0), "c2": (14, 8, 4, 0), "c3": (50, 30, 20, 0), "c4": (50, 30, 20, 0)}

C_ASYNC = {"c1": False, "c2": True, "c3": False, "c4": False}

OPCODE = ["o1", "o2", "o3", "o4", "o5", "o6", "o7"]

O_ASYNC = {"o1": False, "o2": True, "o3": False, "o4": True, "o5": False, "o6": True, "o7": False}

def samples(x):
    if len(x) != 5121 or x[0] != "M":
        raise ValueError("expected M + 5120 hex")
    bits = bin(int(x[1:], 16))[2:].zfill(20480)
    out = []
    for i in range(128):
        w = bits[160 * i:160 * i + 160]
        v = lambda a, b: int(w[a:b], 2)
        s = dict(ce=v(0, 4), rst=v(4, 8), gsr=1 - v(11, 12), op=v(13, 24))
        s["c"] = dict(zip((50, 30, 20, 0, 14, 8, 4), (v(24 + k, 25 + k) for k in range(7))))
        for n, (name, rbits) in enumerate(C_ALUS):
            s[name] = dict(zip(rbits, (v(32 + 4 * n + k, 33 + 4 * n + k) for k in range(4))))
        for n, name in enumerate(OPCODE):
            s[name] = v(48 + 16 * n, 64 + 16 * n)
        out.append(s)
    return out

CM0E, CM1E = CM0, CM1

MA, MB = A0 * B0, A1 * B1

A = CM0E << 36 | B0 << 18 | A0

B = CM1E << 36 | B1 << 18 | A1

C = (CALU & SECOND) | CM0E | CM1E << 27

def result(op, feedback=None):
    xs, ys, zs = op & 3, op >> 2 & 3, op >> 4 & 7
    x = [feedback, MA, A, 0][xs]
    y = [MB << 18 & M54, MB, B, 0][ys]
    # Z = 7 read as all ones here, not as the zero of HD02..HD05 (which had
    # RNDPAT nonzero and the slice GSR disabled): see the README.
    z = [0, None, None, C, A, feedback, RNDPAT, M54][zs]
    if z is None or (x is None and op >> 9 & 3 == 1):
        return None
    return alu(op, x or 0, y, z) & 0xFFFF

TABLE = [result(op) for op in range(2048)]

def r_next(r, d, a, b, asynchronous):
    """The output register (R0 line, CE0, GSR) after edge k; d None: unknown."""
    if a["gsr"] or b["gsr"]:
        return 0
    if a["rst"] & 1 or (asynchronous and b["rst"] & 1):
        return 0
    if a["ce"] & 1:
        return d
    return r

FIELDS = [("X", 0b11), ("Y", 0b1100), ("OP4", 1 << 4), ("OP5+OP6", 0b1100000), ("OP7", 1 << 7), ("OP8", 1 << 8), ("OP9+OP10", 0b11 << 9)]

def masks():
    for n in range(1, 1 << len(FIELDS)):
        yield sum(m for k, (_, m) in enumerate(FIELDS) if n >> k & 1)

def field_names(mask):
    return "+".join(f for f, m in FIELDS if mask & m == m)

IN, OP0, OP1 = 0b1111111, 1 << 7, 0b111 << 8

STAGES = [(rate, dres, ce, rst, kind, gsr)
          for (rate, dres), ce, rst, kind, gsr in itertools.product(
              RATES, [None, 0, 1, 2, 3], [None, 0, 1, 2, 3], ["sync", "async"], [False, True])
          if rst is not None or kind == "sync"]


def stage_next(q, d, a, b, model, state):
    rate, _, ce, rst, kind, gsr = model
    if gsr and (a["gsr"] or b["gsr"]):
        return 0
    # A held asynchronous reset also prevents capture on its release edge.
    if rst is not None and kind == "async" and (a["rst"] >> rst & 1 or b["rst"] >> rst & 1):
        return 0
    active = rate == 1 or (rate == 2 and state == 0)
    if active:
        if rst is not None and a["rst"] >> rst & 1:
            return 0
        if ce is None or a["ce"] >> ce & 1:
            return d
    return q


def divider_next(state, a, model):
    rate, dres = model[:2]
    return 0 if rate != 2 or (dres == "g" and a["gsr"]) else state ^ 1


def describe(model):
    _, _, ce, rst, kind, gsr = model
    return (f"{rate_name(model)}, CE{ce if ce is not None else ' none'}, "
            f"RST{rst if rst is not None else ' none'} {kind}, GSR {'clears' if gsr else 'ignored'}")


def informative(d, a, b, asynchronous):
    return d is not None and bool(a["ce"] & 1) and not (a["gsr"] or b["gsr"] or a["rst"] & 1 or (asynchronous and b["rst"] & 1))


def c_phases(windows, name, bit, data, model):
    found = []
    for w in windows:
        valid = []
        for phase in ((0, 1) if model is not None and model[0] == 2 else (0,)):
            q, state, checked = None, phase, 0
            for a, b in zip(w, w[1:]):
                d = a["c"][data] if model is None else q
                want = r_next(a[name][bit], d, a, b, C_ASYNC[name])
                checked += informative(d, a, b, C_ASYNC[name])
                if want is not None and want != b[name][bit]:
                    break
                if model is not None:
                    q = stage_next(q, a["c"][data], a, b, model, state)
                    state = divider_next(state, a, model)
            else:
                if checked >= 8:
                    valid.append(phase)
        if not valid:
            return None
        found.append(tuple(valid))
    return found


def o_phases(windows, name, stages):
    models = [m for _, chain in stages for m in chain]
    starts = list(itertools.product(*[(0, 1) if m[0] == 2 else (0,) for m in models]))
    found = []
    for w in windows:
        valid = []
        for start in starts:
            qs = [[None]*len(chain) for _, chain in stages]
            states, offset = [], 0
            for _, chain in stages:
                states.append(list(start[offset:offset+len(chain)]))
                offset += len(chain)
            checked = 0
            for a, b in zip(w, w[1:]):
                op, known = a["op"], True
                for (mask, _), q in zip(stages, qs):
                    if q[-1] is None:
                        known = False
                    else:
                        op = op & ~mask | q[-1] & mask
                d = result(op, a[name]) if known else None
                want = r_next(a[name], d, a, b, O_ASYNC[name])
                checked += informative(d, a, b, O_ASYNC[name])
                if want is not None and want != b[name]:
                    break
                for (mask, chain), q, st in zip(stages, qs, states):
                    inputs = [a["op"] & mask] + q[:-1]
                    for i, model in enumerate(chain):
                        q[i] = stage_next(q[i], inputs[i], a, b, model, st[i])
                        st[i] = divider_next(st[i], a, model)
            else:
                if checked >= 8:
                    valid.append(start)
        if not valid:
            return None
        found.append(tuple(valid))
    return found


def configured_stage(name, ce, rst, rate):
    return (*rate, ce, rst, "async" if O_ASYNC[name] else "sync", True)


# One entry per physical register, including OP1's controls shared with OP0.
COMPOSITE = {
    "o4": [("IN_0", IN, 3, 1, 0), ("OP0_0", OP0, 0, 2, 0), ("OP1_0", OP1, 0, 2, 0)],
    "o7": [("IN_0", IN, 1, 2, 0), ("OP0_0", OP0, 2, 1, 0), ("OP0_1", OP0, 1, 3, 1),
           ("OP1_0", OP1, 2, 1, 0), ("OP1_1", OP1, 1, 3, 1)],
}


def composite(name, rates):
    chains = {}
    for (_, mask, ce, rst, _), rate in zip(COMPOSITE[name], rates):
        chains.setdefault(mask, []).append(configured_stage(name, ce, rst, rate))
    return list(chains.items())


def legacy_analyse(lines):
    windows = [samples(line) for line in lines]
    print("window IDs (phase lists use this order):")
    for i, line in enumerate(lines):
        print(f"  {i}: {line[:49]}...")
    print("R output model: every edge, CE0/RST0/GSR, cross-check with hd09m")
    for name, bits in C_ALUS:
        for bit, data in zip(bits, C_DATA[name]):
            fits = [(model, p) for model in [None] + STAGES
                    if (p := c_phases(windows, name, bit, data, model)) is not None]
            print(f"{name} C register -> R[{bit}] (c[{data}]):" + (" NO MODEL" if not fits else ""))
            for model, p in fits:
                print("  " + ("bypass (no register)" if model is None else describe(model))
                      + ("; " + phase_name(p) if model is not None and model[0] == 2 else ""))
    for name in ("o1", "o2", "o3", "o5", "o6"):
        count = 0
        print(f"{name} opcode register:")
        if o_phases(windows, name, []) is not None:
            print("  bypass (no register)")
            count += 1
        for mask in masks():
            for model in STAGES:
                p = o_phases(windows, name, [(mask, [model])])
                if p is not None:
                    count += 1
                    print(f"  {field_names(mask)}: {describe(model)}"
                          + ("; " + phase_name(p) if model[0] == 2 else ""))
        if not count:
            print("  NO MODEL")
    for name, registers in COMPOSITE.items():
        found = []
        for rates in itertools.product(RATES, repeat=len(registers)):
            p = o_phases(windows, name, composite(name, rates))
            if p is not None:
                found.append((rates, p))
        print(f"{name} composite (HD08 fields/controls, independent rates):" + (" NO MODEL" if not found else ""))
        for rates, p in found:
            print("  " + "; ".join(f"{reg[0]}: {rate_name(rate)}" for reg, rate in zip(registers, rates)))
            print("    phase tuple order=" + repr([r[0] for r in registers]) + "; " + phase_name(p))


def analyse(lines):
    import hd09_joint
    hd09_joint.analyse(lines, sys.modules[__name__])


def selftest():
    import random
    rng = random.Random(90809)
    for rate in RATES:
        for phase in ((0, 1) if rate[0] == 2 else (0,)):
            cm = (*rate, 0, 0, "sync", True)
            om = (*rate, 1, 2, "sync", True)
            w = []
            for _ in range(128):
                a = dict(ce=sum((rng.randrange(4) != 0) << b for b in range(4)),
                         rst=sum((rng.randrange(8) == 0) << b for b in range(4)),
                         gsr=int(rng.randrange(32) == 0), op=rng.randrange(2048))
                # Same safe OP mode as the design: no R or CIN for old/new mixtures.
                a["op"] = (a["op"] & ~0x72) | 0x62
                a["c"] = {b: rng.randrange(2) for b in (50, 30, 20, 0, 14, 8, 4)}
                for name, bits in C_ALUS:
                    a[name] = dict.fromkeys(bits, 0)
                for name in OPCODE:
                    a[name] = 0
                w.append(a)
            cq = oq = cr = ore = 0
            state = phase
            for k, a in enumerate(w):
                a["c1"][0], a["o1"] = cr, ore
                if k == 127:
                    break
                b = w[k+1]
                cr = r_next(cr, cq, a, b, False)
                ore = r_next(ore, result(a["op"] & ~IN | oq & IN, ore), a, b, False)
                cq = stage_next(cq, a["c"][0], a, b, cm, state)
                oq = stage_next(oq, a["op"] & IN, a, b, om, state)
                state = divider_next(state, a, cm)
            chunks = []
            for a in w:
                bits = f'{a["ce"]:04b}{a["rst"]:04b}000{1-a["gsr"]}0{a["op"]:011b}'
                bits += ''.join(str(a["c"][b]) for b in (50, 30, 20, 0, 14, 8, 4)) + '0'
                bits += ''.join(str(a[n][b]) for n, bs in C_ALUS for b in bs)
                bits += ''.join(f'{a[n]:016b}' for n in OPCODE)
                chunks.append(f'{int(bits, 2):040X}')
            parsed = [samples('M' + ''.join(chunks))]
            fits = [m for m in STAGES if c_phases(parsed, 'c1', 0, 0, m) is not None]
            assert cm in fits, (cm, fits)
            assert {m[0] for m in fits} == {rate[0]}, (cm, fits)
            p = o_phases(parsed, 'o1', [(IN, [om])])
            assert p is not None and (phase,) in p[0], (om, p)
            # All other capture rates must fail for the known opcode field/controls.
            for other in RATES:
                if other[0] != rate[0]:
                    assert o_phases(parsed, 'o1', [(IN, [(*other, *om[2:])])]) is None
    # A composite trace has independently phased dividers in its five stages.
    # Feed complete line data through the same parser used by the command line.
    name = "o7"
    rates = [(1, None), (2, None), (2, "g"), (0, None), (1, None)]
    chains = composite(name, rates)
    qs = [[0] * len(chain) for _, chain in chains]
    states = [[0], [1, 0], [0, 0]]
    cr = 0
    for k, a in enumerate(w):
        a[name] = cr
        if k == 127:
            break
        b = w[k+1]
        op = a["op"]
        for (mask, _), q in zip(chains, qs):
            op = op & ~mask | q[-1] & mask
        cr = r_next(cr, result(op, cr), a, b, False)
        for (mask, chain), q, st in zip(chains, qs, states):
            inputs = [a["op"] & mask] + q[:-1]
            for j, model in enumerate(chain):
                q[j] = stage_next(q[j], inputs[j], a, b, model, st[j])
                st[j] = divider_next(st[j], a, model)
    chunks = []
    for a in w:
        bits = f'{a["ce"]:04b}{a["rst"]:04b}000{1-a["gsr"]}0{a["op"]:011b}'
        bits += ''.join(str(a["c"][b]) for b in (50, 30, 20, 0, 14, 8, 4)) + '0'
        bits += ''.join(str(a[n][b]) for n, bs in C_ALUS for b in bs)
        bits += ''.join(f'{a[n]:016b}' for n in OPCODE)
        chunks.append(f'{int(bits, 2):040X}')
    parsed = [samples('M' + ''.join(chunks))]
    assert o_phases(parsed, name, chains) is not None
    print("HD09 selftest: full synthetic C/opcode lines, every edge, both divided phases, GSR divider reset, never, composite stages: PASS")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", nargs="?")
    parser.add_argument("--selftest", action="store_true")
    parser.add_argument("--legacy", action="store_true", help="original HD08 model search, for ENABLED reference records")
    args = parser.parse_args()
    if args.selftest:
        selftest()
        import hd09_joint
        hd09_joint.selftest(sys.modules[__name__])
    elif args.log:
        (legacy_analyse if args.legacy else analyse)(read_lines(args.log, 5120))
    else:
        parser.error("supply a log or --selftest")


if __name__ == "__main__":
    main()
