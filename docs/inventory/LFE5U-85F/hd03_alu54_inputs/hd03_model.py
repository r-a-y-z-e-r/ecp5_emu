"""Check the HD03 record against the ALU54B model.
Usage: python3 hd03_model.py observations.log

The model and what it leaves unpredicted are in ../hd02_alu54/alu54_chain.py.
Here: six ALUs a0, b0, a1, b1, a2, b2 in cascade, all on OP = n[12:2].
- a0, b0, b1, a2 have the C operand also on their MULT18 C ports; a1 and b2
  only on the ALU's pins, where C[17:0] and C[44:27] do nothing.
- nextpnr sets MULT18_5.CIBOUT_BYP in a block when one of its ALUs has
  REG_FLAG_CLK=NONE (a0 and b0; b2): R[26:18] and R[53:45] of the block's
  second ALU (b0, b2) are then unregistered.
- a2 has CE0 high, and nextpnr puts that net on the block's CE0 line for b2
  as well: both capture at every clock, whatever b2's CE0 is in the netlist.
- b0's MULT18 and a1's ALU have their C registers on (REG_INPUTC*_CLK=CLK0):
  with operands that hold still for many cycles they show nothing here.
- The b ALUs have other masks (MASKPAT 0F, MASK01 F0), which give the same
  flag constants as the a ALUs' (listed as patterns A)."""
import collections
import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "hd02_alu54"))
from alu54_chain import Alu, SECOND_HALF, check  # noqa: E402
from hd02_model import M54  # noqa: E402

NAMES = ["a0", "b0", "a1", "b1", "a2", "b2"]
STEPS = 8192


def op(n):
    return n >> 2


ALUS = [
    #   name  set op  cin   MULT C  unregistered stuck free   flag register  patterns
    Alu("a0", 1, op, None, True, 0, 0, False, False, "A"),
    Alu("b0", 2, op, "a0", True, SECOND_HALF, 0, False, False, "A"),
    Alu("a1", 1, op, "b0", False, 0, 0, False, True, "A"),
    Alu("b1", 2, op, "a1", True, 0, 0, False, True, "A"),
    Alu("a2", 1, op, "b1", True, 0, 0, True, True, "A"),
    Alu("b2", 2, op, "a2", False, SECOND_HALF, 0, True, False, "A"),
]


def parse(item):
    v = int(item[1:], 16)
    s = {}
    for name in reversed(NAMES):
        s[name + "_f"] = v & 0xFF
        v >>= 8
        s[name + "_r"] = v & M54
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
    text = open(sys.argv[1]).read()
    loads = [set(re.findall(r"'(T[0-9A-F]{170})'", line)) for line in text.splitlines()]
    print(f"{len(loads)} loads, distinct lines {[len(l) for l in loads]}, in all loads {len(set.intersection(*loads))}")
    ok = True
    for k, items in enumerate(loads):
        lines = collections.defaultdict(list)
        for item in items:
            s = parse(item)
            lines[s["n"]].append(s)
        assert len(lines) == STEPS
        print(f"load {k + 1}:")
        ok &= check(ALUS, STEPS, lines)
    print("model fits" if ok else "NO FIT")


if __name__ == "__main__":
    main()
