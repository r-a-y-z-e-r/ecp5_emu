"""Find what the C registers and the opcode registers of the ALU54B do (HD08).
Usage: python3 hd08_analyse.py observations.log [b|c]
(b: the record of hd08b.v, whose o4 and o7 have no IN_1 register; c: of
hd08c.v, hd08b.v with the DSP constants on nets)

Each window holds 128 samples (hd08_gen.py has the layout); sample k is the
state before 25 MHz edge k. The output register R is modelled as measured
(HD06, HD07): it captures under CE0 and resets on RST0 (synchronous, or
asynchronous where RESETMODE makes it), and GSR clears it at once and holds it.

A register stage tried here, for each model: an enable (none, CE0..3), a
reset (none, RST0..3, synchronous or asynchronous) and GSR (ignored, or
asynchronous with the hold). With e, r, g the enable, the reset and GSR before
edge k, and r', g' after it:
  Q(k+1) = 0 if GSR is not ignored and (g or g')
           else 0 if r, or r' and the reset is asynchronous
           else D(k) if e else Q(k).
A stage starts unknown at each window, until it captures or is cleared.

Opcode ALUs (constant operands, OP new at every clock): for each ALU with one
opcode register on, every set of OP fields (X = OP[1:0], Y = OP[3:2],
Z = OP[6:4], OP7, OP8, OP9, OP10) that passes it, with every stage model.
The ALUs with several registers (o7; o4 and o7 of hd08b) are checked with
the stages as o1..o6 found them, on the lines hd08_gen.py gives them.
C ALUs (C new at every clock, R = C or R = A): for each recorded bit, no
register in front of R, or one stage with every model.
Prints every model that gives every predicted sample of every window."""
import ast
import itertools
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "hd02_alu54"))
from hd02_model import M54, alu  # noqa: E402

RNDPAT = 0  # hd08.v leaves it at its default, 0 (hd08.config)

A0, B0, A1, B1 = 0x2A5C3, 0x1B6E9, 0x31D47, 0x0F5A3
CM0, CM1 = 0x2C6B1, 0x13A5E
CALU = 0x2D5B3C1E4F6A97 & M54
SECOND = 0x7FC0000 | 0x3FE00000000000

VARIANT = sys.argv[2] if len(sys.argv) > 2 else ""
lines = set()
loads = []
for line in open(sys.argv[1]):
    items = {x for x in ast.literal_eval(line.partition("distinct=")[2]) if len(x) == 5121}
    loads.append(items)
    lines |= items
print("loads:", len(loads), "full windows per load:", [len(l) for l in loads],
      "in all loads:", len(set.intersection(*loads)), "distinct:", len(lines))

C_ALUS = [("c1", (50, 30, 20, 0)), ("c2", (50, 44, 40, 36)), ("c3", (50, 30, 20, 0)), ("c4", (50, 30, 20, 0))]
# The c bit each recorded R bit shows: R = C for c1, c3, c4 and R = A for c2,
# whose top 18 bits are the first MULT18's C (c[17:0]).
C_DATA = {"c1": (50, 30, 20, 0), "c2": (14, 8, 4, 0), "c3": (50, 30, 20, 0), "c4": (50, 30, 20, 0)}
# RESETMODE: of the ALU and its MULT18 for the C ALUs (all the same), of the
# ALU for the opcode ALUs (their MULT18 SYNC). R[15:0] is in the first half.
C_ASYNC = {"c1": False, "c2": True, "c3": False, "c4": False}
OPCODE = ["o1", "o2", "o3", "o4", "o5", "o6", "o7"]
O_ASYNC = {"o1": False, "o2": True, "o3": False, "o4": True, "o5": False, "o6": True, "o7": False}


def samples(x):
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


windows = [samples(x) for x in sorted(lines)]
print("samples:", 128 * len(windows), "GSR low in", sum(s["gsr"] for w in windows for s in w))

# R[15:0] of the opcode ALUs for every OP.
# The ALU's pins C[17:0] and C[44:27] are the same CIB wires as the MULT18 C
# inputs. hd08.v ties both to different constants, nextpnr writes the wire's
# constant mux once for each, and the chip takes 0 where either is 0: the
# MULT18 C inputs carry CM & the ALU's constant (hd08.config, CIB.JCnMUX).
CM0E = CM0 & CALU & 0x3FFFF
CM1E = CM1 & CALU >> 27 & 0x3FFFF
if VARIANT == "c":  # the constants are nets, and the ALU's C pins the MULT18's
    CM0E, CM1E = CM0, CM1
