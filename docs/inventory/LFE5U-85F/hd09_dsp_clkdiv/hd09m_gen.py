"""Does CLKn_DIV=DISABLED change DSP output register capture rates?

Design table: HD07 unchanged apart from divider parameters on all DSP cells.
  ea: ALU SYNC, MULT18 ASYNC/SYNC, RST0; eb: SYNC, SYNC/ASYNC, RST0.
  ec: SYNC throughout, first MULT18 RST2; ed: ASYNC throughout, RST1.
  ma: MULT18 ASYNC RST1; mb: SYNC RST0; mc: SYNC RST0; md: ASYNC RST0.
All have output registers on CLK0, GSR enabled, and CLK0_DIV..3_DIV disabled.
CLK0..3 all take clk_25mhz so remapping retains the active CLK0 divider setting;
unused physical divider fields revert to ENABLED.
HD07 contains no enabled MULT18 input or pipeline stages: hd09p adds those.
Traffic: HD07's LFSRs drive C, two banks of CE (3/4), RST (1/8), GSR (1/32).
Line: M + 3840 hex + CR LF, 256 samples of 60 bits, oldest in the MSBs.
Layout and alignment are exactly HD07: controls, C bits, four ALU nibbles,
C[16,1], four MULT18 nibbles; sample k is before rising edge k.
Free-running traffic continues during UART. Variant s repeats window zero.
Usage: python3 hd09m_gen.py [s] > hd09m[s].v
"""
import sys
from hd09_gen import BASE, disabled, reference


def generate(once=False):
    text = reference(BASE / "hd07_dsp_gsr" / "hd07_gen.py", "s" if once else "")
    text = text.replace("HD07", "HD09m").replace("hd07_gen.py", "hd09m_gen.py")
    return "// All DSP CLK pins use clk_25mhz; remapping preserves CLK0_DIV=DISABLED.\n" + disabled(text)


if __name__ == "__main__":
    if sys.argv[1:] not in ([], ["s"]):
        raise SystemExit("Usage: hd09m_gen.py [s]")
    print(generate(bool(sys.argv[1:])), end="")
