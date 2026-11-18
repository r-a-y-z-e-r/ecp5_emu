#!/usr/bin/env python3
"""Collect EBR addressing and mode evidence without implementing memory behavior."""
import argparse
import hashlib
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import re
import shutil
import subprocess

from collect_dqs_evidence import read_csv
from inventory_trellis import sha256, write_csv
from resource_audit import packed

ROOT = Path(__file__).resolve().parents[1]
DEVICE = "LFE5U-85F"


def storage_bit(width, address, bit):
    if width <= 4:
        per_word = 8 // width
        return address // per_word, (address % per_word) * width + bit
    return address * (width // 9) + bit // 9, bit % 9


def fingerprint(address, width):
    return ((address * 0x5A3987B) ^ (address >> 3) ^ 0x55AA55AA55) & ((1 << width) - 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", type=Path, default=ROOT / "docs/inventory" / DEVICE)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--database", type=Path, help="Database root (ECP5_DB or TRELLIS_ROOT)")
    parser.add_argument("--nextpnr", type=Path, help="Source checkout (NEXTPNR_ROOT)")
    parser.add_argument("--trellis", type=Path, help="Source checkout (TRELLIS_ROOT or parent of ECP5_DB)")
    parser.add_argument("--yosys-library", type=Path, default=Path("/opt/homebrew/share/yosys/lattice"))
    parser.add_argument("--verify-yosys", action="store_true")
    parser.add_argument("--yosys", type=Path, default=Path(shutil.which("yosys") or "yosys"))
    parser.add_argument("--probe-directory", type=Path, default=ROOT / "target/research/ebr/probes")
    args = resolve_toolchain(parser, parser.parse_args())
    folder = args.inventory
    inventory = json.loads((folder / "device_inventory.json").read_text())
    if inventory["device"] != DEVICE: raise ValueError("collector is scoped to LFE5U-85F")
    database = args.database
    for source in inventory["source_files"]:
        if sha256(database / source["path"]) != source["sha256"]: raise ValueError(f"stale database: {source['path']}")
    for source in inventory["reference_source_files"]:
        if sha256(args.trellis / source["path"]) != source["sha256"]: raise ValueError(f"changed source: {source['path']}")
    contract = json.loads((ROOT / "tools/data/ebr_contract.json").read_text())
    profiles = contract["width_profiles"]
    catalog = json.loads((folder / "decode_catalog.json").read_text())
    ebrs = [r for r in catalog["resources"] if r["family"] == "EBR"]
    resources = {r["resource_id"]: r for r in read_csv(folder / "resource_instances.csv")}
    field_rows, instances = [], []
    for resource in ebrs:
        key = resource["id"]
        tiles = sorted({f["tile"] for field in resource["fields"] for f in field["fragments"]})
        instances.append([key, resources[key]["anchor_tile"], len(resource["fields"]), packed(tiles), "shared_2048x9_INIT_view"])
        for field in resource["fields"]:
            field_rows.append([key, field["name"], field["kind"], packed(field["fragments"])])

    views, lanes, roles, support, overlaps = [], [], [], [], []
    for profile in profiles:
        width, depth, shift = profile["width"], profile["depth"], profile["address_shift"]
        views.append([width, depth, shift, packed(list(range(shift, 14))), profile["visible_bits"],
                      "DP_and_PDP_read" if width != 36 else "PDP_only", "INIT_word_layout;PDP36_output_pin_order_unresolved" if width == 36 else "verified_by_local_synthesis_when_enabled"])
        for address in range(32 // (1 << shift)):
            for bit in range(width):
                word, index = storage_bit(width, address, bit)
                lanes.append([width, address, bit, word, index, "repeat_with_INIT_word_offset_4_and_address_offset_32>>shift"])
        configurations = [("PDPW16KD", "read_B", width)]
        if width != 36:
            configurations += [("DP16KD", "A", width), ("DP16KD", "B", width)]
        else:
            configurations.append(("PDPW16KD", "write_A", width))
        for mode, port, _ in configurations:
            for pin in range(14):
                if pin >= shift:
                    role, index = "word_address", str(pin - shift)
                elif port == "write_A" and pin < 4:
                    role, index = "nine_bit_write_enable", str(pin)
                elif mode == "DP16KD" and width == 18 and pin < 2:
                    role, index = "nine_bit_write_enable", str(pin)
                else:
                    role, index = "not_word_address_in_this_width", ""
                roles.append([mode, port, width, f"AD{'A' if port in ('A', 'write_A') else 'B'}{pin}", role, index])
        if width != 36:
            for write_mode in ("NORMAL", "WRITETHROUGH", "READBEFOREWRITE"):
                allowed = write_mode != "READBEFOREWRITE" or width in (9, 18)
                support.append(["DP16KD", width, write_mode, "documented" if allowed else "outside_documented_width_support",
                                "enum_present_independently_of_width", "Yosys_old_data_allowed_for_all_widths" if write_mode == "READBEFOREWRITE" else "Yosys_and_documentation_agree"])
    for pa in profiles[:-1]:
        for pb in profiles[:-1]:
            wa, wb = pa["width"], pb["width"]
            for aa in range(32 >> pa["address_shift"]):
                a = {storage_bit(wa, aa, b) for b in range(wa)}
                for ab in range(32 >> pb["address_shift"]):
                    common = a & {storage_bit(wb, ab, b) for b in range(wb)}
                    if common:
                        overlaps.append([wa, aa, wb, ab, packed(sorted(common)), "bit_view_overlap;not_a_complete_physical_collision_rule"])

    # Keep the public PDP primitive's lowering separate from direct Yosys DP lowering.
    packer = (args.nextpnr / "ecp5/pack.cc").read_text()
    begin = packer.index("if (ci->type == id_PDPW16KD)")
    section = packer[begin:packer.index("ci->type = id_DP16KD;", begin)]
    lowering = []
    for old, new, width, old_base, new_base in re.findall(r'rename_bus\(ci,\s*"(\w+)",\s*"(\w+)",\s*(\d+),\s*(\d+),\s*(\d+)\)', section):
        lowering += [[f"{old}{i+int(old_base)}", f"{new}{i+int(new_base)}", "nextpnr_public_PDP_lowering"] for i in range(int(width))]
    lowering += [[old, new, "nextpnr_public_PDP_lowering"] for old, new in re.findall(r"ci->renamePort\(id_(\w+), id_(\w+)\)", section)]
    lowering += [["RST", "RSTA", "duplicated_reset"], ["RST", "RSTB", "duplicated_reset"]]
    public = (args.yosys_library / "cells_bb_ecp5.v").read_text()
    pdp_header = re.search(r"module PDPW16KD\((.*?)\);", public, re.S)[1]
    public_ports = {p.strip() for p in pdp_header.split(",")}
    if {r[0] for r in lowering} != public_ports: raise ValueError("incomplete public PDP port lowering")
    mapper = (args.yosys_library / "brams_map_16kd.v").read_text()
    pdp_mapper = mapper[mapper.index("module $__PDPW16KD_"):]
    yosys_outputs = {int(index): pin for pin, index in re.findall(r"\.(DO[AB]\d+)\(DO\[(\d+)\]\)", pdp_mapper)}
    if set(yosys_outputs) != set(range(36)): raise ValueError("incomplete direct Yosys PDP output wiring")
    next_outputs = {old: new for old, new, _ in lowering if old.startswith("DO")}
    comparisons = [[width, bit, yosys_outputs[bit if width == 36 else bit + 18], next_outputs[f"DO{bit}"],
                    "same_pin" if yosys_outputs[bit if width == 36 else bit + 18] == next_outputs[f"DO{bit}"] else "different_lowering_paths;silicon_order_unresolved"]
                   for width in [1, 2, 4, 9, 18, 36] for bit in range(width)]
    init_layout = [[chunk, word, bit, chunk * 32 + word * 2 + bit // 9, bit % 9, word * 20 + bit]
                   for chunk in range(64) for word in range(16) for bit in range(18)]
    cs_rows = [[value, inversion, int(f"{value ^ inversion:03b}"[::-1], 2), value ^ inversion]
               for value in range(8) for inversion in range(8)]

    probes, pin_rows, extra_sources = [], [], []
    probe_summary = dict(status="not_run")
    if args.verify_yosys:
        binary = args.yosys.resolve()
        version = subprocess.run([str(binary), "-V"], capture_output=True, text=True, check=True).stdout.strip()
        scratch = args.probe_directory.resolve(); scratch.mkdir(parents=True, exist_ok=True)
        for profile in profiles:
            width, depth = profile["width"], profile["depth"]
            address_bits = (depth - 1).bit_length()
            name = f"{'pdp' if width == 36 else 'dp'}{width}"
            if width == 36:
                body = "always @(posedge ca) if(wa) mem[aa]<=da;\nalways @(posedge cb) if(wb) qb<=mem[ab];\nassign qa=0;"
            else:
                body = "always @(posedge ca) if(wa) mem[aa]<=da; else qa<=mem[aa];\nalways @(posedge cb) if(wb) mem[ab]<=db; else qb<=mem[ab];"
            source = f"""module top(input ca,cb,wa,wb, input [{address_bits-1}:0] aa,ab, input [{width-1}:0] da,db, output {'wire' if width == 36 else 'reg'} [{width-1}:0] qa, output reg [{width-1}:0] qb);
reg [{width-1}:0] mem [0:{depth-1}];
integer i;
initial for(i=0;i<{depth};i=i+1) mem[i]=((i*64'h5A3987B)^(i>>3)^64'h55AA55AA55);
{body}
endmodule
"""
            src = scratch / f"{name}.v"; src.write_text(source)
            result = subprocess.run([str(binary), "-Q", "-T", "-q", "-p", f"read_verilog {name}.v; synth_ecp5 -top top -json {name}.json", "-l", f"{name}.log"],
                                    cwd=scratch, capture_output=True, text=True, check=True)
            netlist_path = scratch / f"{name}.json"
            module = json.loads(netlist_path.read_text())["modules"]["top"]
            cells = [cell for cell in module["cells"].values() if cell["type"] == "DP16KD"]
            if len(cells) != 1: raise ValueError(f"probe {name} does not map to one EBR")
            cell = cells[0]
            top_bits = {bit: f"{port}[{index}]" for port, data in module["ports"].items() for index, bit in enumerate(data["bits"]) if isinstance(bit, int)}
            for pin, bits in sorted(cell["connections"].items()):
                pin_rows.append([name, pin, packed([top_bits.get(bit, bit) for bit in bits])])
            matched_addresses = {}
            for native in ("A", "B"):
                # Synthesis may exchange the symmetric A and B ports.
                address = [cell["connections"][f"AD{native}{i}"][0] for i in range(profile["address_shift"], 14)]
                matches = [p for p in ("aa", "ab") if address == module["ports"][p]["bits"]]
                if len(matches) != 1:
                    raise ValueError(f"wrong address shift in probe {name}:{native}")
                matched_addresses[native] = matches[0]
                if width == 18:
                    for pin in (0, 1):
                        if cell["connections"][f"AD{native}{pin}"] != cell["connections"][f"WE{native}"]:
                            raise ValueError(f"wrong nine-bit write enable in probe {name}:{native}")
            if set(matched_addresses.values()) != {"aa", "ab"}:
                raise ValueError(f"the two address domains collapsed in probe {name}")
            if width == 36:
                if matched_addresses != {"A": "aa", "B": "ab"}:
                    raise ValueError("PDP write/read address domains exchanged")
                for pin in range(4):
                    if cell["connections"][f"ADA{pin}"] != module["ports"]["wa"]["bits"]:
                        raise ValueError("wrong PDP nine-bit write enable")
            for native in ("A", "B"):
                enables = 4 if width == 36 and native == "A" else 2 if width == 18 else 0
                for pin in range(enables, profile["address_shift"]):
                    if cell["connections"][f"AD{native}{pin}"] != ["0"]:
                        raise ValueError(f"unexpected low address contact in probe {name}:{native}{pin}")
            observed = []
            for chunk in range(64):
                value = int(cell["parameters"][f"INITVAL_{chunk:02X}"], 2)
                for j in range(16):
                    data = (value >> (20 * j)) & ((1 << 18) - 1)
                    observed.extend([data & 511, data >> 9])
            expected = [0] * 2048
            for address in range(depth):
                value = fingerprint(address, width)
                for bit in range(width):
                    word, offset = storage_bit(width, address, bit)
                    expected[word] |= ((value >> bit) & 1) << offset
            if observed != expected: raise ValueError(f"INIT layout differs in probe {name}")
            if width == 36:
                for bit in range(36):
                    if cell["connections"][yosys_outputs[bit]][0] != module["ports"]["qb"]["bits"][bit]:
                        raise ValueError("expanded PDP36 output differs from extracted Yosys wiring")
            payload = b"".join(word.to_bytes(2, "little") for word in observed)
            probes.append([name, width, depth, len(cells), hashlib.sha256(payload).hexdigest(), "addresses_and_all_INIT_words_matched;not_hardware_behavior"])
            extra_sources += [(f"probe_source:{name}", src), (f"probe_netlist:{name}", netlist_path)]
        probe_summary = dict(status="matched", cases=len(probes), version=version, init_words_compared=len(probes)*2048)
        extra_sources.append(("yosys_binary", binary))

    sources = [("collector", Path(__file__)), ("contract", ROOT / "tools/data/ebr_contract.json"),
               ("csv_reader", ROOT / "tools/collect_dqs_evidence.py"), ("inventory_helpers", ROOT / "tools/inventory_trellis.py"),
               ("json_helper", ROOT / "tools/resource_audit.py"),
               ("nextpnr_pack", args.nextpnr / "ecp5/pack.cc"), ("nextpnr_writer", args.nextpnr / "ecp5/bitstream.cc"),
               ("nextpnr_clock_domains", args.nextpnr / "ecp5/arch.cc"),
               ("EBR_fuzzer", args.trellis / "fuzzers/ECP5/041-ebr_config/fuzzer.py")]
    sources += [(name, args.yosys_library / name) for name in ("brams_map_16kd.v", "brams_16kd.txt", "cells_sim_ecp5.v", "cells_bb_ecp5.v")]
    sources += [(name, folder / name) for name in ("device_inventory.json", "decode_catalog.json", "resource_instances.csv")]
    sources += [(key, ROOT / info["path"]) for key, info in contract["sources"].items()]
    sources += extra_sources
    report = dict(schema_version=1, device=DEVICE, ebr_instances=len(ebrs), addressing_profiles=len(views), lane_template_bits=len(lanes),
                  configuration_fields=len(field_rows), configuration_fragments=sum(len(f["fragments"]) for r in ebrs for f in r["fields"]),
                  address_pin_roles=len(roles), write_mode_rows=len(support),
                  documented_mode_rows=sum(row[3] == "documented" for row in support), overlap_examples=len(overlaps),
                  public_PDP_ports=len(public_ports), PDP_lowering_connections=len(lowering), output_comparison_rows=len(comparisons),
                  differing_PDP36_output_bits=sum(row[-1] != "same_pin" for row in comparisons),
                  init_payload_positions=len(init_layout), csdecode_transformations=len(cs_rows), yosys_probe=probe_summary,
                  source_files=[dict(source=name, sha256=sha256(path)) for name, path in sources], limitations=contract["open_questions"])
    output = args.output or folder / "ebr_research"; output.mkdir(parents=True, exist_ok=True)
    write_csv(output / "instances.csv", ["resource_id", "anchor_tile", "field_count", "configuration_tiles", "storage_view"], instances)
    write_csv(output / "field_fragments.csv", ["resource_id", "field", "kind", "fragments"], field_rows)
    write_csv(output / "address_views.csv", ["width", "depth", "address_shift", "address_contacts", "visible_bits", "mode_scope", "evidence_status"], views)
    write_csv(output / "lane_templates.csv", ["width", "word_address_in_block", "data_bit", "INIT_word9_in_block", "bit_in_word9", "repetition"], lanes)
    write_csv(output / "address_pin_roles.csv", ["mode", "port", "width", "native_pin", "role", "index"], roles)
    write_csv(output / "write_modes.csv", ["mode", "width", "write_mode", "documented_support", "Trellis_availability", "tool_status"], support)
    write_csv(output / "mixed_width_overlaps.csv", ["width_A", "address_A", "width_B", "address_B", "shared_INIT_bits", "interpretation"], overlaps)
    write_csv(output / "PDP_lowering.csv", ["public_port", "native_port", "source"], sorted(lowering))
    write_csv(output / "PDP_output_comparison.csv", ["read_width", "logical_output_bit", "direct_Yosys_DP_pin", "nextpnr_public_PDP_pin", "status"], comparisons)
    write_csv(output / "INITVAL_layout.csv", ["parameter_index", "word18_index", "data_bit", "INIT_word9", "bit_in_word9", "INITVAL_bit"], init_layout)
    write_csv(output / "CSDECODE_encoding.csv", ["public_decode", "input_inversion_mask", "Trellis_word_value", "raw_pin_match_value"], cs_rows)
    write_csv(output / "yosys_probes.csv", ["probe", "width", "depth", "ebr_cells", "INIT_view_sha256", "status"], probes)
    write_csv(output / "yosys_pin_connections.csv", ["probe", "native_pin", "top_level_bits_or_constants"], pin_rows)
    (output / "memory_contract.json").write_text(json.dumps(contract, ensure_ascii=False, indent=2) + "\n")
    (output / "evidence_report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(packed({key:value for key,value in report.items() if key not in {"source_files", "limitations"}}))


if __name__ == "__main__": main()
