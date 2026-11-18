"""Does CLKn_DIV=DISABLED change ALU54B register capture rates?

Design table: exactly HD08 variant c, including its BELs and nets for constants.
  o1 IN_0; o2 OP0_0; o3 OP1_0; o5 OP0_1; o6 OP1_1 (one stage each).
  o4 IN_0 + OP0_0 + OP1_0; o7 IN_0 + two OP0 and two OP1 stages.
  c1 C0/C1, SYNC; c2 C0/C1, ASYNC, reads A; c3 C0 only;
  c4 ALU requests C0/C1 but MULT18 bypasses them (control).
Every ALU54B and MULT18X18D explicitly sets CLK0_DIV..CLK3_DIV=DISABLED.
CLK0..3 all connect to clk_25mhz. The nextpnr clock remapper copies the
duplicate pins to CLK0, preserving CLK0_DIV=DISABLED on the active clock.
Unused physical divider fields revert to ENABLED; they do not clock registers.
Traffic: HD08c's three 64-bit LFSRs, random CE/RST/GSR, C and safe OP modes.
Line: M + 5120 hex + CR LF, 128 samples of 160 bits, oldest in the MSBs;
see the unchanged sample concatenation in hd09.v for the complete layout.
Alignment: sample k precedes rising edge k; sample k+1 includes that edge.
The LFSRs and DSPs run during UART transmission. Variant s repeats only the
first window from configuration, also with HD08c's nets and no IN_1.
Usage: python3 hd09_gen.py [s] > hd09[s].v
"""
import contextlib
import io
from pathlib import Path
import re
import runpy
import sys

BASE = Path(__file__).resolve().parent.parent


def reference(filename, variant):
    """Run an existing generator without changing its files or arguments."""
    argv = sys.argv
    out = io.StringIO()
    try:
        sys.argv = [str(filename), variant]
        with contextlib.redirect_stdout(out):
            runpy.run_path(str(filename), run_name="__main__")
    finally:
        sys.argv = argv
    return out.getvalue()


def disabled(text):
    # The remapper erases unmoved divider parameters. Duplicate clock pins
    # make it copy DISABLED from CLK1..3 onto the active physical CLK0.
    for k in range(1, 4):
        text = text.replace(f".CLK{k}(1'b0)", f".CLK{k}(clk_25mhz)")
    params = ", ".join(f'.CLK{k}_DIV("DISABLED")' for k in range(4))
    text = re.sub(r'\b(ALU54B|MULT18X18D)\s*#\(',
                  lambda m: f'{m[1]} #({params}, ', text)
    return "\n".join(line.rstrip() for line in text.splitlines()) + "\n"


def generate(once=False):
    text = reference(BASE / "hd08_alu54_cop" / "hd08_gen.py", "c")
    if once:
        old = "if (ci == 5122) begin ci <= 0; state <= REC; end"
        assert old in text
        text = text.replace(old, old.replace("REC", "SEND"))
    text = text.replace("HD08c", "HD09s" if once else "HD09")
    text = text.replace("hd08_gen.py c", "hd09_gen.py" + (" s" if once else ""))
    note = "// All DSP CLK pins use clk_25mhz; remapping preserves CLK0_DIV=DISABLED.\n"
    if once:
        note += "// Only the first 128 samples from configuration are sent repeatedly.\n"
    return note + disabled(text)


if __name__ == "__main__":
    if sys.argv[1:] not in ([], ["s"]):
        raise SystemExit("Usage: hd09_gen.py [s]")
    print(generate(bool(sys.argv[1:])), end="")
