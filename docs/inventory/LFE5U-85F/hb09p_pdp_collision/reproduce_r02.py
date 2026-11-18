"""Build exact X03; diagnose current event/fast behavior without board access.

python3 reproduce_r02.py /tmp/r02-replay [--emulator /absolute/path/ecp5_emu]
The measured kernel returns NEW. Fast may still refuse until its owner applies hunk A.
"""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import re
import subprocess

from check_inference import build

ROOT = Path(__file__).resolve().parents[4]


def values_at(path):
    """VCD snapshots by time, keyed by displayed HDL names, independent of IDs."""
    ids, values, rows, time = {}, {}, {}, 0
    for line in path.read_text().splitlines():
        parts = line.split()
        if line.startswith("$var"):
            ids[parts[3]] = parts[4].lstrip("\\")
        elif line.startswith("#"):
            rows[time] = dict(values)
            time = int(line[1:])
        elif line and line[0] in "01xz" and line[1:] in ids:
            name = ids[line[1:]]
            if name.startswith("q["):
                values[name] = line[0]
    rows[time] = dict(values)
    return rows


def run(scratch, emulator):
    mapped = build(scratch, 32, 512)
    stem = scratch / "ram_32_512"
    ports = json.loads(stem.with_suffix(".json").read_text())["modules"]["top"]["ports"]
    with (ROOT / "docs/inventory/LFE5U-85F/package_pins.csv").open() as stream:
        pins = iter(r["pin"] for r in csv.DictReader(stream) if r["package"] == "CABGA381" and r["pin"] != "G2")
        lpf = ['LOCATE COMP "clk" SITE "G2";', 'FREQUENCY PORT "clk" 25 MHZ;']
        for name, port in ports.items():
            for i in range(len(port["bits"])):
                bit = name if len(port["bits"]) == 1 else f"{name}[{i}]"
                if name != "clk":
                    lpf.append(f'LOCATE COMP "{bit}" SITE "{next(pins)}";')
                lpf.append(f'IOBUF PORT "{bit}" IO_TYPE=LVCMOS33 PULLMODE=NONE;')
    stem.with_suffix(".lpf").write_text("\n".join(lpf) + "\n")
    routed = scratch / "x03_routed.json"
    config = scratch / "x03.config"
    with (scratch / "nextpnr.log").open("w") as log:
        subprocess.run(["nextpnr-ecp5", "--85k", "--package", "CABGA381", "--json", str(stem.with_suffix(".json")),
                        "--lpf", str(stem.with_suffix(".lpf")), "--write", str(routed),
                        "--textcfg", str(config), "--seed", "1"], stdout=log, stderr=subprocess.STDOUT, check=True)
    module = json.loads(routed.read_text())["modules"]["top"]
    cell = next(c for c in module["cells"].values() if c["type"] == "DP16KD")
    col, row = re.match(r"X(\d+)/Y(\d+)/", cell["attributes"]["NEXTPNR_BEL"]).groups()
    loc = f"R{row}C{col}"
    base = [str(emulator), "emulate", str(config), "--names", str(routed), "--startup", "configured",
            "--scope", "observed", "--clock", "net:clk", "--period-ns", "40", "--probe", "net:q"]
    for bus, width, value in (("a", 9, 0), ("d", 32, 1)):
        for i in range(width):
            base += ["--set", f"net:{bus}[{i}]={(value >> i) & 1}"]
    results = {}
    for we, fast, cycles in ((0, False, 5), (0, True, 5), (1, False, 5), (1, True, 1), (1, True, 2)):
        label = f"we{we}_{'fast' if fast else 'event'}_{cycles}"
        args = base + ["--set", f"net:we={we}", "--cycles", str(cycles), "--vcd", str(scratch / f"{label}.vcd")]
        if fast:
            args += ["--fast"]
        else:
            args += ["--output", str(scratch / f"{label}.json")]
            for side in "AB":
                for bit in range(18):
                    args += ["--probe", f"{loc}:JDO{side}{bit}_EBR"]
        process = subprocess.run(args, capture_output=True, text=True)
        (scratch / f"{label}.log").write_text(process.stdout + process.stderr)
        results[label] = {"exit": process.returncode, "error": process.stderr.strip()}
        if fast and we == 1 and cycles == 2:
            assert process.returncode == 0 or "logic reads an output of EBR" in process.stderr
        else:
            assert process.returncode == 0, label
    event = json.loads((scratch / "we1_event_5.json").read_text())
    assert event["ebr_notices"] == [], "measured PDP collisions must resolve without an X notice"
    observed = next(o["values"] for o in event["observations"] if o["time_ns"] == 60)
    assert all(observed[f"{loc}:JDO{s}{i}_EBR"] == ("one" if s == "A" and i == 0 else "zero")
               for s, width in (("A", 18), ("B", 14)) for i in range(width))
    assert all(observed[f"q[{i}]"] == ("one" if i == 0 else "zero") for i in range(32))
    # Compare both executors' entire stable WE=0 output trace, including all
    # intermediate sampled times emitted by either VCD writer.
    traces = [values_at(scratch / f"we0_{kind}_5.vcd") for kind in ("event", "fast")]
    for time in sorted(set(traces[0]) | set(traces[1])):
        snapshots = [trace[max(t for t in trace if t <= time)] for trace in traces]
        assert snapshots[0] == snapshots[1] and len(snapshots[0]) == 32, time
    summary = {"stimulus": "configured; clk 40 ns; A=0; D=1; WE=0/1", "mapping": mapped,
               "config_sha256": hashlib.sha256(config.read_bytes()).hexdigest(),
               "ebr_location": loc, "resolved_rows9": [0, 1, 2, 3], "conservative_collision_notices": 0,
               "at_60_ns": {"DOA[17:0]": "0x00001", "DOB[13:0]": "0x0000", "q[31:0]": "0x00000001"},
               "idle_equal_trace": True, "executors": results}
    (scratch / "reproduction_after.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scratch", type=Path)
    parser.add_argument("--emulator", type=Path, default=ROOT / "target/release/ecp5_emu")
    args = parser.parse_args()
    args.scratch.mkdir(parents=True, exist_ok=True)
    if any(c.isspace() for c in str(args.scratch)):
        parser.error("use a scratch path without whitespace")
    run(args.scratch, args.emulator.resolve())
