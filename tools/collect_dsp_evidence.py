#!/usr/bin/env python3
"""Collect DSP configuration evidence without creating behavioural models."""
import argparse
from collections import Counter, defaultdict
import csv
import hashlib
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import re


ROOT = Path(__file__).resolve().parents[1]
DEVICE = "LFE5U-85F"


def packed(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save_csv(path, header, rows):
    with path.open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(header)
        writer.writerows(rows)


def collect(catalog, resources):
    def block(resource_id):
        visited = set()
        while resources[resource_id]["family"] != "DSP_BLOCK":
            if resource_id in visited:
                raise ValueError("cycle in DSP ownership")
            visited.add(resource_id)
            resource_id = resources[resource_id]["parent_id"]
        return resource_id

    fields = []
    for resource in catalog["resources"]:
        if resource["family"] not in ("MULT18", "ALU54", "DSP_SLICE"):
            continue
        for field in resource["fields"]:
            if field["kind"] != "enum":
                continue
            options = defaultdict(set)
            for fragment in field["fragments"]:
                for value, bits in fragment["options"].items():
                    options[value].update(tuple(bit) for bit in bits)
            coordinates = set()
            for bits in options.values():
                polarities = defaultdict(set)
                for frame, bit, polarity in bits:
                    if not (0 <= frame < catalog["frames"] and 0 <= bit < catalog["bits_per_frame"]):
                        raise ValueError("DSP constraint outside CRAM")
                    polarities[frame, bit].add(polarity)
                    coordinates.add((frame, bit))
                if any(len(values) > 1 for values in polarities.values()):
                    raise ValueError("contradictory DSP option")
            signature = tuple((value, tuple(sorted(bits))) for value, bits in sorted(options.items()))
            fields.append(dict(resource=resource["id"], family=resource["family"],
                block=block(resource["id"]), name=field["name"], signature=signature,
                coordinates=coordinates, fragments=field["fragments"],
                complete=all(set(fragment["options"]) == set(options) for fragment in field["fragments"])))

    buckets = defaultdict(list)
    per_block = defaultdict(list)
    for field in fields:
        if field["coordinates"]:
            buckets[field["block"], field["signature"]].append(field)
        if ".REG_" in field["name"]:
            per_block[field["block"]].append(field)

    aliases, patterns = [], Counter()
    for (owner, signature), members in sorted(buckets.items()):
        if len({field["resource"] for field in members}) < 2:
            continue
        members.sort(key=lambda field: (field["resource"], field["name"]))
        names = tuple(sorted(field["name"] for field in members))
        patterns[names] += 1
        aliases.append([owner, packed([[field["resource"], field["name"]] for field in members]),
            all(field["complete"] for field in members), packed(signature),
            packed(sorted({fragment["source"] for field in members for fragment in field["fragments"]})),
            "identical_enum_encoding_only"])

    overlaps = []
    for owner, members in sorted(per_block.items()):
        members.sort(key=lambda field: (field["resource"], field["name"]))
        for index, first in enumerate(members):
            for second in members[index + 1:]:
                if first["resource"] == second["resource"]:
                    continue
                shared = first["coordinates"] & second["coordinates"]
                if shared:
                    overlaps.append([owner, first["resource"], first["name"], second["resource"], second["name"],
                        len(shared), len(first["coordinates"]), len(second["coordinates"]),
                        first["signature"] == second["signature"], packed(sorted(shared))])

    groups = [row for row in resources.values() if row["family"] == "DSP_BLOCK"]
    for group in groups:
        children = Counter(row["family"] for row in resources.values()
            if row["family"] in ("MULT18", "ALU54", "DSP_SLICE") and block(row["resource_id"]) == group["resource_id"])
        if children != Counter(MULT18=4, ALU54=2, DSP_SLICE=2):
            raise ValueError(f"unexpected DSP group members: {group['resource_id']}")
    return aliases, overlaps, patterns, len(groups)


def parameter_coverage(catalog, library_path, writer_path):
    library, writer = library_path.read_text(), writer_path.read_text()
    rows, summary = [], {}
    for family, primitive, function in (("MULT18", "MULT18X18D", "write_mult18"), ("ALU54", "ALU54B", "write_alu54")):
        module = re.search(r"module " + primitive + r"\b(.*?)endmodule", library, re.S)
        if not module:
            raise ValueError(f"missing declaration: {primitive}")
        # These two pinned writers have direct add_enum/add_word calls. Loops
        # constructing input mux names are outside the primitive parameter list.
        function_body = re.search(r"void " + function + r"\([^)]*\)\s*\{(.*?)(?=\n    void )", writer, re.S)
        if not function_body:
            raise ValueError(f"missing writer: {function}")
        emitted = set(re.findall(r'tg\.config\.add_(?:enum|word)\(dsp \+ "\.([^"\s]+)"', function_body.group(1)))
        instances = [resource for resource in catalog["resources"] if resource["family"] == family]
        counts = Counter(field["name"].split(".", 1)[1] for resource in instances for field in resource["fields"])
        missing, partial = [], []
        parameters = list(re.finditer(r"parameter\s+(\w+)\s*=\s*([^;]+);", module.group(1)))
        for match in parameters:
            name, default = match.groups()
            count = counts[name]
            status = "all_instances" if count == len(instances) else "some_instances" if count else "no_same_name_field"
            if count == 0:
                missing.append(name)
            elif count != len(instances):
                partial.append(name)
            offset = module.start(1) + match.start()
            line = library.count("\n", 0, offset) + 1
            rows.append([primitive, name, default.strip().strip('"'), len(instances), count, status,
                name in emitted, f"yosys:lattice/cells_bb_ecp5.v:{line}",
                "same_name_lookup_only; missing_does_not_mean_absent_hardware"])
        summary[primitive] = dict(declared_parameters=len(parameters), no_same_name_field=missing,
                                  some_instances=partial, direct_writer_parameter_names=sorted(emitted))
    return rows, summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", type=Path, default=ROOT / "docs/inventory/LFE5U-85F")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--trellis", type=Path, help="Source checkout (TRELLIS_ROOT or parent of ECP5_DB)")
    parser.add_argument("--nextpnr", type=Path, help="Source checkout (NEXTPNR_ROOT)")
    parser.add_argument("--yosys-library", type=Path, default=Path("/opt/homebrew/share/yosys/lattice/cells_bb_ecp5.v"))
    parser.add_argument("--sysdsp-pdf", type=Path, required=True)
    parser.add_argument("--library-pdf", type=Path, required=True)
    args = resolve_toolchain(parser, parser.parse_args())
    inventory, output = args.inventory, args.output or args.inventory / "dsp_research"
    catalog_path, resources_path = inventory / "decode_catalog.json", inventory / "resource_instances.csv"
    catalog = json.loads(catalog_path.read_text())
    if catalog["device"] != DEVICE or catalog["schema_version"] != 1:
        raise ValueError("this collector is scoped to the LFE5U-85F catalogue")
    with resources_path.open() as stream:
        resources = {row["resource_id"]: row for row in csv.DictReader(stream)}
    aliases, overlaps, patterns, groups = collect(catalog, resources)
    parameters, coverage = parameter_coverage(catalog, args.yosys_library, args.nextpnr / "ecp5/bitstream.cc")
    sources = [("project:decode_catalog.json", catalog_path), ("project:resource_instances.csv", resources_path),
               ("project:collect_dsp_evidence.py", Path(__file__)), ("yosys:lattice/cells_bb_ecp5.v", args.yosys_library),
               ("lattice:FPGA-TN-02205-1.3.pdf", args.sysdsp_pdf), ("lattice:Diamond-3.13-FPGA-Libraries.pdf", args.library_pdf)]
    sources += [("trellis:" + name, args.trellis / name) for name in (
        "libtrellis/src/Bels.cpp", "docs/architecture/tiles.rst", "fuzzers/ECP5/070-dsp_routing/fuzzer.py",
        "fuzzers/ECP5/071-dsp_constmux/fuzzer.py", "fuzzers/ECP5/073-mult18_config/fuzzer.py",
        "fuzzers/ECP5/075-alu54b_config/fuzzer.py", "fuzzers/ECP5/076-dsp_cibo/fuzzer.py",
        "fuzzers/ECP5/077-copy_dsp_config/fuzzer.py")]
    sources += [("nextpnr:" + name, args.nextpnr / name) for name in (
        "ecp5/arch_place.cc", "ecp5/bitstream.cc", "ecp5/docs/primitives.md")]
    source_hashes = [dict(source=name, sha256=sha256(path)) for name, path in sources]
    for pdf in (args.sysdsp_pdf, args.library_pdf):
        if not pdf.read_bytes().startswith(b"%PDF-"):
            raise ValueError(f"not a PDF: {pdf}")
    report = dict(schema_version=1, device=DEVICE,
        scope="Configuration encoding identities and overlaps; no inferred physical register ownership or behaviour.",
        dsp_blocks=groups, enum_encoding_alias_classes=len(aliases),
        alias_classes_per_block=dict(sorted(Counter(row[0] for row in aliases).items())),
        alias_member_patterns=[dict(fields=names, occurrences=count) for names, count in sorted(patterns.items())],
        incomplete_domain_alias_classes=sum(not row[2] for row in aliases),
        cross_resource_register_bit_overlaps=len(overlaps),
        declared_parameters=len(parameters), parameter_coverage=coverage,
        documents=[dict(id="FPGA-TN-02205-1.3", url="https://www.latticesemi.com/view_document?document_id=50469",
                        relevant_printed_pages=[6, 7, 8, 9, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25]),
                   dict(id="Diamond-3.13-FPGA-Libraries", url="https://www.latticesemi.com/view_document?document_id=53950",
                        relevant_printed_pages=[186, 187, 188, 189, 197, 198, 199, 200, 201, 202, 203, 587, 588, 589, 590, 591,
                                                625, 626, 627, 628, 629, 745, 746, 747, 748, 749, 750])],
        source_files=source_hashes)
    # All inputs and structural checks above complete before any output write.
    output.mkdir(parents=True, exist_ok=True)
    save_csv(output / "enum_encoding_aliases.csv",
        ["dsp_block", "members", "all_fragment_domains_complete", "combined_option_constraints", "sources", "interpretation"], aliases)
    save_csv(output / "register_bit_overlaps.csv",
        ["dsp_block", "resource_a", "field_a", "resource_b", "field_b", "shared_bit_count", "field_a_bit_count", "field_b_bit_count",
         "identical_encoding", "shared_cram_coordinates"], overlaps)
    save_csv(output / "parameter_coverage.csv",
        ["primitive", "parameter", "declaration_default", "resource_count", "same_name_field_count", "same_name_coverage",
         "direct_writer_call", "declaration_source", "interpretation"], parameters)
    (output / "evidence_report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(packed({key: report[key] for key in ("dsp_blocks", "enum_encoding_alias_classes", "incomplete_domain_alias_classes",
                                               "cross_resource_register_bit_overlaps", "declared_parameters")}))


if __name__ == "__main__":
    main()
