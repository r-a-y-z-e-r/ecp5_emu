"""Measure the OSCG divider of every OSC.DIV bit pattern in the Trellis database.
Each pattern is loaded as hc06_2.config with only the two OSC.DIV lines changed.
Usage: sweep.py PATTERNS.json OUT.json"""
import json, re, subprocess, sys, time, statistics
import os

serial_port = os.environ.get("ECP5_SERIAL_PORT")
if not serial_port:
    raise SystemExit("Set ECP5_SERIAL_PORT to the board UART device before running this script.")

import serial
groups = json.load(open(sys.argv[1]))
base = open("hc06_2.config").read()
assert base.count("enum: OSC.DIV 2\n") == 2
pattern = re.compile(r"E=([0-9A-F]{6}) S=([0-9A-F]{6}) W=([0-9A-F]{4}) F=([0-9A-F]{6})")
def measure(value):
    open("sweep.config", "w").write(base.replace("enum: OSC.DIV 2\n", f"enum: OSC.DIV {value}\n"))
    subprocess.run(["ecppack", "--compress", "sweep.config", "sweep.bit"], check=True, capture_output=True)
    load = subprocess.run(["openFPGALoader", "-b", "ulx3s", "sweep.bit"], capture_output=True, text=True)
    assert "DONE" in load.stdout + load.stderr
    with serial.Serial(serial_port, 115200, timeout=0.2) as port:
        port.reset_input_buffer()
        end, buf = time.time() + 2.5, b""
        while time.time() < end:
            buf += port.read(4096)
    windows = {}
    for m in map(pattern.search, buf.decode("ascii", "replace").splitlines()):
        if m:
            windows[int(m.group(3), 16)] = int(m.group(4), 16)
    f = list(windows.values())
    return {"windows": len(f), "mean": statistics.mean(f), "min": min(f), "max": max(f)}
results = {"reference_before": measure(2), "patterns": []}
for g in groups:
    r = measure(g[0])
    r["values"] = g
    results["patterns"].append(r)
    print(g, r, flush=True)
results["reference_after"] = measure(2)
json.dump(results, open(sys.argv[2], "w"), indent=1)
print("done", flush=True)
