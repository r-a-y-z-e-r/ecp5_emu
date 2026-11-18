#!/usr/bin/env python3
"""Collect EBR control wiring and board experiments; never simulate the EBR array."""
import argparse
from collections import Counter
import itertools
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import re
import shutil
import subprocess

from inventory_trellis import sha256, write_csv
from resource_audit import packed

ROOT = Path(__file__).resolve().parents[1]
DEVICE = "LFE5U-85F"
CASES = ("base", "sync_ungated", "sync_gated", "sync_write_survives", "async", "fabric_pipeline", "falling_edge")
PINS = ("CE", "OCE", "WE", "RST", "BE0", "BE1")


def probe_source(kind):
    body = ""
    for p in ("a", "b"):
        q = f"qr{p}" if kind == "fabric_pipeline" else f"q{p}"
        ops = f"""if(w{p}) begin
  if(be{p}[0]) mem[a{p}][8:0]<=d{p}[8:0];
  if(be{p}[1]) mem[a{p}][17:9]<=d{p}[17:9];
end else {q}<=mem[a{p}];"""
        edge = "negedge" if kind == "falling_edge" else "posedge"
        if kind in ("base", "falling_edge", "fabric_pipeline"):
            body += f"always @({edge} c{p}) if(ce{p}) begin {ops} end\n"
        elif kind == "sync_ungated":
            body += f"always @(posedge c{p}) if(r{p}) {q}<=0; else if(ce{p}) begin {ops} end\n"
        elif kind == "sync_gated":
            body += f"always @(posedge c{p}) if(ce{p}) begin if(r{p}) {q}<=0; else begin {ops} end end\n"
        elif kind == "sync_write_survives":
            body += f"always @(posedge c{p}) begin if(ce{p}) begin {ops} end if(r{p}) {q}<=0; end\n"
        elif kind == "async":
            body += f"always @(posedge c{p}) if(ce{p} && w{p}) begin if(be{p}[0]) mem[a{p}][8:0]<=d{p}[8:0]; if(be{p}[1]) mem[a{p}][17:9]<=d{p}[17:9]; end\n"
            body += f"always @(posedge c{p} or posedge r{p}) if(r{p}) {q}<=0; else if(ce{p} && !w{p}) {q}<=mem[a{p}];\n"
        if kind == "fabric_pipeline":
            body += f"always @(posedge c{p}) if(oce{p}) q{p}<=qr{p};\n"
    extra = "reg [17:0] qra,qrb;\n" if kind == "fabric_pipeline" else ""
    return """module top(input ca,cb,cea,ceb,ra,rb,wa,wb,ocea,oceb,
input [9:0] aa,ab,input [17:0] da,db,input [1:0] bea,beb,
output reg [17:0] qa,qb);
reg [17:0] mem [0:1023];
integer i; initial for(i=0;i<1024;i=i+1) mem[i]=i^18'h25555;
""" + extra + body + "endmodule\n"


def control_netlist_evaluator(module):
    drivers = {}
    for cell in module["cells"].values():
        for port, direction in cell["port_directions"].items():
            if direction == "output":
                for bit in cell["connections"][port]:
                    if isinstance(bit, int):
                        if bit in drivers: raise ValueError("multiple netlist drivers")
                        drivers[bit] = cell

    def evaluate(bit, values, visiting=None):
        if bit in ("0", "1"): return int(bit)
        if bit in values: return values[bit]
        visiting = set() if visiting is None else visiting
        if bit in visiting: raise ValueError("combinational loop in control cone")
        cell = drivers[bit]; connections = cell["connections"]
        def pin(name): return evaluate(connections[name][0], values, visiting | {bit})
        if cell["type"] == "LUT4":
            index = sum(pin(p) << i for i, p in enumerate("ABCD"))
            value = (int(cell["parameters"]["INIT"], 2) >> index) & 1
        elif cell["type"] == "PFUMX": value = pin("ALUT" if pin("C0") else "BLUT")
        elif cell["type"] == "L6MUX21": value = pin("D1" if pin("SD") else "D0")
        else: raise ValueError(f"control cone needs unsupported or sequential cell {cell['type']}")
        values[bit] = value
        return value
    return evaluate


