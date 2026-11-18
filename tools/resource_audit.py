"""Audit evidence and configuration fragments without inferring behaviour."""
from collections import defaultdict
import json
import re


def packed(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def pattern(tokens):
    # '-' is an empty constraint group, not a zero-valued configuration bit.
    return tuple(sorted({t for t in tokens if t != "-"}))


def audit_config(grid, inventory, fields, bit_rows):
    groups, bit_owners = defaultdict(list), defaultdict(list)
    for field in fields:
        groups[field[1], field[3], field[4]].append(field)
    for bit in bit_rows:
        bit_owners[bit[5], bit[6]].append(bit)
    fragment_aliases, group_rows, conflicts, word_layout = [], [], [], []
    for (resource, kind, name), fragments in sorted(groups.items()):
        defaults = sorted({f[5] for f in fragments}, key=lambda x: (x is not None, x or ""))
        combined_aliases = []
        if kind == "word":
            widths = {len(inventory["tile_types"][grid[f[2]]["type"]]["words"][name]["bit_groups"]) for f in fragments}
            if len(widths) != 1:
                raise ValueError(f"inconsistent word fragment widths: {resource}:{name}: {widths}")
            for f in fragments:
                tile = grid[f[2]]
                desc = inventory["tile_types"][tile["type"]]["words"][name]
                for index, tokens in enumerate(desc["bit_groups"]):
                    bits = []
                    for token in pattern(tokens):
                        frame, bit = map(int, re.fullmatch(r"!?F(\d+)B(\d+)", token).groups())
                        bits.append([tile["start_frame"] + frame, tile["start_bit"] + bit, not token.startswith("!")])
                    word_layout.append([f[0], resource, name, index, len(desc["bit_groups"]),
                                        "observed" if bits else "no_bits_in_fragment", packed(bits)])
        if kind == "enum":
            descriptions = []
            for f in fragments:
                tile = grid[f[2]]
                desc = inventory["tile_types"][tile["type"]]["enums"][name]
                descriptions.append((tile, desc))
                aliases = defaultdict(list)
                for option in desc["options"]:
                    aliases[pattern(option["bits"])].append(option["value"])
                for bits, options in sorted(aliases.items()):
                    if len(options) > 1:
                        fragment_aliases.append([f[0], resource, packed(sorted(options)), len(bits), packed(bits), f[7]])
            # Equality of complete write patterns is evidence of an encoding
            # alias. Distinct patterns do not establish a correct decoder.
            common = set.intersection(*(set(o["value"] for o in d["options"]) for _, d in descriptions))
            combined = defaultdict(list)
            for value in sorted(common):
                bits = set()
                for tile, desc in descriptions:
                    option = next(o for o in desc["options"] if o["value"] == value)
                    for token in pattern(option["bits"]):
                        frame, bit = map(int, re.fullmatch(r"!?F(\d+)B(\d+)", token).groups())
                        bits.add((tile["start_frame"] + frame, tile["start_bit"] + bit, not token.startswith("!")))
                polarities = defaultdict(set)
                for frame, bit, polarity in bits: polarities[frame, bit].add(polarity)
                for (frame, bit), values in polarities.items():
                    if len(values) > 1:
                        conflicts.append([resource, name, value, frame, bit])
                combined[tuple(sorted(bits))].append(value)
            combined_aliases = sorted(sorted(v) for v in combined.values() if len(v) > 1)
        group_rows.append([resource, kind, name, len(fragments), packed(sorted(f[2] for f in fragments)),
                           packed(defaults), len(defaults) > 1, packed(combined_aliases)])
    shared = []
    for (frame, bit), owners in sorted(bit_owners.items()):
        resources = {b[1] for b in owners}
        if len(resources) > 1:
            shared.append([frame, bit, packed(sorted(resources)), packed(sorted({b[0] for b in owners}))])
    return group_rows, fragment_aliases, shared, conflicts, word_layout


def audit_unmodelled(inventory, bindings, normalize_wire):
    families = ("CIBTEST", "CLKTEST", "BREFTEST", "PVTTEST", "START", "BCINRD", "BCLVDSO", "POR",
                "PRADD9", "PRADD18", "MULT9", "ALU24")
    result = {"scope": "Evidence from used tile types only; raw site tuples are not resource capacities.", "families": {}}
    for family in families:
        refs = [r for r in bindings if r[3] == family]
        matcher = re.compile(r"(?:^|_)" + re.escape(family) + r"(?:_|$)")
        settings, muxes, fixed, wires = [], [], [], set()
        for kind, tile in sorted(inventory["tile_types"].items()):
            source = tile["source"]
            for category in ("enums", "words"):
                for name, d in tile[category].items():
                    if name.split(".")[0] == family or re.fullmatch(re.escape(family) + r"_\d+", name.split(".")[0]):
                        settings.append({"tile_type": kind, "name": name, "source": f"{source}:{d['line']}"})
            for sink, mux in tile["muxes"].items():
                names = [normalize_wire(n) for n in [sink, *(s["source"] for s in mux["sources"])]]
                observed = [n for n in names if n is not None and matcher.search(n)]
                if observed:
                    wires.update(observed)
                    muxes.append({"tile_type": kind, "sink": sink, "source": f"{source}:{mux['line']}"})
            for connection in tile["fixed_connections"]:
                names = [normalize_wire(connection[c]) for c in ("source", "sink")]
                observed = [n for n in names if n is not None and matcher.search(n)]
                if observed:
                    wires.update(observed)
                    fixed.append({"tile_type": kind, "source_wire": connection["source"], "sink_wire": connection["sink"],
                                  "source": f"{source}:{connection['line']}"})
        result["families"][family] = dict(site_reference_records=len(refs),
                distinct_vendor_site_tuples=len({(r[2], r[4], r[5]) for r in refs}),
                tile_instances=inventory["tile_types"].get(family, {}).get("instances", 0),
                site_binding_relations=dict(sorted((rel, sum(r[7] == rel for r in refs)) for rel in sorted({r[7] for r in refs}))),
                setting_descriptors=settings, mux_descriptors=muxes, fixed_connection_descriptors=fixed,
                observed_wire_names=sorted(wires),
                interpretation="Observed names and references only; no inferred pin directions, internal state or behavioural model.")
    return result


def compile_decode_catalog(grid, inventory, fields, resources):
    """Compile bit constraints for the Rust reader, retaining every fragment."""
    grouped = defaultdict(lambda: defaultdict(list))
    for field in fields:
        tile = grid[field[2]]
        desc = inventory["tile_types"][tile["type"]]["enums" if field[3] == "enum" else "words"][field[4]]

        def physical(tokens):
            result = []
            for token in pattern(tokens):
                frame, bit = map(int, re.fullmatch(r"!?F(\d+)B(\d+)", token).groups())
                result.append([tile["start_frame"] + frame, tile["start_bit"] + bit, not token.startswith("!")])
            return result

        fragment = dict(tile=field[2], source=field[7], default=field[5])
        if field[3] == "enum":
            fragment["options"] = {o["value"]: physical(o["bits"]) for o in desc["options"]}
        else:
            fragment["word_bits"] = [physical(g) for g in desc["bit_groups"]]
        grouped[field[1]][field[3], field[4]].append(fragment)
    return dict(schema_version=1, device=inventory["device"],
                frames=inventory["device_info"]["frames"], bits_per_frame=inventory["device_info"]["bits_per_frame"],
                source_files=inventory["source_files"],
                resources=[dict(id=key, family=resources[key]["family"],
                    fields=[dict(kind=kind, name=name, fragments=fragments)
                            for (kind, name), fragments in sorted(values.items())])
                           for key, values in sorted(grouped.items())])
