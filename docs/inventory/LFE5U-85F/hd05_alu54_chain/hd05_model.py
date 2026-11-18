"""Check the HD05 record against the ALU54B model.
Usage: python3 hd05_model.py observations.log

The model is in ../hd02_alu54/alu54_chain.py. Here: the eight ALUs of HD04
whose outputs are all registered (hd05.v), so every value of every step is
predicted: the opcode sweep in two orders (pa; qa with CIN), CIN >> 18 with
its fill (pb, qb, ra), a second ALU on the first ALU's CE0 net (rb), flags with
other patterns (sa, sb). The first line ("I") shows the state before any
pulse."""
import collections
import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "hd02_alu54"))
from alu54_chain import Alu, check  # noqa: E402
from hd02_model import M54  # noqa: E402

STEPS = 8192
READ = 0b01000011111


def opa(n):
    return n >> 2


def opb(n):
    return (opa(n) * 1237 + 77) & 0x7FF


def read(n):
    return READ


ALUS = [
    #   name  set op   cin   MULT C  unregistered stuck free  flag register  patterns
    Alu("pa", 1, opa, None, True, 0, 0, False, True, "A"),
    Alu("pb", 2, read, "pa", True, 0, 0, False, True, "A"),
    Alu("qa", 2, opb, "pb", True, 0, 0, False, True, "A"),
    Alu("qb", 1, read, "qa", True, 0, 0, False, True, "A"),
    Alu("ra", 1, read, "qb", True, 0, 0, False, True, "A"),
    Alu("rb", 2, opa, "ra", True, 0, 0, False, True, "A"),
    Alu("sa", 1, opa, None, True, 0, 0, False, True, "Z"),
    Alu("sb", 2, opa, "sa", True, 0, 0, False, True, "O"),
]


def parse(item):
    v = int(item[1:], 16)
    s = {}
    for a in reversed(ALUS):
        s[a.name + "_f"] = v & 0xFF
        v >>= 8
        s[a.name + "_r"] = v & M54
        v >>= 56
    for k in (2, 1):
        d = {}
        for field, bits in [("c", 56), ("b1", 20), ("a1", 20), ("b0", 20), ("a0", 20), ("sg", 4)]:
            d[field] = v & (1 << bits) - 1
            v >>= bits
        s[k] = d
    s["n"] = v
    return s


def main():
    digits = 4 + 70 + 16 * len(ALUS)
    text = open(sys.argv[1]).read()
    loads = [set(re.findall(r"'([TI][0-9A-F]{%d})'" % digits, line)) for line in text.splitlines()]
    print(f"{len(loads)} loads, distinct lines {[len(l) for l in loads]}, in all loads {len(set.intersection(*loads))}")
    ok = True
    for k, items in enumerate(loads):
        lines = collections.defaultdict(list)
        initial = [parse(item) for item in items if item[0] == "I"]
        for item in items:
            if item[0] == "T":
                s = parse(item)
                lines[s["n"]].append(s)
        assert len(lines) == STEPS and len(initial) == 1
        print(f"load {k + 1}:")
        ok &= check(ALUS, STEPS, lines, initial[0])
    print("model fits" if ok else "NO FIT")


if __name__ == "__main__":
    main()
