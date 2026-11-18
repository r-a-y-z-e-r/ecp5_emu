#!/usr/bin/env python3
"""Inventory Trellis source data without changing the simulator or the database.

The CSV site records deliberately retain their owning tile. Vendor site
coordinates, fabric coordinates and physical resource counts are distinct.
"""

import argparse
from collections import Counter
import csv
import hashlib
import json
import os
from pathlib import Path
from toolchain_paths import resolve_toolchain, portable_path
import re
import subprocess
import sys


def revision(directory):
    result = subprocess.run(
        ["git", "-C", str(directory), "rev-parse", "HEAD"],
        capture_output=True, text=True, check=False,
    )
    return result.stdout.strip() if result.returncode == 0 else None


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def family(kind):
    if kind == "PLC2":
        return "logic"
    if kind.startswith(("CIB", "MIB_CIB", "VCIB")):
        return "interconnect"
    if kind.startswith(("MIB_DSP", "MIB2_DSP")):
        return "dsp"
    if kind.startswith("MIB_EBR"):
        return "ebr"
    if "CMUX" in kind or "SPINE" in kind or kind.startswith(("LMID", "RMID", "TMID", "BMID")):
        return "clock_and_mixed"
    if kind.startswith("TAP_DRIVE"):
        return "clock_distribution"
    if kind.startswith(("DDRDLL", "ECLK")):
        return "io_clocks"
    if kind.startswith("PLL"):
        return "pll"
    if kind.startswith("EFB"):
        return "io_and_system"
    if kind.startswith(("PIC", "PIO", "SPIC")):
        return "io"
    if kind.startswith("BANKREF"):
        return "io_bank"
    if kind.startswith("DUMMY"):
        return "reserved"
    if kind in {"DTR", "POR", "OSC", "PVT_COUNT2", "VIQ_BUF"}:
        return "system_test_or_special"
    raise ValueError(f"unclassified tile type: {kind}")


def site_family(name):
    if name.startswith("SLICE_"):
        return "SLICE"
    if re.fullmatch(r"PIO[ABCD]", name):
        return "PIO"
    if re.fullmatch(r"S?IOLOGIC[ABCD]", name):
        return "IOLOGIC"
    if name.startswith("CLKTEST"):
        return "CLKTEST"
    head = name.split("_")[0]
    if head in {"ALU24", "ALU54", "MULT9", "MULT18", "PRADD9", "PRADD18"}:
        return head
    return re.sub(r"\d+$", "", head)


def parse_bits(path):
    result = {"muxes": {}, "words": {}, "enums": {}, "fixed_connections": []}
    section = None
    entry = None
    for number, raw in enumerate(path.read_text().splitlines(), 1):
        parts = raw.split("#", 1)[0].split()
        if not parts:
            continue
        if parts[0].startswith("."):
            section = parts[0]
            if section == ".mux":
                entry = {"line": number, "sources": []}
                result["muxes"][parts[1]] = entry
            elif section == ".config":
                entry = {"line": number, "default": parts[2], "bit_groups": []}
                result["words"][parts[1]] = entry
            elif section == ".config_enum":
                entry = {"line": number, "default": parts[2] if len(parts) > 2 else None, "options": []}
                result["enums"][parts[1]] = entry
            elif section == ".fixed_conn":
                result["fixed_connections"].append({"line": number, "sink": parts[1], "source": parts[2]})
                entry = None
            else:
                raise ValueError(f"{path}:{number}: unknown section {section}")
        elif section == ".mux":
            entry["sources"].append({"source": parts[0], "bits": parts[1:], "line": number})
        elif section == ".config_enum":
            entry["options"].append({"value": parts[0], "bits": parts[1:], "line": number})
        elif section == ".config":
            entry["bit_groups"].append(parts)
        else:
            raise ValueError(f"{path}:{number}: entry outside a database section")
    for name, word in result["words"].items():
        if len(word["default"]) != len(word["bit_groups"]):
            raise ValueError(f"{path}: inconsistent word width for {name}")
    return result


