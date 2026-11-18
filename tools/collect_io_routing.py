#!/usr/bin/env python3
"""Trace IO boundary paths and retain mode-dependent/raw dedicated-net views."""
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

    # Include every record of any used type that names IO or dedicated bank/lane nets.
    # General fabric traversal and combinational paths through BELs are excluded.
    records = {}
    for kind, desc in inventory["tile_types"].items():
        values = [("fixed", c["source"], c["sink"], (), c["line"]) for c in desc["fixed_connections"]]
        values += [("mux", option["source"], sink, tuple(option["bits"]), option["line"])
                   for sink, mux in desc["muxes"].items() for option in mux["sources"]]
        if any(any("IOLOGIC" in name or name.endswith("_PIO") or "DQSG_" in name or "BNK_ECLK" in name
                   for name in (value[1], value[2])) for value in values):
            records[kind] = values
    tiles = read_csv(folder / "tile_instances.csv")
    requests, edges, templates = {}, [], set()

    def resolve(tile, name):
        row, col = int(tile["fabric_row"]), int(tile["fabric_col"])
        key = row, col, name
        if key not in requests:
            requests[key] = endpoint(row, col, name, inventory["device_info"]["max_row"], inventory["device_info"]["max_col"])
        return requests[key]

    for tile in tiles:
        desc = inventory["tile_types"][tile["type"]]
        for kind, raw_source, raw_sink, tokens, line in records.get(tile["type"], []):
            source, sink = resolve(tile, raw_source), resolve(tile, raw_sink)
            constraints = requirements(tokens, tile)
            reference = f"{desc['source']}:{line}"
            templates.add((tile["type"], kind, raw_source, raw_sink, packed(tokens), reference))
            if source is not None and sink is not None:
                edges.append((source, sink, kind, constraints, reference, tile["tile"]))
    edges.sort()
    up, down = defaultdict(list), defaultdict(list)
    for edge in edges:
        up[edge[1]].append(edge)
        down[edge[0]].append(edge)

    def evidence(path):
        return [dict(source=e[0], sink=e[1], kind=e[2], bits=e[3], reference=e[4], tile=e[5]) for e in path]

    def find_path(source, sink, limit=3):
        queue = deque([(source, ())])
        visited = {source}
        while queue:
            here, path = queue.popleft()
            if here == sink:
                return path
            if len(path) == limit:
                continue
            for edge in down[here]:
                if edge[1] not in visited:
                    visited.add(edge[1])
                    queue.append((edge[1], path + (edge,)))
        return None

    def named_upstream(sink, basename, limit=3):
        queue = deque([(sink, ())])
        visited, found = {sink}, []
        while queue:
            here, path = queue.popleft()
            if here[2] == basename:
                found.append((here, path))
                continue
            if len(path) == limit:
                continue
            for edge in up[here]:
                if edge[0] not in visited:
                    visited.add(edge[0])
                    queue.append((edge[0], (edge,) + path))
        return found

    native = dict(status="not_run", scope="wire-name normalization; not a full RoutingGraph")
    native_bits = dict(status="not_run")
    extra_sources = []
    if args.verify_pytrellis:
        checks = verify_native(database, DEVICE, {key: inventory["tile_types"][key] for key in records})
        import pytrellis
        count = 0
        for tile_type, values in records.items():
            data = pytrellis.get_tile_bitdata(pytrellis.TileLocator("ECP5", DEVICE, tile_type))
            for kind, source, sink, tokens, _ in values:
                if kind != "mux":
                    continue
                actual = {(bit.frame, bit.bit, not bit.inv) for bit in data.get_mux_data_for_sink(sink).arcs[source].bits.bits}
                expected = set()
                for token in tokens:
                    if token != "-":
                        inverted, frame, bit = re.fullmatch(r"(!?)F(\d+)B(\d+)", token).groups()
                        expected.add((int(frame), int(bit), not inverted))
                if actual != expected:
                    raise ValueError(f"native mux pattern differs: {tile_type}:{source}->{sink}")
                count += 1
        native_bits = dict(status="matched", mux_option_patterns=count, catalogue_checks=checks)
        extra_sources.append(("pytrellis_module", Path(pytrellis.__file__)))
    if args.native_probe:
        ordered = sorted(requests)
        result = subprocess.run([str(args.native_probe.resolve()), str(database)],
            input="".join(f"{r} {c} {name}\n" for r, c, name in ordered), text=True, capture_output=True, check=True)
        lines = result.stdout.splitlines()
        if len(lines) != len(ordered):
            raise ValueError("native response length differs")
        for request, actual in zip(ordered, lines):
            values = actual.split("\t")
            value = None if actual == "invalid" else (int(values[0]), int(values[1]), values[2])
            if requests[request] != value:
                raise ValueError(f"native normalization mismatch: {request}")
        library_lines = [line.split("=", 1)[1] for line in result.stderr.splitlines() if line.startswith("libtrellis_path=")]
        if len(library_lines) != 1:
            raise ValueError("probe did not identify native library")
        native.update(status="matched", requests=len(ordered))
        extra_sources += [("native_probe_binary", args.native_probe.resolve()), ("native_library", Path(library_lines[0]))]

    ports = read_csv(folder / "iologic_research/resource_ports.csv")
    portmap = {(p["resource_id"], p["port"]): p for p in ports}
    native_outputs, native_inputs = defaultdict(list), defaultdict(list)
    adjacency = []
    for p in ports:
        location = node(p)
        (native_outputs if p["direction"] == "out" else native_inputs)[location].append([p["resource_id"], p["port"]])
        expected = up[location] if p["direction"] == "in" else down[location]
        adjacency.append([p["resource_id"], p["port"], p["direction"], *location,
                          sum(e[2] == "fixed" for e in up[location]), sum(e[2] == "mux" for e in up[location]),
                          len(down[location]), "connected_in_expected_direction" if expected else "no_edge_in_expected_direction"])
    instances = read_csv(folder / "iologic_research/instances.csv")
    config = {r["id"]: r["fields"] for r in json.loads((folder / "iologic_research/config_catalogue.json").read_text())["resources"]}
    bit_owners = defaultdict(set)
    for path, frame_name, bit_name in ((folder / "config_field_bits.csv", "cram_frame", "cram_bit"),
                                      (folder / "iologic_research/config_field_bits.csv", "global_frame", "global_bit")):
        for bit in read_csv(path):
            bit_owners[int(bit[frame_name]), int(bit[bit_name])].add(bit["field_id"])

    delay_rows, output_rows, route_controls = [], [], []
    for inst in instances:
        key, family = inst["resource_id"], inst["family"]
        letter = key[-1]
        di, delayed = node(portmap[key, "DI"]), node(portmap[key, "INDD"])
        raw = node(portmap[inst["pio_resource"], "O"])
        selection = (di[0], di[1], f"JDI{letter}")
        delayed_edges = [e for e in up[selection] if e[0] == delayed and e[2] == "mux"]
        direct_edges = [e for e in up[selection] if e[0] == raw]
        tail = [e for e in up[di] if e[0] == selection and e[2] == "fixed"]
        if len(delayed_edges) != 1 or len(direct_edges) != 1 or len(tail) != 1:
            raise ValueError(f"unexpected input bypass topology: {key}")
        delay = delayed_edges[0]
        if len(delay[3]) != 1:
            raise ValueError(f"delay selector does not have one bit: {key}")
        delay_rows.append([key, packed(raw), packed(delayed), packed(selection), packed(di),
                           packed(evidence(direct_edges + tail)), packed(evidence(delayed_edges + tail)),
                           direct_edges[0][2], packed(delay[3]), "mode_interpretation_required"])
        route_controls.append([key, "input_delay_select", packed(delay[3]),
                               packed(sorted(set().union(*(bit_owners[f, b] for f, b, _ in delay[3])))), delay[4]])
        paths = {}
        for name, start, end in (("direct_output", (key, "IOLDO"), (inst["pio_resource"], "IOLDO")),
                                 ("delayed_output", (key, "IOLDOD"), (inst["pio_resource"], "IOLDO")),
                                 ("output_feedback", (key, "IOLDO"), (key, "IOLDOI")),
                                 ("tristate", (key, "IOLTO"), (inst["pio_resource"], "IOLTO"))):
            path = find_path(node(portmap[start]), node(portmap[end]))
            if path is None:
                raise ValueError(f"missing IO boundary path: {key}:{name}")
            paths[name] = evidence(path)
        controls = [[resource, field["name"]] for resource in (key, inst["pio_resource"])
                    for field in config.get(resource, []) if field["name"].split(".", 1)[1] in
                    {"MODE", "DELAY.OUTDEL", "IOLTOMUX", "DATAMUX_OREG", "DATAMUX_ODDR", "DATAMUX_MDDR", "TRIMUX_TSREG"}]
        output_rows.append([key, inst["pio_resource"], packed(paths), packed(controls), "conditional_interpretation_required"])

    shared_outputs, shared_inputs, mixed = [], [], []
    for sink, incoming in sorted(up.items()):
        fixed = [e for e in incoming if e[2] == "fixed"]
        mux = [e for e in incoming if e[2] == "mux"]
        output_sources = [(e, pin) for e in fixed for pin in native_outputs[e[0]]]
        if len({tuple(pin) for _, pin in output_sources}) > 1:
            role = "direct_delayed_output_views" if {pin[1] for _, pin in output_sources} == {"IOLDO", "IOLDOD"} else "paired_gearbox_output_views"
            shared_outputs.append([*sink, packed(sorted(pin for _, pin in output_sources)),
                                   packed(evidence([e for e, _ in output_sources])), role])
        if fixed and mux:
            mixed.append([*sink, packed(evidence(fixed)), packed(evidence(mux)), "qualify_fixed_view_before_union"])
    for source, outgoing in sorted(down.items()):
        destinations = [(e, pin) for e in outgoing if e[2] == "fixed" for pin in native_inputs[e[1]]
                        if "/PIO" not in pin[0]]
        if len({tuple(pin) for _, pin in destinations}) > 1:
            role = "paired_gearbox_input_alias" if any(pin[1] in {"TXDATA4", "TXDATA5", "TXDATA6"} for _, pin in destinations) else "dedicated_dqs_fanout"
            shared_inputs.append([*source, packed(sorted(pin for _, pin in destinations)), packed(evidence([e for e, _ in destinations])), role])

    clocks = {(p["resource_id"], p["port"]): p for p in read_csv(folder / "clock_ports.csv")}
    banks = {}
    for (key, pin), port in clocks.items():
        match = re.fullmatch(r"G_BANK(\d+)ECLK([01])", port["wire"])
        if pin == "ECLKO" and match:
            banks[int(match[1]), int(match[2])] = port
    dqs_rows, eclk_rows, clock_targets = [], [], []
    dedicated_roots = defaultdict(set)
    for inst in instances:
        if inst["family"] != "IOLOGIC":
            continue
        key = inst["resource_id"]
        clock_targets.append((key, int(inst["bank"])))
        for pin in ("DQSR90", "DQSW", "DQSW270", "RDPNTR0", "RDPNTR1", "RDPNTR2", "WRPNTR0", "WRPNTR1", "WRPNTR2"):
            source, sink = clocks[inst["dqs_resource"], pin], portmap[key, pin]
            path = find_path(node(source), node(sink))
            aliases = [e[0] for e in up[node(sink)] if e[0][2] == "DQSG_" + pin]
            if len(set(aliases)) != 1:
                raise ValueError(f"expected one DQS lane alias: {key}:{pin}")
            dedicated_roots["DQS", aliases[0]].add(inst["dqs_resource"])
            dqs_rows.append([key, pin, inst["dqs_resource"], inst["dqs_group"], packed(node(source)), packed(node(sink)),
                             "raw_path" if path is not None else "group_relation_without_raw_path", packed(evidence(path or ())),
                             "iodb_group_and_nextpnr_pack_dqsbuf_process_dqs_port"])
    # DQSBUF uses the bank of its lane, with its own ECLK selection field.
    lane_banks = defaultdict(set)
    for inst in instances:
        if inst["dqs_resource"]:
            lane_banks[inst["dqs_resource"]].add(int(inst["bank"]))
    for key, values in sorted(lane_banks.items()):
        if len(values) != 1:
            raise ValueError(f"DQS lane crosses bank assignments: {key}")
        clock_targets.append((key, next(iter(values))))
    for key, bank in clock_targets:
        sink = node((portmap if (key, "ECLK") in portmap else clocks)[key, "ECLK"])
        for index in range(2):
            source = banks[bank, index]
            candidates = named_upstream(sink, f"BNK_ECLK{index}")
            if len(candidates) != 1:
                raise ValueError(f"expected one local edge clock path: {key}:{index}: {len(candidates)}")
            local_bank, path = candidates[0]
            dedicated_roots["ECLK", local_bank].add(source["resource_id"])
            requirements_set = sorted({bit for edge in path for bit in edge[3]})
            full = find_path(node(source), sink)
            eclk_rows.append([key, bank, index, source["resource_id"], packed(node(source)), packed(local_bank),
                              packed(sink), packed(requirements_set), packed(evidence(path)),
                              "raw_path" if full is not None else "bank_relation_without_raw_path",
                              any(e[1] == path[0][1] and e[2] == "fixed" for e in up[path[0][1]]) and index == 1])
            if index == 1:
                if len(requirements_set) != 1:
                    raise ValueError(f"unexpected ECLK1 selector: {key}")
                route_controls.append([key, "edge_clock_select", packed(requirements_set),
                    packed(sorted(set().union(*(bit_owners[f, b] for f, b, _ in requirements_set)))), path[0][4]])
    root_rows = [[role, *location, len(up[location]), packed(sorted(owners)), packed(evidence(up[location]))]
                 for (role, location), owners in sorted(dedicated_roots.items())]
    if any(len(owners) != 1 for owners in dedicated_roots.values()):
        raise ValueError("local dedicated wire has conflicting group/bank assignments")

    # Independently check every selector against *all* named fields in the raw inventory,
    # including PIO/bank settings outside the previously bound research catalogues.
    selector_bits = {tuple(bit[:2]) for row in route_controls for bit in json.loads(row[2])}
    local_fields = defaultdict(list)
    for tile_type, description in inventory["tile_types"].items():
        for category in ("enums", "words"):
            for name, field in description[category].items():
                groups = [o["bits"] for o in field["options"]] if category == "enums" else field["bit_groups"]
                positions = {tuple(map(int, re.fullmatch(r"!?F(\d+)B(\d+)", token).groups()))
                             for group in groups for token in group if token != "-"}
                local_fields[tile_type].extend((frame, bit, category, name) for frame, bit in sorted(positions))
    named_overlaps = []
    for tile in tiles:
        sf, sb = int(tile["start_frame"]), int(tile["start_bit"])
        if not any(sf <= f < sf + int(tile["frame_count"]) and sb <= b < sb + int(tile["bit_count"])
                   for f, b in selector_bits):
            continue
        for frame, bit, category, name in local_fields[tile["type"]]:
            if (sf + frame, sb + bit) in selector_bits:
                named_overlaps.append([sf + frame, sb + bit, tile["tile"], category, name])

    # Record names not exposed as native BEL pins, without inventing new resources.
    known_nodes = {node(p) for p in ports}
    unnamed = [[*location, len(up[location]), len(down[location])]
               for location in sorted(set(up) | set(down))
               if ("IOLOGIC" in location[2] or location[2].endswith("_PIO")) and location not in known_nodes]
    sources = [("collector", Path(__file__)), ("normalization_and_bit_rules", ROOT / "tools/collect_dsp_routing.py"),
               ("native_probe_source", ROOT / "tools/trellis_wire_probe.cpp")]
    sources.append(("native_catalogue_check", ROOT / "tools/inventory_trellis.py"))
    sources += [(name, folder / name) for name in ("device_inventory.json", "tile_instances.csv", "clock_ports.csv", "config_field_bits.csv",
                "iologic_research/instances.csv", "iologic_research/resource_ports.csv", "iologic_research/config_field_bits.csv", "iologic_research/config_catalogue.json")]
    sources += [(f"Trellis:{name}", args.trellis / name) for name in ("libtrellis/src/RoutingGraph.cpp", "libtrellis/src/Bels.cpp")]
    sources += [("nextpnr:ecp5/pack.cc", args.nextpnr / "ecp5/pack.cc")]
    sources += extra_sources
    report = dict(schema_version=1, device=DEVICE, tile_types=len(records), template_records=len(templates),
                  instantiated_edge_records=len(edges), native_wire_check=native,
                  native_mux_bits_check=native_bits,
                  rejected_name_requests=sum(value is None for value in requests.values()),
                  ports_with_directed_adjacency=sum(p[-1] == "connected_in_expected_direction" for p in adjacency),
                  input_delay_paths=len(delay_rows), raw_fixed_input_bypasses=sum(p[7] == "fixed" for p in delay_rows),
                  output_boundary_paths=len(output_rows) * 4,
                  shared_output_sinks=len(shared_outputs), shared_input_terminals=len(shared_inputs), fixed_mux_overlaps=len(mixed),
                  shared_output_roles=dict(sorted(Counter(r[-1] for r in shared_outputs).items())),
                  shared_input_roles=dict(sorted(Counter(r[-1] for r in shared_inputs).items())),
                  route_controls=len(route_controls), route_bits=len({tuple(bit[:2]) for r in route_controls for bit in json.loads(r[2])}),
                  route_controls_without_named_field=sum(r[3] == "[]" for r in route_controls),
                  selector_bits_checked_against_all_named_fields=len(selector_bits),
                  selector_bits_with_any_named_field=len({tuple(row[:2]) for row in named_overlaps}),
                  dqs_links=len(dqs_rows), dqs_link_states=dict(sorted(Counter(r[6] for r in dqs_rows).items())),
                  edge_clock_links=len(eclk_rows), edge_clock_link_states=dict(sorted(Counter(r[9] for r in eclk_rows).items())),
                  edge_clock_targets_by_family=dict(sorted(Counter("DQSBUF" if key in lane_banks else "IOLOGIC" for key, _ in clock_targets).items())),
                  dedicated_aliases_without_raw_driver=dict(sorted(Counter(r[0] for r in root_rows if r[4] == 0).items())),
                  unnamed_primitive_wire_nodes=len(unnamed),
                  limitations=["A fixed record can describe a mode-specific view; sources must not all be unioned.",
                               "Paths do not traverse internal BEL logic or the full general fabric.",
                               "Dedicated bank/lane relations use metadata and nextpnr rules; missing physical segments are not invented.",
                               "Delay and edge-clock selector patterns do not establish all mode compatibility conditions.",
                               "The raw named-field audit includes all inventoried settings; routing mux choices remain separate."],
                  source_files=[dict(source=name, sha256=sha256(path)) for name, path in sources])
    output = args.output or folder / "io_routing_research"
    output.mkdir(parents=True, exist_ok=True)
    write_csv(output / "connection_templates.csv", ["tile_type", "edge_kind", "source", "sink", "local_bits", "reference"], sorted(templates))
    write_csv(output / "port_adjacency.csv", ["resource_id", "port", "direction", "wire_row", "wire_col", "wire", "fixed_inputs", "mux_inputs", "outgoing_edges", "status"], adjacency)
    write_csv(output / "input_delay_paths.csv", ["resource_id", "pad_output", "delay_output", "selection_wire", "logic_DI", "raw_direct_path", "raw_delay_path", "direct_edge_kind", "delay_selection_bits", "interpretation"], delay_rows)
    write_csv(output / "output_paths.csv", ["resource_id", "pio_resource", "paths", "related_control_fields", "interpretation"], output_rows)
    write_csv(output / "route_controls.csv", ["resource_id", "role", "selection_bits", "named_field_owners", "reference"], route_controls)
    write_csv(output / "shared_output_sinks.csv", ["sink_row", "sink_col", "sink_wire", "source_ports", "edges", "interpretation"], shared_outputs)
    write_csv(output / "shared_input_terminals.csv", ["source_row", "source_col", "source_wire", "destination_ports", "edges", "interpretation"], shared_inputs)
    write_csv(output / "fixed_mux_overlaps.csv", ["sink_row", "sink_col", "sink_wire", "fixed_inputs", "mux_inputs", "interpretation"], mixed)
    write_csv(output / "dqs_links.csv", ["resource_id", "port", "dqs_resource", "dqs_group_label", "source_wire", "sink_wire", "connection_status", "raw_path", "relation_evidence"], dqs_rows)
    write_csv(output / "edge_clock_links.csv", ["resource_id", "bank", "clock_index", "buffer_resource", "buffer_output", "local_bank_wire", "sink_wire", "selection_bits", "local_path", "connection_status", "fixed_default_overlaps_clock1"], eclk_rows)
    write_csv(output / "unnamed_primitive_wires.csv", ["wire_row", "wire_col", "wire", "incoming_edges", "outgoing_edges"], unnamed)
    write_csv(output / "selector_named_field_overlaps.csv", ["global_frame", "global_bit", "tile", "kind", "name"], sorted(named_overlaps))
    write_csv(output / "rejected_name_requests.csv", ["tile_row", "tile_col", "raw_name"], sorted(key for key, value in requests.items() if value is None))
    write_csv(output / "dedicated_wire_roots.csv", ["family", "wire_row", "wire_col", "wire", "raw_driver_count", "group_driver_resources", "raw_driver_edges"], root_rows)
    (output / "routing_report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(packed({key: value for key, value in report.items() if key not in ("source_files", "limitations")}))


if __name__ == "__main__":
    main()
