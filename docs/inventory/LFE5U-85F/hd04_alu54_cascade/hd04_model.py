"""Check the HD04 record against the ALU54B model.
Usage: python3 hd04_model.py observations.log

The model and what it leaves unpredicted are in ../hd02_alu54/alu54_chain.py.
Here: twelve ALUs in five blocks (hd04.v). What each shows:
- pa, qa: the opcode sweep in two orders, qa with CIN from pb;
- pb, qb, ra: R = CIN >> 18 at every step: its top 18 bits are CIN[53] &
  SIGNEDIB of the reading ALU itself;
- rb: its CE0 is high in the netlist, but it captures once per step: nextpnr
  puts the first ALU's CE0 net (ra's, the pulse) on the block's CE0 line;
- sa, sb: the flags with other patterns: those of a zero word;
- ta, tb, ua, ub: output registers off in part or in whole. Each group of R
  is unregistered when its MULT18's CIBOUT_BYP is on, else on its
  REG_OUTPUTn_CLK, else (NONE) stuck at 0:
    R[17:0]   REG_OUTPUT0_CLK, first MULT18's bypass
    R[26:18]  REG_OUTPUT0_CLK, second MULT18's bypass
    R[44:27]  REG_OUTPUT1_CLK, first MULT18's bypass
    R[53:45]  REG_OUTPUT0_CLK, second MULT18's bypass
  nextpnr sets the first MULT18's bypass when REG_OUTPUT0_CLK is NONE and the
  second MULT18's of the block's second slice when REG_FLAG_CLK is NONE.
The first line ("I") shows the state before any pulse: registers 0, flag
registers 0, unregistered bits and flags already showing their values."""
import collections
import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "hd02_alu54"))
from alu54_chain import Alu, FIRST_HALF, GROUPS, check  # noqa: E402
from hd02_model import M54  # noqa: E402

STEPS = 8192
READ = 0b01000011111


def opa(n):
    return n >> 2


def opb(n):
    return (opa(n) * 1237 + 77) & 0x7FF


def read(n):
    return READ


STUCK_B_D = GROUPS["b"] | GROUPS["d"]
ALUS = [
    #   name  set op   cin   MULT C  unregistered stuck      free   flag register  patterns
    Alu("pa", 1, opa, None, True, 0, 0, False, True, "A"),
    Alu("pb", 2, read, "pa", True, 0, 0, False, True, "A"),
    Alu("qa", 2, opb, "pb", True, 0, 0, False, True, "A"),
    Alu("qb", 1, read, "qa", True, 0, 0, False, True, "A"),
    Alu("ra", 1, read, "qb", True, 0, 0, False, True, "A"),
    Alu("rb", 2, opa, "ra", True, 0, 0, False, True, "A"),
    Alu("sa", 1, opa, None, True, 0, 0, False, True, "Z"),
    Alu("sb", 2, opa, "sa", True, 0, 0, False, True, "O"),
    Alu("ta", 1, opa, None, True, FIRST_HALF, STUCK_B_D, False, True, "A"),
    Alu("tb", 2, opa, "ta", True, M54, 0, False, False, "A"),
    Alu("ua", 1, opa, None, True, 0, GROUPS["c"], False, True, "A"),
    Alu("ub", 2, opa, "ua", True, FIRST_HALF, STUCK_B_D, False, True, "A"),
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
