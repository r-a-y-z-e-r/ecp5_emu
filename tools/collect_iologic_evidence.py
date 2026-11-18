#!/usr/bin/env python3
"""Collect IOLOGIC/SIOLOGIC evidence, without changing the runtime catalogue."""
import argparse
from collections import Counter, defaultdict
import csv
import io as textio
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import re
import subprocess

from bind_trellis_sites import bind_config_fields
from inventory_trellis import sha256, write_csv
from resource_audit import audit_config, compile_decode_catalog, packed


ROOT = Path(__file__).resolve().parents[1]
DEVICE = "LFE5U-85F"
IO_FAMILIES = {"IOLOGIC", "SIOLOGIC"}
PIO_MUXES = {"DATAMUX_OREG", "DATAMUX_ODDR", "DATAMUX_MDDR", "TRIMUX_TSREG"}


def csv_rows(path):
    with path.open(newline="") as stream:
        return list(csv.DictReader(stream))


def ports_for(resource):
    """Independent transcription of the pinned native constructor's interface."""
    family, name = resource["family"], resource["name"]
    row, col = int(resource["fabric_row"]), int(resource["fabric_col"])
    letter = name[-1]
    if family == "PIO":
        pins = [("I", "in", f"PADDO{letter}_PIO"), ("T", "in", f"PADDT{letter}_PIO"),
                ("O", "out", f"JPADDI{letter}_PIO"), ("IOLDO", "in", f"IOLDO{letter}_PIO"),
                ("IOLTO", "in", f"IOLTO{letter}_PIO")]
    else:
        raw_in = ["DI", "IOLDOI", "PADDI"]
        raw_out = ["IOLDO", "IOLDOD", "IOLTO", "INDD"]
        j_in = ["CLK", "CE", "LSR", "LOADN", "MOVE", "DIRECTION", "TSDATA0", "TXDATA0", "TXDATA1"]
        j_out = ["RXDATA0", "RXDATA1", "INFF", "CFLAG"]
        if family == "IOLOGIC":
            raw_in += ["ECLK", "DQSR90", "DQSW270", "DQSW"]
            raw_in += [f"{prefix}PNTR{i}" for prefix in ("RD", "WR") for i in range(3)]
            j_in += ["TSDATA1", "TXDATA2", "TXDATA3"]
            j_out += ["RXDATA2", "RXDATA3"]
            if letter in "AC":
                j_in += ["TXDATA4", "TXDATA5", "TXDATA6", "SLIP"]
                j_out += ["RXDATA4", "RXDATA5", "RXDATA6"]
        pins = [(pin, direction, f"{j}{pin}{letter}_{family}")
                for values, direction, j in ((raw_in, "in", ""), (raw_out, "out", ""),
                                              (j_in, "in", "J"), (j_out, "out", "J")) for pin in values]
    return [[resource["resource_id"], pin, direction, row, col, wire] for pin, direction, wire in pins]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", type=Path, default=ROOT / "docs/inventory" / DEVICE)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--database", type=Path, help="Database root (ECP5_DB or TRELLIS_ROOT)")
    parser.add_argument("--trellis", type=Path, help="Source checkout (TRELLIS_ROOT or parent of ECP5_DB)")
    parser.add_argument("--nextpnr", type=Path, help="Source checkout (NEXTPNR_ROOT)")
    parser.add_argument("--yosys-library", type=Path, default=Path("/opt/homebrew/share/yosys/lattice"))
    parser.add_argument("--native-probe", type=Path)
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
    grid = json.loads((database / "ECP5" / DEVICE / "tilegrid.json").read_text())
    io = json.loads((database / "ECP5" / DEVICE / "iodb.json").read_text())
    resources = {r["resource_id"]: r for r in csv_rows(folder / "resource_instances.csv")}
    bindings = csv_rows(folder / "site_bindings.csv")
    selected = {key: r for key, r in resources.items() if r["family"] in IO_FAMILIES | {"PIO"}}
    logic = {key: r for key, r in selected.items() if r["family"] in IO_FAMILIES}
    targets = {}
    for binding in bindings:
        if binding["resource_id"] not in selected or binding["relation"] != "instance":
            continue
        key = binding["tile"], binding["site_name"]
        if key in targets and targets[key] != binding["resource_id"]:
            raise ValueError(f"conflicting site binding: {key}")
        targets[key] = binding["resource_id"]
    all_fields, all_bits = bind_config_fields(grid, inventory, targets)
    fields = [f for f in all_fields if f[1] in logic or f[4].split(".", 1)[1] in PIO_MUXES]
    field_ids = {f[0] for f in fields}
    bits = [b for b in all_bits if b[0] in field_ids]
    unbound = [(tile, kind, name) for tile, data in sorted(grid.items())
               for kind, category in (("enum", "enums"), ("word", "words"))
               for name in inventory["tile_types"][data["type"]][category]
               if name.startswith("IOLOGIC") and (tile, name.split(".")[0]) not in targets]
    if unbound:
        raise ValueError(f"unbound IOLOGIC fields: {unbound[:3]}")
    groups, aliases, shared, conflicts, words = audit_config(grid, inventory, fields, bits)
    catalog = compile_decode_catalog(grid, inventory, fields, selected)
    ports = sorted(p for resource in selected.values() for p in ports_for(resource))
    native = {"status": "not_run", "scope": "BEL interfaces; not full routing or behaviour"}
    extra_sources = []
    if args.native_probe:
        requests = [f"{r['fabric_row']} {r['fabric_col']} {'ABCD'.index(r['name'][-1])} {int(r['family'] == 'SIOLOGIC')}"
                    for _, r in sorted(logic.items())]
        result = subprocess.run([str(args.native_probe.resolve()), str(database)],
                                input="\n".join(requests) + "\n", text=True, capture_output=True, check=True)
        actual = set()
        for p in csv.DictReader(textio.StringIO(result.stdout), delimiter="\t"):
            key = f"{DEVICE}/R{p['row']}C{p['col']}/{p['bel']}"
            if key not in selected or p["bel_type"] != selected[key]["family"]:
                raise ValueError(f"unexpected native BEL: {key}")
            actual.add((key, p["port"], p["direction"], int(p["wire_row"]), int(p["wire_col"]), p["wire"]))
        expected = {tuple(p) for p in ports}
        if actual != expected:
            raise ValueError(f"native interface mismatch: {len(actual - expected)} extra, {len(expected - actual)} missing")
        library_lines = [line.split("=", 1)[1] for line in result.stderr.splitlines() if line.startswith("libtrellis_path=")]
        if len(library_lines) != 1:
            raise ValueError("probe did not identify its native library")
        extra_sources += [("native_probe_binary", args.native_probe.resolve()), ("native_library", Path(library_lines[0]))]
        native.update(status="matched", resources=len(selected), ports=len(actual))

    metadata = {(p["row"], p["col"], p["pio"]): p for p in io["pio_metadata"]}
    packages = defaultdict(list)
    for package, pins in sorted(io["packages"].items()):
        for pin, loc in sorted(pins.items()):
            packages[loc["row"], loc["col"], loc["pio"]].append([package, pin])
    fields_by_resource = defaultdict(list)
    for field in fields:
        fields_by_resource[field[1]].append(field)
    instances, capabilities = [], []
    common_modes = ("IREG_OREG", "DELAYF", "DELAYG", "IDDRX1F", "ODDRX1F")
    side_modes = ("IDDRX2F", "ODDRX2F", "IDDRX2DQA", "ODDRX2DQA", "ODDRX2DQSB", "OSHX2A", "TSHX2DQA", "TSHX2DQSA")
    for key, resource in sorted(logic.items()):
        row, col = int(resource["fabric_row"]), int(resource["fabric_col"])
        letter = resource["name"][-1]
        family = resource["family"]
        meta = metadata[row, col, letter]
        sibling_letter = "ABCD"["ABCD".index(letter) ^ 1]
        sibling = f"{DEVICE}/R{row}C{col}/{family}{sibling_letter}"
        if sibling not in logic:
            sibling = ""
        pio = f"{DEVICE}/R{row}C{col}/PIO{letter}"
        if pio not in selected:
            raise ValueError(f"missing PIO for {key}")
        dqs = meta.get("dqs", "")
        dqs_id = ""
        if dqs:
            # DQ names mark data pins, DQS names mark strobe pins in the same lane.
            match = re.fullmatch(r"([LR])DQ(?:SN|S)?(\d+)", dqs)
            if not match:
                raise ValueError(f"unknown DQS group name: {dqs}")
            side, dqs_row = match.groups()
            dqs_id = f"{DEVICE}/R{dqs_row}C{0 if side == 'L' else inventory['device_info']['max_col']}/DQSBUF"
            if resources.get(dqs_id, {}).get("family") != "DQSBUFM":
                raise ValueError(f"missing DQS resource: {dqs_id}")
        instances.append([key, family, resource["anchor_tile"], pio, sibling, meta["bank"], dqs, dqs_id,
                          meta.get("function", ""), len(ports_for(resource)), len(fields_by_resource[key]),
                          packed(packages[row, col, letter]), "mapped" if fields_by_resource[key] else "no_named_settings_in_database"])
        for primitive in common_modes + side_modes + ("IDDR71B", "ODDR71B"):
            wide = primitive in ("IDDR71B", "ODDR71B")
            allowed = primitive in common_modes or (family == "IOLOGIC" and (not wide or letter in "AC"))
            allocation = sibling if wide and allowed else ""
            notes = []
            if wide and allowed:
                notes.append("uses DDR register bank of paired pin; directional sharing needs finer verification")
            if primitive == "IDDRX2F" and allowed:
                notes.append("documented ALIGNWD not moved by pinned nextpnr packer")
            if not fields_by_resource[key]:
                notes.append("physical interface exists; named configuration unavailable")
            capabilities.append([key, primitive, "available" if allowed else "not_available_at_location", allocation,
                                 "docs_and_pinned_tool_structure;not_behaviourally_verified", "; ".join(notes)])

    # Retain shared ownership across the selected IO fields and the prior hard-block catalogue.
    owners = defaultdict(set)
    for bit in bits:
        owners[bit[5], bit[6]].add((bit[1], bit[0]))
    io_bits = set(owners)
    for bit in csv_rows(folder / "config_field_bits.csv"):
        physical = int(bit["cram_frame"]), int(bit["cram_bit"])
        if physical in io_bits:
            owners[physical].add((bit["resource_id"], bit["field_id"]))
    external = [[frame, bit, packed(sorted({o[0] for o in values})), packed(sorted({o[1] for o in values}))]
                for (frame, bit), values in sorted(owners.items()) if any(o[0] not in selected for o in values)]
    candidate_rows = [[b[name] for name in ("tile", "site_index", "site_name", "resource_id", "relation", "rule")]
                      for b in bindings if b["resource_id"] in logic and b["relation"] != "instance"]
    sources = [("collector", Path(__file__)), ("native_probe_source", ROOT / "tools/trellis_io_probe.cpp"),
               ("binding_rules", ROOT / "tools/bind_trellis_sites.py"), ("config_audit", ROOT / "tools/resource_audit.py")]
    sources += [(name, folder / name) for name in ("device_inventory.json", "resource_instances.csv", "site_bindings.csv", "config_field_bits.csv")]
    sources += [(f"Trellis:{name}", args.trellis / name) for name in ("libtrellis/src/Bels.cpp", "libtrellis/src/Chip.cpp")]
    sources += [(f"nextpnr:{name}", args.nextpnr / name) for name in ("ecp5/pack.cc", "ecp5/bitstream.cc")]
    sources += [(f"Yosys:{name}", args.yosys_library / name) for name in ("cells_bb_ecp5.v", "cells_sim_ecp5.v")]
    sources += [(f"fuzzer:{name}", args.trellis / "fuzzers/ECP5" / name / "fuzzer.py")
                for name in ("060-iologic_modes", "061-basic_ddr", "063-oddrxn", "064-iddrxn", "065-mddrx", "066-iodelay", "067-ioreg")]
    for filename in ("FPGA-TN-02035-1.3.pdf", "FPGA-TN-02032-1.4.pdf"):
        source = ROOT / "target/research/iologic" / filename
        if source.exists():
            sources.append((filename, source))
    sources += extra_sources
    report = dict(schema_version=1, device=DEVICE, families=dict(sorted(Counter(r["family"] for r in selected.values()).items())),
                  native_interface_check=native, port_count=len(ports),
                  iologic_ports=sum(p[0] in logic for p in ports), field_fragments=len(fields),
                  iologic_field_fragments=sum(f[1] in logic for f in fields), whole_fields=len(groups),
                  bit_references=len(bits), unique_cram_bits=len(io_bits),
                  missing_iologic_settings=sorted(key for key in logic if not fields_by_resource[key]),
                  unresolved_candidate_references=len(candidate_rows),
                  local_enum_aliases=len(aliases), complete_field_aliases=sum(json.loads(g[7]) != [] for g in groups),
                  default_discrepancies=sum(g[6] for g in groups), option_polarity_conflicts=len(conflicts),
                  shared_cram_bits_within_selection=len(shared), shared_cram_bits_with_prior_catalogue=len(external),
                  dqs_members=sum(bool(r[6]) for r in instances), dqs_groups=len({r[7] for r in instances if r[7]}),
                  seven_to_one_locations=sum(r[1] == "IDDR71B" and r[2] == "available" for r in capabilities),
                  source_files=[dict(source=name, sha256=sha256(path)) for name, path in sources],
                  limitations=["Interface verification does not verify internal register topology or routing.",
                               "7:1 modes consume the paired DDR register bank; simultaneous RX/TX sharing is not proven.",
                               "SPICB0 SIOLOGICA has ports but no named settings in this database.",
                               "ALIGNWD descriptions conflict; IDDRX2F alignment is not packed by this nextpnr revision.",
                               "Only PIO data/tristate mux fields are collected here; other PIO/bank settings remain separate.",
                               "No runtime decoder extension or behavioural implementation is included."])
    output = args.output or folder / "iologic_research"
    output.mkdir(parents=True, exist_ok=True)
    write_csv(output / "resource_ports.csv", ["resource_id", "port", "direction", "wire_row", "wire_col", "wire"], ports)
    write_csv(output / "instances.csv", ["resource_id", "family", "anchor_tile", "pio_resource", "paired_resource", "bank", "dqs_group", "dqs_resource", "pin_function", "port_count", "field_count", "package_pins", "configuration_status"], instances)
    write_csv(output / "mode_capabilities.csv", ["resource_id", "primitive", "location_availability", "paired_register_resource", "evidence_status", "notes"], capabilities)
    write_csv(output / "config_field_bindings.csv", ["field_id", "resource_id", "tile", "kind", "name", "default", "bit_count", "source"], fields)
    write_csv(output / "config_field_bits.csv", ["field_id", "resource_id", "tile", "local_frame", "local_bit", "global_frame", "global_bit"], bits)
    write_csv(output / "config_field_groups.csv", ["resource_id", "kind", "name", "fragment_count", "tiles", "defaults", "default_discrepancy", "complete_encoding_aliases"], groups)
    write_csv(output / "enum_fragment_aliases.csv", ["field_id", "resource_id", "options", "bit_count", "local_bits", "source"], aliases)
    write_csv(output / "shared_config_bits.csv", ["global_frame", "global_bit", "resource_ids", "field_ids"], shared)
    write_csv(output / "prior_catalogue_shared_bits.csv", ["global_frame", "global_bit", "resource_ids", "field_ids"], external)
    write_csv(output / "candidate_references.csv", ["tile", "site_index", "site_name", "resource_id", "relation", "rule"], candidate_rows)
    (output / "config_catalogue.json").write_text(json.dumps(catalog, ensure_ascii=False, separators=(",", ":")) + "\n")
    (output / "evidence_report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(packed({key: value for key, value in report.items() if key not in ("source_files", "limitations")}))


if __name__ == "__main__":
    main()
