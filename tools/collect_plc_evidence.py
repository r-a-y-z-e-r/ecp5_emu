#!/usr/bin/env python3
"""Collect PLC/LUTRAM structure, configuration templates and external-model evidence."""
import argparse
from collections import Counter, defaultdict
import csv
import io
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import re
import shutil
import subprocess

from inventory_trellis import sha256, write_csv, verify_native
from plc_research_probes import probes
from resource_audit import packed

ROOT = Path(__file__).resolve().parents[1]
DEVICE = "LFE5U-85F"


def read_csv(path):
    with path.open(newline="") as f: return list(csv.DictReader(f))


def bits(tokens):
    result = []
    for token in tokens:
        if token == "-": continue
        match = re.fullmatch(r"(!?)F(\d+)B(\d+)", token)
        if not match: raise ValueError(f"bad bit token: {token}")
        result.append((int(match[2]),int(match[3]),not bool(match[1])))
    return sorted(result)


def swap_address(address):
    return ((address & 1) << 3) | (address & 6) | ((address & 8) >> 3)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory",type=Path,default=ROOT/"docs/inventory"/DEVICE)
    parser.add_argument("--output",type=Path)
    parser.add_argument("--trellis",type=Path, help="Source checkout (TRELLIS_ROOT or parent of ECP5_DB)")
    parser.add_argument("--nextpnr",type=Path, help="Source checkout (NEXTPNR_ROOT)")
    parser.add_argument("--yosys-library",type=Path,default=Path("/opt/homebrew/share/yosys/lattice"))
    parser.add_argument("--native-probe",type=Path,default=ROOT/"target/trellis_plc_probe")
    parser.add_argument("--probe-directory",type=Path,default=ROOT/"target/research/plc/probes")
    for command in ("yosys","iverilog","vvp"):
        parser.add_argument(f"--{command}",type=Path,default=Path(shutil.which(command) or command))
    parser.add_argument("--database", type=Path, help="Database root (ECP5_DB or TRELLIS_ROOT)")
    args = resolve_toolchain(parser, parser.parse_args());folder=args.inventory
    inventory_path=folder/"device_inventory.json";inventory=json.loads(inventory_path.read_text())
    if inventory["device"] != DEVICE: raise ValueError("collector is scoped to LFE5U-85F")
    database=args.database
    for source in inventory["source_files"]:
        if sha256(database/source["path"]) != source["sha256"]: raise ValueError(f"stale database: {source['path']}")
    for source in inventory["reference_source_files"]:
        if sha256(args.trellis/source["path"]) != source["sha256"]: raise ValueError(f"changed Trellis source: {source['path']}")
    desc=inventory["tile_types"]["PLC2"]
    tiles=[t for t in read_csv(folder/"tile_instances.csv") if t["type"]=="PLC2"]
    resources=read_csv(folder/"resource_instances.csv")
    plcs={r["anchor_tile"]:r for r in resources if r["family"]=="PLC2"}
    slices=[r for r in resources if r["family"]=="SLICE"]
    if set(plcs) != {t["tile"] for t in tiles} or len(slices)!=4*len(tiles): raise ValueError("PLC/slice instance mismatch")
    contract_path=ROOT/"tools/data/plc_contract.json";contract=json.loads(contract_path.read_text())
    question_ids={q['id'] for q in contract['open_questions']}
    if {q for case in contract['hardware_cases'] for q in case['questions']} != question_ids:
        raise ValueError("hardware descriptions do not reference all PLC questions")
    scratch=args.probe_directory.resolve();scratch.mkdir(parents=True,exist_ok=True)
    requests="".join(f"{r} {c}\n" for r,c in sorted((int(t["fabric_row"]),int(t["fabric_col"])) for t in tiles))
    (scratch/"native-requests.txt").write_text(requests)
    # Use stdin for the complete native-constructor coverage request.
    native=subprocess.run([str(args.native_probe.resolve()),str(database)],input=requests,text=True,capture_output=True)
    if native.returncode: raise RuntimeError(native.stderr)
    if f"verified_plcs={len(tiles)}" not in native.stderr: raise ValueError("incomplete native PLC coverage")
    (scratch/"native-ports.tsv").write_text(native.stdout);(scratch/"native-probe.log").write_text(native.stderr)
    ports=list(csv.DictReader(io.StringIO(native.stdout),delimiter="\t"))
    node_keys={(p['view'],p['bel'],p['bel_type'],int(p['z'])) for p in ports}
    if len(node_keys)!=21 or len(ports)!=300: raise ValueError("native PLC interface changed")
    if Counter(p["view"] for p in ports)!=Counter(split=176,coarse=124): raise ValueError("native PLC view mismatch")
    nodes=[[view,name,kind,z,"alternative_coarse_view" if view=="coarse" else "C_combinational_overlap" if kind=="TRELLIS_RAMW" else "split_view"] for view,name,kind,z in sorted(node_keys)]
    instances=[]
    for t in tiles:
        instances.append([plcs[t['tile']]['resource_id'],t['tile'],t['fabric_row'],t['fabric_col'],t['start_frame'],t['start_bit'],t['frame_count'],t['bit_count'],int(t['start_frame'])+50,int(t['start_bit'])+11,"matched_native_template"])

    enums,words,owners,all_positions=[],[],defaultdict(set),set()
    for name,enum in desc["enums"].items():
        for option in enum["options"]:
            constraints=bits(option["bits"])
            enums.append([name,option["value"],enum["default"],packed(constraints),f"{desc['source']}:{option['line']}"])
            for frame,bit,_ in constraints: owners[frame,bit].add("enum:"+name);all_positions.add((frame,bit))
    for name,word in desc["words"].items():
        for index,group in enumerate(word["bit_groups"]):
            constraints=bits(group)
            words.append([name,index,packed(constraints),word["default"],f"{desc['source']}:{word['line']}"])
            for frame,bit,_ in constraints: owners[frame,bit].add("word:"+name);all_positions.add((frame,bit))
    routes=[]
    for c in desc["fixed_connections"]:routes.append(["fixed",c["source"],c["sink"],"[]",f"{desc['source']}:{c['line']}"])
    for sink,mux in desc["muxes"].items():
        for option in mux["sources"]:
            constraints=bits(option["bits"])
            routes.append(["mux",option["source"],sink,packed(constraints),f"{desc['source']}:{option['line']}"])
            for frame,bit,_ in constraints:owners[frame,bit].add("route:"+sink);all_positions.add((frame,bit))
    for t in tiles:
        if any(f>=int(t['frame_count']) or b>=int(t['bit_count']) for f,b in all_positions):raise ValueError("template outside PLC window")
    shared=[[f,b,packed(sorted(names))] for (f,b),names in sorted(owners.items()) if len(names)>1]
    mode_fields={name:{o['value']:set(bits(o['bits'])) for o in e['options']} for name,e in desc['enums'].items() if name.endswith('.MODE')}
    positions=sorted({(f,b) for options in mode_fields.values() for group in options.values() for f,b,_ in group})
    mode_patterns=[]
    for mask in range(1<<len(positions)):
        state={pos:bool(mask>>i&1) for i,pos in enumerate(positions)}
        matched={};specific={}
        for name,options in mode_fields.items():
            found=[opt for opt,group in options.items() if all(state[f,b]==v for f,b,v in group)]
            matched[name]=found
            specific[name]=[opt for opt in found if not any(options[opt] < options[other] for other in found)]
        mode_patterns.append([mask,packed([[*pos,state[pos]] for pos in positions]),packed(matched),packed(specific),"encoding_observation;not_silicon_legality"])
    wire_groups=defaultdict(list)
    for p in ports:
        if p['view']=='split':wire_groups[p['wire']].append([p['bel'],p['port'],p['direction']])
    wire_aliases=[[wire,packed(pins)] for wire,pins in sorted(wire_groups.items()) if len(pins)>1]
    mappings=[["RAMW_input",r["logical"],"SLICEC.RAMW",r["native"]] for r in contract["ramw_inputs"]]
    for lane in range(4):
        bel=f"SLICE{'AB'[lane//2]}.K{lane%2}"
        mappings += [["read_address",r['logical'],bel,r['native']] for r in contract['ram_read_inputs']]
        mappings.append(["write_data",f"WDO{lane}",bel,"WD"])
    init_mapping=[]
    word_lookup={name:{index:constraints for n,index,constraints,_,_ in words if n==name} for name in desc['words']}
    for address in range(16):
        physical=swap_address(address)
        for lane in range(4):
            name=f"SLICE{'AB'[lane//2]}.K{lane%2}.INIT"
            init_mapping.append([address,lane,4*address+lane,name,physical,word_lookup[name][physical],"nextpnr_source_correspondence;hardware_L08"])

    # Verify the actual bit groups, beyond the catalogue name/default check.
    native_catalogue=verify_native(database,DEVICE,{"PLC2":desc})
    import pytrellis
    bitdb=pytrellis.get_tile_bitdata(pytrellis.TileLocator("ECP5",DEVICE,"PLC2"))
    normalized=lambda group:sorted((b.frame,b.bit,not b.inv) for b in group.bits)
    checked=0
    for name,index,constraints,_,_ in words:
        if normalized(bitdb.get_data_for_setword(name).bits[index]) != [tuple(b) for b in json.loads(constraints)]: raise ValueError("native INIT bits differ")
        checked+=1
    for name,value,_,constraints,_ in enums:
        if normalized(bitdb.get_data_for_enum(name).options[value]) != [tuple(b) for b in json.loads(constraints)]:raise ValueError("native enum bits differ")
        checked+=1
    for kind,source,sink,constraints,_ in routes:
        if kind=='mux':
            if normalized(bitdb.get_mux_data_for_sink(sink).arcs[source].bits) != [tuple(b) for b in json.loads(constraints)]:raise ValueError("native mux bits differ")
            checked+=1

    synthesis,models,initial,limitations,extra=probes(scratch,args.yosys_library,args.yosys.resolve(),args.iverilog.resolve(),args.vvp.resolve())
    lib=[line.split('=',1)[1] for line in native.stderr.splitlines() if line.startswith('libtrellis_path=')]
    if len(lib)!=1:raise ValueError("cannot identify native library")
    sources=[("collector",Path(__file__)),("probe_helper",ROOT/"tools/plc_research_probes.py"),("contract",contract_path),
             ("inventory_helpers",ROOT/"tools/inventory_trellis.py"),("json_helper",ROOT/"tools/resource_audit.py"),
             ("native_probe_source",ROOT/"tools/trellis_plc_probe.cpp"),("native_probe",args.native_probe),
             ("libtrellis",Path(lib[0])),("pytrellis",Path(pytrellis.__file__)),("inventory",inventory_path),
             ("tile_instances",folder/"tile_instances.csv"),("resources",folder/"resource_instances.csv"),
             ("PLC2_database",database/desc['source'])]
    sources += [(name,scratch/name) for name in ('native-requests.txt','native-ports.tsv','native-probe.log')]
    sources += [(name,args.nextpnr/"ecp5"/name) for name in ('cells.cc','pack.cc','bitstream.cc','arch_place.cc','arch.h')]
    sources += [(name,args.trellis/name) for name in ('libtrellis/src/Bels.cpp','libtrellis/src/Chip.cpp','libtrellis/src/BitDatabase.cpp','fuzzers/ECP5/003-lut_init/fuzzer.py','fuzzers/ECP5/010-plc2_modes/fuzzer.py')]
    sources += [(name,args.yosys_library/name) for name in ('common_sim.vh','lutrams_trellis.txt','lutrams_map_trellis.v')]
    sources += [(name,ROOT/info['path']) for name,info in contract['sources'].items()]
    sources += [(name,getattr(args,name).resolve()) for name in ('yosys','iverilog','vvp')]
    sources += extra
    yosys_version=subprocess.run([str(args.yosys.resolve()),'-V'],text=True,capture_output=True,check=True).stdout.strip()
    report=dict(schema_version=1,device=DEVICE,yosys_version=yosys_version,plcs=len(tiles),slices=len(slices),lut4=len(tiles)*8,fabric_ff=len(tiles)*8,
                native_ports=dict(template=len(ports),split_per_PLC=176,coarse_per_PLC=124,compared=len(tiles)*len(ports),all_PLCs_matched=True),
                configuration=dict(enum_fields=len(desc['enums']),enum_options=len(enums),word_fields=len(desc['words']),INIT_bit_positions=len(words),mux_descriptors=len(desc['muxes']),mux_options=sum(r[0]=='mux' for r in routes),fixed_connections=len(desc['fixed_connections']),shared_bit_positions=len(shared),mode_bit_positions=len(positions),mode_patterns=len(mode_patterns),native_bit_groups_checked=checked),
                native_catalogue=native_catalogue,synthesis_cases=len(synthesis),software_model_cases=len(models),software_word_observations=sum(row[4] for row in models),
                model_limitation_observations=len(limitations),hardware='not_run',
                source_files=[dict(source=name,sha256=sha256(path)) for name,path in sources],open_questions=contract['open_questions'])
    output=args.output or folder/"plc_research";output.mkdir(parents=True,exist_ok=True)
    write_csv(output/'instances.csv',['resource_id','tile','row','col','start_frame','start_bit','frame_count','bit_count','RAM_mode_frame','RAM_mode_bit','native_status'],instances)
    write_csv(output/'slices.csv',list(slices[0]),[[r[key] for key in slices[0]] for r in slices])
    write_csv(output/'native_ports.csv',list(ports[0]),[[r[key] for key in ports[0]] for r in ports])
    write_csv(output/'node_templates.csv',['view','native_name','native_type','z','interpretation'],nodes)
    write_csv(output/'enum_templates.csv',['field','value','default','local_constraints','source'],enums)
    write_csv(output/'INIT_layout.csv',['field','logical_bit_lsb0','local_constraints','default','source'],words)
    write_csv(output/'routing_templates.csv',['kind','source_wire','sink_wire','local_constraints','source'],routes)
    write_csv(output/'shared_bits.csv',['local_frame','local_bit','owners'],shared)
    write_csv(output/'mode_patterns.csv',['mask','local_state','matching_patterns','most_specific_patterns','interpretation'],mode_patterns)
    write_csv(output/'shared_wires.csv',['local_wire','native_pins'],wire_aliases)
    write_csv(output/'RAMW_mapping.csv',['kind','logical_signal','native_bel','native_port'],mappings)
    write_csv(output/'RAM_INIT_mapping.csv',['logical_address','data_lane','logical_INITVAL_bit','physical_INIT_field','physical_LUT_index','local_constraints','evidence'],init_mapping)
    write_csv(output/'synthesis_probes.csv',['case','width','depth','write_edge','LUTRAM_cells','all_cells','evidence'],synthesis)
    write_csv(output/'software_models.csv',['model','variant','WCKMUX','WREMUX','word_observations','evidence'],models)
    write_csv(output/'model_INIT_observations.csv',['model','variant','address','requested_INIT_value','observed_hex','evidence'],initial)
    write_csv(output/'model_limitations.csv',['issue','stimulus','observed','interpretation'],limitations)
    for name,value in (('plc_contract.json',contract),('evidence_report.json',report)):
        (output/name).write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n')
    print(packed({k:v for k,v in report.items() if k not in ('source_files','open_questions')}))


if __name__=='__main__':main()
