"""Check native MULT18 ROA/ROB to ALU54 A/B wiring; no ALU opcode semantics."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
from toolchain_paths import resolve_toolchain
import re

ROOT = Path(__file__).resolve().parents[1]
INV = ROOT / "docs/inventory/LFE5U-85F"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def rows(path):
    with path.open() as stream:
        return list(csv.DictReader(stream))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trellis", type=Path, help="Source checkout (TRELLIS_ROOT or parent of ECP5_DB)")
    parser.add_argument("--output", type=Path, default=INV / "dsp_forwarding_evidence.json")
    args = resolve_toolchain(parser, parser.parse_args())
    resources = {r["resource_id"]: r for r in rows(INV / "resource_instances.csv")}
    ports = {(r["resource_id"], r["port"]): r for r in rows(INV / "resource_ports.csv")}
    templates, instances, sources = {}, [], set()
    multipliers = set()
    for alu in sorted((r for r in resources.values() if r["family"] == "ALU54"), key=lambda r: r["resource_id"]):
        tile_type = alu["anchor_tile"].split(":")[1]
        relative = f"database/ECP5/tiledata/{tile_type}/bits.db"
        path = args.trellis / relative
        sources.add(relative)
        if tile_type not in templates:
            fixed = []
            for number, line in enumerate(path.read_text().splitlines(), 1):
                match = re.fullmatch(r"\.fixed_conn (J[AB]\d+_ALU54) (W[23]_JRO[AB]\d+_MULT18)", line)
                if match:
                    fixed.append({"sink": match[1], "source": match[2], "line": number})
            assert len(fixed) == 72, (tile_type, len(fixed))
            templates[tile_type] = {"path": relative, "connections": fixed}
        peers = sorted((r for r in resources.values() if r["parent_id"] == alu["parent_id"] and r["family"] == "MULT18"), key=lambda r: int(r["fabric_col"]))
        assert len(peers) == 2
        for side, mult in zip(["A", "B"], peers):
            multipliers.add(mult["resource_id"])
            for bit in range(36):
                bus, n = ("ROA", bit) if bit < 18 else ("ROB", bit - 18)
                source = ports[mult["resource_id"], f"{bus}{n}"]
                sink = ports[alu["resource_id"], f"{side}{bit}"]
                assert source["direction"] == "out" and sink["direction"] == "in"
                assert source["database_symbol_known"] == sink["database_symbol_known"] == "True"
                candidates = [c for c in templates[tile_type]["connections"] if c["sink"] == sink["wire"]]
                assert len(candidates) == 1
                match = re.fullmatch(r"W([23])_(.*)", candidates[0]["source"])
                assert (int(source["wire_row"]), int(source["wire_col"]), source["wire"]) == (int(sink["wire_row"]), int(sink["wire_col"]) - int(match[1]), match[2])
        instances.append({"slice": alu["parent_id"], "alu": alu["resource_id"], "tile_type": tile_type,
                          "A_from": peers[0]["resource_id"], "B_from": peers[1]["resource_id"]})
    assert len(instances) == 78 and len(multipliers) == 156
    result = {
        "schema_version": 1, "device": "LFE5U-85F", "hardware_results": 0,
        "slices": len(instances), "multipliers": len(multipliers), "operand_bits_checked": len(instances) * 72,
        "mapping": {"ALU_A": "first native MULT18: ROA[17:0] then ROB[17:0]", "ALU_B": "second native MULT18: ROA[17:0] then ROB[17:0]"},
        "templates": templates, "instances": instances,
        "limits": [
            "Native inventory and exact Trellis fixed connections establish port wiring, not register behavior",
            "Execution profile: parallel SOURCEA/B=0; ROA/ROB read the optional INPUTA/B stage, before PIPELINE/OUTPUT",
            "D05 data/sign/reset/edge contract still requires a board or vendor behavioral model",
            "MULT9 alternatives, shift/preadder/C forwarding, ALU operations and shared output aliases remain unqualified",
        ],
        "source_files": [{"path": str(p.relative_to(ROOT)), "sha256": digest(p)} for p in [INV / "resource_instances.csv", INV / "resource_ports.csv", Path(__file__).resolve()]],
        "trellis_sources": [{"path": p, "sha256": digest(args.trellis / p)} for p in sorted(sources)],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: result[key] for key in ["slices", "multipliers", "operand_bits_checked", "hardware_results"]}))


if __name__ == "__main__":
    main()
