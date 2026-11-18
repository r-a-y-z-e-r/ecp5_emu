"""The ALU54B model for the chained records (HD03, HD04, HD05): a step-by-step
simulation of several ALUs that share an opcode sweep, compared with every
line the board printed. Used by hd03_model.py, hd04_model.py, hd05_model.py.

The operation is HD02's (hd02_model.py). What this adds, per ALU:
- A = {C input of its first MULT18, ROB, ROA}, B likewise of the second;
  C = {C[53:45], second MULT18's C, C[26:18], first MULT18's C};
- CIN = R of the ALU before it, as the cascade shows it;
  OP[6:4] = 1 takes CIN >> 18 with its top 18 bits CIN[53] & SIGNEDIB;
- every register captures once per step (the pulse), unless its block's CE0
  line is high all the time (`free`): then it follows its inputs, and an
  operation that reads R runs on while the operands shift. nextpnr puts the
  first ALU's CE0 net on that line for both ALUs of a block;
- the bits in `unregistered` show the result itself, before and after the
  pulse and through the cascade; an operation that reads R through them
  loops on the chip, and its value is not predicted;
- the bits in `stuck` (a register without a clock) are 0;
- the flags are constants: those of a zero word against the patterns. A flag
  register is 0 until the first pulse.
A value that is not predicted (a loop, or a free register that reads R, or
anything that reads those through CIN) is skipped, and counted."""
import collections
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd02_model import M54, RNDPAT, alu, sx  # noqa: E402

GROUPS = dict(a=0x3FFFF, b=0x7FC0000, c=0x1FFFF8000000, d=0x3FE00000000000)
FIRST_HALF = GROUPS["a"] | GROUPS["c"]    # behind the first MULT18's CIBOUT_BYP
SECOND_HALF = GROUPS["b"] | GROUPS["d"]   # behind the second MULT18's
PATTERNS = dict(
    A=(0x1555555555555A, 0x3FFFFFFFFFFFF0),
    Z=(0, 0),
    O=(M54, M54),
)

Alu = collections.namedtuple(
    "Alu", "name set op cin mult_c unregistered stuck free flag_register patterns")


def flags_of(patterns):
    """EQZ, EQZM, EQOM, EQPAT, EQPATB, OVER, UNDER, OVERUNDER as a byte."""
    mcpat, maskpat = PATTERNS[patterns]
    unmasked = ~maskpat & M54
    bits = [1, 1, 0, int(mcpat & unmasked == 0), int(~mcpat & unmasked == 0), 0, 0, 0]
    return sum(b << (7 - k) for k, b in enumerate(bits))


def result(a, n, d, r, cin):
    """The result of ALU `a` at step `n` with operands `d`; None when it
    reads something not predicted."""
    op = a.op(n)
    xs, ys, zs = op & 3, op >> 2 & 3, op >> 4 & 7
    if (xs == 0 or zs == 5) and r is None:
        return None
    if zs in (1, 2) and cin is None:
        return None
    sa0, sb0, sa1, sb1 = (d["sg"] >> k & 1 for k in range(4))
    ma = sx((sx(d["a0"], 18, sa0) * sx(d["b0"], 18, sb0)) & (1 << 36) - 1, 36, sa0 | sb0) & M54
    mb = sx((sx(d["a1"], 18, sa1) * sx(d["b1"], 18, sb1)) & (1 << 36) - 1, 36, sa1 | sb1) & M54
    c = d["c"]
    cm0, cm1 = (c & 0x3FFFF, c >> 27 & 0x3FFFF) if a.mult_c else (0, 0)
    a54 = cm0 << 36 | d["b0"] << 18 | d["a0"]
    b54 = cm1 << 36 | d["b1"] << 18 | d["a1"]
    c54 = (c & SECOND_HALF) | cm0 | cm1 << 27
    cin = cin or 0
    fill = 0x3FFFF << 36 if cin >> 53 & 1 and (sa1 | sb1) else 0
    x = [r or 0, ma, a54, 0][xs]
    y = [mb << 18 & M54, mb, b54, 0][ys]
    z = [0, cin >> 18 | fill, cin, c54, a54, r or 0, RNDPAT, 0][zs]
    return alu(op, x, y, z)


class Chain:
    """The ALUs in cascade order, with their registers."""

    def __init__(self, alus):
        self.alus = alus
        self.register = {a.name: 0 for a in alus}
        self.pulsed = False

    def operand(self, a):
        """R as ALU `a`'s own operand: unknown through unregistered bits."""
        r = self.register[a.name]
        if a.unregistered or r is None:
            return None
        return r & ~a.stuck & M54

    def visible(self, n, sets):
        """R of every ALU as the fabric and the cascade see it now; free
        registers settle on the way."""
        seen = {}
        for a in self.alus:
            cin = seen[a.cin] if a.cin else 0
            d = sets[a.set]
            if a.free:
                op = a.op(n)
                reads_r = op & 3 == 0 or op >> 4 & 7 == 5
                self.register[a.name] = None if reads_r else result(a, n, d, None, cin)
            r = self.register[a.name]
            value = None if r is None else r & ~a.unregistered & ~a.stuck & M54
            if a.unregistered:
                now = result(a, n, d, self.operand(a), cin)
                registered = ~a.unregistered & ~a.stuck & M54
                value = None if now is None or (registered and r is None) else (value or 0) | now & a.unregistered
            seen[a.name] = value
        return seen

    def step(self, n, sets):
        """The pulse of step `n`: returns R of every ALU after it."""
        before = self.visible(n, sets)
        captured = {}
        for a in self.alus:
            cin = before[a.cin] if a.cin else 0
            captured[a.name] = result(a, n, sets[a.set], self.operand(a), cin)
        self.register = captured
        self.pulsed = True
        return self.visible(n, sets)

    def flags(self, a):
        return flags_of(a.patterns) if self.pulsed or not a.flag_register else 0


def check(alus, steps, lines, initial=None):
    """`lines`: step -> the parsed lines of that step (one, or two where the
    second pass differs from the first); each a dict with the operand sets
    under 1 and 2 and <name>_r, <name>_f. Compares the first pass, from the
    configured state. Prints the counts; returns whether every predicted
    value was the board's."""
    chain = Chain(alus)
    ok = True
    if initial is not None:
        seen = chain.visible(0, initial)
        bad = [a.name for a in alus
               if (seen[a.name] is not None and seen[a.name] != initial[a.name + "_r"])
               or chain.flags(a) != initial[a.name + "_f"]]
        print(f"before the first pulse: R and flags differ in {bad or 'none'}")
        ok &= not bad
    counts = {a.name: collections.Counter() for a in alus}
    for n in range(steps):
        variants = lines[n]
        seen = chain.step(n, variants[0])
        for a in alus:
            c = counts[a.name]
            if chain.flags(a) not in {v[a.name + "_f"] for v in variants}:
                c["flags differ"] += 1
            if seen[a.name] is None:
                c["not predicted"] += 1
            elif seen[a.name] in {v[a.name + "_r"] for v in variants}:
                c["equal"] += 1
            else:
                c["differ"] += 1
    for a in alus:
        c = counts[a.name]
        print(f"{a.name}: R equal {c['equal']}, differ {c['differ']}, not predicted {c['not predicted']}; "
              f"flags differ {c['flags differ']}")
        ok &= c["differ"] == 0 and c["flags differ"] == 0
    return ok
