"""Does CLKn_DIV=DISABLED change MULT18 input and pipeline capture rates?

Design table: HD09m's ALUs and recording logic; replace ma..md stages by:
  ma MULT18_0: INPUTA CLK0, CE1/RST2, ASYNC; B=0x20001, A={0,c[16:1],0}.
  mb MULT18_1: INPUTB CLK0, CE2/RST3, SYNC; A=0x20001, B={0,c[16:1],0}.
  mc MULT18_4: PIPELINE CLK0, CE3/RST1, SYNC; same operands as mb.
  md MULT18_5: OUTPUT CLK0, CE0/RST0, ASYNC; same operands as mb.
Each has exactly one active stage. All CLKn_DIV parameters are DISABLED.
CLK0..3 all take clk_25mhz; nextpnr retains DISABLED on active physical CLK0,
with unused physical divider fields reverting to ENABLED.
Traffic, line format and alignment: identical to HD09m (M + 3840 hex,
256 pre-edge 60-bit samples), including P[33,18,16,1] = delayed c[16,1,16,1].
The ALUs remain HD09m controls. LFSRs run through UART; s repeats window zero.
Usage: python3 hd09p_gen.py [s] > hd09p[s].v
"""
import re
import sys
from hd09m_gen import generate as output_design

PROBES = {"ma": ("INPUTA", 1, 2), "mb": ("INPUTB", 2, 3),
          "mc": ("PIPELINE", 3, 1), "md": ("OUTPUT", 0, 0)}


def generate(once=False):
    text = output_design(once)
    # Match only the four standalone cells, leaving the ALU multiplier pairs intact.
    pattern = r'MULT18X18D #\([^;]+?\n    (ma|mb|mc|md)\([^;]+;'

    def change(match):
        cell, name = match[0], match[1]
        reg, ce, rst = PROBES[name]
        cell = cell.replace('.REG_OUTPUT_CLK("CLK0")', '.REG_OUTPUT_CLK("NONE")')
        cell = cell.replace(f'.REG_{reg}_CLK("NONE")', f'.REG_{reg}_CLK("CLK0")')
        cell = re.sub(r', \.REG_OUTPUT_RST\("RST\d"\)', '', cell)
        if reg != "OUTPUT":
            cell = cell.replace('.GSR("ENABLED")',
                                f'.REG_{reg}_CE("CE{ce}"), .REG_{reg}_RST("RST{rst}"), .GSR("ENABLED")')
        if name == "ma":
            for k in range(18):
                a = f"c[{k}]" if 1 <= k <= 16 else "1'b0"
                b = f"1'b{int(k in (0, 17))}"
                cell = re.sub(rf'\.A{k}\([^)]*\)', f'.A{k}({a})', cell)
                cell = re.sub(rf'\.B{k}\([^)]*\)', f'.B{k}({b})', cell)
        return cell

    text, count = re.subn(pattern, change, text)
    assert count == 4, count
    # Replace the inherited design description, whose output-only table no longer applies.
    text = text[text.index("module top"):]
    header = "\n".join(("// " + line).rstrip() for line in __doc__.splitlines()) + "\n"
    return header + "\n".join(line.rstrip() for line in text.splitlines()) + "\n"


if __name__ == "__main__":
    if sys.argv[1:] not in ([], ["s"]):
        raise SystemExit("Usage: hd09p_gen.py [s]")
    print(generate(bool(sys.argv[1:])), end="")
