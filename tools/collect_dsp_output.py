#!/usr/bin/env python3
"""Record the DSP output clock overlay and logical MULT bank views, not ALU FF ownership."""
import argparse
import collections
import csv
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INV = ROOT / "docs/inventory/LFE5U-85F"


def checksum(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=INV / "dsp_output_evidence.json")
    args = parser.parse_args()
    catalog_path = INV / "decode_catalog.json"
    resource_path = INV / "resource_instances.csv"
    overlap_path = INV / "dsp_research/register_bit_overlaps.csv"
    catalog = json.loads(catalog_path.read_text())
    assert catalog["device"] == "LFE5U-85F"
    with resource_path.open() as stream:
        resources = {r["resource_id"]: r for r in csv.DictReader(stream)}
    with overlap_path.open() as stream:
        overlaps = list(csv.DictReader(stream))
    bank_products = collections.defaultdict(list)
    records = []
    shapes = collections.Counter()
    for resource in catalog["resources"]:
        if resource["family"] != "MULT18":
            continue
        fields = {f["name"].split(".", 1)[1]: f for f in resource["fields"]}
        bypass, clock = fields["CIBOUT_BYP"], fields["REG_OUTPUT_CLK"]

        def options(field, value):
            return {tuple(bit) for fragment in field["fragments"] for bit in fragment["options"][value]}

        assert all(set(f["options"]) == {"ON", "OFF"} for f in bypass["fragments"])
        assert all(set(f["options"]) == {"NONE", "CLK0", "CLK1", "CLK2", "CLK3"} for f in clock["fragments"])
        off, on = options(bypass, "OFF"), options(bypass, "ON")
        clock_coordinates = {(f, b) for key in ("NONE", "CLK0", "CLK1", "CLK2", "CLK3") for f, b, _ in options(clock, key)}
        assert off == options(clock, "CLK0")
        assert len(off) in (8, 12) and all(v for _, _, v in off)
        assert len(on) == 2 and all(v and (f, b) not in clock_coordinates for f, b, v in on)
        shape = (len(bypass["fragments"]), len(clock["fragments"]), len(off))
        shapes[shape] += 1
        native = resources[resource["id"]]
        owner = native["parent_id"]
        assert resources[owner]["family"] == "DSP_SLICE"
        bank_products[owner].append(native)
        records.append({
            "resource": resource["id"], "slice": owner,
            "bypass_selector_bits": sorted(on), "off_clock0_overlay_bits": sorted(off),
            "bypass_fragments": len(bypass["fragments"]), "output_clock_fragments": len(clock["fragments"]),
            "output_clock_field": "REG_OUTPUT_CLK" in fields,
            "output_ce_field": "REG_OUTPUT_CE" in fields,
            "output_reset_field": "REG_OUTPUT_RST" in fields,
        })
    assert len(records) == 156 and len(bank_products) == 78
    assert shapes == {(3, 2, 8): 78, (3, 3, 12): 39, (4, 3, 12): 39}
    assert not any(r["output_ce_field"] for r in records)
    assert sum(r["output_reset_field"] for r in records) == 78
    banks = []
    for owner, products in sorted(bank_products.items()):
        products.sort(key=lambda r: int(r["fabric_col"]))
        assert len(products) == 2
        assert [r["name"] for r in products] in (["MULT18_0", "MULT18_1"], ["MULT18_4", "MULT18_5"])
        banks.append({"slice": owner, "width": 72, "logical_product_views": [
            {"resource": r["resource_id"], "logical_bit_start": i * 36, "width": 36}
            for i, r in enumerate(products)
        ], "ALU_flag_framing_mapping": "unassigned_D01"})
    sources = [catalog_path, resource_path, overlap_path, INV / "dsp_research/evidence_report.json", Path(__file__).resolve()]
    result = {
        "schema_version": 1, "device": "LFE5U-85F",
        "scope": "exact CIBOUT_BYP OFF=CLK0 encoding identity and two logical MULT product views; no physical FF or ALU aliases assigned",
        "slices": 78, "multipliers": 156, "shared_bank_bits": 5616,
        "selector_bits": 312, "output_reset_fields": 78, "output_ce_fields": 0,
        "shape_counts": [{"bypass_fragments": b, "clock_fragments": c, "overlay_bits": n, "instances": count} for (b, c, n), count in sorted(shapes.items())],
        "instances": sorted(records, key=lambda r: r["resource"]), "bank_views": banks,
        "output_field_overlaps": [r for r in overlaps if ".REG_OUTPUT" in str(r) or ".REG_FLAG" in str(r)],
        "evidence_limits": [
            "Shared 72-bit capacity and two 36-bit products: pinned sysDSP guide Figure 2.3; index order is the emulator's logical coordinate system",
            "No assertion that logical bit indices are silicon FF coordinates or that ALU/flag bits use an inferred placement",
            "Missing output CE and partial reset encodings require all effective CE inputs high, all effective RST inputs low and GSR disabled in the executable profile",
            "Registered SIGNEDP, highspeed, framing, other widths and arbitrary Diamond bitstreams remain unqualified",
        ],
        "source_files": [{"path": str(p.relative_to(ROOT)), "sha256": checksum(p)} for p in sources],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: result[k] for k in ("slices", "multipliers", "shared_bank_bits", "selector_bits", "output_reset_fields", "output_ce_fields")}))


if __name__ == "__main__":
    main()