MA, MB = A0 * B0, A1 * B1
A = CM0E << 36 | B0 << 18 | A0
B = CM1E << 36 | B1 << 18 | A1
C = (CALU & SECOND) | CM0E | CM1E << 27


def result(op):
    xs, ys, zs = op & 3, op >> 2 & 3, op >> 4 & 7
    x = [None, MA, A, 0][xs]
    y = [MB << 18 & M54, MB, B, 0][ys]
    # Z = 7 read as all ones here, not as the zero of HD02..HD05 (which had
    # RNDPAT nonzero and the slice GSR disabled): see the README.
    z = [0, 0, 0, C, A, None, RNDPAT, M54][zs]
    if x is None or z is None:
        return None  # reads R: never selected by a whole OP of the program
    return alu(op, x, y, z) & 0xFFFF


TABLE = [result(op) for op in range(2048)]
# A stage's clock: every edge (div 1), or every other edge (div 2): a divider
# that toggles at each edge and takes the active edges in state 0; GSR may
# set it to state 0 (divider reset "g") or nothing does (None).
STAGES = [(div, dres, ce, rst, kind, gsr)
          for div, dres, ce, rst, kind, gsr in itertools.product(
              [1, 2], [None, "g"], [None, 0, 1, 2, 3], [None, 0, 1, 2, 3], ["sync", "async"], [False, True])
          if not (rst is None and kind == "async") and not (div == 1 and dres)]


def stage_next(q, d, a, b, model, active):
    """The stage after edge k: a, b the samples before edge k and k+1;
    active: whether its clock has this edge."""
    div, dres, ce, rst, kind, gsr = model
    if gsr and (a["gsr"] or b["gsr"]):
        return 0
    if rst is not None and kind == "async" and b["rst"] >> rst & 1:
        return 0
    if not active:
        return q
    if rst is not None and a["rst"] >> rst & 1:
        return 0
    if ce is None or a["ce"] >> ce & 1:
        return d
    return q


def divider_next(state, a, model):
    """The divider state after edge k (state 0: edge k was active)."""
    div, dres = model[:2]
    if div == 1:
        return 0
    if dres == "g" and a["gsr"]:
        return 0
    return state ^ 1


def r_next(r, d, a, b, asynchronous):
    """The output register (R0 line, CE0, GSR) after edge k; d None: unknown."""
    if a["gsr"] or b["gsr"]:
        return 0
    if a["rst"] & 1 or (asynchronous and b["rst"] & 1):
        return 0
    if a["ce"] & 1:
        return d
    return r


def describe(model):
    div, dres, ce, rst, kind, gsr = model
    return (("every other edge" + (", GSR resets the divider" if dres else "") + ", " if div == 2 else "")
            + f"CE{ce if ce is not None else ' none'}, RST{rst if rst is not None else ' none'}"
            + (f" {kind}" if rst is not None else "") + (", GSR clears" if gsr else ", GSR ignored"))


# ---- C ALUs -------------------------------------------------------------
def c_window(w, name, bit, data, model, phase):
    q, state = None, phase
    for k in range(127):
        a, b = w[k], w[k + 1]
        d = a["c"][data] if model is None else q
        want = r_next(a[name][bit], d, a, b, C_ASYNC[name])
        if want is not None and want != b[name][bit]:
            return False
        if model is not None:
            q = stage_next(q, a["c"][data], a, b, model, state == 0)
            state = divider_next(state, a, model)
    return True


def c_fits(name, bit, data, model):
    """model None: no register in front of R. A divider starts in either
    state in each window: it runs on between windows."""
    phases = (0, 1) if model is not None and model[0] == 2 else (0,)
    return all(any(c_window(w, name, bit, data, model, p) for p in phases) for w in windows)


for name, rbits in C_ALUS:
    for bit, data in zip(rbits, C_DATA[name]):
        found = []
        if c_fits(name, bit, data, None):
            found.append("no register")
        found += [describe(m) for m in STAGES if c_fits(name, bit, data, m)]
        print(f"{name} R[{bit}] (c[{data}]): {found or 'NO MODEL FITS'}")

# ---- opcode ALUs --------------------------------------------------------
FIELDS = [("X", 0b11), ("Y", 0b1100), ("OP4", 1 << 4), ("OP5+OP6", 0b1100000), ("OP7", 1 << 7), ("OP8", 1 << 8), ("OP9+OP10", 0b11 << 9)]