def run(command, directory):
    result = subprocess.run(command, cwd=directory, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"{Path(command[0]).name} failed ({result.returncode}):\n{result.stderr[-5000:]}")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "docs/inventory" / DEVICE / "ebr_control_research")
    parser.add_argument("--probe-directory", type=Path, default=ROOT / "target/research/ebr/control-probes")
    parser.add_argument("--yosys-library", type=Path, default=Path("/opt/homebrew/share/yosys/lattice"))
    parser.add_argument("--nextpnr", type=Path, help="Source checkout (NEXTPNR_ROOT)")
    parser.add_argument("--yosys", type=Path, default=Path(shutil.which("yosys") or "yosys"))
    parser.add_argument("--iverilog", type=Path, default=Path(shutil.which("iverilog") or "iverilog"))
    parser.add_argument("--vvp", type=Path, default=Path(shutil.which("vvp") or "vvp"))
    parser.add_argument("--database", type=Path, help="Database root (ECP5_DB or TRELLIS_ROOT)")
    args = resolve_toolchain(parser, parser.parse_args())
    contract_path = ROOT / "tools/data/ebr_control_contract.json"
    contract = json.loads(contract_path.read_text())
    inventory_path = ROOT / "docs/inventory" / DEVICE / "device_inventory.json"
    inventory = json.loads(inventory_path.read_text())
    for item in inventory["source_files"]:
        if sha256(args.database / item["path"]) != item["sha256"]:
            raise ValueError(f"stale database: {item['path']}")
    binary, iverilog, vvp = args.yosys.resolve(), args.iverilog.resolve(), args.vvp.resolve()
    scratch = args.probe_directory.resolve(); scratch.mkdir(parents=True, exist_ok=True)
    version = run([str(binary), "-V"], scratch).stdout.strip()
    case_rows, connection_rows, vector_rows, sources = [], [], [], []
    native_pins = lambda p: [f"CE{p}", f"OCE{p}", f"WE{p}", f"RST{p}", f"AD{p}0", f"AD{p}1"]
    for case in CASES:
        source = scratch / f"{case}.v"; source.write_text(probe_source(case))
        run([str(binary), "-Q", "-T", "-q", "-p",
             f"read_verilog {case}.v; synth_ecp5 -top top -json {case}.json; write_verilog -noattr {case}-mapped.v",
             "-l", f"{case}.log"], scratch)
        netlist_path = scratch / f"{case}.json"
        module = json.loads(netlist_path.read_text())["modules"]["top"]
        memories = [(name, cell) for name, cell in module["cells"].items() if cell["type"] == "DP16KD"]
        if len(memories) != 1: raise ValueError(f"{case}: expected one EBR")
        cell_name, cell = memories[0]; ports, connections, params = module["ports"], cell["connections"], cell["parameters"]
        cells = dict(sorted(Counter(c["type"] for c in module["cells"].values()).items()))
        if params["REGMODE_A"] != "NOREG" or params["REGMODE_B"] != "NOREG": raise ValueError("unexpected automatic output-register absorption")
        if case == "fabric_pipeline" and cells.get("TRELLIS_FF") != 36: raise ValueError("pipeline probe no longer has 36 fabric FF")
        if case == "sync_write_survives" and cells.get("TRELLIS_FF") != 2: raise ValueError("reset-overlay probe changed")
        top_bits = {bit: f"{name}[{i}]" for name, port in ports.items() for i, bit in enumerate(port["bits"]) if isinstance(bit, int)}
        for pin, bits in sorted(connections.items()):
            connection_rows.append([case, pin, packed([top_bits.get(bit, f"internal_net:{bit}" if isinstance(bit, int) else bit) for bit in bits])])
        case_rows.append([case, packed(cells), packed({k:v for k,v in params.items() if not k.startswith("INITVAL_")}), "control_wiring_only;no_EBR_behavior_executed"])
        evaluate = control_netlist_evaluator(module)
        sim_vectors, expected_strings = [], []
        for native in ("A", "B"):
            address = [connections[f"AD{native}{i}"][0] for i in range(4, 14)]
            logicals = [p for p in ("a", "b") if address == ports[f"a{p}"]["bits"]]
            if len(logicals) != 1: raise ValueError("address domain not preserved")
            logical = logicals[0]
            assert params[f"CLK{native}MUX"] == ("INV" if case == "falling_edge" else f"CLK{native}")
            for ce, we, reset, be0, be1, oce in itertools.product((0, 1), repeat=6):
                inputs = {name: 0 for name, port in ports.items() if port["direction"] == "input"}
                inputs.update({f"ce{logical}":ce, f"w{logical}":we, f"r{logical}":reset,
                               f"be{logical}":be0 | (be1 << 1), f"oce{logical}":oce})
                values = {bit:(inputs[name] >> i) & 1 for name, port in ports.items() if port["direction"] == "input" for i, bit in enumerate(port["bits"])}
                observed = [evaluate(connections[pin][0], values) for pin in native_pins(native)]
                rst_expected = reset if case in ("sync_ungated", "async") else reset & ce if case == "sync_gated" else 0
                if observed[3] != rst_expected: raise ValueError(f"{case}: changed reset conditioning")
                # Check input wiring; do not assign an EBR reset-versus-write priority.
                actual_mask = sum((observed[0] & observed[2] & observed[4+i]) << i for i in (0,1))
                if params["RESETMODE"] == "SYNC" and observed[3] and actual_mask:
                    raise ValueError("native synchronous reset and write are not mutually exclusive")
                wanted_mask = (be0 | be1 << 1) if ce and we and not (reset and case in ("sync_ungated", "sync_gated")) else 0
                if actual_mask != wanted_mask: raise ValueError(f"{case}: masked-write contract differs")
                vector_rows.append([case, logical.upper(), native, ce, we, reset, be0 | be1 << 1, oce, *observed, actual_mask,
                                    "combinational_pins_and_synthesis_contract;not_chip_behavior"])
                sim_vectors.append((inputs, native))
                expected_strings.append("".join(map(str, observed)))
        declarations = []
        for name, port in ports.items():
            width = "" if len(port["bits"]) == 1 else f"[{len(port['bits'])-1}:0] "
            declarations.append(f"{'reg' if port['direction']=='input' else 'wire'} {width}{name}" + ("=0;" if port["direction"]=="input" else ";"))
        statements = []
        for index, (inputs, native) in enumerate(sim_vectors):
            assignment = " ".join(f"{name}={value};" for name,value in inputs.items())
            access = ",".join(f"dut.\\{cell_name} .{pin}" for pin in native_pins(native))
            statements.append(f'{assignment} #1; $display("{index}:%b", {{{access}}});')
        testbench = scratch / f"{case}-pins-tb.v"
        testbench.write_text("module tb;\n" + "\n".join(declarations) + "\ntop dut(.*);\ninitial begin\n" + "\n".join(statements) + "\n$finish; end\nendmodule\n")
        executable = f"{case}-pins-sim"
        run([str(iverilog), "-g2012", "-DNO_INCLUDES", "-s", "tb", "-I", str(args.yosys_library), "-o", executable,
             str(args.yosys_library / "cells_sim_ecp5.v"), f"{case}-mapped.v", testbench.name], scratch)
        simulation = run([str(vvp), executable], scratch).stdout
        found = re.findall(r"^(\d+):([01xz]+)$", simulation, re.M)
        if found != [(str(i), value) for i, value in enumerate(expected_strings)]:
            raise ValueError(f"{case}: Icarus control-pin evaluation differs")
        (scratch / f"{case}-pins.log").write_text(simulation)
        sources += [(f"probe_source:{case}", source), (f"probe_netlist:{case}", netlist_path),
                    (f"probe_mapped:{case}", scratch / f"{case}-mapped.v"), (f"pin_testbench:{case}", testbench),
                    (f"pin_trace:{case}", scratch / f"{case}-pins.log")]
    questions = {q for case in contract["hardware_cases"] for q in case["questions"]}
    if questions != {f"B{i:02}" for i in range(1,9)}: raise ValueError("hardware queue does not cover all EBR questions")
    sources += [("collector", Path(__file__)), ("contract", contract_path), ("inventory", inventory_path),
                ("csv_helpers", ROOT / "tools/inventory_trellis.py"), ("json_helper", ROOT / "tools/resource_audit.py"),
                ("yosys_binary", binary), ("iverilog_binary", iverilog), ("vvp_binary", vvp),
                ("nextpnr_writer", args.nextpnr / "ecp5/bitstream.cc")]
    sources += [(name, args.yosys_library / name) for name in ("brams_16kd.txt", "brams_map_16kd.v", "cells_sim_ecp5.v", "common_sim.vh", "ccu2c_sim.vh")]
    sources += [(name, ROOT / item["path"]) for name,item in contract["sources"].items()]
    report = dict(schema_version=1, device=DEVICE, contract_rules=len(contract["rules"]), synthesis_cases=len(case_rows),
                  native_connections=len(connection_rows), control_vectors=len(vector_rows),
                  verification=dict(yosys_version=version, combinational_control_pins="matched_Icarus", write_input_masks="matched", EBR_behavior="not_executed", hardware="not_run"),
                  hardware_case_groups=len(contract["hardware_cases"]), covered_questions=sorted(questions),
                  source_files=[dict(source=name, sha256=sha256(path)) for name,path in sources])
    output = args.output; output.mkdir(parents=True, exist_ok=True)
    write_csv(output / "yosys_cases.csv", ["case", "cells", "EBR_parameters", "evidence_scope"], case_rows)
    write_csv(output / "yosys_connections.csv", ["case", "native_pin", "connected_bits"], connection_rows)
    write_csv(output / "yosys_control_vectors.csv", ["case", "logical_port", "native_port", "source_CE", "source_WE", "source_RST", "source_BE", "source_OCE", *[f"native_{p}" for p in PINS], "write_mask_at_native_inputs", "evidence_scope"], vector_rows)
    for name, value in (("control_contract.json", {k:v for k,v in contract.items() if k != "hardware_cases"}),
                        ("hardware_cases.json", dict(schema_version=1, status="not_run", cases=contract["hardware_cases"])),
                        ("evidence_report.json", report)):
        (output/name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
    print(packed({k:v for k,v in report.items() if k != "source_files"}))


if __name__ == "__main__": main()
