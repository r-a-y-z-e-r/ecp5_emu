"""Load a bitstream into ULX3S SRAM and collect UART report lines.
Usage: run.py BITSTREAM LOADS SECONDS"""
import re, subprocess, sys, time, datetime
import os

serial_port = os.environ.get("ECP5_SERIAL_PORT")
if not serial_port:
    raise SystemExit("Set ECP5_SERIAL_PORT to the board UART device before running this script.")

import serial
bit, loads, seconds = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
pattern = re.compile(r"P=[0-9A-F]{128}")
for n in range(1, loads + 1):
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    load = subprocess.run(["openFPGALoader", "-b", "ulx3s", bit], capture_output=True, text=True)
    ok = "DONE" in load.stdout + load.stderr
    with serial.Serial(serial_port, 115200, timeout=0.2) as port:
        port.reset_input_buffer()
        end = time.time() + seconds
        buf = b""
        while time.time() < end:
            buf += port.read(256)
    lines = buf.decode("ascii", "replace").splitlines()
    parsed = [m.group(0) for m in map(pattern.search, lines) if m]
    print(f"load {n} at {stamp}: loader_done={ok} lines={len(parsed)}", flush=True)
    for line in sorted(set(parsed)):
        print(f"  {line}", flush=True)
