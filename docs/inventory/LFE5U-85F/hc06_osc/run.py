"""Load an OSCG bitstream into ULX3S SRAM and collect its window counts.
Usage: run.py BITSTREAM LOADS SECONDS"""
import re, subprocess, sys, time, datetime, statistics
import os

serial_port = os.environ.get("ECP5_SERIAL_PORT")
if not serial_port:
    raise SystemExit("Set ECP5_SERIAL_PORT to the board UART device before running this script.")

import serial
bit, loads, seconds = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
pattern = re.compile(r"E=([0-9A-F]{6}) S=([0-9A-F]{6}) W=([0-9A-F]{4}) F=([0-9A-F]{6})")
for n in range(1, loads + 1):
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    load = subprocess.run(["openFPGALoader", "-b", "ulx3s", bit], capture_output=True, text=True)
    ok = "DONE" in load.stdout + load.stderr
    with serial.Serial(serial_port, 115200, timeout=0.2) as port:
        port.reset_input_buffer()
        end = time.time() + seconds
        buf = b""
        while time.time() < end:
            buf += port.read(4096)
    rows = [tuple(int(x, 16) for x in m.groups()) for m in map(pattern.search, buf.decode("ascii", "replace").splitlines()) if m]
    es, ss = sorted({r[0] for r in rows}), sorted({r[1] for r in rows})
    windows = {r[2]: r[3] for r in rows}
    f = list(windows.values())
    if not f:
        print(f"load {n} at {stamp}: loader_done={ok} no report lines ({len(buf)} bytes)", flush=True)
        continue
    print(f"load {n} at {stamp}: loader_done={ok} lines={len(rows)} windows={len(f)} E={es} S={ss}"
          f" F min={min(f)} max={max(f)} mean={statistics.mean(f):.2f} sd={statistics.pstdev(f):.2f}"
          f" first={sorted(windows.items())[:3]}", flush=True)