def verify_native(database, device, types):
    for directory in (os.environ.get("ECP5_TRELLIS_LIB"), "/opt/homebrew/lib/trellis", "/usr/local/lib/trellis"):
        if directory:
            sys.path.append(directory)
    import pytrellis
    pytrellis.load_database(str(database))
    for name, parsed in types.items():
        native = pytrellis.get_tile_bitdata(pytrellis.TileLocator("ECP5", device, name))
        if set(native.get_sinks()) != set(parsed["muxes"]):
            raise ValueError(f"{name}: native mux catalogue differs")
        if set(native.get_settings_words()) != set(parsed["words"]):
            raise ValueError(f"{name}: native word catalogue differs")
        if set(native.get_settings_enums()) != set(parsed["enums"]):
            raise ValueError(f"{name}: native enum catalogue differs")
        fixed = sorted((c.sink, c.source) for c in native.get_fixed_conns())
        if fixed != sorted((c["sink"], c["source"]) for c in parsed["fixed_connections"]):
            raise ValueError(f"{name}: native fixed connections differ")
        for sink, mux in parsed["muxes"].items():
            if set(native.get_mux_data_for_sink(sink).get_sources()) != {s["source"] for s in mux["sources"]}:
                raise ValueError(f"{name}.{sink}: native mux sources differ")
        for key, word in parsed["words"].items():
            native_word = native.get_data_for_setword(key)
            default = "".join("1" if b else "0" for b in reversed(list(native_word.defval)))
            if default != word["default"] or len(native_word.bits) != len(word["bit_groups"]):
                raise ValueError(f"{name}.{key}: native word width/default differs")
        for key, enum in parsed["enums"].items():
            native_enum = native.get_data_for_enum(key)
            if (native_enum.defval or None) != enum["default"]:
                raise ValueError(f"{name}.{key}: native enum default differs")
            if set(native_enum.get_options()) != {o["value"] for o in enum["options"]}:
                raise ValueError(f"{name}.{key}: native enum options differ")
    return {"tile_types_checked": len(types), "checks": ["mux_sinks", "mux_sources", "fixed_connections", "word_names", "word_widths", "word_defaults", "enum_names", "enum_defaults", "enum_options"]}


