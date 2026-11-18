"""Retain compact four-state DSP forwarding traces from the native test fixtures."""
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
INV = ROOT / "docs/inventory/LFE5U-85F"
LEVEL = {"zero": "0", "one": "1", "x": "x", "z": "z"}
PORT = re.compile(r"R(\d+)C(\d+):J(RO[AB])(\d+)_MULT18")


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def checksum(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def main():
    profiles, source_runs = [], []
    for name in ["dsp-registers-sync", "dsp-registers-async", "dsp-output"]:
        folder = ROOT / "target" / name
        path = folder / "native.json"
        native = json.loads(path.read_text())
        baseline = native["runs"][0]["samples"]
        assert len(baseline) == 513 and len(native["runs"]) == 3
        assert [(r["source"], r["scope"]) for r in native["runs"]] == [("config", "observed"), ("bit", "observed"), ("bit", "all_plc_registers")]

        def order(label):
            row, col, bus, bit = PORT.fullmatch(label).groups()
            return int(row), int(col), bus, int(bit)

        columns = sorted((key for key in baseline[0]["values"] if PORT.fullmatch(key)), key=order)
        assert len(columns) == 144
        observations = [{"time_ns": s["time_ns"], "bits": "".join(LEVEL[s["values"][key]] for key in columns)} for s in baseline]
        runs = []
        for run in native["runs"]:
            assert run["samples"] == baseline
            runs.append({key: run[key] for key in ["source", "scope", "counts", "state", "statistics", "multipliers"]} | {"full_samples_sha256": canonical(run["samples"]), "forwarding_trace_sha256": canonical(observations)})
        cli_path = folder / "cli.json"
        cli = json.loads(cli_path.read_text())["observations"]
        assert len(cli) == 514
        cli_columns = sorted(cli[0]["values"])
        assert len(cli_columns) == (16 if name == "dsp-output" else 28)
        for sample, actual in zip(baseline, cli[1:]):
            assert actual["values"] == {key: sample["values"][key] for key in cli_columns}
        profiles.append({"fixture": name, "profiles": native["profiles"], "pins": native["pins"], "columns": columns,
                         "bit_order": "columns ordered by native location, ROA then ROB, bit 0 first; each character is 0/1/x/z",
                         "observations": observations, "runs": runs,
                         "cli": {"columns": cli_columns, "observations": ["".join(LEVEL[s["values"][key]] for key in cli_columns) for s in cli]}})
        for file in [path, cli_path, folder / "top.config", folder / "top.bit", folder / "top.v", folder / "top.lpf"]:
            source_runs.append({"path": str(file.relative_to(ROOT)), "sha256": checksum(file)})
    out = INV / "dsp_forwarding_native.json"
    write(out, {"schema_version": 1, "hardware_results": 0, "forwarding_bit_comparisons": 9 * 513 * 144, "profiles": profiles})
    test_logs = {}
    for name, count in [("dsp-forward-tests.log", 106), ("dsp-forward-unit.log", 13), ("dsp-forward-native-regression.log", 12)]:
        path = ROOT / "target" / name
        log = path.read_text()
        assert "FAILED" not in log
        passed = sum(int(n) for n in re.findall(r"test result: ok\. (\d+) passed;", log))
        assert passed == count, (name, passed)
        test_logs[name] = {"passed": passed, "sha256": checksum(path)}
    sources = ["src/kernel/dsp.rs", "src/compile/dsp.rs", "tests/dsp.rs", "tests/dsp_register_native.rs", "tests/dsp_output_native.rs", "tools/collect_dsp_forwarding.py", "tools/summarize_dsp_forwarding.py", "docs/DSP_FORWARDING.md", "docs/inventory/LFE5U-85F/dsp_forwarding_evidence.json", "docs/inventory/LFE5U-85F/dsp_forwarding_native.json"]
    report = {
        "schema_version": 1, "date": "2026-10-23", "hardware_results": 0,
        "scope": "MULT18 parallel ROA/ROB execution and full native fixed wiring to ALU; D01-D10 remain open",
        "native_runs": 9, "samples_per_run": 513, "forwarding_bits_per_sample": 144,
        "forwarding_bit_comparisons": 9 * 513 * 144, "equal_traces_per_fixture": True,
        "cli_runs": 3, "cli_observations": 514, "cli_probe_counts": [28, 28, 16],
        "rust_tests": {"regular": 106, "native_DSP_targeted": 2, "other_native": 12, "total_unique": 120, "expanded_dsp_tests_rechecked": 13, "external_ECP5_TEST_BIT": "not_run"},
        "test_logs": test_logs,
        "source_files": [{"path": p, "sha256": checksum(ROOT / p)} for p in sources],
        "generated_inputs": source_runs,
        "limits": ["software checks do not establish silicon equivalence", "ALU arithmetic and registered SIGNEDP remain unimplemented", "output profile retains CE=1/RST=0/GSR disabled"],
    }
    write(INV / "dsp_forwarding_validation.json", report)
    print(json.dumps({key: report[key] for key in ["native_runs", "forwarding_bit_comparisons", "cli_runs", "cli_probe_counts", "hardware_results"]}))


if __name__ == "__main__":
    main()
