"""Regenerate in memory, synthesize every HD09 design, and verify DSP parameters.
Writes temporary JSON only in this directory, then removes it. Does not route.
Usage: PYTHONDONTWRITEBYTECODE=1 python3 check.py
"""
import collections
import json
from pathlib import Path
import subprocess
import sys
sys.dont_write_bytecode = True
import hd09_gen
import hd09m_gen
import hd09p_gen

ROOT = Path(__file__).resolve().parent


def main():
    print(subprocess.check_output(["yosys", "-V"], text=True).strip())
    out = ROOT / ".check.json"
    try:
        for name, gen, expected in [("hd09", hd09_gen, (11, 22)), ("hd09m", hd09m_gen, (4, 12)), ("hd09p", hd09p_gen, (4, 12))]:
            for once in (False, True):
                v = name + ("s" if once else "") + ".v"
                assert (ROOT/v).read_text() == gen.generate(once), f"regenerate {v}"
                subprocess.run(["yosys", "-q", "-p", f"read_verilog {v}; synth_ecp5 -top top -json .check.json"], cwd=ROOT, check=True)
                cells = json.loads(out.read_text())["modules"]["top"]["cells"]
                counts = collections.Counter(c["type"] for c in cells.values())
                assert (counts["ALU54B"], counts["MULT18X18D"]) == expected
                for cell in cells.values():
                    if cell["type"] in ("ALU54B", "MULT18X18D"):
                        assert all(cell["parameters"].get(f"CLK{k}_DIV") == "DISABLED" for k in range(4))
                        assert all(cell["connections"][f"CLK{k}"] == cell["connections"]["CLK0"] for k in range(1,4)), "clock alias required for nextpnr divider remapping"
                if name == "hd09p":
                    for n, (reg, ce, rst) in hd09p_gen.PROBES.items():
                        p = cells[n]["parameters"]
                        for r in ("INPUTA", "INPUTB", "PIPELINE", "OUTPUT"):
                            assert p[f"REG_{r}_CLK"] == ("CLK0" if r == reg else "NONE"), (n, r, p)
                        if reg != "OUTPUT":
                            assert p[f"REG_{reg}_CE"] == f"CE{ce}"
                            assert p[f"REG_{reg}_RST"] == f"RST{rst}"
                print(v, dict(sorted(counts.items())), "all CLKn_DIV=DISABLED: PASS", flush=True)
                out.unlink()
    finally:
        out.unlink(missing_ok=True)
    for script in ("hd09_analyse.py", "hd09m_analyse.py"):
        subprocess.run([sys.executable, "-B", str(ROOT/script), "--selftest"], check=True)


if __name__ == "__main__":
    main()
