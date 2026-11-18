#!/usr/bin/env python3
"""Collect DQS/DDRDLL contracts and configuration evidence, not runtime models."""
import argparse
from collections import Counter, defaultdict, deque
import csv
import io as textio
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import re
import subprocess

from collect_dsp_routing import endpoint, requirements
from inventory_trellis import sha256, write_csv, verify_native
from resource_audit import packed

ROOT = Path(__file__).resolve().parents[1]
DEVICE = "LFE5U-85F"


def read_csv(path):
    with path.open(newline="") as stream:
        return list(csv.DictReader(stream))


def node(port):
    return int(port["wire_row"]), int(port["wire_col"]), port["wire"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", type=Path, default=ROOT / "docs/inventory" / DEVICE)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--database", type=Path, help="Database root (ECP5_DB or TRELLIS_ROOT)")
    parser.add_argument("--trellis", type=Path, help="Source checkout (TRELLIS_ROOT or parent of ECP5_DB)")
    parser.add_argument("--nextpnr", type=Path, help="Source checkout (NEXTPNR_ROOT)")
    parser.add_argument("--yosys-library", type=Path, default=Path("/opt/homebrew/share/yosys/lattice"))
    parser.add_argument("--native-probe", type=Path)
    parser.add_argument("--verify-pytrellis", action="store_true")
    args = resolve_toolchain(parser, parser.parse_args())
    folder = args.inventory
    inventory = json.loads((folder / "device_inventory.json").read_text())
    if inventory["device"] != DEVICE:
        raise ValueError("collector is scoped to LFE5U-85F")
    database = args.database
    for source in inventory["source_files"]:
        if sha256(database / source["path"]) != source["sha256"]:
            raise ValueError(f"stale inventory: {source['path']}")
    for source in inventory["reference_source_files"]:
        if sha256(args.trellis / source["path"]) != source["sha256"]:
            raise ValueError(f"changed Trellis source: {source['path']}")
    resources = {r["resource_id"]: r for r in read_csv(folder / "resource_instances.csv")}
    core = {key: r for key, r in resources.items() if r["family"] in {"DQSBUFM", "DDRDLL"}}
    consumers = {key: r for key, r in resources.items() if r["family"] in {"DQSBUFM", "DLLDELD"}}
    ports = {(p["resource_id"], p["port"]): p for p in read_csv(folder / "clock_ports.csv")}
    catalog = json.loads((folder / "decode_catalog.json").read_text())
    catalog["resources"] = [r for r in catalog["resources"] if r["id"] in core]
    fields = {r["id"]: r["fields"] for r in catalog["resources"]}
    contract = json.loads((ROOT / "tools/data/dqs_control_contract.json").read_text())

    # Configuration masks are evidence about encoding, not a hardware legality table.
    masks, word_layout = [], []
    for key, values in sorted(fields.items()):
        for field in values:
            if field["kind"] == "word":
                for fragment in field["fragments"]:
                    for index, bits in enumerate(fragment["word_bits"]):
                        word_layout.append([key, field["name"], index, packed(bits), fragment["source"]])
                continue
            if not field["name"].endswith((".MODE", ".FORCE_MAX_DELAY")):
                continue
            options = defaultdict(set)
            for fragment in field["fragments"]:
                for option, bits in fragment["options"].items():
                    options[option].update(tuple(b) for b in bits)
            positions = sorted({bit[:2] for values in options.values() for bit in values})
            for mask in range(1 << len(positions)):
                state = {pos: bool(mask & (1 << index)) for index, pos in enumerate(positions)}
                matches = sorted(option for option, bits in options.items() if all(state[b[:2]] == b[2] for b in bits))
                kind = "all_field_bits_clear" if mask == 0 else "all_field_bits_set" if mask == (1 << len(positions)) - 1 else "mixed_pattern"
                masks.append([key, field["name"], packed([[*pos, state[pos]] for pos in positions]), packed(matches), kind,
                              "write_patterns_only;interpret_with_mode_context"])

    blackboxes = (args.yosys_library / "cells_bb_ecp5.v").read_text()
    writer = (args.nextpnr / "ecp5/bitstream.cc").read_text()
    dqs_start = writer.index("void write_dqsbuf(")
    dll_start = writer.index("else if (ci->type == id_DDRDLL)")
    writer_sections = {"DQSBUFM": writer[dqs_start:writer.index("void run(", dqs_start)],
                       "DDRDLL": writer[dll_start:writer.index("} else {", dll_start)]}
    modules = {name: (header, body) for name, header, body in re.findall(r"module\s+(\w+)\((.*?)\);(.*?)endmodule", blackboxes, re.S)}
    interfaces, parameters = [], []
    module_names = {"DDRDLL": "DDRDLLA", "DQSBUFM": "DQSBUFM"}
    for family, module in module_names.items():
        _, body = modules[module]
        documented = {name.strip(): direction for direction, names in re.findall(r"\b(input|output)\s+([^;]+);", body)
                      for name in names.split(",")}
        prefix = "DDRDLL." if family == "DDRDLL" else "DQS."
        representative = next(key for key, r in core.items() if r["family"] == family)
        field_map = {f["name"]: f for f in fields[representative]}
        fallbacks = dict(re.findall(r'(?:str|int)_or_default\(ci->params,\s*id_(\w+),\s*("[^"]*"|\d+)\)', writer_sections[family]))
        for param, default in re.findall(r"parameter\s+(\w+)\s*=\s*([^;]+);", body):
            name = prefix + param
            field = field_map.get(name)
            enum_options = sorted({opt for frag in field["fragments"] for opt in frag.get("options", {})}) if field else []
            default_state = "present" if default.strip().strip('"') in enum_options else "not_named_option" if enum_options else "word_or_unmapped"
            parameters.append([family, param, default.strip(), fallbacks.get(param, ""), name if field else "", packed(enum_options),
                               packed(sorted({frag["default"] for frag in field["fragments"]})) if field else "[]",
                               default_state, "blackbox_declaration;not_behaviour"])
        for (key, pin), port in sorted(ports.items()):
            if key not in core or core[key]["family"] != family:
                continue
            expected = {"input": "in", "output": "out"}.get(documented.get(pin))
            if expected is not None and expected != port["direction"]:
                raise ValueError(f"public/native direction differs: {key}:{pin}")
            interfaces.append([key, module, pin, port["direction"], *node(port),
                               "public_and_native" if expected else "native_only;function_unresolved"])
        native_names = {pin for (key, pin) in ports if key == representative}
        if set(documented) - native_names:
            raise ValueError(f"public pins absent from native interface: {module}")
    parameters += [["DDRDLL", "LOCK_CYC", "unspecified", "", "", "[]", "[]", "not_in_Yosys", "library_p251;simulation_only;no_cram_expected"],
                   ["DQSBUFM", "WRITE_LEVELING", '"0T"', "", "", '["0T","1T"]', "[]", "not_in_Yosys", "library_p295;hardware_mapping_unresolved"]]
    # Preserve the writer's actual eight-bit conversion, including MINUS 256 -> 0.
    for side in ("li", "lo"):
        if f"(256 - {side}_del_value) & 0xFF" not in writer:
            raise ValueError("nextpnr delay conversion changed; review the collector")
    encodings = [[adjustment, magnitude, magnitude if adjustment == "PLUS" else (256 - magnitude) & 255,
                  "both_DQS_LI_DEL_VAL_and_DQS_LO_DEL_VAL", "nextpnr_writer_encoding;range_disputed_in_library"]
                 for adjustment, values in (("PLUS", range(256)), ("MINUS", range(1, 257))) for magnitude in values]

    descriptions, templates, edges, requests = {}, set(), [], {}
    for tile_type, desc in inventory["tile_types"].items():
        values = [("fixed", c["source"], c["sink"], (), c["line"]) for c in desc["fixed_connections"]]
        values += [("mux", opt["source"], sink, tuple(opt["bits"]), opt["line"])
                   for sink, mux in desc["muxes"].items() for opt in mux["sources"]]
        if any(any("DDRDEL" in n or n.endswith(("_DQS", "_DDRDLL", "DQSECLK")) for n in value[1:3]) for value in values):
            descriptions[tile_type] = values
    def resolve(tile, name):
        row, col = int(tile["fabric_row"]), int(tile["fabric_col"])
        request = row, col, name
        if request not in requests:
            requests[request] = endpoint(row, col, name, inventory["device_info"]["max_row"], inventory["device_info"]["max_col"])
        return requests[request]
    tiles = read_csv(folder / "tile_instances.csv")
    for tile in tiles:
        desc = inventory["tile_types"][tile["type"]]
        for kind, raw_source, raw_sink, tokens, line in descriptions.get(tile["type"], []):
            source, sink = resolve(tile, raw_source), resolve(tile, raw_sink)
            bits = requirements(tokens, tile)
            reference = f"{desc['source']}:{line}"
            templates.add((tile["type"], kind, raw_source, raw_sink, packed(tokens), reference))
            if source is not None and sink is not None:
                edges.append((source, sink, kind, bits, reference, tile["tile"]))
    edges.sort()
    up, down = defaultdict(list), defaultdict(list)
    for edge in edges:
        up[edge[1]].append(edge)
        down[edge[0]].append(edge)
    def evidence(path):
        return [dict(source=e[0], sink=e[1], kind=e[2], bits=e[3], reference=e[4], tile=e[5]) for e in path]
    def find_path(source, sink, limit=3):
        queue, visited = deque([(source, ())]), {source}
        while queue:
            here, path = queue.popleft()
            if here == sink:
                return path
            if len(path) == limit:
                continue
            for edge in down[here]:
                if edge[1] not in visited:
                    visited.add(edge[1]); queue.append((edge[1], path + (edge,)))
        return None
    delay_paths, clock_inputs, route_bits = [], [], set()
    route_uses = defaultdict(set)
    for target in sorted(consumers):
        sink = node(ports[target, "DDRDEL"])
        count = 0
        for source in sorted(key for key, r in core.items() if r["family"] == "DDRDLL"):
            path = find_path(node(ports[source, "DDRDEL"]), sink)
            if path is None:
                continue
            bits = sorted({bit for edge in path for bit in edge[3]})
            route_bits.update(bit[:2] for bit in bits)
            for bit in bits: route_uses[bit[:2]].add((target, "delay_source_select"))
            delay_paths.append([source, target, consumers[target]["family"], packed(bits), packed(evidence(path)),
                                "source_selection_only;DDRDEL_transport_and_gate_semantics_unresolved"])
            count += 1
        if count != 2:
            raise ValueError(f"expected two DDRDLL sources: {target}:{count}")
    for key, resource in sorted(core.items()):
        if resource["family"] != "DDRDLL":
            continue
        native_clock = node(ports[key, "CLK"])
        tail = up[native_clock]
        if len(tail) != 1:
            raise ValueError(f"unexpected DLL reference-clock boundary: {key}")
        middle = tail[0][0]
        options = [edge for edge in up[middle] if edge[2] == "mux"]
        if len(options) != 3:
            raise ValueError(f"expected three DLL reference clock options: {key}")
        for edge in options:
            route_bits.update(bit[:2] for bit in edge[3])
            for bit in edge[3]: route_uses[bit[:2]].add((key, "reference_clock_select"))
            clock_inputs.append([key, packed(edge[0]), packed(edge[3]), packed(evidence([edge, tail[0]]))])

    named_owners = defaultdict(set)
    for tile in tiles:
        sf, sb = int(tile["start_frame"]), int(tile["start_bit"])
        if not any(sf <= f < sf + int(tile["frame_count"]) and sb <= b < sb + int(tile["bit_count"]) for f, b in route_bits):
            continue
        desc = inventory["tile_types"][tile["type"]]
        for category in ("enums", "words"):
            for name, field in desc[category].items():
                groups = [o["bits"] for o in field["options"]] if category == "enums" else field["bit_groups"]
                for token in {t for group in groups for t in group if t != "-"}:
                    frame, bit = map(int, re.fullmatch(r"!?F(\d+)B(\d+)", token).groups())
                    position = sf + frame, sb + bit
                    if position in route_bits:
                        named_owners[position].add(f"{tile['tile']}/{category}/{name}")
    prior_bits = {tuple(bit[:2]) for row in read_csv(folder / "io_routing_research/route_controls.csv") for bit in json.loads(row["selection_bits"])}
    selector_rows = [[*position, packed(sorted(route_uses[position])), packed(sorted(named_owners.get(position, ()))), position in prior_bits]
                     for position in sorted(route_bits)]

    native = dict(status="not_run")
    extras = []
    if args.native_probe:
        ordered = sorted(requests)
        result = subprocess.run([str(args.native_probe.resolve()), str(database)],
            input="".join(f"{r} {c} {n}\n" for r, c, n in ordered), capture_output=True, text=True, check=True)
        lines = result.stdout.splitlines()
        if len(lines) != len(ordered):
            raise ValueError("native normalization response length differs")
        for request, line in zip(ordered, lines):
            values = line.split("\t")
            actual = None if line == "invalid" else (int(values[0]), int(values[1]), values[2])
            if actual != requests[request]:
                raise ValueError(f"native normalization differs: {request}")
        library = next(line.split("=", 1)[1] for line in result.stderr.splitlines() if line.startswith("libtrellis_path="))
        native = dict(status="matched", requests=len(ordered), scope="wire_names_and_coordinates_only")
        extras += [("native_probe_binary", args.native_probe.resolve()), ("native_library", Path(library))]
    native_bits = dict(status="not_run")
    if args.verify_pytrellis:
        checks = verify_native(database, DEVICE, {t: inventory["tile_types"][t] for t in descriptions})
        import pytrellis
        compared = 0
        for tile_type, values in descriptions.items():
            data = pytrellis.get_tile_bitdata(pytrellis.TileLocator("ECP5", DEVICE, tile_type))
            for kind, source, sink, tokens, _ in values:
                if kind != "mux": continue
                actual = {(b.frame, b.bit, not b.inv) for b in data.get_mux_data_for_sink(sink).arcs[source].bits.bits}
                expected = set()
                for token in tokens:
                    if token != "-":
                        inv, frame, bit = re.fullmatch(r"(!?)F(\d+)B(\d+)", token).groups()
                        expected.add((int(frame), int(bit), not inv))
                if actual != expected:
                    raise ValueError(f"native mux pattern differs: {tile_type}:{sink}:{source}")
                compared += 1
        native_bits = dict(status="matched", patterns=compared, catalogue=checks)
        extras.append(("pytrellis_module", Path(pytrellis.__file__)))
    instances = read_csv(folder / "iologic_research/instances.csv")
    fifo_views = [[r["resource_id"], r["dqs_resource"], 8, 3, 3, "unresolved", "unresolved", "memory_input_mode_requirement;not_independent_BEL"]
                  for r in instances if r["family"] == "IOLOGIC"]
    sources = [("collector", Path(__file__)), ("contract", ROOT / "tools/data/dqs_control_contract.json"),
               ("normalization_rules", ROOT / "tools/collect_dsp_routing.py"), ("native_check", ROOT / "tools/inventory_trellis.py"),
               ("native_probe_source", ROOT / "tools/trellis_wire_probe.cpp"), ("Yosys_blackboxes", args.yosys_library / "cells_bb_ecp5.v"),
               ("nextpnr_writer", args.nextpnr / "ecp5/bitstream.cc"), ("nextpnr_packer", args.nextpnr / "ecp5/pack.cc"),
               ("DQS_fuzzer", args.trellis / "fuzzers/ECP5/130-dqsbuf/fuzzer.py"),
               ("DDRDLL_fuzzer", args.trellis / "fuzzers/ECP5/133-ddrdll/fuzzer.py")]
    sources += [(name, folder / name) for name in ("device_inventory.json", "resource_instances.csv", "clock_ports.csv", "decode_catalog.json", "tile_instances.csv", "iologic_research/instances.csv", "io_routing_research/route_controls.csv")]
    sources += [(key, ROOT / info["path"]) for key, info in contract["sources"].items()]
    sources += extras
    report = dict(schema_version=1, device=DEVICE, core_resources=dict(Counter(r["family"] for r in core.values())),
                  core_ports=len(interfaces), native_only_ports=sum(r[-1].startswith("native_only") for r in interfaces),
                  whole_fields=sum(len(v) for v in fields.values()), fragments=sum(len(f["fragments"]) for v in fields.values() for f in v),
                  mode_masks=sum(row[1].endswith(".MODE") for row in masks), force_max_delay_masks=sum(row[1].endswith(".FORCE_MAX_DELAY") for row in masks),
                  word_bit_positions=len(word_layout), delay_conversion_rows=len(encodings), control_rules=len(contract["rules"]),
                  fifo_views=len(fifo_views), delay_source_paths=len(delay_paths), reference_clock_options=len(clock_inputs),
                  routing_control_bits=len(route_bits), routing_tile_types=len(descriptions), routing_edge_records=len(edges),
                  routing_bits_in_prior_IO_catalogue=len(route_bits & prior_bits), routing_bits_in_named_fields=sum(bool(values) for values in named_owners.values()),
                  native_wire_check=native, native_mux_bits_check=native_bits,
                  source_files=[dict(source=name, sha256=sha256(path)) for name, path in sources],
                  limitations=contract["open_questions"])
    output = args.output or folder / "dqs_research"
    output.mkdir(parents=True, exist_ok=True)
    write_csv(output / "interfaces.csv", ["resource_id", "public_primitive", "port", "direction", "wire_row", "wire_col", "wire", "status"], interfaces)
    write_csv(output / "parameter_coverage.csv", ["family", "parameter", "public_default", "writer_fallback", "named_field", "enum_options", "fragment_defaults", "declared_default_status", "evidence_status"], parameters)
    write_csv(output / "configuration_masks.csv", ["resource_id", "field", "cram_values", "raw_matching_options", "mask_class", "interpretation"], masks)
    write_csv(output / "delay_word_layout.csv", ["resource_id", "field", "logical_bit", "cram_bits", "reference"], word_layout)
    write_csv(output / "delay_parameter_encoding.csv", ["adjustment", "magnitude", "encoded_byte", "applies_to", "status"], encodings)
    write_csv(output / "delay_source_paths.csv", ["source_ddrdll", "target_resource", "target_family", "selection_bits", "path", "interpretation"], delay_paths)
    write_csv(output / "reference_clock_inputs.csv", ["ddrdll_resource", "source_wire", "selection_bits", "path"], clock_inputs)
    write_csv(output / "fifo_views.csv", ["iologic_resource", "dqs_resource", "depth", "read_pointer_bits", "write_pointer_bits", "pointer_encoding", "word_width", "status"], fifo_views)
    write_csv(output / "connection_templates.csv", ["tile_type", "edge_kind", "source", "sink", "local_bits", "reference"], sorted(templates))
    write_csv(output / "routing_selector_bits.csv", ["global_frame", "global_bit", "resource_roles", "named_field_owners", "in_prior_IO_catalogue"], selector_rows)
    (output / "config_catalogue.json").write_text(packed(catalog) + "\n")
    (output / "control_contract.json").write_text(json.dumps(contract, ensure_ascii=False, indent=2) + "\n")
    (output / "evidence_report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(packed({key: value for key, value in report.items() if key not in {"source_files", "limitations"}}))


if __name__ == "__main__":
    main()
