"""Synthesize HS02 with native PRLD parameters despite Yosys's ABC9 init bug.

Usage: python3 -B synth.py [s]
Writes hs02.json or hs02s.json in this directory; does not route or pack.
After normal FF mapping, replace only the TRELLIS_FF library whitebox with
its parameter/port interface. This prevents ABC9 elaborating initial Q=M,
while preserving every native FF instance, parameter and connection.
"""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent


def script(v, output):
    return (f"read_verilog {v}; synth_ecp5 -top top -run begin:map_luts; "
            "delete =TRELLIS_FF; read_verilog -lib hs02_ff_bb.vh; "
            f"synth_ecp5 -top top -run map_luts: -json {output}")


if __name__ == "__main__":
    if sys.argv[1:] not in ([], ["s"]):
        raise SystemExit("Usage: synth.py [s]")
    stem = "hs02s" if sys.argv[1:] else "hs02"
    subprocess.run(["yosys", "-q", "-p", script(stem+".v", stem+".json")], cwd=ROOT, check=True)
