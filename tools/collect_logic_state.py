#!/usr/bin/env python3
"""Collect fabric FF/CCU2 configuration and external-model research evidence."""
import argparse
from collections import Counter
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import shutil
import subprocess

from collect_plc_evidence import bits, read_csv
from inventory_trellis import sha256, verify_native, write_csv
from logic_state_probes import probes
from resource_audit import packed

ROOT = Path(__file__).resolve().parents[1]
DEVICE = "LFE5U-85F"


def matches(state, group):
    return all(state[f, b] == value for f, b, value in group)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", type=Path, default=ROOT / "docs/inventory" / DEVICE)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--probe-directory", type=Path, default=ROOT / "target/research/logic_state/probes")
    parser.add_argument("--trellis", type=Path, help="Source checkout (TRELLIS_ROOT or parent of ECP5_DB)")
    parser.add_argument("--nextpnr", type=Path, help="Source checkout (NEXTPNR_ROOT)")
    parser.add_argument("--yosys-library", type=Path, default=Path("/opt/homebrew/share/yosys/lattice"))
    parser.add_argument("--wire-probe", type=Path, default=ROOT / "target/trellis_wire_probe")
    for name in ("yosys", "iverilog", "vvp"):
        parser.add_argument(f"--{name}", type=Path, default=Path(shutil.which(name) or name))
    parser.add_argument("--database", type=Path, help="Database root (ECP5_DB or TRELLIS_ROOT)")
    args = resolve_toolchain(parser, parser.parse_args())
    folder = args.inventory
    inventory_path = folder / "device_inventory.json"
    inventory = json.loads(inventory_path.read_text())
    if inventory["device"] != DEVICE: raise ValueError("wrong device")
    database = args.database
    for entry in inventory["source_files"]:
        if sha256(database / entry["path"]) != entry["sha256"]: raise ValueError("stale database")
    desc = inventory["tile_types"]["PLC2"]
    contract_path = ROOT / "tools/data/logic_state_contract.json"
    contract = json.loads(contract_path.read_text())
    question_ids = {q["id"] for q in contract["open_questions"]}
    if len(question_ids) != len(contract["open_questions"]): raise ValueError("duplicate questions")
    for case in contract["hardware_cases"]:
        if not set(case["questions"]) <= question_ids: raise ValueError("unknown hardware question")

    # Reuse the full-device native-interface audit; do not construct extra FF capacity.
    base = folder / "plc_research"
    previous_report = json.loads((base / "evidence_report.json").read_text())
    if not previous_report["native_ports"]["all_PLCs_matched"]: raise ValueError("missing PLC interface audit")
    ports = [p for p in read_csv(base / "native_ports.csv") if p["view"] == "split"]
    ff_templates = []
    for z in range(8):
        slice_name, reg = "SLICE" + "ABCD"[z // 2], z % 2
        bel = f"{slice_name}.FF{reg}"
        pins = {p["port"]: p["wire"] for p in ports if p["bel"] == bel}
        if set(pins) != {"CLK", "LSR", "CE", "DI", "M", "Q"}: raise ValueError("FF interface mismatch")
        ff_templates.append([bel, z*4+1, packed(pins), packed([f"{slice_name}.REG{reg}.{n}" for n in ("SD", "REGSET", "LSRMODE")]),
                             packed([f"{slice_name}.{n}" for n in ("CEMUX", "GSR")]), f"MUXCLK{z//2}", f"MUXLSR{z//2}",
                             "same_physical_FF_as_coarse_SLICE;native_ports_from_PLC_audit"])

    fields = {name: value for name, value in desc["enums"].items()
              if name.startswith(("CLK", "LSR")) or any(part in name for part in (".REG", ".GSR", ".CEMUX", ".CCU2.INJECT1")) or name.endswith(".MODE")}
    enums = [[name, option["value"], value["default"], packed(bits(option["bits"])), f"{desc['source']}:{option['line']}"]
             for name, value in sorted(fields.items()) for option in value["options"]]
    route_names = {f"{kind}{i}" for kind, count in (("F", 8), ("MUXCLK", 4), ("MUXLSR", 4), ("CE", 4), ("CLK", 2), ("LSR", 2)) for i in range(count)}
    routes = [[name, option["source"], packed(bits(option["bits"])), f"{desc['source']}:{option['line']}"]
              for name in sorted(route_names) for option in desc["muxes"][name]["sources"]]
    patterns = []
    for name in sorted([f"SLICE{s}.CEMUX" for s in "ABCD"] + [f"{kind}{i}" for kind in ("MUXCLK", "MUXLSR") for i in range(4)]):
        options = {o["value"]: set(bits(o["bits"])) for o in fields[name]["options"]} if name in fields else {
            o["source"]: set(bits(o["bits"])) for o in desc["muxes"][name]["sources"]}
        positions = sorted({(f, b) for group in options.values() for f, b, _ in group})
        for mask in range(1 << len(positions)):
            state = {position: bool(mask >> i & 1) for i, position in enumerate(positions)}
            found = [key for key, group in options.items() if matches(state, group)]
            specific = [key for key in found if not any(options[key] < options[other] for other in found)]
            patterns.append([name, mask, packed([[f,b,state[f,b]] for f,b in positions]), packed(found), packed(specific),
                             "raw_encoding_only;empty_multiple_or_partial_match_not_silicon_legality"])

    overlap = []
    all_shared = read_csv(base / "shared_bits.csv")
    owners = {"enum:" + name for name in fields} | {"route:" + name for name in route_names}
    for row in all_shared:
        if set(json.loads(row["owners"])) & owners:
            overlap.append([row["local_frame"], row["local_bit"], row["owners"]])
    output_profiles = []
    for z in range(8):
        name = f"SLICE{'ABCD'[z//2]}.CCU2.INJECT1_{z%2}"
        options = fields[name]["options"]
        positions = {(f,b) for o in options for f,b,_ in bits(o["bits"])}
        if len(positions) != 1: raise ValueError("carry injection changed")
        f,b = next(iter(positions))
        route_options = desc["muxes"][f"F{z}"]["sources"]
        for value in (False, True):
            state = {(f,b): value}
            inject = [o["value"] for o in options if matches(state, bits(o["bits"]))]
            found = [o for o in route_options if matches(state, bits(o["bits"]))]
            specific = [o["source"] for o in found if not any(set(bits(o["bits"])) < set(bits(other["bits"])) for other in found)]
            if len(inject) != 1 or len(specific) != 1: raise ValueError("ambiguous normal output code")
            output_profiles.append([f"F{z}", f,b,int(value),specific[0],name,inject[0],
                                    "LOGIC_route_and_CCU2_injection_are_alternative_mode_contexts;physical_qualification_open_S04"])

    catalogue = verify_native(database, DEVICE, {"PLC2": desc})
    import pytrellis
    bitdb = pytrellis.get_tile_bitdata(pytrellis.TileLocator("ECP5", DEVICE, "PLC2"))
    native_bits = lambda group: sorted((b.frame,b.bit,not b.inv) for b in group.bits)
    for name, value, _, group, _ in enums:
        if native_bits(bitdb.get_data_for_enum(name).options[value]) != [tuple(x) for x in json.loads(group)]: raise ValueError("native enum bits mismatch")
    for sink, source, group, _ in routes:
        if native_bits(bitdb.get_mux_data_for_sink(sink).arcs[source].bits) != [tuple(x) for x in json.loads(group)]: raise ValueError("native route bits mismatch")

    # Globalise the carry exit at every PLC. A canonical wire on a non-PLC coordinate
    # is recorded as unresolved termination, not assigned an invented input level.
    plcs = read_csv(base / "instances.csv")
    positions = {(int(p["row"]),int(p["col"])): p for p in plcs}
    requests = "".join(f"{p['row']} {p['col']} E1_HFIE0000\n" for p in plcs)
    native = subprocess.run([str(args.wire_probe), str(database)], input=requests, text=True, capture_output=True, check=True)
    endpoints = native.stdout.splitlines()
    if len(endpoints) != len(plcs): raise ValueError("incomplete carry endpoints")
    links = []
    for p, endpoint in zip(plcs,endpoints):
        if endpoint == "invalid":
            links.append([p["resource_id"],p["tile"],"FCO_SLICE -> FCO", "invalid", "", "", "", "outside_native_wire_domain"])
            continue
        row, col, wire = endpoint.split("\t"); destination = positions.get((int(row),int(col)))
        links.append([p["resource_id"],p["tile"],"FCO_SLICE -> FCO",row,col,wire,
                      destination["resource_id"] if destination else "",
                      "next_PLC_FCI_candidate;MODE_required" if destination else "no_PLC_BEL_at_endpoint;termination_open_S04"])
    scratch = args.probe_directory.resolve()
    scratch.mkdir(parents=True,exist_ok=True)
    (scratch / "carry-wire-requests.txt").write_text(requests)
    (scratch / "carry-wire-endpoints.tsv").write_text(native.stdout)
    (scratch / "carry-wire-probe.log").write_text(native.stderr)
    ff, startup, ff_obs, ccu, ccu_obs, synth, extra_sources = probes(scratch,args.yosys_library,args.yosys.resolve(),args.iverilog.resolve(),args.vvp.resolve())
    truth = []
    for inject in ("NO", "YES"):
        for p in (0,1):
            for g in (0,1):
                for c in (0,1):
                    truth.append([inject,p,g,c,p ^ (c if inject == "NO" else 0),c if p else (g if inject == "NO" else 0),
                                  "abstract_half_functions;p_and_g_share_INIT;public_model_not_hardware"])

    # Preserve selected diagnostic observations with their exact variant IDs.
    variant_id = next(r[0] for r in ff if r[1:8] == ["0","CLK","LSR","ASYNC","RESET","PRLD","ENABLED"])
    diagnostic = [[row[1],variant_id,row[8],"ASYNC_PRLD_model_samples_M_on_LSR_or_clock_events;S02"]
                  for row in ff_obs if row[7] == variant_id and row[1] in ("assert_lsr","held_lsr_M_change","held_lsr_clock")]
    if [r[2] for r in diagnostic] != ["0","0","1"]: raise ValueError("preload model sampling changed")
    ccu_results = {(row[0],row[1]):row[2:] for row in ccu_obs}
    adapter_rows = []
    for kind, init0, init1, inj0, inj1 in (("feed_in","0x000a","0xffff","NO","YES"),
                                         ("feed_out_loopback","0x0000","0x000a","NO","NO")):
        variant = next(row[0] for row in ccu if row[1:5] == [init0,init1,inj0,inj1])
        for cin in (0,1):
            for data in ((0,1) if kind == "feed_in" else (cin,)):
                vector = (14 | data) | (15 << 4) | (cin << 8) if kind == "feed_in" else 15 | ((14 | cin) << 4) | (cin << 8)
                s0,s1,cout = ccu_results[variant,vector]
                if cout != data or (kind == "feed_out_loopback" and s0 != cin): raise ValueError("carry adapter identity changed")
                adapter_rows.append([kind,variant,vector,cin,data,s0,s1,cout,"nextpnr_source_pattern;unconnected_C_D_tied_high;public_model_identity;not_P_and_R"])

    sources = [("collector",Path(__file__)),("probe_helper",ROOT / "tools/logic_state_probes.py"),("contract",contract_path),
               ("inventory_helpers",ROOT / "tools/inventory_trellis.py"),("PLC_helpers",ROOT / "tools/collect_plc_evidence.py"),
               ("inventory",inventory_path),("PLC2_database",database / desc["source"]),("wire_probe",args.wire_probe),
               ("wire_probe_source",ROOT / "tools/trellis_wire_probe.cpp"),("pytrellis",Path(pytrellis.__file__))]
    lib_path = next(line.split("=",1)[1] for line in native.stderr.splitlines() if line.startswith("libtrellis_path="))
    audited_hashes = {entry["source"]:entry["sha256"] for entry in previous_report["source_files"]}
    for name,path in (("libtrellis",Path(lib_path)),("pytrellis",Path(pytrellis.__file__)),
                      ("libtrellis/src/Bels.cpp",args.trellis/"libtrellis/src/Bels.cpp")):
        if sha256(path) != audited_hashes[name]: raise ValueError("PLC native interface audit needs regeneration")
    sources.append(("libtrellis",Path(lib_path)))
    sources += [(name,scratch/name) for name in ("carry-wire-requests.txt","carry-wire-endpoints.tsv","carry-wire-probe.log")]
    for name in ("native_ports.csv","instances.csv","shared_bits.csv","evidence_report.json"):
        sources.append(("PLC_audit:"+name,base/name))
    for name in ("common_sim.vh","cells_map_trellis.v","cells_ff.vh","latches_map.v","ccu2c_sim.vh","arith_map_ccu2c.v"):
        sources.append(("yosys:"+name,args.yosys_library/name))
    for name in ("pack.cc","bitstream.cc","arch_place.cc","cells.cc"):
        sources.append(("nextpnr:"+name,args.nextpnr/"ecp5"/name))
    for name in ("005-reg_config","007-plc2_cemux","008-plc2_clkmux","009-plc2_lsr","011-ccu2_inject"):
        sources.append(("fuzzer:"+name,args.trellis/"fuzzers/ECP5"/name/"fuzzer.py"))
    sources += [(name,ROOT/entry["path"]) for name,entry in contract["sources"].items()] + extra_sources
    sources += [(name,getattr(args,name).resolve()) for name in ("yosys","iverilog","vvp")]
    report = dict(schema_version=1,device=DEVICE,scope=contract["scope"],
                  reused_native_interface_audit=dict(PLC_count=len(plcs),FF_count=len(plcs)*8,port_templates=48,source="plc_research/evidence_report.json"),
                  configuration=dict(fields=len(fields),enum_options=len(enums),route_options=len(routes),native_bit_groups_checked=len(enums)+len(routes),
                                     raw_control_patterns=len(patterns),shared_bit_positions=len(overlap),output_context_rows=len(output_profiles)),
                  carry_exit_count=len(links),carry_exit_status=dict(sorted(Counter(r[-1] for r in links).items())),
                  FF_model_variants=len(ff),FF_startup_observations=len(startup),FF_transition_observations=len(ff_obs)-len(startup),
                  FF_GSR_parameter_trace_pairs=len(ff)//2,CCU2_model_variants=len(ccu),CCU2_input_vectors_per_variant=512,CCU2_output_triples=len(ccu_obs),
                  carry_adapter_observations=len(adapter_rows),
                  synthesis_cases=len(synth),synthesis_status=dict(sorted(Counter(row[1] for row in synth).items())),
                  yosys_version=run_version(args.yosys),hardware="not_run",runtime_changed=False,native_catalogue=catalogue,
                  source_files=[dict(source=name,sha256=sha256(path)) for name,path in sources],open_questions=contract["open_questions"])
    output = args.output or folder / "logic_state_research";output.mkdir(parents=True,exist_ok=True)
    tables = {
        "FF_templates.csv": (["native_bel","native_z","ports","individual_fields","shared_fields","clock_selector","reset_selector","evidence"],ff_templates),
        "enum_templates.csv": (["field","value","database_default","local_constraints","source"],enums),
        "routing_templates.csv": (["sink","source","local_constraints","evidence"],routes),
        "control_patterns.csv": (["field_or_mux","mask","local_state","matching","most_specific","interpretation"],patterns),
        "shared_bits.csv": (["local_frame","local_bit","owners"],overlap),
        "output_context.csv": (["sink","local_frame","local_bit","CRAM_value","LOGIC_route","CCU2_field","CCU2_value","interpretation"],output_profiles),
        "carry_exits.csv": (["PLC_id","tile","source_path","destination_row","destination_col","destination_wire","destination_PLC_id","status"],links),
        "FF_models.csv": (["variant","CEMUX","CLKMUX","LSRMUX","SRMODE","REGSET","LSRMODE","GSR","transition_observations","evidence"],ff),
        "FF_startup.csv": (["variant","parameters","observed_Q","interpretation"],startup),
        "FF_observations.csv": (["step","event","CLK","LSR","CE","DI","M","variant","Q","evidence"],ff_obs),
        "preload_diagnostic.csv": (["event","variant","observed_Q","interpretation"],diagnostic),
        "CCU2_half_truth.csv": (["INJECT1","p_LUT4","g_LUT2","carry_in","sum","carry_out","evidence"],truth),
        "CCU2_models.csv": (["variant","INIT0","INIT1","INJECT1_0","INJECT1_1","vectors","evidence"],ccu),
        "CCU2_observations.csv": (["variant","input_vector_ABCD0_ABCD1_CIN_lsb0","S0","S1","COUT"],ccu_obs),
        "carry_adapters.csv": (["kind","variant","input_vector","CIN","fabric_input_or_loopback","S0","S1","COUT","evidence"],adapter_rows),
        "synthesis_probes.csv": (["case","status","cell_counts","FF_parameters","netlist_connections","evidence"],synth),
    }
    for name,(columns,rows) in tables.items(): write_csv(output/name,columns,rows)
    for name,value in (("logic_state_contract.json",contract),("evidence_report.json",report)):
        (output/name).write_text(json.dumps(value,ensure_ascii=False,indent=2)+"\n")
    print(packed({k:v for k,v in report.items() if k not in ("source_files","open_questions","native_catalogue")}))


def run_version(command):
    return subprocess.run([str(command.resolve()),"-V"],text=True,capture_output=True,check=True).stdout.strip()


if __name__ == "__main__": main()