def o_window(w, name, stages, phase):
    qs = [[None] * len(models) for _, models in stages]
    states = [[phase] * len(models) for _, models in stages]
    for k in range(127):
        a, b = w[k], w[k + 1]
        op, known = a["op"], True
        for (mask, _), q in zip(stages, qs):
            if q[-1] is None:
                known = False
            else:
                op = op & ~mask | q[-1] & mask
        d = TABLE[op] if known else None
        want = r_next(a[name], d, a, b, O_ASYNC[name])
        if want is not None and want != b[name]:
            return False
        for (mask, models), q, st in zip(stages, qs, states):
            inputs = [a["op"] & mask] + q[:-1]
            for n, model in enumerate(models):
                q[n] = stage_next(q[n], inputs[n], a, b, model, st[n] == 0)
                st[n] = divider_next(st[n], a, model)
    return True


def o_fits(name, stages):
    """stages: list of (mask, [model, ...]) — the OP bits in mask pass the
    models' stages in turn. Every stage starts unknown; dividers start in
    either state."""
    div2 = any(m[0] == 2 for _, models in stages for m in models)
    return all(any(o_window(w, name, stages, p) for p in ((0, 1) if div2 else (0,))) for w in windows)


def masks():
    for n in range(1, 1 << len(FIELDS)):
        yield sum(m for k, (_, m) in enumerate(FIELDS) if n >> k & 1)


def field_names(mask):
    return "+".join(f for f, m in FIELDS if mask & m == m)


SINGLE = ["o1", "o2", "o3", "o5", "o6"] + ([] if VARIANT in ("b", "c") else ["o4"])
found_stage = {}
for name in sorted(SINGLE):
    found = [(mask, model) for mask in masks() for model in STAGES if o_fits(name, [(mask, [model])])]
    if o_fits(name, []):
        found.insert(0, (0, None))
    groups = {}
    for mask, model in found:
        groups.setdefault(mask, []).append(model)
    print(f"{name}: no register in front of OP fits" if (0, None) in found else f"{name}:", end="")
    for mask, models in groups.items():
        if mask:
            print(f"\n  {field_names(mask)}: {[describe(m) for m in models]}", end="")
    print("" if groups else " NO MODEL FITS")
    found_stage[name] = groups


# ---- several registers --------------------------------------------------
IN, OP0, OP1 = 0b1111111, 1 << 7, 0b111 << 8


def stage(ce, rst, name):
    """A full-rate opcode stage as found: the reset kind is RESETMODE of the
    ALU, and GSR clears it."""
    return (1, None, ce, rst, "async" if O_ASYNC[name] else "sync", True)


def mismatches(name, stages):
    """Clocks whose R the model does not give, of those it predicts; after
    one, the stages start unknown again."""
    miss = n = 0
    for w in windows:
        qs = [[None] * len(models) for _, models in stages]
        for k in range(127):
            a, b = w[k], w[k + 1]
            op, known = a["op"], True
            for (mask, _), q in zip(stages, qs):
                if q[-1] is None:
                    known = False
                else:
                    op = op & ~mask | q[-1] & mask
            want = r_next(a[name], TABLE[op] if known else None, a, b, O_ASYNC[name])
            if want is not None:
                n += 1
                if want != b[name]:
                    miss += 1
                    qs = [[None] * len(q) for q in qs]
                    continue
            for (mask, models), q in zip(stages, qs):
                inputs = [a["op"] & mask] + q[:-1]
                for i, model in enumerate(models):
                    q[i] = stage_next(q[i], inputs[i], a, b, model, True)
    return miss, n


if VARIANT in ("b", "c"):
    COMPOSITE = {
        "o4": [(IN, [stage(3, 1, "o4")]), (OP0, [stage(0, 2, "o4")]), (OP1, [stage(0, 2, "o4")])],
        "o7": [(IN, [stage(1, 2, "o7")]),
               (OP0, [stage(2, 1, "o7"), stage(1, 3, "o7")]),
               (OP1, [stage(2, 1, "o7"), stage(1, 3, "o7")])],
    }
else:
    # IN_1 as a stage on its lines: the record does not follow it.
    COMPOSITE = {
        "o4": [(IN, [stage(3, 1, "o4")])],
        "o7": [(IN, [stage(1, 2, "o7"), stage(3, 1, "o7")]),
               (OP0, [stage(2, 1, "o7"), stage(1, 3, "o7")]),
               (OP1, [stage(2, 1, "o7"), stage(1, 3, "o7")])],
    }
for name, stages in COMPOSITE.items():
    print(f"{name} with the found stages: {'fits' if o_fits(name, stages) else 'NO FIT'}; "
          f"clocks not given: %d of %d" % mismatches(name, stages))
