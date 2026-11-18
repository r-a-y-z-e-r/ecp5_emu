#!/usr/bin/env python3
"""Trace DSP wire namespaces and database paths; do not infer executable nets."""
import argparse
from collections import Counter, defaultdict, deque
import csv
import hashlib
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SMALL = ("PRADD9", "PRADD18", "MULT9", "ALU24")
FAMILY = re.compile(r"J(.+)_(PRADD18|PRADD9|MULT18|MULT9|ALU54|ALU24)$")
OFFSET = re.compile(r"([NS]\d+)?([EW]\d+)?_(.*)")


def packed(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def csv_rows(path):
    with path.open() as stream:
        return list(csv.DictReader(stream))


def write_csv(path, header, rows):
    with path.open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(header)
        writer.writerows(rows)


def documented_ports():
    def bus(prefix, width):
        return {prefix + str(i) for i in range(width)}
    controls = set().union(*(bus(prefix, 4) for prefix in ("CLK", "CE", "RST")))
    ports = {}
    for family, width in (("PRADD9", 9), ("PRADD18", 18)):
        inputs = controls | {"SOURCEA", "OPPRE"} | set().union(*(bus(b, width) for b in ("PA", "PB", "C", "SRIA", "SRIB")))
        outputs = set().union(*(bus(b, width) for b in ("PO", "SROA", "SROB")))
        ports[family] = {**dict.fromkeys(inputs, "in"), **dict.fromkeys(outputs, "out")}
    inputs = controls | {"SIGNEDA", "SIGNEDB", "SOURCEA", "SOURCEB"} | set().union(*(bus(b, 9) for b in ("A", "B", "C", "SRIA", "SRIB")))
    outputs = {"SIGNEDP"} | bus("P", 18) | set().union(*(bus(b, 9) for b in ("SROA", "SROB", "ROA", "ROB", "ROC")))
    ports["MULT9"] = {**dict.fromkeys(inputs, "in"), **dict.fromkeys(outputs, "out")}
    inputs = controls | {"SIGNEDIA", "SIGNEDIB", "OPADDNSUB", "OPCINSEL"} | bus("MA", 18) | bus("MB", 18) | bus("CIN", 24) | bus("CFB", 24)
    ports["ALU24"] = {**dict.fromkeys(inputs, "in"), **dict.fromkeys(bus("R", 24) | bus("CO", 24), "out")}
    return ports


def endpoint(row, col, name, max_row, max_col):
    if name.startswith(("25K_", "45K_", "85K_")):
        if not name.startswith("85K_"):
            return None
        name = name[4:]
    if col >= 69:
        name = name.replace("PCSA", "PCSB", 1)
    if name.startswith(("G_", "L_", "R_")):
        if name.startswith("G_") and not any(s in name for s in ("VPTX", "HPBX", "HPRX")):
            row, col = 0, 0
    else:
        match = OFFSET.fullmatch(name)
        if match:
            vertical, horizontal, name = match.groups()
            if vertical:
                row += int(vertical[1:]) * (1 if vertical[0] == "S" else -1)
            if horizontal:
                col += int(horizontal[1:]) * (1 if horizontal[0] == "E" else -1)
        if not (0 <= row <= max_row and 0 <= col <= max_col):
            return None
    return row, col, name


def requirements(tokens, tile):
    bits = set()
    for token in tokens:
        if token == "-":
            continue
        match = re.fullmatch(r"(!?)F(\d+)B(\d+)", token)
        if not match:
            raise ValueError(f"invalid bit token: {token}")
        inverted, frame, bit = match.groups()
        if int(frame) >= int(tile["frame_count"]) or int(bit) >= int(tile["bit_count"]):
            raise ValueError("mux bit outside owning tile")
        bits.add((int(tile["start_frame"]) + int(frame), int(tile["start_bit"]) + int(bit), not inverted))
    return tuple(sorted(bits))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", type=Path, default=ROOT / "docs/inventory/LFE5U-85F")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--database", type=Path, help="Database root (ECP5_DB or TRELLIS_ROOT)")
    parser.add_argument("--trellis", type=Path, help="Source checkout (TRELLIS_ROOT or parent of ECP5_DB)")
    parser.add_argument("--native-probe", type=Path)
    args = resolve_toolchain(parser, parser.parse_args())
    folder = args.inventory
    inventory = json.loads((folder / "device_inventory.json").read_text())
    if inventory["device"] != "LFE5U-85F":
        raise ValueError("collector is scoped to LFE5U-85F")
    for source in inventory["source_files"]:
        if sha(args.database / source["path"]) != source["sha256"]:
            raise ValueError(f"stale inventory: {source['path']}")
    tiles = csv_rows(folder / "tile_instances.csv")
    resources = csv_rows(folder / "resource_instances.csv")
    sites = csv_rows(folder / "site_bindings.csv")
    blocks = {row["resource_id"]: row for row in resources if row["family"] == "DSP_BLOCK"}
    max_row = max(int(tile["fabric_row"]) for tile in tiles)
    max_col = max(int(tile["fabric_col"]) for tile in tiles)
    requests, edges, templates = {}, [], set()
    def resolve(tile, name):
        request = int(tile["fabric_row"]), int(tile["fabric_col"]), name
        if request not in requests:
            requests[request] = endpoint(*request, max_row, max_col)
        return requests[request]
    owned = {}
    for tile in tiles:
        match = re.fullmatch(r"MIB2?_DSP(\d)", tile["type"])
        if not match and not tile["type"].startswith("DSP_SPINE_"):
            continue
        index = int(match[1]) if match else 8
        owner = f"LFE5U-85F/R{tile['fabric_row']}C{int(tile['fabric_col']) - index}/DSP_BLOCK"
        if owner not in blocks:
            raise ValueError(f"DSP tile without block: {tile['tile']}")
        owned[tile["tile"]] = owner
        desc = inventory["tile_types"][tile["type"]]
        records = [("fixed", c["source"], c["sink"], (), c["line"]) for c in desc["fixed_connections"]]
        records += [("mux", option["source"], sink, tuple(option["bits"]), option["line"])
                    for sink, mux in desc["muxes"].items() for option in mux["sources"]]
        for kind, raw_source, raw_sink, tokens, line in records:
            source, sink = resolve(tile, raw_source), resolve(tile, raw_sink)
            constraints = requirements(tokens, tile)
            reference = f"{desc['source']}:{line}"
            templates.add((tile["type"], kind, raw_source, raw_sink, packed(tokens), reference))
            if source is not None and sink is not None:
                edges.append((source, sink, kind, constraints, reference, tile["tile"], owner))

    native = dict(performed=False, compared_requests=0)
    extra_sources = []
    if args.native_probe:
        ordered = sorted(requests)
        result = subprocess.run([str(args.native_probe.resolve()), str(args.database)],
            input="".join(f"{r} {c} {name}\n" for r, c, name in ordered), text=True, capture_output=True, check=True)
        lines = result.stdout.splitlines()
        if len(lines) != len(ordered):
            raise ValueError("native wire probe response length differs")
        for request, actual in zip(ordered, lines):
            values = actual.split("\t")
            value = None if actual == "invalid" else (int(values[0]), int(values[1]), values[2])
            if value != requests[request]:
                raise ValueError(f"native wire mismatch: {request}: {value} != {requests[request]}")
        library = next(line.split("=", 1)[1] for line in result.stderr.splitlines() if line.startswith("libtrellis_path="))
        native = dict(performed=True, compared_requests=len(ordered), library_sha256=sha(Path(library)))
        extra_sources += [("native_probe_binary", args.native_probe)]

    ports = documented_ports()
    observed, node_ports = defaultdict(set), defaultdict(set)
    adjacency, fixed, mux = defaultdict(list), defaultdict(list), defaultdict(list)
    for edge in edges:
        source, sink, kind, _, _, _, _ = edge
        adjacency[source].append(edge)
        (fixed if kind == "fixed" else mux)[sink].append(edge)
        for node in (source, sink):
            match = FAMILY.fullmatch(node[2])
            if match and match[2] in SMALL:
                pin, family = match.groups()
                observed[family].add(pin)
                node_ports[node[0], node[1], family].add(pin)
    port_rows = []
    source_pages = {"PRADD9": "sysDSP:21-23;Diamond:747-750", "PRADD18": "sysDSP:23-25;Diamond:745-747",
                    "MULT9": "Diamond:625-629", "ALU24": "Diamond:186-189;CFB name also present in diagram"}
    for family in SMALL:
        for pin in sorted(set(ports[family]) | observed[family]):
            status = "name_match" if pin in ports[family] and pin in observed[family] else "documented_unobserved" if pin in ports[family] else "observed_unmapped_name"
            port_rows.append([family, pin, ports[family].get(pin, "unknown"), pin in observed[family], status, source_pages[family]])

    groups = defaultdict(list)
    for site in sites:
        if site["label_family"] in SMALL:
            groups[site["resource_id"], site["site_name"]].append(site)
    namespaces, candidates = [], set()
    for (owner, name), refs in sorted(groups.items()):
        family, index = name.rsplit("_", 1)
        index = int(index)
        block = blocks[owner]
        row, col = int(block["fabric_row"]), int(block["fabric_col"]) + index
        key = row, col, family
        candidates.add(key)
        found = node_ports.get(key, set())
        side = "LEFT" if index < 4 else "RIGHT"
        parent_col = int(block["fabric_col"]) + (0 if side == "LEFT" else 4)
        parent = f"LFE5U-85F/R{row}C{parent_col}/DSP_SLICE_{side}"
        if not found:
            raise ValueError(f"site label without wire namespace: {owner}:{name}")
        namespaces.append([owner, name, family, row, col, parent, len(refs), len(found),
            packed(sorted(set(ports[family]) - found)), packed(sorted(found - set(ports[family]))),
            packed(sorted({(int(r['vendor_pos_row']), int(r['vendor_pos_col'])) for r in refs})),
            "wire_namespace_candidate; 9bit_lane_pairing_unresolved"])

    issues = []
    for sink in sorted(set(fixed) & set(mux)):
        if not re.fullmatch(r"J(?:MULTA|P|Q|CFB|DSPC)\d+", sink[2]):
            continue
        fixed_edges, mux_edges = fixed[sink], mux[sink]
        owners = {e[6] for e in fixed_edges + mux_edges}
        if len(owners) != 1:
            raise ValueError("mixed block ownership of local mux sink")
        issues.append([next(iter(owners)), *sink, packed(sorted({e[0] for e in fixed_edges})),
            packed(sorted({(e[0], e[3]) for e in mux_edges})), packed(sorted({e[4] for e in fixed_edges + mux_edges})),
            "unqualified_fixed_and_programmable_sources; requires_mode_interpretation"])

    paths = []
    for row, col, family in sorted(candidates):
        if family not in ("PRADD9", "PRADD18"):
            continue
        width = 9 if family == "PRADD9" else 18
        for bit in range(width):
            start = row, col, f"JPO{bit}_{family}"
            queue = deque([(start, ())])
            while queue:
                node, route = queue.popleft()
                match = FAMILY.fullmatch(node[2])
                if route and match:
                    if match[2] in ("MULT9", "MULT18") and re.fullmatch(r"A\d+", match[1]):
                        bits = sorted({b for e in route for b in e[3]})
                        polarities = defaultdict(set)
                        for frame, offset, value in bits:
                            polarities[frame, offset].add(value)
                        if any(len(v) != 1 for v in polarities.values()):
                            raise ValueError("contradictory path requirements")
                        paths.append([route[0][6], *start, *node, packed(bits),
                            packed([[e[5], e[4], e[2]] for e in route]),
                            any(e[2] == "mux" and e[1] in fixed for e in route),
                            "database_path_only; mode_compatibility_and_default_gating_unverified"])
                    continue
                if len(route) >= 3:
                    continue
                for edge in adjacency.get(node, ()):
                    if edge[1] != start and not any(e[0] == edge[1] for e in route):
                        queue.append((edge[1], route + (edge,)))

    # Relate the route selections to the existing resource-field catalogue.
    # Absence here means the bit is expressed as routing, not a missing bit.
    catalog = json.loads((folder / "decode_catalog.json").read_text())
    field_owners = defaultdict(set)
    for resource in catalog["resources"]:
        for field in resource["fields"]:
            for fragment in field["fragments"]:
                for group in list(fragment.get("options", {}).values()) + fragment.get("word_bits", []):
                    for frame, bit, _ in group:
                        field_owners[frame, bit].add((resource["id"], field["name"]))
    controls = defaultdict(lambda: dict(bits=set(), targets=set(), defaults=set(), block=None))
    for path in paths:
        if not path[6].endswith("_MULT18"):
            continue
        control = controls[path[4], path[5]]
        control["bits"].update(tuple(bit) for bit in json.loads(path[7]))
        control["targets"].add(path[6].split("_", 1)[0][2:])
        control["defaults"].add(path[9])
        control["block"] = path[0]
    mult_by_location = {(int(r["fabric_row"]), int(r["fabric_col"])): r["resource_id"]
                        for r in resources if r["family"] == "MULT18"}
    control_rows = []
    for (row, col), control in sorted(controls.items()):
        owners = sorted({owner for frame, bit, _ in control["bits"] for owner in field_owners[frame, bit]})
        control_rows.append([control["block"], mult_by_location[row, col], row, col,
            packed(sorted(control["bits"])), packed(sorted(control["targets"], key=int)),
            packed(owners), packed(sorted(control["defaults"])), "routing_selection; complete_mode_unresolved"])
    selection_bits = {(frame, bit) for control in controls.values() for frame, bit, _ in control["bits"]}

    sources = [(name, folder / name) for name in ("device_inventory.json", "tile_instances.csv", "resource_instances.csv", "site_bindings.csv",
        "decode_catalog.json", "dsp_research/evidence_report.json")]
    sources += [("collector", Path(__file__)), ("native_probe_source", ROOT / "tools/trellis_wire_probe.cpp"),
                ("Trellis:RoutingGraph.cpp", args.trellis / "libtrellis/src/RoutingGraph.cpp")]
    sources += extra_sources
    report = dict(schema_version=1, device=inventory["device"],
        scope="Directed database paths and wire namespace candidates; no executable routing or physical lane allocation.",
        dsp_blocks=len(blocks), tile_instances=len(owned), tile_types=len({tile['type'] for tile in tiles if tile['tile'] in owned}),
        tile_types_with_connection_records=len({t[0] for t in templates}),
        template_records=len(templates), instantiated_edge_records=len(edges),
        rejected_wire_requests=sum(value is None for value in requests.values()),
        native_wire_check=native, port_name_states=dict(sorted(Counter(r[4] for r in port_rows).items())),
        candidate_namespaces_by_family=dict(sorted(Counter(r[2] for r in namespaces).items())),
        raw_site_references=sum(r[6] for r in namespaces),
        namespaces_missing_documented_ports=sum(r[8] != "[]" for r in namespaces),
        unassigned_external_primitive_namespaces=sorted(set(node_ports) - candidates),
        fixed_mux_overlap_sinks=len(issues), fixed_mux_overlap_per_block=dict(sorted(Counter(r[0] for r in issues).items())),
        fixed_mux_overlap_names=dict(sorted(Counter(re.sub(r"\d+", "*", r[3]) for r in issues).items())),
        preadder_to_multiplier_paths=len(paths), preadder_to_multiplier_per_block=dict(sorted(Counter(r[0] for r in paths).items())),
        paths_with_unqualified_fixed_default=sum(r[9] for r in paths),
        preadder_route_controls=len(control_rows), preadder_selection_cram_bits=len(selection_bits),
        preadder_selection_bits_without_named_resource_field=sum(not field_owners[bit] for bit in selection_bits),
        limitations=["A fixed connection can describe a mode-specific view; do not union all primitive views.",
            "Observed 9-bit site labels do not identify both physical 9-bit lanes.",
            "ALU24 OP5/OP7 are not assigned to documented OPADDNSUB/OPCINSEL by name alone.",
            "External primitive names at block boundaries are retained without creating physical resources.",
            "Path traversal stops after three edges or on reaching another primitive; no internal arithmetic is traversed."],
        source_files=[dict(source=name, sha256=sha(path)) for name, path in sources])
    output = args.output or folder / "dsp_routing_research"
    output.mkdir(parents=True, exist_ok=True)
    write_csv(output / "port_catalogue.csv", ["family", "port", "documented_direction", "observed_in_database", "name_status", "source_pages"], port_rows)
    write_csv(output / "endpoint_namespaces.csv", ["dsp_block", "vendor_site_name", "family", "fabric_row", "fabric_col", "candidate_slice_id",
        "raw_reference_count", "observed_port_names", "documented_unobserved_ports", "observed_unmapped_ports", "vendor_positions", "interpretation"], namespaces)
    write_csv(output / "connection_templates.csv", ["tile_type", "edge_kind", "source", "sink", "local_selection_bits", "source_reference"], sorted(templates))
    write_csv(output / "fixed_mux_overlaps.csv", ["dsp_block", "sink_row", "sink_col", "sink_name", "unqualified_fixed_sources", "programmable_sources",
        "source_references", "interpretation"], issues)
    write_csv(output / "preadder_paths.csv", ["dsp_block", "source_row", "source_col", "source_wire", "target_row", "target_col", "target_wire",
        "selection_requirements", "path_evidence", "has_unqualified_fixed_default", "interpretation"], paths)
    write_csv(output / "preadder_controls.csv", ["dsp_block", "mult18_resource", "fabric_row", "fabric_col", "selection_requirements",
        "observed_A_bit_indices", "named_resource_field_owners", "unqualified_fixed_default_states", "interpretation"], control_rows)
    (output / "routing_report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(packed({k: report[k] for k in ("dsp_blocks", "template_records", "instantiated_edge_records", "port_name_states",
        "candidate_namespaces_by_family", "fixed_mux_overlap_sinks", "preadder_to_multiplier_paths", "native_wire_check")}))


if __name__ == "__main__":
    main()
