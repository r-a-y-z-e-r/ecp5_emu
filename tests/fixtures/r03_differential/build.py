#!/usr/bin/env python3
"""Route plain-constraint differential IOs; keep configs/logs under target/r03-map."""
import concurrent.futures
import csv
import json
import pathlib
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[3]
OUT = ROOT / "target/r03-map"
OUT.mkdir(parents=True, exist_ok=True)
DESIGNS = {
    "INPUT": "module top(input clk,d,output reg q); always @(posedge clk)q<=d; endmodule",
    "OUTPUT": "module top(input clk,d,output reg q); always @(posedge clk)q<=d; endmodule",
    "BIDIR": "module top(input clk,d,en,inout p,output reg q); wire v; assign p=en?d:1'bz; assign v=p; always @(posedge clk)q<=v; endmodule",
}
for direction, verilog in DESIGNS.items():
    (OUT / f"{direction}.v").write_text(verilog)
    subprocess.run(["yosys", "-Qq", "-p", f"read_verilog {direction}.v; synth_ecp5 -top top -json {direction}.json"], cwd=OUT, check=True)

row = next(r for r in csv.DictReader((ROOT / "docs/inventory/LFE5U-85F/settings_coverage.csv").open()) if r["group"] == "PIO" and r["field"] == "BASE_TYPE")
def differential(base):
    s = base.split("_", 1)[-1]
    return any(d in s for d in ("LVDS", "LVPECL", "SLVS")) or s.endswith("D") or "D_" in s

cases = [(b, "A2") for b in row["options"].split(" | ") if differential(b)]
# IO_TYPE names accepted by nextpnr but absent for this direction in Trellis.
cases += [(f"{d}_{s}", "A2") for d in DESIGNS for s in ("LVDS25E", "BLVDS25", "BLVDS25E", "MLVDS25", "MLVDS25E", "SLVS", "SUBLVDS", "LVPECL33", "LVPECL33E", "LVCMOS12D", "LVCMOS15D")]
cases += [(f"{d}_LVDS", ball) for d in ("INPUT", "OUTPUT") for ball in ("A10", "A12", "C18", "L16", "K2", "B2")]
cases += [(f"OUTPUT_{s}", "B9") for s in ("LVCMOS12D", "LVCMOS15D")]

def route(case):
    base, ball = case
    direction, standard = base.split("_", 1)
    name = f"{base}_{ball}"
    port = {"INPUT": "d", "OUTPUT": "q", "BIDIR": "p"}[direction]
    pins = {"clk": "G2", "d": "L4", "q": "B11"}
    if direction == "BIDIR":
        pins.update(en="R1", p=ball)
    pins[port] = ball
    lpf = "".join(f'LOCATE COMP "{p}" SITE "{b}"; IOBUF PORT "{p}" IO_TYPE={standard if p == port else "LVCMOS25"} PULLMODE=NONE;\n' for p, b in pins.items())
    (OUT / f"{name}.lpf").write_text(lpf)
    result = subprocess.run(["nextpnr-ecp5", "--85k", "--package", "CABGA381", "--json", f"{direction}.json", "--lpf", f"{name}.lpf", "--textcfg", f"{name}.config", "--seed", "1", "-q"], cwd=OUT, capture_output=True, text=True)
    (OUT / f"{name}.log").write_text(result.stdout + result.stderr)
    record = dict(base_type=base, ball=ball, routed=result.returncode == 0)
    if result.returncode:
        record["error"] = " ".join(line.strip() for line in result.stderr.splitlines() if "ERROR" in line or "what():" in line) or result.stderr.strip()
    else:
        text = (OUT / f"{name}.config").read_text()
        record["written_base_types"] = sorted(set(re.findall(r"enum: PIO[A-D]\.BASE_TYPE (\S+)", text)))
        packed = subprocess.run(["ecppack", f"{name}.config", f"{name}.bit"], cwd=OUT, capture_output=True, text=True)
        record["packed"] = packed.returncode == 0
        if packed.returncode:
            record["pack_error"] = next((line.strip() for line in packed.stderr.splitlines() if "what():" in line or "Error" in line or "ERROR" in line), packed.stderr.strip().splitlines()[0])
    return record

with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    records = list(pool.map(route, sorted(set(cases))))
destination = pathlib.Path(__file__).with_name("mapping.json")
destination.write_text(json.dumps(records, indent=2) + "\n")
print(f"{len(records)} constraints checked; {sum(r['routed'] and r.get('packed', False) for r in records)} route and pack: {destination}")
