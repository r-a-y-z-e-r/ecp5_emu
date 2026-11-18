"""Retain operand-only traces from actual two-ALU native configurations."""
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
INV = ROOT / "docs/inventory/LFE5U-85F"
LEVEL = {"zero": "0", "one": "1", "x": "x", "z": "z"}
PORT = re.compile(r"R(\d+)C(\d+):J(RO[AB])(\d+)_MULT18")


def checksum(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def main():
    profiles, inputs = [], []
    for mode in ["sync", "async"]:
        folder = ROOT / "target" / f"dsp-alu-operands-{mode}"
        native = json.loads((folder / "native.json").read_text())
        assert native["hardware_results"] == 0 and len(native["runs"]) == 2
        baseline = native["runs"][0]["samples"]
        assert len(baseline) == 257

        def order(label):
            row, col, bus, bit = PORT.fullmatch(label).groups()
            return int(row), int(col), bus, int(bit)

        columns = sorted(baseline[0]["values"], key=order)
        assert len(columns) == 144
        observations = [{"time_ns": s["time_ns"], "bits": "".join(LEVEL[s["values"][key]] for key in columns)} for s in baseline]
        runs = []
        for run in native["runs"]:
            assert run["samples"] == baseline and run["state"]["known_dsp_shared_output_bits"] == 0
            assert len(run["multipliers"]) == 4 and all(m["execution_view"] == "operands_only" and m["output_register"] is None for m in run["multipliers"])
            assert len(run["alu_settings"]) == 2 and all(a["enabled"] for a in run["alu_settings"])
            runs.append({key: run[key] for key in ["source", "counts", "multipliers", "statistics", "state", "alu_settings"]} | {"full_samples_sha256": canonical(run["samples"])})
        cli = json.loads((folder / "cli.json").read_text())["observations"]
        cli_columns = sorted(cli[0]["values"])
        assert len(cli_columns) == 16 and len(cli) == 258
        for s, actual in zip(baseline, cli[1:]):
            assert actual["values"] == {key: s["values"][key] for key in cli_columns}
        profiles.append({"mode": mode, "pins": native["pins"], "profiles": native["profiles"], "columns": columns,
                         "bit_order": "columns ordered by location, ROA then ROB, bit 0 first; 0/1/x/z",
                         "observations": observations, "runs": runs,
                         "cli": {"columns": cli_columns, "observations": ["".join(LEVEL[s["values"][key]] for key in cli_columns) for s in cli]}})
        (INV / f"dsp_alu_inputs_{mode}_fixture.v").write_bytes((folder / "top.v").read_bytes())
        for name in ["native.json", "cli.json", "top.bit", "top.config", "top.v", "top.lpf", "events.json"]:
            p = folder / name
            inputs.append({"path": str(p.relative_to(ROOT)), "sha256": checksum(p)})
    assert (ROOT / "target/dsp-alu-operands-sync/top.lpf").read_bytes() == (ROOT / "target/dsp-alu-operands-async/top.lpf").read_bytes()
    (INV / "dsp_alu_inputs_fixture.lpf").write_bytes((ROOT / "target/dsp-alu-operands-sync/top.lpf").read_bytes())
    write(INV / "dsp_alu_inputs_native.json", {"schema_version": 1, "hardware_results": 0, "forwarding_bit_comparisons": 4 * 257 * 144, "profiles": profiles})
    logs = {}
    for name, count in [("dsp-alu-operands-tests.log", 107), ("dsp-alu-operands-native.log", 1), ("dsp-alu-operands-regression.log", 14)]:
        p = ROOT / "target" / name
        s = p.read_text()
        assert "FAILED" not in s
        passed = sum(map(int, re.findall(r"test result: ok\. (\d+) passed;", s)))
        assert passed == count, (name, passed)
        logs[name] = {"passed": passed, "sha256": checksum(p)}
    source_files = ["src/kernel/dsp.rs", "src/kernel/mod.rs", "src/compile/dsp.rs", "tests/dsp.rs", "tests/dsp_alu_operands_native.rs", "tools/summarize_dsp_alu_inputs.py", "docs/DSP_ALU_INPUTS.md", "PLAN.md", "docs/IMPLEMENTATION_PLAN.md", "docs/RESOURCE_COVERAGE.md", "docs/RESEARCH_STATUS.md", "docs/DSP_FORWARDING.md", "docs/HARDWARE_VALIDATION.md", "docs/DEVICE_PASSPORT.md"]
    artifacts = ["dsp_alu_inputs_native.json", "dsp_alu_inputs_sync_fixture.v", "dsp_alu_inputs_async_fixture.v", "dsp_alu_inputs_fixture.lpf"]
    report = {
        "schema_version": 1, "date": "2026-10-23", "hardware_results": 0,
        "scope": "limited MULT INPUTA/B operand observation with active ALU54; no product/ALU execution",
        "user_board": {"model": "ULX3S", "source": "user_reply_2026-10-23", "revision": None, "FPGA_marking_and_package": "not_verified"},
        "native_runs": 4, "samples_per_run": 257, "forwarding_bits_per_sample": 144,
        "forwarding_bit_comparisons": 4 * 257 * 144, "equal_traces_per_mode": True,
        "cli_runs": 2, "cli_observations": 258, "cli_probes": 16,
        "rust_tests": {"regular": 107, "native_new": 1, "native_regression": 14, "total_unique": 122, "external_ECP5_TEST_BIT": "not_run"},
        "test_logs": logs, "unresolved_questions": 78, "prepared_hardware_series": 37,
        "source_files": [{"path": p, "sha256": checksum(ROOT / p)} for p in source_files],
        "artifacts": [{"path": str((INV / p).relative_to(ROOT)), "sha256": checksum(INV / p)} for p in artifacts],
        "generated_inputs": inputs,
        "limits": ["ALU OP table and arithmetic remain D04", "no P/SIGNEDP or ALU result/flag drivers", "all-plc with active ALU rejected", "D05 input register contract is not silicon validation", "fixture leaves unused dedicated CIN/CFB/SIGNEDCIN unconnected; no ALU result oracle"],
    }
    write(INV / "dsp_alu_inputs_validation.json", report)
    print(json.dumps({key: report[key] for key in ["native_runs", "forwarding_bit_comparisons", "cli_runs", "hardware_results"]}))


if __name__ == "__main__":
    main()
