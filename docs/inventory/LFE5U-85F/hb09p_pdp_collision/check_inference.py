"""Rebuild the plain student RAM mapping matrix in a scratch directory.

python3 check_inference.py /tmp/hb09p-inference > mapping.json
No -no-rw-check, ram_style, primitive attributes or altered memory passes.
"""
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import subprocess


def rtl(width, depth):
    return f'''module top(input clk,we,input [{(depth-1).bit_length()-1}:0] a,
  input [{width-1}:0] d,output reg [{width-1}:0] q);
  reg [{width-1}:0] m[0:{depth-1}]; integer i;
  initial for(i=0;i<{depth};i=i+1)m[i]=0;
  always @(posedge clk) begin if(we)m[a]<=d; q<=m[a]; end
endmodule
'''


def build(directory, width, depth):
    stem = directory / f"ram_{width}_{depth}"
    stem.with_suffix(".v").write_text(rtl(width, depth))
    with stem.with_suffix(".yosys.log").open("w") as log:
        subprocess.run(["yosys", "-Q", "-p",
                        f"read_verilog {stem}.v; synth_ecp5 -top top -json {stem}.json"],
                       stdout=log, stderr=subprocess.STDOUT, check=True)
    module = json.loads(stem.with_suffix(".json").read_text())["modules"]["top"]
    blocks = []
    for name, cell in module["cells"].items():
        if "16KD" not in cell["type"]:
            continue
        p, c = cell["parameters"], cell["connections"]
        widths = [int(p[f"DATA_WIDTH_{s}"], 2) for s in "AB"]
        blocks.append({"cell": name, "primitive": cell["type"],
                       "physical_mode": "PDPW16KD" if widths[0] == 36 else "DP16KD",
                       "widths_AB": widths,
                       "write_modes_AB": [p.get(f"WRITEMODE_{s}", "NORMAL (default)") for s in "AB"],
                       "reg_modes_AB": [p[f"REGMODE_{s}"] for s in "AB"],
                       "reset": p["RESETMODE"], "release": p["ASYNC_RESET_RELEASE"],
                       "controls": {k: c[k][0] if isinstance(c[k][0], str) else "signal"
                                    for k in ("CEA", "CEB", "OCEA", "OCEB", "WEA", "WEB")},
                       "same_clock": c["CLKA"] == c["CLKB"]})
    return {"width": width, "depth": depth, "blocks": blocks,
            "cells": dict(sorted(Counter(c["type"] for c in module["cells"].values()).items()))}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scratch", type=Path)
    args = parser.parse_args()
    args.scratch.mkdir(parents=True, exist_ok=True)
    # Yosys's command language needs quoted paths; use a scratch path without
    # spaces for the generated script fragments, as in the documented command.
    if any(c.isspace() for c in str(args.scratch)):
        parser.error("use a scratch path without whitespace")
    cases = [(w, d) for w in (16, 18, 32, 36) for d in (32, 64, 128, 256, 512, 1024, 2048, 4096)]
    with ThreadPoolExecutor(max_workers=4) as pool:
        rows = list(pool.map(lambda wd: build(args.scratch, *wd), cases))
    print(json.dumps({"yosys": subprocess.check_output(["yosys", "-V"], text=True).strip(),
                      "command": "read_verilog CASE.v; synth_ecp5 -top top -json CASE.json",
                      "cases": rows}, indent=2))