def write_csv(path, columns, rows):
    with path.open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(columns)
        writer.writerows(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, help="Database root (ECP5_DB or TRELLIS_ROOT)")
    parser.add_argument("--device", default="LFE5U-85F")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify-pytrellis", action="store_true")
    args = resolve_toolchain(parser, parser.parse_args())
    root = args.database.resolve()
    device_dir = root / "ECP5" / args.device
    grid = json.loads((device_dir / "tilegrid.json").read_text())
    io = json.loads((device_dir / "iodb.json").read_text())
    info = json.loads((root / "devices.json").read_text())["families"]["ECP5"]["devices"][args.device]
    counts = Counter(tile["type"] for tile in grid.values())
    types = {}
    sources = []
    for filename in [root / "devices.json", *(device_dir / name for name in ("tilegrid.json", "iodb.json", "globals.json"))]:
        sources.append({"path": str(filename.relative_to(root)), "sha256": sha256(filename)})
    for kind, count in sorted(counts.items()):
        path = root / "ECP5/tiledata" / kind / "bits.db"
        parsed = parse_bits(path)
        parsed.update({"family": family(kind), "instances": count, "source": str(path.relative_to(root)), "sha256": sha256(path)})
        types[kind] = parsed
        sources.append({"path": parsed["source"], "sha256": parsed["sha256"]})

    instances = []
    sites = []
    for name, tile in sorted(grid.items()):
        match = re.search(r"R(\d+)C(\d+):", name)
        if not match:
            raise ValueError(f"tile has no fabric coordinates: {name}")
        row, col = map(int, match.groups())
        if row > info["max_row"] or col > info["max_col"]:
            raise ValueError(f"tile outside the device: {name}")
        if tile["start_frame"] + tile["cols"] > info["frames"] or tile["start_bit"] + tile["rows"] > info["bits_per_frame"]:
            raise ValueError(f"configuration window outside CRAM: {name}")
        instances.append([name, tile["type"], row, col, tile["start_frame"], tile["start_bit"], tile["cols"], tile["rows"], len(tile["sites"])])
        for site in tile["sites"]:
            sites.append([name, tile["type"], site["name"], site_family(site["name"]), site["pos_row"], site["pos_col"]])

    settings = []
    for kind, data in types.items():
        for name, enum in data["enums"].items():
            settings.append([kind, "enum", name, enum["default"], " | ".join(o["value"] for o in enum["options"]), len(enum["options"]), f"{data['source']}:{enum['line']}"])
        for name, word in data["words"].items():
            settings.append([kind, "word", name, word["default"], "", len(word["bit_groups"]), f"{data['source']}:{word['line']}"])

    pin_rows = []
    metadata = {(p["row"], p["col"], p["pio"]): p for p in io["pio_metadata"]}
    if len(metadata) != len(io["pio_metadata"]):
        raise ValueError("duplicate physical PIO metadata")
    for package, pins in sorted(io["packages"].items()):
        for pin, location in sorted(pins.items()):
            meta = metadata.get((location["row"], location["col"], location["pio"]), {})
            pin_rows.append([package, pin, location["row"], location["col"], location["pio"], meta.get("bank", ""), meta.get("dqs", ""), meta.get("function", "")])

    site_tuples = {(s[2], s[4], s[5]) for s in sites}
    site_counts = Counter(site_family(s[0]) for s in site_tuples)
    footprint_counts = {
        "logic_tiles": counts["PLC2"], "slices": counts["PLC2"] * 4,
        "lut4": counts["PLC2"] * 8, "fabric_flipflops": counts["PLC2"] * 8,
        "ebr18k": sum(counts[t] for t in ("MIB_EBR0", "EBR_CMUX_UR", "EBR_CMUX_LR", "MIB_EBR2", "MIB_EBR4", "MIB_EBR6")),
        "mult18x18": sum(counts[t] for t in ("MIB_DSP0", "MIB_DSP1", "MIB_DSP4", "MIB_DSP5")),
        "alu54": counts["MIB_DSP3"] + counts["MIB_DSP7"],
        "pll": sum(counts[t] for t in ("PLL0_UL", "PLL0_UR", "PLL0_LL", "PLL0_LR")),
        "ddr_dll": sum(counts[t] for t in ("DDRDLL_UL", "DDRDLL_UR", "DDRDLL_LL", "DDRDLL_LR")),
        "pio_metadata_entries": len(metadata), "serdes_dcu_anchors": counts["DCU0"],
    }
    stats = {
        "tiles": len(grid), "tile_types": len(types),
        "mux_descriptors": sum(len(t["muxes"]) for t in types.values()),
        "mux_source_choices": sum(len(m["sources"]) for t in types.values() for m in t["muxes"].values()),
        "enum_descriptors": sum(len(t["enums"]) for t in types.values()),
        "enum_options": sum(len(e["options"]) for t in types.values() for e in t["enums"].values()),
        "word_descriptors": sum(len(t["words"]) for t in types.values()),
        "fixed_connection_descriptors": sum(len(t["fixed_connections"]) for t in types.values()),
        "site_reference_records": len(sites), "distinct_site_name_coordinate_tuples": len(site_tuples),
        "site_tuple_counts_are_resource_capacities": False,
    }
    verified = verify_native(root, args.device, types) if args.verify_pytrellis else None
    reference_paths = [
        "libtrellis/src/Chip.cpp", "libtrellis/src/Bels.cpp",
        "libtrellis/src/RoutingGraph.cpp", "libtrellis/src/BitDatabase.cpp",
        "libtrellis/src/ChipConfig.cpp", "libtrellis/src/TileConfig.cpp",
        "tools/ecp_vlog.py", "tools/extract_tilegrid.py",
        "docs/architecture/tiles.rst", "docs/libtrellis/textconfig.rst",
    ]
    inventory = {
        "schema_version": 1, "device": args.device, "device_info": info,
        "database_revision": revision(root), "trellis_revision": revision(root.parent),
        "database_root": portable_path(root), "statistics": stats,
        "resource_counts_from_trellis_construction_rules": footprint_counts,
        "resource_count_rules_source": "libtrellis/src/Chip.cpp get_routing_graph_ecp5; libtrellis/src/Bels.cpp",
        "site_tuple_counts_by_label_family": dict(sorted(site_counts.items())),
        "package_pio_counts": {p: len(pins) for p, pins in sorted(io["packages"].items())},
        "pio_metadata": io["pio_metadata"],
        "globals": json.loads((device_dir / "globals.json").read_text()),
        "native_catalogue_verification": verified, "source_files": sources, "tile_types": types,
        "reference_source_files": [{"path": p, "sha256": sha256(root.parent / p)} for p in reference_paths if (root.parent / p).is_file()],
        "limitations": [
            "Site records are references attached to tiles, not independently allocatable resource instances.",
            "Vendor site positions are retained separately from fabric tile coordinates.",
            "Resource count rules are architecture rules, not an extraction of the full RoutingGraph.",
            "Catalogue verification compares names, choices, defaults and fixed endpoints, not native bit-group semantics.",
            "This inventory does not decode an application bitstream or validate resource behaviour.",
        ],
    }
    output = args.output or Path("docs/inventory") / args.device
    output.mkdir(parents=True, exist_ok=True)
    (output / "device_inventory.json").write_text(json.dumps(inventory, ensure_ascii=False, indent=2) + "\n")
    write_csv(output / "tile_instances.csv", ["tile", "type", "fabric_row", "fabric_col", "start_frame", "start_bit", "frame_count", "bit_count", "site_references"], instances)
    write_csv(output / "site_references.csv", ["tile", "type", "site_name", "label_family", "vendor_pos_row", "vendor_pos_col"], sites)
    write_csv(output / "settings.csv", ["tile_type", "kind", "name", "default", "options", "width_or_options", "source"], settings)
    write_csv(output / "package_pins.csv", ["package", "pin", "fabric_row", "fabric_col", "pio", "bank", "dqs", "function"], pin_rows)
    write_csv(output / "tile_types.csv", ["type", "family", "instances", "muxes", "source_choices", "enums", "words", "fixed_connections"], [
        [name, t["family"], t["instances"], len(t["muxes"]), sum(len(m["sources"]) for m in t["muxes"].values()), len(t["enums"]), len(t["words"]), len(t["fixed_connections"])]
        for name, t in types.items()
    ])
    print(json.dumps({"output": str(output), "statistics": stats, "resource_counts": footprint_counts, "native_verification": verified}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
